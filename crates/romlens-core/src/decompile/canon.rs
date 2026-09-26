//! The `full` level's last step before structuring: the CPU's registers as
//! C variables, and calls with their arguments and results.
//!
//! - **Calls.** A call to a routine with a signature (`signature::Abi`)
//!   passes what it reads and takes what it returns: `a = SUB_8123(x, &y);`.
//!   A tail call is a call and a return. Code that reads the registers
//!   themselves (a call through a table, `mvn`, a `PHP` nothing pairs, a
//!   call nothing analysed) gets them through the globals of `snes.h`:
//!   `X = x;` before it and `x = X;` after, for what it reads and what is
//!   read after it.
//! - **Variables.** Each register's definitions and uses are joined into
//!   webs (every definition that reaches a use is in that use's web). A web
//!   is `u8` where every definition is an 8-bit value or every use reads
//!   only the low byte, else `u16`; a flag's is `bool`. The webs of one
//!   register and type share one variable, since one register never holds
//!   two of them at once: `x`, or `x8` and `x16` where it is both.
//! - **Parameters and results.** The entry's web is the parameter; each
//!   return returns the first result and stores the rest through pointers.
//!
//! C's own conversions do the truncation the widths did: storing into a
//! `u8` keeps the low byte, and reading one gives a value below 256.

use std::collections::{BTreeMap, BTreeSet};

use crate::decompile::cfg::{Cfg, Term};
use crate::decompile::dataflow::{self, Conventions, Flow, Loc, LocSet, narrow};
use crate::decompile::function::Function;
use crate::decompile::ir::{
    BinOp, CType, CallTarget, ExitValue, Expr, Line, Place, Reg, Slot, Stmt, UnOp, VarDecl, Width,
};
use crate::decompile::lift::Lifted;
use crate::decompile::signature::{Abi, Program};
use crate::rom::image::RomImage;

fn slot_of_loc(l: Loc) -> Option<Slot> {
    match l {
        Loc::Al | Loc::Ah => Some(Slot::A),
        Loc::X => Some(Slot::X),
        Loc::Y => Some(Slot::Y),
        Loc::C => Some(Slot::C),
        Loc::N => Some(Slot::N),
        Loc::V => Some(Slot::V),
        Loc::Z => Some(Slot::Z),
        _ => None,
    }
}

fn slots(set: &LocSet) -> BTreeSet<Slot> {
    set.iter().filter_map(|l| slot_of_loc(*l)).collect()
}

fn slot_index(k: Slot) -> usize {
    Slot::ALL.iter().position(|s| *s == k).unwrap()
}

/// A call to `t` with its arguments and results. Where it returns only
/// A's low byte, does not write the high byte, and the high byte is read
/// after, the result goes through a temporary and into the low byte:
/// `t5 = SUB_8123(); a = (a & 0xFF00) | t5;`.
fn invoke(
    t: crate::memory::address::SnesAddress,
    program: &Program,
    high_read_after: bool,
    temps: &mut u32,
    stack: Option<Vec<Expr>>,
) -> Vec<Stmt> {
    let abi = &program.abis[&t];
    // The arguments its caller pushed: the temporaries that hold them, or
    // where they are on the stack when the pushes stayed pushes.
    let stack = stack.unwrap_or_else(|| {
        abi.stack
            .iter()
            .map(|&(_, m, ty)| {
                Expr::mem(
                    Expr::sum(Expr::Reg(Reg::S, Width::W16), Expr::Const(m)),
                    if ty == CType::U8 {
                        Width::W8
                    } else {
                        Width::W16
                    },
                )
            })
            .collect()
    });
    let keeps_high = abi.ret == Some((Slot::A, CType::U8))
        && high_read_after
        && program
            .summaries
            .get(&t)
            .is_some_and(|s| !s.writes.contains(&Loc::Ah));
    let mut ret = abi.ret.map(|(k, _)| k.place());
    let mut then = Vec::new();
    if keeps_high {
        *temps += 1;
        ret = Some(Place::Temp(*temps));
        then.push(Stmt::Assign {
            dst: Place::Reg(Reg::A, Width::W8),
            value: Expr::Temp(*temps),
        });
    }
    let mut out = vec![Stmt::Invoke {
        target: t,
        args: abi
            .params
            .iter()
            .map(|(k, _)| k.expr())
            .chain(stack)
            .collect(),
        ret,
        outs: abi.outs.iter().map(|(k, _)| k.place()).collect(),
    }];
    out.extend(then);
    out
}

fn to_global(k: Slot) -> Stmt {
    Stmt::Assign {
        dst: Place::Global(k),
        value: k.expr(),
    }
}

fn from_global(k: Slot) -> Stmt {
    Stmt::Assign {
        dst: k.place(),
        value: Expr::Global(k),
    }
}

