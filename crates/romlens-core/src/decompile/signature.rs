//! What each routine reads, writes and returns, over the whole program.
//!
//! - **Writes** is the least fixed point: a routine writes what its own
//!   statements write, plus what its callees and tail calls write.
//! - **Reads** is what is live at its entry, and **returns** is what its
//!   callers read after it returns (only what it writes). Both are least
//!   fixed points too: they start empty and grow together until nothing
//!   changes. Each is a monotone function of the other (more read after a
//!   call is more returned; more returned is more live at a return), so the
//!   growth ends, at the smallest answer every routine's code agrees with.
//! - A routine something reaches in a way this cannot see (a vector, a
//!   table, a pointer, code outside every routine) is **open**: its returns
//!   stay the default.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::decompile::cfg::{Cfg, Term};
use crate::decompile::dataflow::{
    self, Conventions, FLAGS, Flow, Loc, LocSet, REGISTERS, all_fixed,
};
use crate::decompile::function::{self, Callee, Function, Transfer};
use crate::decompile::ir::{CType, CallTarget, Slot, Stmt, Width};
use crate::decompile::lift::{self, LiftOptions, Lifted};
use crate::memory::address::SnesAddress;
use crate::model::xref::XRefKind;
use crate::rom::image::RomImage;

/// One routine, lifted.
pub struct Unit {
    pub f: Function,
    pub cfg: Cfg,
    pub lifted: Lifted,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    pub reads: LocSet,
    pub writes: LocSet,
    pub returns: LocSet,
    /// Saved on entry and restored on every return: callers see these
    /// unchanged, so they are in neither `reads` nor `writes`.
    pub preserves: LocSet,
    /// Reached in ways the analysis does not see; returns are the default.
    pub open: bool,
    /// Bytes of its caller's stack it reads as arguments.
    pub stack_args: u32,
}

/// Every routine and its summary.
pub struct Program {
    pub entries: BTreeSet<SnesAddress>,
    pub units: BTreeMap<SnesAddress, Unit>,
    pub summaries: BTreeMap<SnesAddress, Summary>,
    /// Each routine's C signature at the `full` level.
    pub abis: BTreeMap<SnesAddress, Abi>,
    /// How the routines were lifted, with the direct page `direct_page`
    /// worked out where none was given.
    pub lift: LiftOptions,
    /// Values passed in RAM (`memory_args`).
    pub memory: MemoryArgs,
    /// Per routine, the bytes of its caller's stack it reads as arguments.
    pub stack_args: BTreeMap<SnesAddress, u32>,
    /// Per routine taking them as parameters at the `full` level: each
    /// piece's offset above the caller's S at the call, and its bytes.
    pub stack_params: BTreeMap<SnesAddress, Vec<(u32, u32)>>,
}

/// Values passed in memory: RAM a caller stores just before a call, which
/// the routine it calls reads before it writes (the pointer at `$00` set
/// for a copy routine). Canonical addresses, a byte each.
#[derive(Debug, Clone, Default)]
pub struct MemoryArgs {
    /// Per routine: the bytes its callers pass it.
    pub params: BTreeMap<SnesAddress, BTreeSet<u32>>,
    /// Per call, by the caller's entry and the call's file offset: the bytes
    /// it passes.
    pub at_call: BTreeMap<(SnesAddress, u32), BTreeSet<u32>>,
}

/// A routine's C signature at the `full` level, from its summary: a
/// parameter for each register and flag it reads, and for each it returns
/// the first as the value (A, then X, Y, the carry, N, V, Z) and the rest
/// through pointers. S, D and DBR stay the globals. A takes 16 bits where
/// its high byte is read or returned; X and Y take 8 where they are 8 bits
/// wide at the entry (a parameter) or at every return (a result).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Abi {
    pub params: Vec<(Slot, CType)>,
    /// Arguments its caller pushed: where each is at entry (`arg3`), its
    /// offset above the caller's S at the call, and its type.
    pub stack: Vec<(u32, u32, CType)>,
    pub ret: Option<(Slot, CType)>,
    pub outs: Vec<(Slot, CType)>,
}

impl Abi {
    fn of(s: &Summary, entry_x8: bool, exit_x8: bool) -> Abi {
        let ty = |slot: Slot, set: &LocSet, x8: bool| match slot {
            Slot::A if set.contains(&Loc::Ah) => CType::U16,
            Slot::A => CType::U8,
            Slot::X | Slot::Y if x8 => CType::U8,
            Slot::X | Slot::Y => CType::U16,
            _ => CType::Bool,
        };
        let has = |slot: Slot, set: &LocSet| match slot {
            Slot::A => set.contains(&Loc::Al) || set.contains(&Loc::Ah),
            Slot::X => set.contains(&Loc::X),
            Slot::Y => set.contains(&Loc::Y),
            Slot::C => set.contains(&Loc::C),
            Slot::N => set.contains(&Loc::N),
            Slot::V => set.contains(&Loc::V),
            Slot::Z => set.contains(&Loc::Z),
        };
        let params = Slot::ALL
            .iter()
            .filter(|&&k| has(k, &s.reads))
            .map(|&k| (k, ty(k, &s.reads, entry_x8)))
            .collect();
        let mut results: Vec<(Slot, CType)> = Slot::ALL
            .iter()
            .filter(|&&k| has(k, &s.returns))
            .map(|&k| (k, ty(k, &s.returns, exit_x8)))
            .collect();
        let ret = (!results.is_empty()).then(|| results.remove(0));
        Abi {
            params,
            stack: Vec::new(),
            ret,
            outs: results,
        }
    }