fn contains_bcd(s: &Stmt) -> bool {
    let mut hit = false;
    if let Stmt::Assign { value, .. } = s {
        value.walk(&mut |e| {
            if matches!(e, Expr::Call("bcd_add" | "bcd_sub", _)) {
                hit = true;
            }
        });
    }
    hit
}

/// Statements that read or write the registers without saying so in an
/// expression: they get them through the globals.
fn reads_globals(s: &Stmt) -> bool {
    matches!(
        s,
        Stmt::Call(_)
            | Stmt::Asm { .. }
            | Stmt::Effect(
                "PHP" | "PLP" | "mvn" | "mvp" | "mvn8" | "mvp8" | "BRK" | "COP",
                _
            )
    ) || contains_bcd(s)
}

/// Whether a value fits in 8 bits, whatever the registers hold.
fn fits8(e: &Expr) -> bool {
    match e {
        Expr::Const(v) => *v <= 0xFF,
        Expr::Cast(Width::W8, _)
        | Expr::Reg(_, Width::W8)
        | Expr::Flag(_)
        | Expr::Mem {
            width: Width::W8, ..
        } => true,
        Expr::Call("pull8", _) => true,
        Expr::Bin(BinOp::And, a, b) => fits8(a) || fits8(b),
        Expr::Bin(BinOp::Or | BinOp::Xor, a, b) => fits8(a) && fits8(b),
        Expr::Bin(BinOp::Shr, a, _) => fits8(a),
        e => e.is_boolean(),
    }
}

/// One register access the webs are built from.
enum Ev<'e> {
    /// A read of the register (`low`: only its low byte matters).
    Use {
        slot: Slot,
        low: bool,
        e: &'e mut Expr,
    },
    /// A write of all of it. `pin`: a result through a pointer, whose
    /// variable must have exactly its type.
    Def {
        slot: Slot,
        fits8: bool,
        pin: Option<CType>,
        /// A 16-bit result that comes back into 8-bit index registers: cut
        /// to 8 bits where its variable is wider.
        clip: bool,
        p: &'e mut Place,
        v: Option<&'e mut Expr>,
    },
    /// The statement the next accesses are in.
    At(Pos),
    /// An 8-bit write of the accumulator that keeps its high byte: a read
    /// and a write.
    Partial { p: &'e mut Place, v: &'e mut Expr },
}

/// Where a statement is in its block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Pos {
    Line(usize),
    Before(usize),
    After(usize),
    End,
}

/// The context a statement's accesses need: the widths at its instruction
/// and the signatures of the routines it calls.
struct Ctx<'a> {
    abis: &'a BTreeMap<crate::memory::address::SnesAddress, Abi>,
}

fn walk_expr(e: &mut Expr, low: bool, ev: &mut dyn FnMut(Ev)) {
    match e {
        Expr::Reg(r, w) if Slot::of_reg(*r).is_some() => {
            let slot = Slot::of_reg(*r).unwrap();
            let low = low || *w == Width::W8;
            ev(Ev::Use { slot, low, e })
        }
        Expr::Flag(f) => {
            let slot = Slot::of_flag(*f);
            ev(Ev::Use { slot, low: true, e })
        }
        Expr::Mem { addr, .. } => walk_expr(addr, false, ev),
        Expr::Un(_, x) => walk_expr(x, false, ev),
        Expr::Signed(w, x) => walk_expr(x, *w == Width::W8, ev),
        Expr::Cast(w, x) => walk_expr(x, *w == Width::W8, ev),
        Expr::Bin(op, a, b) => {
            // The low byte of a sum, a difference, a bitwise operation or a
            // left shift depends only on the low bytes of its operands.
            let keeps = low
                && matches!(
                    op,
                    BinOp::Add | BinOp::Sub | BinOp::And | BinOp::Or | BinOp::Xor
                );
            let mask8 = *op == BinOp::And
                && (a.as_const().is_some_and(|m| m <= 0xFF)
                    || b.as_const().is_some_and(|m| m <= 0xFF));
            let left = keeps || mask8 || (low && *op == BinOp::Shl);
            walk_expr(a, left, ev);
            walk_expr(b, keeps || mask8, ev);
        }
        Expr::Call(_, args) => args.iter_mut().for_each(|a| walk_expr(a, false, ev)),
        _ => {}
    }
}

fn walk_stmt(s: &mut Stmt, x8: bool, ctx: &Ctx, ev: &mut dyn FnMut(Ev)) {
    match s {
        Stmt::Assign { dst, value } => {
            let low = matches!(
                dst,
                Place::Reg(Reg::A, Width::W8)
                    | Place::Mem {
                        width: Width::W8,
                        ..
                    }
            );
            walk_expr(value, low, ev);
            if let Place::Mem { addr, .. } = dst {
                walk_expr(addr, false, ev);
            }
            match dst {
                Place::Reg(Reg::A, Width::W8) => ev(Ev::Partial { p: dst, v: value }),
                Place::Reg(r, _) if Slot::of_reg(*r).is_some() => {
                    let slot = Slot::of_reg(*r).unwrap();
                    // In 8-bit index mode the hardware keeps X's and Y's
                    // high bytes zero.
                    let fits = fits8(value) || (slot != Slot::A && x8);
                    ev(Ev::Def {
                        slot,
                        fits8: fits,
                        pin: None,
                        clip: false,
                        p: dst,
                        v: Some(value),
                    })
                }
                Place::Flag(f) => {
                    let slot = Slot::of_flag(*f);
                    ev(Ev::Def {
                        slot,
                        fits8: true,
                        pin: None,
                        clip: false,
                        p: dst,
                        v: Some(value),
                    })
                }
                _ => {}
            }
        }
        Stmt::Invoke {
            target,
            args,
            ret,
            outs,
        } => {
            let abi = &ctx.abis[target];
            for (a, (_, t)) in args.iter_mut().zip(&abi.params) {
                walk_expr(a, *t != CType::U16, ev);
            }
            // Going on with 8-bit index registers, X and Y come back with
            // clear high bytes, whatever the callee left (as the listing
            // assumes).
            let narrow = |k: Slot, t: CType| t != CType::U16 || (x8 && k != Slot::A);
            if let (Some(p @ (Place::Reg(..) | Place::Flag(_))), Some((k, t))) = (ret, abi.ret) {
                ev(Ev::Def {
                    slot: k,
                    fits8: narrow(k, t),
                    pin: None,
                    clip: t == CType::U16 && x8 && k != Slot::A,
                    p,
                    v: None,
                })
            }
            for (p, (k, t)) in outs.iter_mut().zip(&abi.outs) {
                ev(Ev::Def {
                    slot: *k,
                    fits8: narrow(*k, *t),
                    pin: Some(*t),
                    clip: *t == CType::U16 && x8 && *k != Slot::A,
                    p,
                    v: None,
                })
            }
        }
        Stmt::Effect(_, args) => args.iter_mut().for_each(|a| walk_expr(a, false, ev)),
        Stmt::Eval(e) => walk_expr(e, false, ev),
        Stmt::Call(CallTarget::Table { index, .. }) => walk_expr(index, false, ev),
        _ => {}
    }
}

/// Every access in block `b`, in order.
fn walk_block(
    f: &Function,
    lifted: &mut Lifted,
    b: usize,
    own: &Abi,
    ctx: &Ctx,
    ev: &mut dyn FnMut(Ev),
) {
    let lb = &mut lifted.blocks[b];
    for (i, line) in lb.lines.iter_mut().enumerate() {
        let x8 = f.steps[line.step].insn.flags_before.eff_x();
        ev(Ev::At(Pos::Line(i)));
        walk_stmt(&mut line.stmt, x8, ctx, ev);
    }
    ev(Ev::At(Pos::End));
    if let Some(c) = &mut lb.cond {
        walk_expr(c, false, ev);
    }
    if let Some((s, _)) = &mut lb.switch {
        walk_expr(s, false, ev);
    }
    if let Some(x) = &mut lb.exit {
        let x8 = lb
            .term_step
            .map(|i| f.steps[i].insn.flags_after.eff_x())
            .unwrap_or(false);
        for (j, s) in x.before.iter_mut().enumerate() {
            ev(Ev::At(Pos::Before(j)));
            walk_stmt(s, x8, ctx, ev);
        }
        for (j, s) in x.after.iter_mut().enumerate() {
            ev(Ev::At(Pos::After(j)));
            walk_stmt(s, x8, ctx, ev);
        }
        ev(Ev::At(Pos::End));
        for ((_, e), (_, t)) in x.outs.iter_mut().zip(&own.outs) {
            walk_expr(e, *t != CType::U16, ev);
        }
        if let (Some(e), Some((_, t))) = (&mut x.ret, own.ret) {
            walk_expr(e, t != CType::U16, ev);
        }
    }
}

struct Webs {
    parent: Vec<usize>,
}

impl Webs {
    fn find(&mut self, x: usize) -> usize {
        let mut r = x;
        while self.parent[r] != r {
            r = self.parent[r];
        }
        let mut y = x;
        while self.parent[y] != r {
            let n = self.parent[y];
            self.parent[y] = r;
            y = n;
        }
        r
    }

    fn union(&mut self, a: usize, b: usize) {
        let (x, y) = (self.find(a), self.find(b));
        if x != y {
            self.parent[x.max(y)] = x.min(y);
        }
    }
}

type State = [BTreeSet<usize>; 7];

fn entry_state() -> State {
    std::array::from_fn(|i| [i].into_iter().collect())
}