    /// C for `static void SHIM(void)` that calls `name` with this
    /// signature from the globals of `snes.h` and puts its results back in
    /// them: how code that knows only the registers runs a `full`-level
    /// routine.
    pub fn global_shim(&self, shim: &str, name: &str) -> String {
        let global_as = |k: Slot, t: CType| match t {
            CType::U8 => format!("(u8){}", k.global()),
            _ => k.global().to_owned(),
        };
        let store = |k: Slot, t: CType, v: &str| match (k, t) {
            (Slot::A, CType::U8) => format!("A = (A & 0xFF00) | (u8)({v});"),
            _ => format!("{} = {v};", k.global()),
        };
        let mut s = format!("static void {shim}(void) {{\n");
        for (k, t) in &self.outs {
            s.push_str(&format!("    {} o_{};\n", t.name(), k.name()));
        }
        let mut args: Vec<String> = self.params.iter().map(|(k, t)| global_as(*k, *t)).collect();
        // What the caller pushed, just above S.
        args.extend(self.stack.iter().map(|(_, m, t)| match t {
            CType::U8 => format!("STACK8(S + {m})"),
            _ => format!("STACK16(S + {m})"),
        }));
        args.extend(self.outs.iter().map(|(k, _)| format!("&o_{}", k.name())));
        let call = format!("{name}({})", args.join(", "));
        match self.ret {
            Some((k, t)) => {
                s.push_str(&format!("    {} r = {call};\n", t.name()));
                s.push_str(&format!("    {}\n", store(k, t, "r")));
            }
            None => s.push_str(&format!("    {call};\n")),
        }
        for (k, t) in &self.outs {
            s.push_str(&format!(
                "    {}\n",
                store(*k, *t, &format!("o_{}", k.name()))
            ));
        }
        s.push_str("}\n");
        s
    }

    /// `u8 NAME(u16 x, u16 *y_out)`.
    pub fn c_signature(&self, name: &str) -> String {
        let mut args: Vec<String> = self
            .params
            .iter()
            .map(|(k, t)| format!("{} {}", t.name(), k.name()))
            .collect();
        args.extend(
            self.stack
                .iter()
                .map(|(e, _, t)| format!("{} arg{e}", t.name())),
        );
        args.extend(
            self.outs
                .iter()
                .map(|(k, t)| format!("{} *{}_out", t.name(), k.name())),
        );
        let args = if args.is_empty() {
            "void".to_owned()
        } else {
            args.join(", ")
        };
        let ret = self.ret.map(|(_, t)| t.name()).unwrap_or("void");
        format!("{ret} {name}({args})")
    }
}

/// The one value the program ever gives the direct page, where it has
/// one: every write of D (`TCD`, `PLD`) sets the reset value 0 again, from
/// a constant loaded into A just before or from a saved copy of D itself.
/// Where the analysis does not know D, D is then that value. With a trace,
/// only the writes it saw run count: data decoded as code, which never
/// runs, can hold any `TCD`.
fn direct_page(units: &BTreeMap<SnesAddress, Unit>, ran: &dyn Fn(u32) -> bool) -> Option<u16> {
    use crate::decompile::ir::{Expr, Place, Reg};
    const RESET: u32 = 0;
    for u in units.values() {
        // Temporaries that hold a copy of D (a PHD the stack pairs made
        // one), or the reset value.
        let mut copies: BTreeMap<u32, bool> = BTreeMap::new();
        for lb in &u.lifted.blocks {
            for l in &lb.lines {
                if let Stmt::Assign {
                    dst: Place::Temp(t),
                    value,
                } = &l.stmt
                {
                    let ok = matches!(value, Expr::Reg(Reg::D, _) | Expr::Const(RESET));
                    let e = copies.entry(*t).or_insert(true);
                    *e &= ok;
                }
            }
        }
        for lb in &u.lifted.blocks {
            // A constant loaded into all of A in this block, so far.
            let mut a: Option<u32> = None;
            for l in &lb.lines {
                match &l.stmt {
                    Stmt::Assign {
                        dst: Place::Reg(Reg::D, _),
                        value,
                    } => {
                        if !ran(u.f.steps[l.step].insn.file_offset.0) {
                            continue;
                        }
                        let ok = match value {
                            Expr::Reg(Reg::A, Width::W16) => a == Some(RESET),
                            Expr::Const(v) => *v == RESET,
                            Expr::Temp(t) => copies.get(t) == Some(&true),
                            _ => false,
                        };
                        if !ok {
                            return None;
                        }
                    }
                    Stmt::Assign {
                        dst: Place::Reg(Reg::A, Width::W16),
                        value: Expr::Const(v),
                    } => a = Some(*v),
                    Stmt::Assign {
                        dst: Place::Reg(Reg::A, _),
                        ..
                    }
                    | Stmt::Call(_)
                    | Stmt::Asm { .. } => a = None,
                    Stmt::Effect("mvn" | "mvp" | "mvn8" | "mvp8" | "BRK" | "COP", _) => a = None,
                    _ => {}
                }
            }
        }
    }
    Some(RESET as u16)
}