/// Make `lifted` the `full` level's C: see the module comment. `conv` is
/// what the clean-up used.
pub fn canonicalize(
    rom: &RomImage,
    f: &Function,
    cfg: &Cfg,
    lifted: &mut Lifted,
    program: &Program,
    conv: &Conventions,
) {
    let Some(own) = program.abis.get(&f.entry).cloned() else {
        return;
    };
    dataflow::wide_adds(lifted);
    let ctx = Ctx {
        abis: &program.abis,
    };
    let flow = Flow::new(rom, conv);
    let live_out = dataflow::liveness(&flow, cfg, lifted);
    let n = cfg.blocks.len();

    // Calls with their arguments, the globals where code reads them, and
    // the ways out.
    let own_results = |x: &mut ExitValue| {
        x.outs = own.outs.iter().map(|(k, _)| (*k, k.expr())).collect();
        x.ret = own.ret.map(|(k, _)| k.expr());
    };
    for b in 0..n {
        let after = dataflow::live_after_lines(&flow, cfg, lifted, &live_out, b);
        let old = std::mem::take(&mut lifted.blocks[b].lines);
        let mut out: Vec<Line> = Vec::with_capacity(old.len());
        // A call's pushed arguments, from the line before it.
        let mut stack_args: Option<Vec<Expr>> = None;
        for (i, line) in old.into_iter().enumerate() {
            let step = line.step;
            if let Stmt::Effect("stack_args", args) = &line.stmt {
                stack_args = Some(args.clone());
                continue;
            }
            let put = |out: &mut Vec<Line>, stmt| {
                out.push(Line {
                    stmt,
                    step,
                    merged: Vec::new(),
                })
            };
            match &line.stmt {
                Stmt::Call(CallTarget::Direct(t)) if program.abis.contains_key(t) => {
                    let stmts = invoke(
                        *t,
                        program,
                        after[i].contains(&Loc::Ah),
                        &mut lifted.temps,
                        stack_args.take(),
                    );
                    for (k, stmt) in stmts.into_iter().enumerate() {
                        out.push(Line {
                            stmt,
                            step,
                            merged: if k == 0 {
                                line.merged.clone()
                            } else {
                                Vec::new()
                            },
                        });
                    }
                }
                s if reads_globals(s) => {
                    let info = flow.info(s);
                    let (uses, defs) = if contains_bcd(s) {
                        (
                            [Slot::C].into_iter().collect(),
                            [Slot::C, Slot::V]
                                .into_iter()
                                .filter(|k| {
                                    after[i].contains(&if *k == Slot::C { Loc::C } else { Loc::V })
                                })
                                .collect(),
                        )
                    } else {
                        let live: LocSet = info.defs.intersection(&after[i]).copied().collect();
                        (slots(&info.uses), slots(&live))
                    };
                    for k in &uses {
                        put(&mut out, to_global(*k));
                    }
                    out.push(line);
                    let defs: BTreeSet<Slot> = defs;
                    for k in defs {
                        put(&mut out, from_global(k));
                    }
                }
                _ => out.push(line),
            }
        }
        lifted.blocks[b].lines = out;

        let mut x = ExitValue::default();
        match &cfg.blocks[b].term {
            Term::Return | Term::Halt => own_results(&mut x),
            Term::Tail(t) => {
                match program.abis.get(t) {
                    Some(_) => {
                        let high = own.ret == Some((Slot::A, CType::U16));
                        x.before
                            .extend(invoke(*t, program, high, &mut lifted.temps, None));
                        x.call_done = true;
                    }
                    None => {
                        let uses = slots(&conv.default_call);
                        x.before.extend(uses.into_iter().map(to_global));
                        let back: BTreeSet<Slot> =
                            own.ret.iter().chain(&own.outs).map(|(k, _)| *k).collect();
                        x.after.extend(back.into_iter().map(from_global));
                    }
                }
                own_results(&mut x);
            }
            Term::Unknown(_) => {
                let mut uses = slots(&conv.default_call);
                uses.extend(slots(&conv.exit));
                x.before.extend(uses.into_iter().map(to_global));
                own_results(&mut x);
            }
            _ => continue,
        }
        lifted.blocks[b].exit = Some(x);
    }

    // Number the definitions: the entry's seven, then each block's in
    // order.
    let mut def_slot: Vec<Slot> = Slot::ALL.to_vec();
    let mut def_fits: Vec<bool> = Slot::ALL
        .iter()
        .map(|k| {
            let param = own.params.iter().find(|(p, _)| p == k).map(|(_, t)| *t);
            match param {
                Some(t) => t != CType::U16,
                None => {
                    k.is_flag()
                        || (*k != Slot::A
                            && f.index_of(f.entry_offset)
                                .is_some_and(|i| f.steps[i].insn.flags_before.eff_x()))
                }
            }
        })
        .collect();
    let mut def_pin: Vec<Option<CType>> = vec![None; Slot::ALL.len()];
    let mut first_def: Vec<usize> = vec![0; n];
    for (b, first) in first_def.iter_mut().enumerate() {
        *first = def_slot.len();
        walk_block(f, lifted, b, &own, &ctx, &mut |ev| match ev {
            Ev::Def {
                slot, fits8, pin, ..
            } => {
                def_slot.push(slot);
                def_fits.push(fits8);
                def_pin.push(pin);
            }
            Ev::Partial { .. } => {
                def_slot.push(Slot::A);
                def_fits.push(false);
                def_pin.push(None);
            }
            Ev::Use { .. } | Ev::At(_) => {}
        });
    }

    // What reaches each block, by fixed point.
    let mut state_in: Vec<Option<State>> = vec![None; n];
    state_in[cfg.entry] = Some(entry_state());
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &cfg.rpo {
            let Some(mut st) = state_in[b].clone() else {
                continue;
            };
            let mut id = first_def[b];
            walk_block(f, lifted, b, &own, &ctx, &mut |ev| match ev {
                Ev::Def { slot, .. } => {
                    st[slot_index(slot)] = [id].into_iter().collect();
                    id += 1;
                }
                Ev::Partial { .. } => {
                    st[0] = [id].into_iter().collect();
                    id += 1;
                }
                Ev::Use { .. } | Ev::At(_) => {}
            });
            for s in cfg.succs(b) {
                let new = match &state_in[s] {
                    None => st.clone(),
                    Some(old) => std::array::from_fn(|k| old[k].union(&st[k]).copied().collect()),
                };
                if state_in[s].as_ref() != Some(&new) {
                    state_in[s] = Some(new);
                    changed = true;
                }
            }
        }
    }

    // Join each use's reaching definitions into one web, and note how
    // each web is read.
    let mut webs = Webs {
        parent: (0..def_slot.len()).collect(),
    };
    let mut use_low: Vec<(usize, bool)> = Vec::new();
    for b in 0..n {
        let mut st = state_in[b].clone().unwrap_or_else(entry_state);
        let mut id = first_def[b];
        walk_block(f, lifted, b, &own, &ctx, &mut |ev| match ev {
            Ev::Use { slot, low, .. } => {
                let reach: Vec<usize> = st[slot_index(slot)].iter().copied().collect();
                for w in reach.windows(2) {
                    webs.union(w[0], w[1]);
                }
                use_low.push((reach[0], low));
            }
            Ev::Def { slot, .. } => {
                st[slot_index(slot)] = [id].into_iter().collect();
                id += 1;
            }
            Ev::Partial { .. } => {
                let reach: Vec<usize> = st[0].iter().copied().collect();
                for &r in &reach {
                    webs.union(r, id);
                }
                use_low.push((id, false));
                st[0] = [id].into_iter().collect();
                id += 1;
            }
            Ev::At(_) => {}
        });
    }

    // Each web's type.
    let mut all_fit: BTreeMap<usize, bool> = BTreeMap::new();
    let mut all_low: BTreeMap<usize, bool> = BTreeMap::new();
    let mut used: BTreeSet<usize> = BTreeSet::new();
    for (d, fits) in def_fits.iter().enumerate() {
        let r = webs.find(d);
        let e = all_fit.entry(r).or_insert(true);
        *e &= fits;
    }
    for &(d, low) in &use_low {
        let r = webs.find(d);
        used.insert(r);
        let e = all_low.entry(r).or_insert(true);
        *e &= low;
    }
    let web_type = |r: usize, webs: &mut Webs| -> CType {
        let k = def_slot[webs.find(r)];
        if k.is_flag() {
            CType::Bool
        } else if all_fit.get(&r).copied().unwrap_or(true)
            || all_low.get(&r).copied().unwrap_or(true)
        {
            CType::U8
        } else {
            CType::U16
        }
    };
    let roots: BTreeSet<usize> = (0..def_slot.len()).map(|d| webs.find(d)).collect();
    let mut types: BTreeMap<usize, CType> = BTreeMap::new();
    for &r in &roots {
        types.insert(r, web_type(r, &mut webs));
    }
    // A web a result is stored into through a pointer takes that result's
    // type, where its results agree on one.
    let mut pins: BTreeMap<usize, BTreeSet<CType>> = BTreeMap::new();
    for (d, pin) in def_pin.iter().enumerate() {
        if let Some(t) = pin {
            pins.entry(webs.find(d)).or_default().insert(*t);
        }
    }
    for (r, ts) in &pins {
        if ts.len() == 1 {
            types.insert(*r, *ts.iter().next().unwrap());
        }
    }
    // A parameter's web takes the parameter's type.
    let mut param_of: BTreeMap<usize, (Slot, CType)> = BTreeMap::new();
    for &(k, t) in &own.params {
        let r = webs.find(slot_index(k));
        param_of.insert(r, (k, t));
    }

    // Variables: one per register and type, named for the register. A
    // web needs one if it is read, or written anywhere but the entry.
    let mut defined: BTreeSet<usize> = BTreeSet::new();
    for d in Slot::ALL.len()..def_slot.len() {
        defined.insert(webs.find(d));
    }
    let mut groups: BTreeMap<Slot, BTreeSet<CType>> = BTreeMap::new();
    for &r in &roots {
        if used.contains(&r) || defined.contains(&r) || param_of.contains_key(&r) {
            groups.entry(def_slot[r]).or_default().insert(types[&r]);
        }
    }
    for &(k, t) in &own.params {
        groups.entry(k).or_default().insert(t);
    }
    // A result whose type its web did not take goes through a variable of
    // its own type.
    for (d, pin) in def_pin.iter().enumerate() {
        if let Some(t) = pin
            && types[&webs.find(d)] != *t
        {
            groups.entry(def_slot[d]).or_default().insert(*t);
        }
    }
    let name_of = |k: Slot, t: CType, groups: &BTreeMap<Slot, BTreeSet<CType>>| -> String {
        if groups.get(&k).is_some_and(|g| g.len() > 1) {
            let bits = match t {
                CType::U8 => "8",
                CType::U16 => "16",
                CType::Bool | CType::Int => "",
            };
            format!("{}{bits}", k.name())
        } else {
            k.name().to_owned()
        }
    };
    let mut vars: Vec<VarDecl> = Vec::new();
    let mut var_of_group: BTreeMap<(Slot, CType), u32> = BTreeMap::new();
    // Parameters first, in signature order.
    for &(k, t) in &own.params {
        var_of_group.insert((k, t), vars.len() as u32);
        vars.push(VarDecl {
            name: name_of(k, t, &groups),
            ty: t,
            slot: k,
            param: true,
            unused: false,
        });
    }
    for (&k, ts) in &groups {
        for &t in ts {
            var_of_group.entry((k, t)).or_insert_with(|| {
                vars.push(VarDecl {
                    name: name_of(k, t, &groups),
                    ty: t,
                    slot: k,
                    param: false,
                    unused: false,
                });
                vars.len() as u32 - 1
            });
        }
    }
    let var_of = |d: usize, webs: &mut Webs| -> u32 {
        let r = webs.find(d);
        var_of_group[&(def_slot[r], types[&r])]
    };

    // Rewrite every access.
    for b in 0..n {
        let mut st = state_in[b].clone().unwrap_or_else(entry_state);
        let mut id = first_def[b];
        let mut at = Pos::End;
        // Copies from a result's own variable into its web's, and where.
        let mut copies: Vec<(Pos, Stmt)> = Vec::new();
        walk_block(f, lifted, b, &own, &ctx, &mut |ev| match ev {
            Ev::At(p) => at = p,
            Ev::Use { slot, e, .. } => {
                let d = *st[slot_index(slot)].iter().next().unwrap();
                let v = var_of(d, &mut webs);
                let eight = matches!(e, Expr::Reg(_, Width::W8));
                *e = if eight && vars[v as usize].ty == CType::U16 {
                    Expr::Cast(Width::W8, Box::new(Expr::Var(v)))
                } else {
                    Expr::Var(v)
                };
            }
            Ev::Def {
                slot,
                p,
                v,
                pin,
                clip,
                ..
            } => {
                let var = var_of(id, &mut webs);
                *p = Place::Var(var);
                // A value nothing reads is not kept (a result through a
                // pointer still needs somewhere to go).
                let root = webs.find(id);
                if pin.is_none() && !used.contains(&root) && !param_of.contains_key(&root) {
                    *p = Place::Temp(DEAD);
                    st[slot_index(slot)] = [id].into_iter().collect();
                    id += 1;
                    return;
                }
                if let Some(t) = pin
                    && vars[var as usize].ty != t
                {
                    let own_var = var_of_group[&(slot, t)];
                    *p = Place::Var(own_var);
                    copies.push((
                        at,
                        Stmt::Assign {
                            dst: Place::Var(var),
                            value: Expr::Var(own_var),
                        },
                    ));
                }
                if clip && vars[var as usize].ty == CType::U16 {
                    copies.push((
                        at,
                        Stmt::Assign {
                            dst: Place::Var(var),
                            value: Expr::Cast(Width::W8, Box::new(Expr::Var(var))),
                        },
                    ));
                }
                if let Some(v) = v {
                    let w = match vars[var as usize].ty {
                        CType::U8 => Some(Width::W8),
                        CType::U16 => Some(Width::W16),
                        CType::Bool | CType::Int => None,
                    };
                    if let Some(w) = w {
                        *v = narrow(v.clone(), w);
                    }
                }
                st[slot_index(slot)] = [id].into_iter().collect();
                id += 1;
            }
            Ev::Partial { p, v } => {
                let var = var_of(id, &mut webs);
                *p = Place::Var(var);
                let value = narrow(v.clone(), Width::W8);
                *v = if vars[var as usize].ty == CType::U16 {
                    Expr::bin(
                        BinOp::Or,
                        Expr::bin(BinOp::And, Expr::Var(var), Expr::Const(0xFF00)),
                        Expr::cast(Width::W8, value),
                    )
                } else {
                    value
                };
                st[0] = [id].into_iter().collect();
                id += 1;
            }
        });
        // Each copy goes right after its call.
        for (pos, stmt) in copies.into_iter().rev() {
            let lb = &mut lifted.blocks[b];
            match pos {
                Pos::Line(i) => {
                    let step = lb.lines[i].step;
                    lb.lines.insert(
                        i + 1,
                        Line {
                            stmt,
                            step,
                            merged: Vec::new(),
                        },
                    );
                }
                Pos::Before(j) => {
                    if let Some(x) = &mut lb.exit {
                        x.before.insert(j + 1, stmt);
                    }
                }
                Pos::After(j) => {
                    if let Some(x) = &mut lb.exit {
                        x.after.insert(j + 1, stmt);
                    }
                }
                Pos::End => {}
            }
        }
    }

    // Where the entry's value is read but is no parameter, it comes from
    // the global; where a parameter's web took another type, from the
    // parameter.
    let mut init: Vec<Stmt> = Vec::new();
    for (i, k) in Slot::ALL.iter().enumerate() {
        let r = webs.find(i);
        if !used.contains(&r) {
            continue;
        }
        let v = var_of(i, &mut webs);
        match own.params.iter().find(|(p, _)| p == k) {
            Some(&(_, t)) => {
                let pv = var_of_group[&(*k, t)];
                if pv != v {
                    init.push(Stmt::Assign {
                        dst: Place::Var(v),
                        value: Expr::Var(pv),
                    });
                }
            }
            None => init.push(Stmt::Assign {
                dst: Place::Var(v),
                value: Expr::Global(*k),
            }),
        }
    }
    if !init.is_empty() {
        let step = lifted.blocks[cfg.entry]
            .lines
            .first()
            .map(|l| l.step)
            .unwrap_or(cfg.blocks[cfg.entry].steps.start);
        let mut lines: Vec<Line> = init
            .into_iter()
            .map(|stmt| Line {
                stmt,
                step,
                merged: Vec::new(),
            })
            .collect();
        lines.append(&mut lifted.blocks[cfg.entry].lines);
        lifted.blocks[cfg.entry].lines = lines;
    }

    // Casts a variable's type makes say nothing.
    let types_of: Vec<CType> = vars.iter().map(|v| v.ty).collect();
    for lb in &mut lifted.blocks {
        for l in &mut lb.lines {
            tidy_stmt(&mut l.stmt, &types_of);
        }
        if let Some(c) = &mut lb.cond {
            *c = tidy(c.clone(), &types_of);
        }
        if let Some((s, _)) = &mut lb.switch {
            *s = tidy(s.clone(), &types_of);
        }
        if let Some(x) = &mut lb.exit {
            for s in x.before.iter_mut().chain(x.after.iter_mut()) {
                tidy_stmt(s, &types_of);
            }
            for (_, e) in &mut x.outs {
                *e = tidy(e.clone(), &types_of);
            }
            if let Some(e) = &mut x.ret {
                *e = tidy(e.clone(), &types_of);
            }
        }
    }
    // What the widths made a truncation and the types made nothing
    // (`x = x;`), and values nothing reads: a call's result is dropped, an
    // assignment goes unless its value has an effect.
    let drop = |s: &mut Stmt| -> bool {
        match s {
            Stmt::Assign {
                dst: Place::Var(v),
                value: Expr::Var(w),
            } => v == w,
            Stmt::Assign {
                dst: Place::Temp(DEAD),
                value,
            } => {
                if value.has_effects() {
                    *s = Stmt::Eval(value.clone());
                    false
                } else {
                    true
                }
            }
            Stmt::Invoke { ret, .. } => {
                if *ret == Some(Place::Temp(DEAD)) {
                    *ret = None;
                }
                false
            }
            _ => false,
        }
    };
    // The widths are in the types now: the notes that said them go.
    let width_note = |s: &Stmt| matches!(s, Stmt::Note(n) if n.ends_with("X and Y"));
    for lb in &mut lifted.blocks {
        lb.lines.retain(|l| !width_note(&l.stmt));
        lb.lines.retain_mut(|l| !drop(&mut l.stmt));
        if let Some(x) = &mut lb.exit {
            x.before.retain_mut(|s| !drop(s));
            x.after.retain_mut(|s| !drop(s));
        }
    }
    lifted.vars = vars;
    lifted.abi = Some(own);
    mark_unused(lifted);
}

/// Mark the variables nothing in the printed code mentions, which are not
/// declared.
pub fn mark_unused(lifted: &mut Lifted) {
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    let mut note = |e: &Expr| {
        e.walk(&mut |x| {
            if let Expr::Var(v) = x {
                seen.insert(*v);
            }
        })
    };
    let place_var = |p: &Place| match p {
        Place::Var(v) => Some(*v),
        _ => None,
    };
    let mut places: Vec<u32> = Vec::new();
    for lb in &lifted.blocks {
        let stmts = lb
            .lines
            .iter()
            .map(|l| &l.stmt)
            .chain(lb.exit.iter().flat_map(|x| x.before.iter().chain(&x.after)));
        for st in stmts {
            match st {
                Stmt::Assign { dst, value } => {
                    places.extend(place_var(dst));
                    if let Place::Mem { addr, .. } = dst {
                        note(addr);
                    }
                    note(value);
                }
                Stmt::Invoke {
                    args, ret, outs, ..
                } => {
                    args.iter().for_each(&mut note);
                    places.extend(ret.iter().filter_map(place_var));
                    places.extend(outs.iter().filter_map(place_var));
                }
                Stmt::Effect(_, args) => args.iter().for_each(&mut note),
                Stmt::Eval(e) => note(e),
                Stmt::Call(CallTarget::Table { index, .. }) => note(index),
                _ => {}
            }
        }
        lb.cond.iter().for_each(&mut note);
        if let Some((e, _)) = &lb.switch {
            note(e);
        }
        if let Some(x) = &lb.exit {
            x.outs.iter().for_each(|(_, e)| note(e));
            x.ret.iter().for_each(&mut note);
        }
    }
    seen.extend(places);
    for (i, v) in lifted.vars.iter_mut().enumerate() {
        v.unused = !v.param && !seen.contains(&(i as u32));
    }
}