/// The bytes of RAM at constant addresses that `e` reads, canonical.
fn ram_reads(rom: &RomImage, e: &crate::decompile::ir::Expr, out: &mut BTreeSet<u32>) {
    use crate::decompile::ir::Expr;
    e.walk(&mut |x| {
        if let Expr::Mem { addr: inner, width } = x
            && let Expr::Const(a) = **inner
        {
            out.extend(ram_bytes(rom, a, width.bytes()));
        }
    })
}

/// The canonical RAM bytes `n` bytes from `a`; none outside RAM.
fn ram_bytes(rom: &RomImage, a: u32, n: u32) -> Vec<u32> {
    use crate::memory::map::MemoryClass;
    (0..n)
        .filter_map(|k| {
            let c = crate::model::project::Project::canonical(
                rom,
                SnesAddress::from_u24(a.wrapping_add(k) & 0xFF_FFFF),
            );
            matches!(
                rom.map().classify(c),
                MemoryClass::Wram | MemoryClass::LowRam | MemoryClass::Sram
            )
            .then_some(c.as_u24())
        })
        .collect()
}

/// Values passed in memory (`MemoryArgs`): each routine's RAM read before
/// it is written on some path from the entry (through its callees too),
/// then at each call the stores the caller makes to those bytes after its
/// last call before it.
fn memory_args(rom: &RomImage, units: &BTreeMap<SnesAddress, Unit>) -> MemoryArgs {
    use crate::decompile::ir::{Expr, Place};
    let callees = |s: &Stmt| -> Vec<SnesAddress> {
        match s {
            Stmt::Call(CallTarget::Direct(t)) => vec![*t],
            Stmt::Call(CallTarget::Table { targets, .. }) => targets.clone(),
            _ => vec![],
        }
    };
    let stored = |s: &Stmt| -> Vec<u32> {
        match s {
            Stmt::Assign {
                dst:
                    Place::Mem {
                        addr: Expr::Const(a),
                        width,
                    },
                ..
            } => ram_bytes(rom, *a, width.bytes()),
            _ => vec![],
        }
    };
    let reads_of = |s: &Stmt, out: &mut BTreeSet<u32>| match s {
        Stmt::Assign { dst, value } => {
            ram_reads(rom, value, out);
            if let Place::Mem { addr, .. } = dst {
                ram_reads(rom, addr, out);
            }
        }
        Stmt::Effect(_, args) => args.iter().for_each(|a| ram_reads(rom, a, out)),
        Stmt::Eval(e) => ram_reads(rom, e, out),
        _ => {}
    };
    // What each routine reads before writing, growing to a fixed point.
    let mut reads_first: BTreeMap<SnesAddress, BTreeSet<u32>> =
        units.keys().map(|a| (*a, BTreeSet::new())).collect();
    for _ in 0..MAX_ROUNDS {
        let mut changed = false;
        for (a, u) in units {
            let n = u.cfg.blocks.len();
            // Bytes written on every path to each block.
            let mut state_in: Vec<Option<BTreeSet<u32>>> = vec![None; n];
            state_in[u.cfg.entry] = Some(BTreeSet::new());
            let mut again = true;
            while again {
                again = false;
                for &b in &u.cfg.rpo {
                    let Some(mut st) = state_in[b].clone() else {
                        continue;
                    };
                    for l in &u.lifted.blocks[b].lines {
                        st.extend(stored(&l.stmt));
                    }
                    for s in u.cfg.succs(b) {
                        let new = match &state_in[s] {
                            None => st.clone(),
                            Some(old) => old.intersection(&st).copied().collect(),
                        };
                        if state_in[s].as_ref() != Some(&new) {
                            state_in[s] = Some(new);
                            again = true;
                        }
                    }
                }
            }
            let mut first = reads_first[a].clone();
            for (b, block) in u.cfg.blocks.iter().enumerate() {
                let Some(mut st) = state_in[b].clone() else {
                    continue;
                };
                let lb = &u.lifted.blocks[b];
                for l in &lb.lines {
                    let mut r = BTreeSet::new();
                    reads_of(&l.stmt, &mut r);
                    for t in callees(&l.stmt) {
                        if let Some(c) = reads_first.get(&t) {
                            r.extend(c.iter().copied());
                        }
                    }
                    first.extend(r.difference(&st).copied());
                    st.extend(stored(&l.stmt));
                }
                let mut r = BTreeSet::new();
                if let Some(c) = &lb.cond {
                    ram_reads(rom, c, &mut r);
                }
                if let Term::Tail(t) = &block.term
                    && let Some(c) = reads_first.get(t)
                {
                    r.extend(c.iter().copied());
                }
                first.extend(r.difference(&st).copied());
            }
            if first != reads_first[a] {
                reads_first.insert(*a, first);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // The stores just before each call that it reads first.
    let mut out = MemoryArgs::default();
    for (a, u) in units {
        for lb in &u.lifted.blocks {
            for (i, l) in lb.lines.iter().enumerate() {
                let targets = callees(&l.stmt);
                if targets.is_empty() {
                    continue;
                }
                let wanted: BTreeSet<u32> = targets
                    .iter()
                    .filter_map(|t| reads_first.get(t))
                    .flatten()
                    .copied()
                    .collect();
                let mut passed = BTreeSet::new();
                for before in lb.lines[..i].iter().rev() {
                    if !callees(&before.stmt).is_empty() {
                        break;
                    }
                    passed.extend(
                        stored(&before.stmt)
                            .into_iter()
                            .filter(|b| wanted.contains(b)),
                    );
                }
                if passed.is_empty() {
                    continue;
                }
                for t in &targets {
                    let reads = reads_first.get(t);
                    out.params.entry(*t).or_default().extend(
                        passed
                            .iter()
                            .filter(|b| reads.is_some_and(|r| r.contains(b)))
                            .copied(),
                    );
                }
                let off = u.f.steps[l.step].insn.file_offset.0;
                out.at_call.insert((*a, off), passed);
            }
        }
    }
    out
}

/// Whether X is 8 bits wide at the routine's entry, and at every return.
pub fn index_widths(u: &Unit) -> (bool, bool) {
    let f = &u.f;
    let entry = f
        .index_of(f.entry_offset)
        .map(|i| f.steps[i].insn.flags_before.eff_x())
        .unwrap_or(false);
    let mut exit: Option<bool> = None;
    for b in &u.cfg.blocks {
        match b.term {
            Term::Return | Term::Halt if !b.is_stub() => {
                let x8 = f.steps[b.steps.end - 1].insn.flags_after.eff_x();
                exit = Some(exit.unwrap_or(true) && x8);
            }
            Term::Tail(_) => exit = Some(false),
            _ => {}
        }
    }
    (entry, exit.unwrap_or(false))
}

/// Growing sets always stop; this only bounds a pathological program.
const MAX_ROUNDS: usize = 400;

/// The registers that stay globals at every level: whatever a routine
/// leaves in them, its caller runs with.
const GLOBALS: [Loc; 3] = [Loc::S, Loc::D, Loc::Dbr];

impl Program {
    /// `ran`: whether the instruction at a file offset is known to run (a
    /// trace saw it), or may (there is none).
    pub fn build(
        rom: &RomImage,
        snap: &AnalysisSnapshot,
        opts: LiftOptions,
        ran: &dyn Fn(u32) -> bool,
    ) -> Program {
        let entries = function::entries(snap);
        let mut units = BTreeMap::new();
        for &e in &entries {
            if let Ok(f) = function::discover(rom, snap, &entries, e) {
                let cfg = Cfg::build(&f);
                let lifted = lift::lift(&f, &cfg, opts);
                units.insert(f.entry, Unit { f, cfg, lifted });
            }
        }
        // What each routine reads of its caller's stack, before any push
        // is made a temporary: a caller's push a callee reads stays a push.
        let stack_args: BTreeMap<SnesAddress, u32> = units
            .iter()
            .map(|(a, u)| (*a, dataflow::stack_args(&u.f, &u.cfg, &u.lifted)))
            .filter(|(_, n)| *n > 0)
            .collect();
        // Which of them take those as parameters at the `full` level: every
        // routine whose arguments can be named, and that nothing reaches in
        // a way the analysis does not see.
        let unseen = open_routines(snap, &units);
        // Only calls pass arguments: a routine a dispatch table, a pointer
        // or a vector reaches is entered with nothing passed, and reads its
        // arguments from the stack itself.
        let indirect: BTreeSet<SnesAddress> = snap
            .jump_tables
            .iter()
            .flat_map(|t| t.targets.iter().map(|(a, _)| *a))
            .chain(units.keys().copied().filter(|a| {
                snap.xrefs_to(*a).iter().any(|x| {
                    matches!(
                        x.kind,
                        XRefKind::Pointer | XRefKind::Vector | XRefKind::JumpTable
                    )
                })
            }))
            .collect();
        let stack_params: BTreeMap<SnesAddress, Vec<(u32, u32)>> = units
            .iter()
            .filter(|(a, _)| !unseen.contains(a) && !indirect.contains(a))
            .map(|(a, u)| (*a, dataflow::stack_param_ranges(&u.f, &u.cfg, &u.lifted)))
            .filter(|(_, r)| !r.is_empty())
            .collect();
        let none = BTreeMap::new();
        for u in units.values_mut() {
            dataflow::stack_slots(&u.f, &u.cfg, &mut u.lifted, &stack_args, &none);
        }
        // Where nothing gave the direct page and the program only ever
        // sets it to one value, that is the direct page.
        let mut opts = opts;
        if opts.assume_dp.is_none()
            && let Some(dp) = direct_page(&units, ran)
        {
            opts = LiftOptions {
                assume_dp: Some(dp),
                dp_inferred: true,
            };
            for u in units.values_mut() {
                let mut lifted = lift::lift(&u.f, &u.cfg, opts);
                dataflow::stack_slots(&u.f, &u.cfg, &mut lifted, &stack_args, &none);
                u.lifted = lifted;
            }
        }
        let open = open_routines(snap, &units);
        let base = Conventions::default();
        let mut preserves: BTreeMap<SnesAddress, LocSet> =
            units.keys().map(|&a| (a, LocSet::new())).collect();

        // Writes, growing from nothing.
        let mut writes: BTreeMap<SnesAddress, LocSet> =
            units.keys().map(|&a| (a, LocSet::new())).collect();
        let mut settled = false;
        for _ in 0..MAX_ROUNDS {
            let mut changed = false;
            let conv = Conventions {
                call_defs: writes.clone(),
                ..base.clone()
            };
            let flow = Flow::new(rom, &conv);
            for (a, u) in &units {
                // What it keeps depends on what its callees write, so it is
                // worked out again each round.
                preserves.insert(*a, preserved(u, &flow, &writes));
                let mut w = LocSet::new();
                for lb in &u.lifted.blocks {
                    for l in &lb.lines {
                        w.extend(flow.info(&l.stmt).defs);
                    }
                }
                for b in &u.cfg.blocks {
                    match &b.term {
                        Term::Tail(t) => match writes.get(t) {
                            Some(tw) => w.extend(tw.iter().copied()),
                            None => w.extend(all_fixed()),
                        },
                        Term::Unknown(_) => w.extend(all_fixed()),
                        _ => {}
                    }
                }
                w.retain(|l| !matches!(l, Loc::Temp(_)) && !preserves[a].contains(l));
                if w != writes[a] {
                    writes.insert(*a, w);
                    changed = true;
                }
            }
            if !changed {
                settled = true;
                break;
            }
        }
        // Stopped short, a routine may write more than it says.
        if !settled {
            for (a, w) in writes.iter_mut() {
                *w = all_fixed();
                preserves.insert(*a, LocSet::new());
            }
        }

        // Reads and returns, growing from nothing (an open routine's
        // returns are the default from the start).
        let open_returns =
            |a: &SnesAddress| -> LocSet { base.exit.intersection(&writes[a]).copied().collect() };
        let mut reads: BTreeMap<SnesAddress, LocSet> =
            units.keys().map(|&a| (a, LocSet::new())).collect();
        let mut returns: BTreeMap<SnesAddress, LocSet> = units
            .keys()
            .map(|a| {
                let r = if open.contains(a) {
                    open_returns(a)
                } else {
                    LocSet::new()
                };
                (*a, r)
            })
            .collect();
        let mut converged = false;
        for _ in 0..MAX_ROUNDS {
            let mut new_reads = BTreeMap::new();
            let mut read_after: BTreeMap<SnesAddress, LocSet> =
                units.keys().map(|&a| (a, LocSet::new())).collect();
            for (a, u) in &units {
                // What it preserves is not read at its returns: the restore
                // puts back what the save took, so callers see the entry
                // value whether or not the routine looked at it.
                let conv = Conventions {
                    exit: returns[a].clone(),
                    calls: reads.clone(),
                    default_call: base.default_call.clone(),
                    call_defs: writes.clone(),
                    stack_args: stack_args.clone(),
                    stack_params: BTreeMap::new(),
                };
                let flow = Flow::new(rom, &conv);
                let live_out = dataflow::liveness(&flow, &u.cfg, &u.lifted);
                let mut r = dataflow::live_at_entry(&flow, &u.cfg, &u.lifted, &live_out);
                r.retain(|l| !matches!(l, Loc::Temp(_)));
                r.extend(reads[a].iter().copied());
                new_reads.insert(*a, r);
                for (b, lb) in u.lifted.blocks.iter().enumerate() {
                    let calls: Vec<(usize, SnesAddress)> = lb
                        .lines
                        .iter()
                        .enumerate()
                        .flat_map(|(i, l)| match &l.stmt {
                            Stmt::Call(CallTarget::Direct(t)) => vec![(i, *t)],
                            Stmt::Call(CallTarget::Table { targets, .. }) => {
                                targets.iter().map(|t| (i, *t)).collect()
                            }
                            _ => vec![],
                        })
                        .collect();
                    if !calls.is_empty() {
                        let after =
                            dataflow::live_after_lines(&flow, &u.cfg, &u.lifted, &live_out, b);
                        for (i, t) in calls {
                            if let Some(acc) = read_after.get_mut(&t) {
                                acc.extend(after[i].iter().copied());
                            }
                        }
                    }
                    // A tail call returns to this routine's callers.
                    if let Term::Tail(t) = &u.cfg.blocks[b].term
                        && let Some(acc) = read_after.get_mut(t)
                    {
                        acc.extend(returns[a].iter().copied());
                    }
                }
            }
            let new_returns: BTreeMap<SnesAddress, LocSet> = units
                .keys()
                .map(|a| {
                    // An open routine's unseen callers are assumed to read
                    // the default; its known ones read what they read.
                    let mut seen = read_after[a].clone();
                    if open.contains(a) {
                        seen.extend(base.exit.iter().copied());
                    }
                    let mut r: LocSet = seen.intersection(&writes[a]).copied().collect();
                    r.retain(|l| !matches!(l, Loc::Temp(_)));
                    r.extend(returns[a].iter().copied());
                    (*a, r)
                })
                .collect();
            let done = new_reads == reads && new_returns == returns;
            reads = new_reads;
            returns = new_returns;
            if done {
                converged = true;
                break;
            }
        }
        // Stopped short, the sets may be too small: fall back to the
        // assumption every call and return reads everything.
        if !converged {
            for a in units.keys() {
                reads.insert(*a, base.default_call.clone());
                returns.insert(*a, open_returns(a));
            }
        }

        let summaries: BTreeMap<SnesAddress, Summary> = units
            .keys()
            .map(|a| {
                (
                    *a,
                    Summary {
                        reads: reads[a].clone(),
                        writes: writes[a].clone(),
                        returns: returns[a].clone(),
                        preserves: preserves[a].clone(),
                        open: open.contains(a),
                        stack_args: stack_args.get(a).copied().unwrap_or(0),
                    },
                )
            })
            .collect();
        // A routine some call reaches with 16-bit index registers takes
        // them whole, whatever its own entry was decoded with. Its results
        // are 8 bits where it returns with 8-bit index registers, or where
        // every call to it goes on with them (the listing assumes a call
        // leaves the widths as they were, and so does the C).
        let mut wide_calls: BTreeSet<SnesAddress> = BTreeSet::new();
        let mut wide_after: BTreeSet<SnesAddress> = BTreeSet::new();
        let mut called: BTreeSet<SnesAddress> = BTreeSet::new();
        for u in units.values() {
            for (b, block) in u.cfg.blocks.iter().enumerate() {
                if let Term::Tail(t) = &block.term {
                    // It returns to this routine's callers: not followed.
                    wide_after.insert(*t);
                    if block.is_stub() || !u.f.steps[block.steps.end - 1].insn.flags_after.eff_x() {
                        wide_calls.insert(*t);
                    }
                }
                for l in &u.lifted.blocks[b].lines {
                    let targets: Vec<SnesAddress> = match &l.stmt {
                        Stmt::Call(CallTarget::Direct(t)) => vec![*t],
                        Stmt::Call(CallTarget::Table { targets, .. }) => targets.clone(),
                        _ => continue,
                    };
                    let insn = &u.f.steps[l.step].insn;
                    called.extend(targets.iter().copied());
                    if !insn.flags_before.eff_x() {
                        wide_calls.extend(targets.iter().copied());
                    }
                    if !insn.flags_after.eff_x() {
                        wide_after.extend(targets);
                    }
                }
            }
        }
        let abis = units
            .iter()
            .map(|(a, u)| {
                let (entry_x8, exit_x8) = index_widths(u);
                let entry_x8 = entry_x8 && !wide_calls.contains(a);
                let exit_x8 =
                    exit_x8 || (called.contains(a) && !wide_after.contains(a) && !open.contains(a));
                let mut abi = Abi::of(&summaries[a], entry_x8, exit_x8);
                if let Some(pieces) = stack_params.get(a) {
                    let ret = if units[a]
                        .f
                        .steps
                        .iter()
                        .any(|s| s.insn.mnemonic == crate::cpu65816::Mnemonic::RTL)
                    {
                        3
                    } else {
                        2
                    };
                    abi.stack = pieces
                        .iter()
                        .map(|&(m, w)| (m + ret, m, if w == 1 { CType::U8 } else { CType::U16 }))
                        .collect();
                }
                (*a, abi)
            })
            .collect();
        let memory = memory_args(rom, &units);
        Program {
            entries,
            units,
            summaries,
            abis,
            lift: opts,
            memory,
            stack_args,
            stack_params,
        }
    }

    /// The conventions for the `full` level, where the registers are each
    /// routine's own variables: a register it preserves need not be put
    /// back, since its callers keep theirs. S, D and DBR stay globals, so
    /// what the routine leaves in them is always seen after it.
    pub fn canonical_conventions(&self, at: SnesAddress) -> Conventions {
        let mut c = self.conventions(at);
        c.stack_params = self.stack_params.clone();
        if let Some(s) = self.summaries.get(&at) {
            c.exit = s.returns.clone();
            c.exit.extend(GLOBALS);
        }
        c
    }

    /// The conventions for decompiling the routine at `at`.
    pub fn conventions(&self, at: SnesAddress) -> Conventions {
        let base = Conventions::default();
        let mut calls = BTreeMap::new();
        let mut call_defs = BTreeMap::new();
        for (a, s) in &self.summaries {
            calls.insert(*a, s.reads.clone());
            call_defs.insert(*a, s.writes.clone());
        }
        Conventions {
            // What it preserves stays live at its returns, so the restore is
            // printed: callers rely on it.
            // S, D and DBR are seen after it too: a bank set with `PLB` is
            // read by the caller's next load even when the lifter resolved
            // that load's bank itself.
            exit: self
                .summaries
                .get(&at)
                .map(|s| {
                    s.returns
                        .union(&s.preserves)
                        .copied()
                        .chain(GLOBALS)
                        .collect()
                })
                .unwrap_or(base.exit),
            calls,
            default_call: base.default_call,
            call_defs,
            stack_args: self.stack_args.clone(),
            stack_params: BTreeMap::new(),
        }
    }
}

/// The registers and flags that hold their entry value again wherever the
/// routine returns, among those it writes: saved and restored (`PHX` …
/// `PLX`), or left alone on the paths that return early. A forward
/// must-analysis: a location keeps its entry value until something writes
/// it, and gets it back from a temporary that holds a copy of it (a slot
/// written once, from the location while it still held its entry value). A
/// call writes what `flow` says the callee writes; a tail call returns what
/// is still the entry value and not written by the callee.
fn preserved(u: &Unit, flow: &Flow, writes: &BTreeMap<SnesAddress, LocSet>) -> LocSet {
    use crate::decompile::ir::{Expr, Place};
    let tracked: LocSet = all_fixed().into_iter().filter(|l| *l != Loc::S).collect();
    let value_locs = |e: &Expr| -> Option<LocSet> {
        let inner = match e {
            Expr::Cast(_, x) => &**x,
            x => x,
        };
        match inner {
            Expr::Reg(r, w) => Some(reg_locs(*r, *w)),
            Expr::Flag(f) => Some([flag_loc(*f)].into_iter().collect()),
            _ => None,
        }
    };
    // A save must be its temporary's only write, or the slot may hold
    // something else by the time it is restored.
    let mut writes_of: BTreeMap<u32, usize> = BTreeMap::new();
    let mut written = LocSet::new();
    for lb in &u.lifted.blocks {
        for l in &lb.lines {
            if let Stmt::Assign {
                dst: Place::Temp(t),
                ..
            } = &l.stmt
            {
                *writes_of.entry(*t).or_default() += 1;
            }
            written.extend(flow.info(&l.stmt).defs);
        }
    }
    written.retain(|l| tracked.contains(l));
    if written.is_empty() {
        return LocSet::new();
    }
    // What each saving temporary holds the entry value of; recomputed on
    // every pass, since a save counts only if its source still held the
    // entry value on every path to it.
    let n = u.cfg.blocks.len();
    let mut state_in: Vec<Option<LocSet>> = vec![None; n];
    state_in[u.cfg.entry] = Some(tracked.clone());
    let mut holds: BTreeMap<u32, LocSet> = BTreeMap::new();
    let step = |state: &mut LocSet, l: &Stmt, holds: &mut BTreeMap<u32, LocSet>| {
        if let Stmt::Assign { dst, value } = l {
            match dst {
                Place::Temp(t) => {
                    if writes_of.get(t) == Some(&1) {
                        let h = value_locs(value)
                            .filter(|locs| locs.is_subset(state))
                            .unwrap_or_default();
                        holds.insert(*t, h);
                    }
                    return;
                }
                Place::Reg(..) | Place::Flag(_) => {
                    let defs = flow.info(l).defs;
                    for d in &defs {
                        state.remove(d);
                    }
                    if let Expr::Temp(t) = value
                        && let Some(h) = holds.get(t)
                    {
                        for d in defs {
                            if h.contains(&d) {
                                state.insert(d);
                            }
                        }
                    }
                    return;
                }
                _ => {}
            }
        }
        for d in flow.info(l).defs {
            state.remove(&d);
        }
    };
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 64 {
        changed = false;
        rounds += 1;
        for &b in &u.cfg.rpo {
            let Some(mut st) = state_in[b].clone() else {
                continue;
            };
            for l in &u.lifted.blocks[b].lines {
                step(&mut st, &l.stmt, &mut holds);
            }
            for s in u.cfg.succs(b) {
                let new = match &state_in[s] {
                    None => st.clone(),
                    Some(old) => old.intersection(&st).copied().collect(),
                };
                if state_in[s].as_ref() != Some(&new) {
                    state_in[s] = Some(new);
                    changed = true;
                }
            }
        }
    }
    if changed {
        return LocSet::new();
    }
    let mut out: Option<LocSet> = None;
    for (b, block) in u.cfg.blocks.iter().enumerate() {
        let Some(mut st) = state_in[b].clone() else {
            continue;
        };
        match &block.term {
            // The C returns after a halt (`STP(); return;`), so it counts.
            Term::Return | Term::Tail(_) | Term::Halt => {}
            // Leaving in a way nothing follows: nothing is known to be kept.
            Term::Unknown(_) => return LocSet::new(),
            _ => continue,
        }
        for l in &u.lifted.blocks[b].lines {
            step(&mut st, &l.stmt, &mut holds);
        }
        if let Term::Tail(t) = &block.term {
            match writes.get(t) {
                Some(w) => st.retain(|l| !w.contains(l)),
                None => return LocSet::new(),
            }
        }
        out = Some(match out {
            None => st,
            Some(o) => o.intersection(&st).copied().collect(),
        });
    }
    let mut kept = out.unwrap_or_default();
    kept.retain(|l| written.contains(l));
    kept
}

fn reg_locs(r: crate::decompile::ir::Reg, w: crate::decompile::ir::Width) -> LocSet {
    use crate::decompile::ir::{Reg, Width};
    match r {
        Reg::A if w == Width::W8 => [Loc::Al].into_iter().collect(),
        Reg::A => [Loc::Al, Loc::Ah].into_iter().collect(),
        Reg::X => [Loc::X].into_iter().collect(),
        Reg::Y => [Loc::Y].into_iter().collect(),
        Reg::S => [Loc::S].into_iter().collect(),
        Reg::D => [Loc::D].into_iter().collect(),
        Reg::Dbr => [Loc::Dbr].into_iter().collect(),
    }
}

fn flag_loc(f: crate::decompile::ir::Flag) -> Loc {
    use crate::decompile::ir::Flag;
    match f {
        Flag::N => Loc::N,
        Flag::V => Loc::V,
        Flag::Z => Loc::Z,
        Flag::C => Loc::C,
    }
}

/// Routines reached in a way no analysed call or tail call accounts for.
fn open_routines(
    snap: &AnalysisSnapshot,
    units: &BTreeMap<SnesAddress, Unit>,
) -> BTreeSet<SnesAddress> {
    // Which routine each analysed instruction belongs to, and what it does.
    let mut at: BTreeMap<u32, Vec<(SnesAddress, &Transfer)>> = BTreeMap::new();
    for (a, u) in units {
        for s in &u.f.steps {
            at.entry(s.insn.file_offset.0)
                .or_default()
                .push((*a, &s.transfer));
        }
    }
    // A dispatch table's slots, when its dispatcher is in an analysed
    // routine: a reference from a slot is that dispatcher's.
    let in_table = |off: u32, target: SnesAddress| {
        snap.jump_tables.iter().any(|t| {
            off >= t.base
                && off < t.end()
                && at.contains_key(&t.site)
                && t.targets.iter().any(|(a, _)| *a == target)
        })
    };
    let mut open = BTreeSet::new();
    for &target in units.keys() {
        for x in snap.xrefs_to(target) {
            if in_table(x.from.0, target) {
                continue;
            }
            let seen = match x.kind {
                XRefKind::Read | XRefKind::Write | XRefKind::ReadWrite => continue,
                XRefKind::Call => at.get(&x.from.0).is_some_and(|sites| {
                    sites.iter().all(|(_, t)| match t {
                        Transfer::Call {
                            callee: Callee::Direct(c),
                            ..
                        } => *c == target,
                        Transfer::Call {
                            callee: Callee::Table { targets, .. },
                            ..
                        } => targets.contains(&target),
                        _ => false,
                    })
                }),
                // A dispatch table's entry: the switch or call site above
                // accounts for it.
                XRefKind::JumpTable | XRefKind::Pointer => at.contains_key(&x.from.0),
                // From another routine this is one of its tail calls, which
                // the fixed point follows; from the routine itself, a loop.
                XRefKind::Jump | XRefKind::Branch => at.contains_key(&x.from.0),
                _ => false,
            };
            if !seen {
                open.insert(target);
            }
        }
    }
    open
}

/// "Reads A and X; returns A and the carry."
pub fn describe(s: &Summary) -> String {
    let name = |set: &LocSet| -> Vec<&'static str> {
        let mut v = Vec::new();
        match (set.contains(&Loc::Al), set.contains(&Loc::Ah)) {
            (true, true) => v.push("A"),
            (true, false) => v.push("A's low byte"),
            (false, true) => v.push("A's high byte"),
            _ => {}
        }
        for (l, n) in [
            (Loc::X, "X"),
            (Loc::Y, "Y"),
            (Loc::D, "D"),
            (Loc::Dbr, "DBR"),
            (Loc::C, "the carry"),
            (Loc::N, "N"),
            (Loc::V, "V"),
            (Loc::Z, "Z"),
        ] {
            if set.contains(&l) {
                v.push(n);
            }
        }
        v
    };
    let list = |v: Vec<&str>| match v.len() {
        0 => "nothing".to_owned(),
        1 => v[0].to_owned(),
        n => format!("{} and {}", v[..n - 1].join(", "), v[n - 1]),
    };
    let mut reads = name(&s.reads);
    let pushed = match s.stack_args {
        0 => String::new(),
        1 => "a byte its caller pushed".to_owned(),
        n => format!("{n} bytes its caller pushed"),
    };
    if !pushed.is_empty() {
        reads.push(&pushed);
    }
    let returns = name(&s.returns);
    let mut out = format!("Reads {}; returns {}", list(reads), list(returns));
    let kept = name(&s.preserves);
    if !kept.is_empty() {
        out.push_str(&format!("; preserves {}", list(kept)));
    }
    if s.open {
        out.push_str(" (some callers are not known, so returns are assumed)");
    }
    out.push('.');
    out
}

/// Which locations a register and flag mask names, for tests.
pub fn all_tracked() -> LocSet {
    REGISTERS.iter().chain(FLAGS.iter()).copied().collect()
}