/// The place of a value nothing reads, until it is dropped.
const DEAD: u32 = u32::MAX;

fn tidy_stmt(s: &mut Stmt, types: &[CType]) {
    match s {
        Stmt::Assign { dst, value } => {
            if let Place::Mem { addr, .. } = dst {
                *addr = tidy(addr.clone(), types);
            }
            *value = tidy(value.clone(), types);
            if let Place::Var(v) = dst {
                match types[*v as usize] {
                    CType::U8 => *value = narrow(value.clone(), Width::W8),
                    CType::U16 => *value = narrow(value.clone(), Width::W16),
                    CType::Bool | CType::Int => {}
                }
            }
        }
        Stmt::Invoke { args, .. } => {
            for a in args {
                *a = tidy(a.clone(), types);
            }
        }
        Stmt::Effect(_, args) => {
            for a in args {
                *a = tidy(a.clone(), types);
            }
        }
        Stmt::Eval(e) => *e = tidy(e.clone(), types),
        Stmt::Call(CallTarget::Table { index, .. }) => *index = tidy(index.clone(), types),
        _ => {}
    }
}

/// `(u8)x` of a `u8` is `x`; any cast to 16 bits of a variable is the
/// variable.
fn tidy(e: Expr, types: &[CType]) -> Expr {
    let e = match e {
        Expr::Mem { addr, width } => Expr::mem(tidy(*addr, types), width),
        Expr::Un(op, x) => Expr::un(op, tidy(*x, types)),
        Expr::Bin(op, a, b) => Expr::bin(op, tidy(*a, types), tidy(*b, types)),
        Expr::Cast(w, x) => Expr::Cast(w, Box::new(tidy(*x, types))),
        Expr::Signed(w, x) => Expr::Signed(w, Box::new(tidy(*x, types))),
        Expr::Call(n, args) => Expr::Call(n, args.into_iter().map(|a| tidy(a, types)).collect()),
        e => e,
    };
    match e {
        Expr::Cast(w, x) => match *x {
            Expr::Var(v) if w >= Width::W16 || types[v as usize] != CType::U16 => Expr::Var(v),
            x => Expr::cast(w, x),
        },
        Expr::Signed(Width::W16, x) if matches!(*x, Expr::Var(v) if types[v as usize] == CType::U8) => {
            *x
        }
        // !!c of a bool is c.
        Expr::Un(UnOp::LNot, x) => match *x {
            Expr::Un(UnOp::LNot, inner) if matches!(*inner, Expr::Var(v) if types[v as usize] == CType::Bool) => {
                *inner
            }
            x => Expr::Un(UnOp::LNot, Box::new(x)),
        },
        e => e,
    }
}
