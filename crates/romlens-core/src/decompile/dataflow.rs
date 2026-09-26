//! Data flow over the lifted IR: what the `clean` level adds.
//!
//! - **Bytes through the stack.** `PEI ($01); PLB; PLB` pushes two bytes
//!   and pulls them one at a time, the usual way to load the data bank from
//!   memory without touching A: each pull becomes the byte it takes, so it
//!   reads `DBR = ADDR_02`.
//! - **Stack slots.** Pushes and pulls that pair up at the same depth
//!   become saved temporaries, when the routine never looks at `S` itself.
//! - **Dead code.** Liveness over the registers (with the accumulator's two
//!   bytes apart), the four flags and the temporaries; an assignment nothing
//!   reads is dropped, which is where most N, Z and V updates go.
//! - **Propagation.** A value used once is carried into its use, so
//!   `LDA #$80` / `STA $2100` becomes `INIDISP = 0x80`, and a branch tests
//!   the comparison that set its flag.
//! - **Simplification.** `!(a >= b)` is `a < b`; `(X & 0x80) != 0` is
//!   `(s8)X < 0`.
//!
//! What a call and a return read is an assumption until signatures
//! (`signature`) say better: `Conventions` carries it.

use std::collections::{BTreeMap, BTreeSet};

use crate::cpu65816::Mnemonic;
use crate::decompile::cfg::{BlockId, Cfg, Term};
use crate::decompile::function::Function;
use crate::decompile::ir::{BinOp, CallTarget, Expr, Flag, Line, Place, Reg, Stmt, UnOp, Width};
use crate::decompile::lift::Lifted;
use crate::memory::address::SnesAddress;
use crate::memory::map::MemoryClass;
use crate::rom::image::RomImage;

/// A location data flow tracks. The accumulator is two: 8-bit code changes
/// only `Al`, and whether `Ah` (the hidden B) is ever read decides how a
/// store to it prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Loc {
    Al,
    Ah,
    X,
    Y,
    S,
    D,
    Dbr,
    N,
    V,
    Z,
    C,
    Temp(u32),
    /// A flag's copy in a status byte a `PHP` pushed (N, V, Z, C as 0-3),
    /// read back by a `PLP`. A `PHP` reads a flag only where a `PLP` that
    /// may pull its byte puts that flag back and something reads it.
    Saved(u8),
}

/// The flags a status byte holds, in `Loc::Saved` order.
const SAVED_FLAGS: [Loc; 4] = [Loc::N, Loc::V, Loc::Z, Loc::C];

pub type LocSet = BTreeSet<Loc>;

pub const REGISTERS: [Loc; 7] = [Loc::Al, Loc::Ah, Loc::X, Loc::Y, Loc::S, Loc::D, Loc::Dbr];
pub const FLAGS: [Loc; 4] = [Loc::N, Loc::V, Loc::Z, Loc::C];

pub fn all_fixed() -> LocSet {
    REGISTERS.iter().chain(FLAGS.iter()).copied().collect()
}

impl Loc {
    fn of_reg(r: Reg, w: Width) -> Vec<Loc> {
        match r {
            Reg::A if w == Width::W8 => vec![Loc::Al],
            Reg::A => vec![Loc::Al, Loc::Ah],
            Reg::X => vec![Loc::X],
            Reg::Y => vec![Loc::Y],
            Reg::S => vec![Loc::S],
            Reg::D => vec![Loc::D],
            Reg::Dbr => vec![Loc::Dbr],
        }
    }

    fn of_flag(f: Flag) -> Loc {
        match f {
            Flag::N => Loc::N,
            Flag::V => Loc::V,
            Flag::Z => Loc::Z,
            Flag::C => Loc::C,
        }
    }
}

/// What calls and returns read.
#[derive(Debug, Clone)]
pub struct Conventions {
    /// Read at a return: what callers may look at.
    pub exit: LocSet,
    /// Read by a call, per callee; `default_call` for the rest.
    pub calls: BTreeMap<SnesAddress, LocSet>,
    pub default_call: LocSet,
    /// Written by a call, per callee; everything for the rest.
    pub call_defs: BTreeMap<SnesAddress, LocSet>,
    /// Per callee, how many bytes of its caller's stack it reads as
    /// arguments (`dataflow::stack_args`).
    pub stack_args: BTreeMap<SnesAddress, u32>,
    /// At the `full` level, per callee taking its arguments as parameters:
    /// each piece's offset above the caller's S at the call, and its bytes.
    pub stack_params: BTreeMap<SnesAddress, Vec<(u32, u32)>>,
}

impl Default for Conventions {
    /// Without signatures: every register and the carry are read by a call
    /// and at a return; N, V and Z are not, since code almost never passes
    /// a result in them.
    fn default() -> Self {
        let regs: LocSet = REGISTERS.iter().copied().chain([Loc::C]).collect();
        Self {
            exit: regs.clone(),
            calls: BTreeMap::new(),
            default_call: regs,
            call_defs: BTreeMap::new(),
            stack_args: BTreeMap::new(),
            stack_params: BTreeMap::new(),
        }
    }
}

impl Conventions {
    fn call_uses(&self, t: &CallTarget) -> LocSet {
        self.over_targets(t, &self.calls, || self.default_call.clone())
    }

    fn call_defs(&self, t: &CallTarget) -> LocSet {
        self.over_targets(t, &self.call_defs, all_fixed)
    }

    /// A direct callee's entry in `map`; the union over a table's targets;
    /// `default` for anything not known.
    fn over_targets(
        &self,
        t: &CallTarget,
        map: &BTreeMap<SnesAddress, LocSet>,
        default: impl Fn() -> LocSet,
    ) -> LocSet {
        match t {
            CallTarget::Direct(a) => map.get(a).cloned().unwrap_or_else(default),
            CallTarget::Table { targets, .. } => {
                let mut out = LocSet::new();
                for a in targets {
                    match map.get(a) {
                        Some(s) => out.extend(s.iter().copied()),
                        None => return default(),
                    }
                }
                out
            }
            CallTarget::Indirect(_) => default(),
        }
    }
}

/// What one statement reads and writes.
#[derive(Debug, Clone, Default)]
pub struct Info {
    pub uses: LocSet,
    pub defs: LocSet,
    /// Stores to memory.
    pub writes_mem: bool,
    /// Reads memory.
    pub reads_mem: bool,
    /// Has an effect beyond its locations: a call, a helper, a hardware
    /// register read. Such statements keep their order.
    pub effect: bool,
}

pub struct Flow<'a> {
    rom: &'a RomImage,
    conv: &'a Conventions,
}

impl<'a> Flow<'a> {
    pub fn new(rom: &'a RomImage, conv: &'a Conventions) -> Self {
        Self { rom, conv }
    }

    /// A memory read with an effect: a hardware register, or an address
    /// nothing pins down.
    fn effectful_read(&self, addr: &Expr) -> bool {
        let base = match addr {
            Expr::Const(a) => Some(*a),
            Expr::Bin(BinOp::Add, x, y) => match (&**x, &**y) {
                (i, Expr::Const(a)) | (Expr::Const(a), i) if !i.has_effects() => Some(*a),
                _ => None,
            },
            _ => None,
        };
        match base {
            Some(a) => {
                self.rom
                    .map()
                    .classify(SnesAddress::from_u24(a & 0xFF_FFFF))
                    == MemoryClass::Hardware
            }
            None => true,
        }
    }

    pub fn expr_info(&self, e: &Expr, out: &mut Info) {
        e.walk(&mut |x| match x {
            Expr::Reg(r, w) => out.uses.extend(Loc::of_reg(*r, *w)),
            Expr::Flag(f) => {
                out.uses.insert(Loc::of_flag(*f));
            }
            Expr::Temp(t) => {
                out.uses.insert(Loc::Temp(*t));
            }
            Expr::Mem { addr, .. } => {
                out.reads_mem = true;
                if self.effectful_read(addr) {
                    out.effect = true;
                }
            }
            Expr::Call(name, _) => match *name {
                "pull8" | "pull16" => {
                    out.uses.insert(Loc::S);
                    out.defs.insert(Loc::S);
                    out.reads_mem = true;
                    out.effect = true;
                }
                "bcd_add" | "bcd_sub" => {
                    out.uses.insert(Loc::C);
                    out.defs.extend([Loc::C, Loc::V]);
                }
                _ => out.effect = true,
            },
            _ => {}
        });
    }

    pub fn info(&self, s: &Stmt) -> Info {
        let mut i = Info::default();
        match s {
            Stmt::Assign { dst, value } => {
                self.expr_info(value, &mut i);
                match dst {
                    Place::Reg(r, w) => i.defs.extend(Loc::of_reg(*r, *w)),
                    Place::Flag(f) => {
                        i.defs.insert(Loc::of_flag(*f));
                    }
                    Place::Temp(t) => {
                        i.defs.insert(Loc::Temp(*t));
                    }
                    Place::Mem { addr, .. } => {
                        self.expr_info(addr, &mut i);
                        i.writes_mem = true;
                        i.effect = true;
                    }
                    // The `full` level's variables are not tracked here: no
                    // clean-up runs after them. Keep their statements.
                    Place::Var(_) | Place::Global(_) | Place::Out(_) => i.effect = true,
                }
            }
            Stmt::Call(t) => {
                if let CallTarget::Table { index, .. } = t {
                    self.expr_info(index, &mut i);
                }
                i.uses.extend(self.conv.call_uses(t));
                i.defs.extend(self.conv.call_defs(t));
                i.writes_mem = true;
                i.reads_mem = true;
                i.effect = true;
            }
            Stmt::Effect(name, args) => {
                for a in args {
                    self.expr_info(a, &mut i);
                }
                i.effect = true;
                match *name {
                    "push8" | "push16" => {
                        i.uses.insert(Loc::S);
                        i.defs.insert(Loc::S);
                        i.writes_mem = true;
                    }
                    // Which flag each copies is liveness's business
                    // (`step_back`): the saved copies are `Loc::Saved`.
                    "PHP" => {
                        i.uses.extend([Loc::S, Loc::N, Loc::V, Loc::Z, Loc::C]);
                        i.defs.insert(Loc::S);
                        i.writes_mem = true;
                    }
                    "PLP" => {
                        i.uses.extend([
                            Loc::S,
                            Loc::Saved(0),
                            Loc::Saved(1),
                            Loc::Saved(2),
                            Loc::Saved(3),
                        ]);
                        i.defs.extend([Loc::S, Loc::N, Loc::V, Loc::Z, Loc::C]);
                        i.reads_mem = true;
                    }
                    "mvn" | "mvp" | "mvn8" | "mvp8" => {
                        i.uses.extend([Loc::Al, Loc::Ah, Loc::X, Loc::Y]);
                        i.defs.extend([Loc::Al, Loc::Ah, Loc::X, Loc::Y]);
                        i.writes_mem = true;
                        i.reads_mem = true;
                    }
                    "BRK" | "COP" => {
                        i.uses.extend(all_fixed());
                        i.defs.extend(all_fixed());
                        i.writes_mem = true;
                        i.reads_mem = true;
                    }
                    _ => {}
                }
            }
            Stmt::Asm { .. } => {
                i.uses.extend(all_fixed());
                i.defs.extend(all_fixed());
                i.writes_mem = true;
                i.reads_mem = true;
                i.effect = true;
            }
            Stmt::Note(_) => {}
            Stmt::Invoke { args, .. } => {
                for a in args {
                    self.expr_info(a, &mut i);
                }
                i.writes_mem = true;
                i.reads_mem = true;
                i.effect = true;
            }
            Stmt::Eval(e) => {
                self.expr_info(e, &mut i);
                i.effect = true;
            }
        }
        i
    }

    /// What a block's terminator reads.
    pub fn term_uses(&self, term: &Term, cond: Option<&Expr>, switch: Option<&Expr>) -> LocSet {
        let mut i = Info::default();
        match term {
            Term::Branch { .. } => {
                if let Some(c) = cond {
                    self.expr_info(c, &mut i);
                }
            }
            Term::Switch { .. } => {
                if let Some(s) = switch {
                    self.expr_info(s, &mut i);
                }
            }
            // The C returns after a halt too (`STP(); return;`).
            Term::Return | Term::Halt => i.uses.extend(self.conv.exit.iter().copied()),
            // A tail call returns to this routine's callers: what they read
            // and the callee leaves alone passes straight through it.
            Term::Tail(a) => {
                let t = CallTarget::Direct(*a);
                let defs = self.conv.call_defs(&t);
                i.uses.extend(self.conv.call_uses(&t));
                i.uses
                    .extend(self.conv.exit.iter().filter(|l| !defs.contains(l)).copied());
            }
            // Wherever it goes is read as a call's inputs are (the
            // registers and the carry, not N, V or Z), and returns to this
            // routine's callers, so what they read passes through it.
            Term::Unknown(_) => {
                i.uses.extend(self.conv.default_call.iter().copied());
                i.uses.extend(self.conv.exit.iter().copied());
            }
            Term::Fall(_) | Term::Goto(_) => {}
        }
        i.uses
    }
}

/// Whether a statement's reads count at a point where `live` is live after
/// it: not for an assignment to a register, flag or temporary that nothing
/// reads and that has no effect, which is dead (strong liveness, so a value
/// read only by dead code is not live either).
fn counts(s: &Stmt, info: &Info, live: &LocSet) -> bool {
    let local = matches!(s, Stmt::Assign { dst, .. } if !matches!(dst, Place::Mem { .. }));
    !local || info.effect || info.defs.iter().any(|d| live.contains(d))
}

/// Step `live` back over one statement.
fn step_back(s: &Stmt, info: &Info, live: &mut LocSet) {
    match s {
        // Each flag goes to its copy and comes back from it. A `PHP` does
        // not kill the copies: another `PHP` on the way may be the one
        // pulled, so each only may be.
        Stmt::Effect("PHP", _) => {
            for (k, f) in SAVED_FLAGS.iter().enumerate() {
                if live.contains(&Loc::Saved(k as u8)) {
                    live.insert(*f);
                }
            }
            live.insert(Loc::S);
            return;
        }
        Stmt::Effect("PLP", _) => {
            for (k, f) in SAVED_FLAGS.iter().enumerate() {
                if live.remove(f) {
                    live.insert(Loc::Saved(k as u8));
                }
            }
            live.insert(Loc::S);
            return;
        }
        _ => {}
    }
    if !counts(s, info, live) {
        return;
    }
    for d in &info.defs {
        live.remove(d);
    }
    live.extend(info.uses.iter().copied());
}

/// Live-out of every block, by fixed point.
pub fn liveness(flow: &Flow, cfg: &Cfg, lifted: &Lifted) -> Vec<LocSet> {
    let n = cfg.blocks.len();
    let infos: Vec<Vec<Info>> = lifted
        .blocks
        .iter()
        .map(|b| b.lines.iter().map(|l| flow.info(&l.stmt)).collect())
        .collect();
    let term: Vec<LocSet> = (0..n)
        .map(|b| {
            let lb = &lifted.blocks[b];
            flow.term_uses(
                &cfg.blocks[b].term,
                lb.cond.as_ref(),
                lb.switch.as_ref().map(|s| &s.0),
            )
        })
        .collect();
    let mut live_in: Vec<LocSet> = vec![LocSet::new(); n];
    let mut live_out: Vec<LocSet> = vec![LocSet::new(); n];
    let mut order: Vec<BlockId> = cfg.rpo.clone();
    order.reverse();
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &order {
            let mut out = LocSet::new();
            for s in cfg.succs(b) {
                out.extend(live_in[s].iter().copied());
            }
            let mut live = out.clone();
            live.extend(term[b].iter().copied());
            for (l, i) in lifted.blocks[b].lines.iter().zip(&infos[b]).rev() {
                step_back(&l.stmt, i, &mut live);
            }
            if live != live_in[b] {
                live_in[b] = live;
                changed = true;
            }
            live_out[b] = out;
        }
    }
    live_out
}

/// What is live after each line of block `b`, given its live-out.
pub fn live_after_lines(
    flow: &Flow,
    cfg: &Cfg,
    lifted: &Lifted,
    live_out: &[LocSet],
    b: BlockId,
) -> Vec<LocSet> {
    let lb = &lifted.blocks[b];
    let mut live = live_out[b].clone();
    live.extend(flow.term_uses(
        &cfg.blocks[b].term,
        lb.cond.as_ref(),
        lb.switch.as_ref().map(|s| &s.0),
    ));
    let mut after = vec![LocSet::new(); lb.lines.len()];
    for i in (0..lb.lines.len()).rev() {
        after[i] = live.clone();
        let info = flow.info(&lb.lines[i].stmt);
        step_back(&lb.lines[i].stmt, &info, &mut live);
    }
    after
}

/// What is live on entry to the function.
pub fn live_at_entry(flow: &Flow, cfg: &Cfg, lifted: &Lifted, live_out: &[LocSet]) -> LocSet {
    let b = cfg.entry;
    let lb = &lifted.blocks[b];
    let mut live = live_out[b].clone();
    live.extend(flow.term_uses(
        &cfg.blocks[b].term,
        lb.cond.as_ref(),
        lb.switch.as_ref().map(|s| &s.0),
    ));
    for l in lb.lines.iter().rev() {
        let info = flow.info(&l.stmt);
        step_back(&l.stmt, &info, &mut live);
    }
    live
}

/// Run every clean-up to a fixed point.
pub fn clean(rom: &RomImage, f: &Function, cfg: &Cfg, lifted: &mut Lifted, conv: &Conventions) {
    split_pushes(rom, lifted);
    stack_slots(f, cfg, lifted, &conv.stack_args, &conv.stack_params);
    let flow = Flow::new(rom, conv);
    for _ in 0..32 {
        let before = lifted.blocks.iter().map(|b| b.lines.len()).sum::<usize>();
        drop_identities(f, lifted);
        let mut changed = before != lifted.blocks.iter().map(|b| b.lines.len()).sum::<usize>();
        let live_out = liveness(&flow, cfg, lifted);
        changed |= dead_code(&flow, cfg, lifted, &live_out);
        let live_out = liveness(&flow, cfg, lifted);
        changed |= propagate(&flow, cfg, lifted, &live_out);
        if !changed {
            break;
        }
    }
    for b in &mut lifted.blocks {
        for l in &mut b.lines {
            simplify_stmt(&mut l.stmt);
        }
        if let Some(c) = &mut b.cond {
            *c = simplify(c.clone());
        }
    }
    drop_identities(f, lifted);
    let live_out = liveness(&flow, cfg, lifted);
    whole_accumulator(&flow, cfg, lifted, &live_out);
    merge_stores(rom, lifted);
}

/// Byte-by-byte adds and subtracts joined into one of 16 bits.
///
/// 8-bit code adds a 16-bit value a byte at a time, the carry out of the low
/// byte going into the high:
///
/// ```text
/// t = MEM8(p) + MEM8(q);   store t;   hi = MEM8(p + 1) + MEM8(q + 1) + (t > 0xFF);
/// ```
///
/// With `p`, `p + 1` (and `q`, `q + 1`, or a constant, or a byte with no
/// high byte) adjacent, that is `t = MEM16(p) + MEM16(q)` with the high byte
/// `t >> 8`, as C would write it: the low byte is still `t`'s, the high byte
/// is bits 8 to 15 of the sum, and the carry out of it (`> 0xFF`) is the
/// carry out of the 16 bits. The same holds for a subtraction and its
/// borrow. Only where `t` is read nowhere else but as a byte.
pub fn wide_adds(lifted: &mut Lifted) {
    for lb in &mut lifted.blocks {
        for i in 0..lb.lines.len() {
            let Stmt::Assign {
                dst: Place::Temp(t),
                value,
            } = &lb.lines[i].stmt
            else {
                continue;
            };
            let t = *t;
            let Some((op, x_lo, y_lo, carry_in)) = low_half(value) else {
                continue;
            };
            // Where the high byte is computed, and every other read of t
            // takes only its low byte.
            let mut high: Option<(usize, Expr)> = None;
            let mut ok = true;
            for (j, l) in lb.lines.iter().enumerate().skip(i + 1) {
                let mut found = false;
                for_each_expr(&l.stmt, &mut |e| {
                    e.walk(&mut |x| {
                        if let Some(h) = high_half(x, t, op)
                            && high.is_none()
                        {
                            high = Some((j, h.clone()));
                            found = true;
                        }
                    })
                });
                if !found && reads_temp_wide(&l.stmt, t) {
                    ok = false;
                }
            }
            if lb.cond.as_ref().is_some_and(|c| mentions_temp(c, t))
                || lb.switch.as_ref().is_some_and(|(c, _)| mentions_temp(c, t))
            {
                ok = false;
            }
            let Some((j, hi_operands)) = high.filter(|_| ok) else {
                continue;
            };
            let Expr::Bin(_, x_hi, y_hi) = hi_operands else {
                continue;
            };
            let (Some(x16), Some(y16)) = (join(&x_lo, &x_hi), join(&y_lo, &y_hi)) else {
                continue;
            };
            // The high bytes are now read with the low ones: nothing in
            // between may change them.
            let highs: Vec<&Expr> = [&*x_hi, &*y_hi]
                .into_iter()
                .filter_map(|h| match h {
                    Expr::Mem { addr, .. } => Some(&**addr),
                    _ => None,
                })
                .collect();
            let calm = lb.lines[i + 1..j].iter().all(|l| match &l.stmt {
                Stmt::Assign {
                    dst: Place::Mem { addr, .. },
                    ..
                } => !highs.contains(&addr),
                Stmt::Assign { .. } | Stmt::Note(_) => true,
                _ => false,
            });
            if !calm {
                continue;
            }
            let mut wide = Expr::bin(op, x16, y16);
            if let Some(c) = carry_in {
                wide = Expr::bin(op, wide, c);
            }
            lb.lines[i].stmt = Stmt::Assign {
                dst: Place::Temp(t),
                value: wide,
            };
            let shifted = Expr::bin(BinOp::Shr, Expr::Temp(t), Expr::Const(8));
            for_each_expr_mut(&mut lb.lines[j].stmt, &mut |e| {
                *e = replace_high(e.clone(), t, op, &shifted);
            });
        }
    }
}

/// `a ± b [± carry]` of two bytes: the operation, the operands and the
/// carry going in.
fn low_half(e: &Expr) -> Option<(BinOp, Expr, Expr, Option<Expr>)> {
    let byte = |x: &Expr| {
        matches!(
            x,
            Expr::Mem {
                width: Width::W8,
                ..
            } | Expr::Const(0..=0xFF)
        )
    };
    match e {
        Expr::Bin(op @ (BinOp::Add | BinOp::Sub), a, b) => {
            if byte(a) && byte(b) {
                return Some((*op, (**a).clone(), (**b).clone(), None));
            }
            // (a ± b) ± carry
            if let Expr::Bin(inner, x, y) = &**a
                && inner == op
                && byte(x)
                && byte(y)
                && b.is_boolean()
            {
                return Some((*op, (**x).clone(), (**y).clone(), Some((**b).clone())));
            }
            None
        }
        _ => None,
    }
}

/// `x_hi ± y_hi ± (t > 0xFF)`, or `x_hi ± (t > 0xFF)`: the high bytes, as
/// `x_hi op y_hi` (a missing one is 0).
fn high_half(e: &Expr, t: u32, op: BinOp) -> Option<Expr> {
    let carry = |c: &Expr| {
        matches!(c, Expr::Bin(BinOp::Gt, x, k)
            if matches!(**x, Expr::Temp(u) if u == t) && k.as_const() == Some(0xFF))
    };
    let Expr::Bin(o, a, b) = e else {
        return None;
    };
    if *o != op || !carry(b) {
        return None;
    }
    match &**a {
        Expr::Bin(o2, x, y) if *o2 == op => Some(Expr::Bin(op, x.clone(), y.clone())),
        x => Some(Expr::Bin(op, Box::new(x.clone()), Box::new(Expr::Const(0)))),
    }
}

fn replace_high(e: Expr, t: u32, op: BinOp, with: &Expr) -> Expr {
    if high_half(&e, t, op).is_some() {
        return with.clone();
    }
    match e {
        Expr::Mem { addr, width } => Expr::mem(replace_high(*addr, t, op, with), width),
        Expr::Un(o, x) => Expr::Un(o, Box::new(replace_high(*x, t, op, with))),
        Expr::Bin(o, a, b) => Expr::Bin(
            o,
            Box::new(replace_high(*a, t, op, with)),
            Box::new(replace_high(*b, t, op, with)),
        ),
        Expr::Cast(w, x) => Expr::Cast(w, Box::new(replace_high(*x, t, op, with))),
        Expr::Signed(w, x) => Expr::Signed(w, Box::new(replace_high(*x, t, op, with))),
        e => e,
    }
}

/// Two bytes that are one 16-bit value: `MEM8(p)` and `MEM8(p + 1)`, two
/// constants, or a byte and a zero high byte.
fn join(lo: &Expr, hi: &Expr) -> Option<Expr> {
    match (lo, hi) {
        (Expr::Const(l), Expr::Const(h)) => Some(Expr::Const(h << 8 | l)),
        (
            Expr::Mem {
                addr: a,
                width: Width::W8,
            },
            Expr::Mem {
                addr: b,
                width: Width::W8,
            },
        ) => {
            let (ba, oa) = split_address(a);
            let (bb, ob) = split_address(b);
            (ba == bb && ob == oa.wrapping_add(1)).then(|| Expr::mem((**a).clone(), Width::W16))
        }
        (
            lo @ Expr::Mem {
                width: Width::W8, ..
            },
            Expr::Const(0),
        ) => Some(lo.clone()),
        _ => None,
    }
}

fn mentions_temp(e: &Expr, t: u32) -> bool {
    let mut hit = false;
    e.walk(&mut |x| hit |= matches!(x, Expr::Temp(u) if *u == t));
    hit
}

/// Whether a statement reads `t` other than as its low byte (a store of it
/// to a byte, or `(u8)t`).
fn reads_temp_wide(s: &Stmt, t: u32) -> bool {
    let Stmt::Assign { dst, value } = s else {
        let mut hit = false;
        for_each_expr(s, &mut |e| hit |= mentions_temp(e, t));
        return hit;
    };
    if let Place::Mem { addr, .. } = dst
        && mentions_temp(addr, t)
    {
        return true;
    }
    let byte_dst = matches!(
        dst,
        Place::Mem {
            width: Width::W8,
            ..
        } | Place::Reg(Reg::A, Width::W8)
    );
    match value {
        Expr::Temp(u) if *u == t => !byte_dst,
        Expr::Cast(Width::W8, x) if matches!(**x, Expr::Temp(u) if u == t) => false,
        v => mentions_temp(v, t),
    }
}

/// An address as a base and a constant offset: `0x7E0000 + X` is
/// (`X`, 0x7E0000), a plain constant has no base.
fn split_address(e: &Expr) -> (Option<&Expr>, u32) {
    match e {
        Expr::Const(a) => (None, *a),
        Expr::Bin(BinOp::Add, x, y) => match (&**x, &**y) {
            (b, Expr::Const(a)) | (Expr::Const(a), b) => (Some(b), *a),
            _ => (Some(e), 0),
        },
        _ => (Some(e), 0),
    }
}

/// Constant stores that fill adjacent bytes, one after another, become one
/// wider store: `$00 = 0x00; $01 = 0x80; $02 = 0x0E` is the 24-bit pointer
/// `0x0E8000` at `$00`. Never to a hardware register, where each write is
/// its own event.
fn merge_stores(rom: &RomImage, lifted: &mut Lifted) {
    let hardware =
        |a: u32| rom.map().classify(SnesAddress::from_u24(a & 0xFF_FFFF)) == MemoryClass::Hardware;
    // A register loaded with a constant in this run of statements, and how
    // much of it is known (an 8-bit load into A sets only its low byte).
    type Known = BTreeMap<Reg, (u32, Width)>;
    let read = |r: &Reg, w: Width, known: &Known| -> Option<u32> {
        let (v, kw) = known.get(r)?;
        (w <= *kw).then_some(v & w.mask())
    };
    let constant = |e: &Expr, known: &Known| -> Option<u32> {
        match e {
            Expr::Const(v) => Some(*v),
            Expr::Reg(r, w) => read(r, *w, known),
            Expr::Cast(w, x) => match &**x {
                Expr::Reg(r, rw) => read(r, (*w).min(*rw), known),
                _ => None,
            },
            _ => None,
        }
    };
    // A register set to a constant: kept, and seen through.
    let load = |s: &Stmt| -> Option<(Reg, (u32, Width))> {
        match s {
            Stmt::Assign {
                dst: Place::Reg(r, w),
                value: Expr::Const(v),
            } => Some((*r, (*v, *w))),
            _ => None,
        }
    };
    // A constant store's base, first byte and bytes.
    let store = |s: &Stmt, known: &Known| -> Option<(Option<Expr>, u32, Vec<u8>)> {
        let Stmt::Assign {
            dst: Place::Mem { addr, width },
            value,
        } = s
        else {
            return None;
        };
        let v = constant(value, known)?;
        // Not the stack by offset: a merged 24-bit store has no stack form.
        if !matches!(width, Width::W8 | Width::W16) || stack_offset(addr).is_some() {
            return None;
        }
        let (base, off) = split_address(addr);
        if hardware(off) {
            return None;
        }
        let bytes = (0..width.bytes()).map(|i| (v >> (8 * i)) as u8).collect();
        Some((base.cloned(), off, bytes))
    };
    for lb in &mut lifted.blocks {
        let mut out: Vec<Line> = Vec::with_capacity(lb.lines.len());
        let mut known: Known = BTreeMap::new();
        let mut i = 0;
        while i < lb.lines.len() {
            let Some((base, _, _)) = store(&lb.lines[i].stmt, &known) else {
                let line = &lb.lines[i];
                // Follow constants loaded into registers; anything else that
                // writes one forgets it.
                match load(&line.stmt) {
                    Some((r, v)) => {
                        known.insert(r, v);
                    }
                    None => {
                        if let Stmt::Assign {
                            dst: Place::Reg(r, _),
                            ..
                        } = &line.stmt
                        {
                            known.remove(r);
                        } else if !matches!(line.stmt, Stmt::Assign { .. } | Stmt::Note(_)) {
                            known.clear();
                        }
                    }
                }
                out.push(line.clone());
                i += 1;
                continue;
            };
            // The run: stores to the same base, with only notes and
            // constant loads between; those go before the merged store.
            let mut j = i;
            let mut run_known = known.clone();
            let mut bytes: BTreeMap<u32, u8> = BTreeMap::new();
            let mut steps: Vec<usize> = Vec::new();
            let mut notes: Vec<Line> = Vec::new();
            let mut overlap = false;
            let mut stores = 0;
            let mut end = i;
            while j < lb.lines.len() {
                let line = &lb.lines[j];
                if let Some((b, off, bs)) = store(&line.stmt, &run_known)
                    && b == base
                {
                    for (k, v) in bs.iter().enumerate() {
                        overlap |= bytes.insert(off.wrapping_add(k as u32), *v).is_some();
                    }
                    steps.extend(line.steps());
                    stores += 1;
                    j += 1;
                    end = j;
                    continue;
                }
                if matches!(line.stmt, Stmt::Note(_)) {
                    notes.push(line.clone());
                    j += 1;
                    continue;
                }
                if let Some((r, v)) = load(&line.stmt) {
                    run_known.insert(r, v);
                    notes.push(line.clone());
                    j += 1;
                    continue;
                }
                break;
            }
            // What follows the last store is not part of the run.
            let kept = notes.len() - lb.lines[end..j].len();
            notes.truncate(kept);
            let j = end;
            let first = *bytes.keys().next().unwrap();
            let contiguous = bytes
                .keys()
                .enumerate()
                .all(|(n, a)| *a == first + n as u32);
            if stores < 2 || overlap || !contiguous || bytes.len() > 4 {
                out.push(lb.lines[i].clone());
                i += 1;
                continue;
            }
            known = run_known;
            out.extend(notes);
            let values: Vec<u8> = bytes.values().copied().collect();
            // Greedy pieces: a 24-bit store for three bytes, else words.
            let mut at = 0;
            while at < values.len() {
                let left = values.len() - at;
                let n = if left == 3 { 3 } else { left.min(2) };
                let width = match n {
                    1 => Width::W8,
                    2 => Width::W16,
                    _ => Width::W24,
                };
                let v = values[at..at + n]
                    .iter()
                    .enumerate()
                    .fold(0u32, |acc, (k, b)| acc | (*b as u32) << (8 * k));
                let off = Expr::Const(first + at as u32);
                let addr = match &base {
                    Some(b) => Expr::sum(b.clone(), off),
                    None => off,
                };
                out.push(Line {
                    stmt: Stmt::Assign {
                        dst: Place::Mem { addr, width },
                        value: Expr::Const(v),
                    },
                    step: steps[0],
                    merged: steps[1..].to_vec(),
                });
                at += n;
            }
            i = j;
        }
        lb.lines = out;
    }
}

/// Assignments that change nothing: `X = (u8)X` while X is 8 bits wide,
/// `A = (A & 0xFF00) | (u8)A`.
fn drop_identities(f: &Function, lifted: &mut Lifted) {
    for lb in &mut lifted.blocks {
        lb.lines.retain(|l| {
            match &l.stmt {
                Stmt::Assign {
                    dst: Place::Flag(f),
                    value: Expr::Flag(g),
                } if f == g => return false,
                Stmt::Assign {
                    dst: Place::Temp(t),
                    value: Expr::Temp(u),
                } if t == u => return false,
                _ => {}
            }
            let Stmt::Assign {
                dst: Place::Reg(r, dw),
                value,
            } = &l.stmt
            else {
                return true;
            };
            let x8 = f.steps[l.step].insn.flags_before.eff_x();
            let same = match value {
                // X = X.lo is the same X only while X is 8 bits wide.
                Expr::Reg(vr, vw) => {
                    vr == r && (vw == dw || (*r != Reg::A && (*vw == Width::W16 || x8)))
                }
                Expr::Cast(Width::W8, inner) => match &**inner {
                    Expr::Reg(vr, _) if vr == r => {
                        (*r == Reg::A && *dw == Width::W8) || (matches!(r, Reg::X | Reg::Y) && x8)
                    }
                    _ => false,
                },
                _ => false,
            };
            !same
        });
    }
}

/// Drop assignments to locations nothing reads afterwards.
fn dead_code(flow: &Flow, cfg: &Cfg, lifted: &mut Lifted, live_out: &[LocSet]) -> bool {
    let mut changed = false;
    for (b, lb) in lifted.blocks.iter_mut().enumerate() {
        if cfg.blocks[b].is_stub() && lb.lines.is_empty() {
            continue;
        }
        let mut live = live_out[b].clone();
        live.extend(flow.term_uses(
            &cfg.blocks[b].term,
            lb.cond.as_ref(),
            lb.switch.as_ref().map(|s| &s.0),
        ));
        let mut keep = vec![true; lb.lines.len()];
        for (i, line) in lb.lines.iter().enumerate().rev() {
            let info = flow.info(&line.stmt);
            let local = matches!(line.stmt, Stmt::Assign { ref dst, .. } if !matches!(dst, Place::Mem { .. }));
            if local && !info.effect && info.defs.iter().all(|d| !live.contains(d)) {
                keep[i] = false;
                changed = true;
                continue;
            }
            step_back(&line.stmt, &info, &mut live);
        }
        let mut it = keep.iter();
        lb.lines.retain(|_| *it.next().unwrap());
    }
    changed
}

/// The locations an assignment's destination defines, and whether it is a
/// location propagation may carry (not memory).
fn local_defs(dst: &Place) -> Option<Vec<Loc>> {
    match dst {
        Place::Reg(r, w) => Some(Loc::of_reg(*r, *w)),
        Place::Flag(f) => Some(vec![Loc::of_flag(*f)]),
        Place::Temp(t) => Some(vec![Loc::Temp(*t)]),
        _ => None,
    }
}

/// How often a location is read in an expression.
fn count_reads(e: &Expr, defs: &[Loc]) -> usize {
    let mut n = 0;
    e.walk(&mut |x| {
        let hit = match x {
            Expr::Reg(r, w) => Loc::of_reg(*r, *w).iter().any(|l| defs.contains(l)),
            Expr::Flag(f) => defs.contains(&Loc::of_flag(*f)),
            Expr::Temp(t) => defs.contains(&Loc::Temp(*t)),
            _ => false,
        };
        n += hit as usize;
    });
    n
}

fn stmt_reads(s: &Stmt, defs: &[Loc]) -> usize {
    let mut n = 0;
    for_each_expr(s, &mut |e| n += count_reads(e, defs));
    n
}

fn for_each_expr(s: &Stmt, f: &mut impl FnMut(&Expr)) {
    match s {
        Stmt::Assign { dst, value } => {
            if let Place::Mem { addr, .. } = dst {
                f(addr);
            }
            f(value);
        }
        Stmt::Call(CallTarget::Table { index, .. }) => f(index),
        Stmt::Effect(_, args) => args.iter().for_each(f),
        Stmt::Invoke { args, .. } => args.iter().for_each(f),
        _ => {}
    }
}

fn for_each_expr_mut(s: &mut Stmt, f: &mut impl FnMut(&mut Expr)) {
    match s {
        Stmt::Assign { dst, value } => {
            if let Place::Mem { addr, .. } = dst {
                f(addr);
            }
            f(value);
        }
        Stmt::Call(CallTarget::Table { index, .. }) => f(index),
        Stmt::Effect(_, args) => args.iter_mut().for_each(f),
        Stmt::Invoke { args, .. } => args.iter_mut().for_each(f),
        Stmt::Eval(e) => f(e),
        _ => {}
    }
}

/// Replace reads of the location `dst` defines with `v`. `None` when a
/// read cannot take it (a 16-bit read of an accumulator whose high byte
/// the definition did not set).
fn substitute(e: &Expr, dst: &Place, v: &Expr) -> Option<Expr> {
    let rec = |x: &Expr| substitute(x, dst, v);
    Some(match (e, dst) {
        (Expr::Reg(r, w), Place::Reg(dr, dw)) if r == dr => match (w, dw) {
            (Width::W8, _) => Expr::cast(Width::W8, v.clone()),
            // The definition set only the low byte; this reads both.
            (_, Width::W8) if *r == Reg::A => return None,
            // The register held the value truncated to its width.
            _ => Expr::cast(storage(*r), v.clone()),
        },
        (Expr::Flag(f), Place::Flag(df)) if f == df => v.clone(),
        // A temporary is a u32: where the value could go negative as a C
        // int, it stays unsigned.
        (Expr::Temp(t), Place::Temp(dt)) if t == dt => {
            if may_go_negative(v) {
                Expr::Cast(Width::W32, Box::new(v.clone()))
            } else {
                v.clone()
            }
        }
        // The stack by offset keeps S (`substitute_stmt`).
        (Expr::Mem { addr, .. }, Place::Reg(Reg::S, _)) if stack_offset(addr).is_some() => {
            return None;
        }
        (Expr::Mem { addr, width }, _) => Expr::mem(rec(addr)?, *width),
        (Expr::Un(op, x), _) => Expr::un(*op, rec(x)?),
        (Expr::Bin(op, a, b), _) => Expr::bin(*op, rec(a)?, rec(b)?),
        (Expr::Cast(w, x), _) => Expr::cast(*w, rec(x)?),
        (Expr::Signed(w, x), _) => Expr::Signed(*w, Box::new(rec(x)?)),
        (Expr::Call(n, args), _) => Expr::Call(n, args.iter().map(rec).collect::<Option<_>>()?),
        _ => e.clone(),
    })
}

fn substitute_stmt(s: &Stmt, dst: &Place, v: &Expr) -> Option<Stmt> {
    // The stack by offset keeps S as it is: `STACK16(S + 1)` says what
    // `MEM16(0x2000)` hides, and the stack is its own place to the reader.
    // A definition of S those read is not carried into them, and stays.
    if matches!(dst, Place::Reg(Reg::S, _)) && stmt_addresses_stack(s) {
        return None;
    }
    let mut out = s.clone();
    let mut ok = true;
    for_each_expr_mut(&mut out, &mut |e| match substitute(e, dst, v) {
        Some(n) => *e = n,
        None => ok = false,
    });
    ok.then_some(out)
}

/// Whether C, promoting to int, could give `e` a negative value.
fn may_go_negative(e: &Expr) -> bool {
    match e {
        Expr::Bin(BinOp::Sub, ..) | Expr::Un(UnOp::Not, _) | Expr::Signed(..) => true,
        Expr::Bin(BinOp::Add | BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Shl, a, b) => {
            may_go_negative(a) || may_go_negative(b)
        }
        _ => false,
    }
}

/// How wide a register is in `snes.h`.
fn storage(r: Reg) -> Width {
    if r == Reg::Dbr { Width::W8 } else { Width::W16 }
}

/// Whether `v` is cheap and pure enough to copy into more than one read.
fn simple(v: &Expr) -> bool {
    matches!(
        v,
        Expr::Const(_) | Expr::Reg(..) | Expr::Flag(_) | Expr::Temp(_)
    )
}

/// Carry single-use values into their use.
fn propagate(flow: &Flow, cfg: &Cfg, lifted: &mut Lifted, live_out: &[LocSet]) -> bool {
    let mut changed = false;
    let named = &lifted.temp_names;
    for (b, lb) in lifted.blocks.iter_mut().enumerate() {
        let mut i = 0;
        while i < lb.lines.len() {
            // A named temporary (an argument) keeps its name: it is what
            // the reader looks for.
            let keep = matches!(&lb.lines[i].stmt, Stmt::Assign { dst: Place::Temp(t), .. } if named.contains_key(t));
            if !keep && try_propagate(flow, &cfg.blocks[b].term, lb, i, &live_out[b]) {
                changed = true;
            } else {
                i += 1;
            }
        }
    }
    changed
}

fn try_propagate(
    flow: &Flow,
    term: &Term,
    lb: &mut crate::decompile::ir::LiftedBlock,
    i: usize,
    live_out: &LocSet,
) -> bool {
    let Stmt::Assign { dst, value } = &lb.lines[i].stmt else {
        return false;
    };
    let Some(defs) = local_defs(dst) else {
        return false;
    };
    let (dst, v) = (dst.clone(), value.clone());
    let mut vinfo = Info::default();
    flow.expr_info(&v, &mut vinfo);

    // The reads of this definition, up to the next one.
    let mut use_line: Option<usize> = None;
    let mut reads = 0;
    let mut redefined = false;
    let end = lb.lines.len();
    for j in i + 1..=end {
        let (n, info) = if j < end {
            (
                stmt_reads(&lb.lines[j].stmt, &defs),
                flow.info(&lb.lines[j].stmt),
            )
        } else {
            let mut n = 0;
            if let Some(c) = &lb.cond {
                n += count_reads(c, &defs);
            }
            if let Some((s, _)) = &lb.switch {
                n += count_reads(s, &defs);
            }
            let uses = flow.term_uses(term, lb.cond.as_ref(), lb.switch.as_ref().map(|s| &s.0));
            // A read by the terminator that is not in its condition (a
            // return reading A) cannot take a substitution.
            if n == 0 && defs.iter().any(|d| uses.contains(d)) {
                return false;
            }
            (n, Info::default())
        };
        // A read no expression shows (a call's arguments, PHP's flags)
        // cannot take a substitution.
        let implicit = j < end
            && matches!(
                lb.lines[j].stmt,
                Stmt::Call(_)
                    | Stmt::Asm { .. }
                    | Stmt::Effect("PHP" | "mvn" | "mvp" | "mvn8" | "mvp8" | "BRK" | "COP", _)
            );
        if (n == 0 || implicit) && defs.iter().any(|d| info.uses.contains(d)) {
            return false;
        }
        if n > 0 {
            if use_line.is_some() {
                return false;
            }
            use_line = Some(j);
            reads = n;
        }
        if defs.iter().any(|d| info.defs.contains(d)) {
            // Redefined only in part: the rest may still be read later.
            if !defs.iter().all(|d| info.defs.contains(d)) {
                return false;
            }
            redefined = true;
            break;
        }
    }
    let Some(j) = use_line else {
        return false;
    };
    if !redefined && defs.iter().any(|d| live_out.contains(d)) {
        return false;
    }
    if reads > 1 && !simple(&v) {
        return false;
    }
    // Nothing between may change what `v` reads, or reorder its effect.
    for k in i + 1..j.min(end) {
        let info = flow.info(&lb.lines[k].stmt);
        if info.defs.iter().any(|d| vinfo.uses.contains(d)) {
            return false;
        }
        if vinfo.reads_mem && (info.writes_mem || info.effect) {
            return false;
        }
        if vinfo.effect && (info.effect || info.reads_mem) {
            return false;
        }
    }
    if j < end {
        let target = flow.info(&lb.lines[j].stmt);
        if vinfo.effect && target.effect && !matches!(lb.lines[j].stmt, Stmt::Assign { .. }) {
            return false;
        }
        let Some(new) = substitute_stmt(&lb.lines[j].stmt, &dst, &v) else {
            return false;
        };
        lb.lines[j].stmt = new;
        let carried = lb.lines[i].steps();
        lb.lines[j].merged.extend(carried);
    } else {
        if let Some(c) = &lb.cond {
            let Some(n) = substitute(c, &dst, &v) else {
                return false;
            };
            lb.cond = Some(n);
        }
        if let Some((s, vals)) = &lb.switch {
            let Some(n) = substitute(s, &dst, &v) else {
                return false;
            };
            lb.switch = Some((n, vals.clone()));
        }
        let carried = lb.lines[i].steps();
        lb.term_merged.extend(carried);
    }
    lb.lines.remove(i);
    true
}

/// A test names only the instructions whose work it shows. `SBC $0B10;
/// BMI` prints `a = … - ADDR_7E0B10;` then `if ((s16)a < 0)`: the test
/// reads the N flag the SBC set, but the SBC is the line before, so the
/// test is the branch's alone. A `CMP` has no line of its own, so it stays
/// with the test it became.
pub fn tests_own_steps(lifted: &mut Lifted) {
    let printed: BTreeSet<usize> = lifted
        .blocks
        .iter()
        .flat_map(|b| b.lines.iter().map(|l| l.step))
        .collect();
    for b in &mut lifted.blocks {
        b.term_merged.retain(|s| !printed.contains(s));
    }
}

/// Where the high byte is dead after an 8-bit store to the accumulator,
/// store the whole of it: `A = MEM8(0x20);` rather than keeping a byte no
/// one reads.
fn whole_accumulator(flow: &Flow, cfg: &Cfg, lifted: &mut Lifted, live_out: &[LocSet]) {
    for (b, lb) in lifted.blocks.iter_mut().enumerate() {
        let mut live = live_out[b].clone();
        live.extend(flow.term_uses(
            &cfg.blocks[b].term,
            lb.cond.as_ref(),
            lb.switch.as_ref().map(|s| &s.0),
        ));
        let infos: Vec<Info> = lb.lines.iter().map(|l| flow.info(&l.stmt)).collect();
        // Liveness after each line.
        let mut after: Vec<LocSet> = vec![LocSet::new(); lb.lines.len()];
        for i in (0..lb.lines.len()).rev() {
            after[i] = live.clone();
            step_back(&lb.lines[i].stmt, &infos[i], &mut live);
        }
        for (i, line) in lb.lines.iter_mut().enumerate() {
            if let Stmt::Assign {
                dst: Place::Reg(Reg::A, Width::W8),
                value,
            } = &line.stmt
                && !after[i].contains(&Loc::Ah)
            {
                line.stmt = Stmt::Assign {
                    dst: Place::Reg(Reg::A, Width::W16),
                    value: Expr::cast(Width::W8, value.clone()),
                };
            }
        }
    }
}

/// A 16-bit push of a constant or of memory whose two bytes are pulled
/// one at a time straight after, in the same block with only flag updates
/// between: each pull is the byte it takes, low first. `PEA $7E7E; PLB;
/// PLB` is `DBR = 0x7E; DBR = 0x7E`, and dead code keeps the last. Memory
/// is read a byte at a time, so only memory whose reads do nothing else:
/// not hardware registers.
fn split_pushes(rom: &RomImage, lifted: &mut Lifted) {
    let byte = |v: &Expr, k: u32| -> Option<Expr> {
        match v {
            Expr::Const(c) => Some(Expr::Const((c >> (8 * k)) & 0xFF)),
            Expr::Mem {
                addr,
                width: Width::W16,
            } => {
                let a = addr.as_const()?;
                let hw = |a: u32| {
                    rom.map().classify(SnesAddress::from_u24(a & 0xFF_FFFF))
                        == MemoryClass::Hardware
                };
                if hw(a) || hw(a + 1) {
                    return None;
                }
                Some(Expr::mem(Expr::Const(a + k), Width::W8))
            }
            _ => None,
        }
    };
    let pull8 = |s: &Stmt| {
        matches!(
            s,
            Stmt::Assign {
                value: Expr::Call("pull8", _),
                ..
            }
        )
    };
    for block in &mut lifted.blocks {
        let mut i = 0;
        while i < block.lines.len() {
            let Stmt::Effect("push16", args) = &block.lines[i].stmt else {
                i += 1;
                continue;
            };
            let v = args[0].clone();
            // The two pulls, past flag updates.
            let mut pulls = Vec::new();
            let mut j = i + 1;
            while j < block.lines.len() && pulls.len() < 2 {
                match &block.lines[j].stmt {
                    s if pull8(s) => pulls.push(j),
                    Stmt::Assign {
                        dst: Place::Flag(_),
                        ..
                    } => {}
                    _ => break,
                }
                j += 1;
            }
            let (Some(lo), Some(hi)) = (byte(&v, 0), byte(&v, 1)) else {
                i += 1;
                continue;
            };
            if pulls.len() != 2 {
                i += 1;
                continue;
            }
            let push = block.lines[i].step;
            for (&at, b) in pulls.iter().zip([lo, hi]) {
                let line = &mut block.lines[at];
                if let Stmt::Assign { value, .. } = &mut line.stmt {
                    *value = b;
                }
                line.merged.push(push);
            }
            block.lines.remove(i);
        }
    }
}

/// The stack's depth before each line, in bytes the routine pushed: the
/// same on every path, back to zero at every way out, S moved only with
/// nothing pushed.
fn depths(
    f: &Function,
    cfg: &Cfg,
    lifted: &Lifted,
) -> Result<BTreeMap<(BlockId, usize), i32>, &'static str> {
    use Mnemonic::*;
    let moves_s = |step: usize| matches!(f.steps[step].insn.mnemonic, TSC | TSX | TCS | TXS);
    let n = cfg.blocks.len();
    let mut depth_in: Vec<Option<i32>> = vec![None; n];
    depth_in[cfg.entry] = Some(0);
    let mut work = vec![cfg.entry];
    // (block, line) -> depth before the line.
    let mut before: BTreeMap<(BlockId, usize), i32> = BTreeMap::new();
    while let Some(b) = work.pop() {
        let mut d = depth_in[b].unwrap();
        for (i, l) in lifted.blocks[b].lines.iter().enumerate() {
            before.insert((b, i), d);
            // With something pushed, S is not what it was at lift once the
            // slots are temporaries.
            if d != 0 && moves_s(l.step) {
                return Err("it moves S with something pushed");
            }
            d += stack_delta(&l.stmt);
            if d < 0 {
                return Err("it pulls more than it pushed");
            }
        }
        // Every way out leaves the stack as it found it; a push left for
        // whatever runs next (PEA before a computed jump) is not a slot.
        let exits = matches!(
            cfg.blocks[b].term,
            Term::Return | Term::Tail(_) | Term::Unknown(_) | Term::Halt
        );
        let to_stub = cfg.succs(b).iter().any(|&s| cfg.blocks[s].is_stub());
        if (exits || to_stub) && d != 0 {
            return Err("it leaves with bytes still pushed");
        }
        for s in cfg.succs(b) {
            match depth_in[s] {
                None => {
                    depth_in[s] = Some(d);
                    work.push(s);
                }
                Some(old) if old != d => {
                    return Err("the depth of the stack differs between paths");
                }
                Some(_) => {}
            }
        }
    }
    Ok(before)
}

/// Stack effect of a line: pushed bytes (positive) or pulled (negative).
fn stack_delta(s: &Stmt) -> i32 {
    match s {
        Stmt::Effect("push8" | "PHP", _) => 1,
        Stmt::Effect("push16", _) => 2,
        Stmt::Effect("PLP", _) => -1,
        Stmt::Assign {
            value: Expr::Call("pull8", _),
            ..
        } => -1,
        Stmt::Assign {
            value: Expr::Call("pull16", _),
            ..
        } => -2,
        _ => 0,
    }
}

/// The stack by offset a statement addresses: `(k, bytes, written)` for
/// each `S + k` it reads or writes.
fn stack_accesses(s: &Stmt) -> Vec<(u32, u32, bool)> {
    let mut out = Vec::new();
    if let Stmt::Assign {
        dst: Place::Mem { addr, width },
        ..
    } = s
        && let Some(k) = stack_offset(addr)
    {
        out.push((k, width.bytes(), true));
    }
    let mut probe = s.clone();
    for_each_expr_mut(&mut probe, &mut |e| {
        e.walk(&mut |x| {
            if let Expr::Mem { addr, width } = x
                && let Some(k) = stack_offset(addr)
            {
                out.push((k, width.bytes(), false));
            }
        })
    });
    out
}

/// How many bytes of its caller's stack a routine reads or writes as
/// arguments: above the return address (two bytes for `RTS`, three for
/// `RTL`), counted from the last byte the caller pushed. 0 where it
/// addresses none, or the depth cannot be followed.
pub fn stack_args(f: &Function, cfg: &Cfg, lifted: &Lifted) -> u32 {
    let Ok(before) = depths(f, cfg, lifted) else {
        return 0;
    };
    let ret_len: u32 = if f.steps.iter().any(|s| s.insn.mnemonic == Mnemonic::RTL) {
        3
    } else {
        2
    };
    let mut n = 0;
    for (b, block) in lifted.blocks.iter().enumerate() {
        for (i, l) in block.lines.iter().enumerate() {
            let Some(&d) = before.get(&(b, i)) else {
                continue;
            };
            for (k, w, _) in stack_accesses(&l.stmt) {
                let top = k + w - 1;
                if top as i64 > d as i64 + ret_len as i64 {
                    n = n.max(top - d as u32 - ret_len);
                }
            }
        }
    }
    n
}

/// The pieces of its caller's stack a routine reads as arguments, by where
/// each starts at entry (`3` for `LDA $03,S` with nothing pushed) and how
/// many bytes it is, and the first instruction reading one. A routine that
/// writes them, or reads them in overlapping pieces, has none it can name.
fn arg_ranges(
    lifted: &Lifted,
    before: &BTreeMap<(BlockId, usize), i32>,
    ret_len: u32,
) -> Result<(BTreeMap<u32, u32>, Option<usize>), &'static str> {
    let mut ranges: BTreeMap<u32, u32> = BTreeMap::new();
    let mut step = None;
    for (b, block) in lifted.blocks.iter().enumerate() {
        for (i, l) in block.lines.iter().enumerate() {
            let Some(&d) = before.get(&(b, i)) else {
                continue;
            };
            let d = d as u32;
            for (k, w, write) in stack_accesses(&l.stmt) {
                if k <= d + ret_len {
                    continue;
                }
                if write {
                    return Err("it writes arguments its caller pushed");
                }
                if w > 2 {
                    return Err("it reads a long pointer its caller pushed");
                }
                match ranges.insert(k - d, w) {
                    Some(have) if have != w => {
                        return Err("it reads its arguments in overlapping pieces");
                    }
                    _ => {}
                }
                step.get_or_insert(l.step);
            }
        }
    }
    let starts: Vec<(u32, u32)> = ranges.iter().map(|(&e, &w)| (e, w)).collect();
    if starts.windows(2).any(|p| p[0].0 + p[0].1 > p[1].0) {
        return Err("it reads its arguments in overlapping pieces");
    }
    Ok((ranges, step))
}

/// Whether a routine works on what its call left on the stack: it pulls
/// more than it pushed, or reads its own return address (data placed after
/// the call). A call in C leaves nothing there, so its C cannot do the
/// same.
pub fn uses_call_frame(f: &Function, cfg: &Cfg, lifted: &Lifted) -> bool {
    let before = match depths(f, cfg, lifted) {
        Ok(b) => b,
        Err(why) => return why == "it pulls more than it pushed",
    };
    let ret_len: u32 = if f.steps.iter().any(|s| s.insn.mnemonic == Mnemonic::RTL) {
        3
    } else {
        2
    };
    lifted.blocks.iter().enumerate().any(|(b, block)| {
        block.lines.iter().enumerate().any(|(i, l)| {
            let Some(&d) = before.get(&(b, i)) else {
                return false;
            };
            stack_accesses(&l.stmt)
                .iter()
                .any(|&(k, w, _)| (k..k + w).any(|m| m > d as u32 && m <= d as u32 + ret_len))
        })
    })
}

/// A routine's arguments as its callers see them: each piece's offset above
/// the caller's S at the call (the return address taken out, as a call in
/// C leaves none) and its bytes. Empty where it reads none it can name.
pub fn stack_param_ranges(f: &Function, cfg: &Cfg, lifted: &Lifted) -> Vec<(u32, u32)> {
    let Ok(before) = depths(f, cfg, lifted) else {
        return Vec::new();
    };
    let ret_len: u32 = if f.steps.iter().any(|s| s.insn.mnemonic == Mnemonic::RTL) {
        3
    } else {
        2
    };
    match arg_ranges(lifted, &before, ret_len) {
        Ok((ranges, _)) => ranges.into_iter().map(|(e, w)| (e - ret_len, w)).collect(),
        Err(_) => Vec::new(),
    }
}

/// Pushes and pulls that pair at one depth become temporaries.
///
/// Only where the routine never addresses the stack directly, reads or
/// sets `S` only with nothing pushed (a `TCS` that sets the stack up, a
/// `TSX` that looks at it), and the depth agrees on every path and is back
/// to zero at every return. A push nothing in the routine pulls (an
/// argument for a callee, a bank for `PLB` in a different width) stays a
/// push.
pub fn stack_slots(
    f: &Function,
    cfg: &Cfg,
    lifted: &mut Lifted,
    callee_args: &BTreeMap<SnesAddress, u32>,
    params: &BTreeMap<SnesAddress, Vec<(u32, u32)>>,
) {
    let addresses = lifted
        .blocks
        .iter()
        .any(|b| b.lines.iter().any(|l| stmt_addresses_stack(&l.stmt)));
    if let Err(why) = slots(f, cfg, lifted, callee_args, params)
        && addresses
    {
        lifted.warnings.push(format!(
            "the stack is left as memory, `MEM(S + k)`, because {why}"
        ));
    }
}

fn slots(
    f: &Function,
    cfg: &Cfg,
    lifted: &mut Lifted,
    callee_args: &BTreeMap<SnesAddress, u32>,
    params: &BTreeMap<SnesAddress, Vec<(u32, u32)>>,
) -> Result<(), &'static str> {
    use Mnemonic::*;
    // Lines that read or write the stack by offset (`LDA $03,S`).
    let mut ref_lines: BTreeSet<(BlockId, usize)> = BTreeSet::new();
    for (b, block) in lifted.blocks.iter().enumerate() {
        for (i, l) in block.lines.iter().enumerate() {
            if stmt_addresses_stack(&l.stmt) {
                ref_lines.insert((b, i));
            }
        }
    }
    let before = depths(f, cfg, lifted)?;
    let n = cfg.blocks.len();
    // Calls to routines that read their caller's stack: how many bytes.
    let mut call_lines: BTreeMap<(BlockId, usize), (u32, SnesAddress)> = BTreeMap::new();
    for (b, block) in lifted.blocks.iter().enumerate() {
        for (i, l) in block.lines.iter().enumerate() {
            if let Stmt::Call(CallTarget::Direct(a)) = &l.stmt
                && let Some(&k) = callee_args.get(a)
                && k > 0
            {
                call_lines.insert((b, i), (k, *a));
            }
        }
    }
    // Which push put each byte on the stack, followed forward through the
    // graph: a pull pairs with the pushes whose bytes it takes off. A push
    // is 1 or 2 bytes, or P's byte (kind 3); pairs must agree on the kind
    // and line up byte for byte.
    const P: i32 = 3;
    let kind_of = |s: &Stmt| -> Option<(bool, i32)> {
        match s {
            Stmt::Effect("push8", _) => Some((true, 1)),
            Stmt::Effect("push16", _) => Some((true, 2)),
            Stmt::Effect("PHP", _) => Some((true, P)),
            Stmt::Effect("PLP", _) => Some((false, P)),
            Stmt::Assign { .. } if stack_delta(s) < 0 => Some((false, -stack_delta(s))),
            _ => None,
        }
    };
    let bytes_of = |k: i32| if k == 2 { 2 } else { 1 };
    // Push and pull lines, numbered.
    let mut ids: BTreeMap<(BlockId, usize), usize> = BTreeMap::new();
    let mut kinds: Vec<(bool, i32)> = Vec::new();
    for &(b, i) in before.keys() {
        if let Some(k) = kind_of(&lifted.blocks[b].lines[i].stmt) {
            ids.insert((b, i), kinds.len());
            kinds.push(k);
        }
    }
    // The stack as bytes, bottom first: for each, the pushes that may have
    // put it there and which of their bytes it is.
    type Stack = Vec<BTreeSet<(usize, i32)>>;
    let mut parent: Vec<usize> = (0..kinds.len()).collect();
    fn find(p: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut y = x;
        while p[y] != r {
            let n = p[y];
            p[y] = r;
            y = n;
        }
        r
    }
    // Pulls whose bytes did not line up with one kind of push.
    let mut spoiled: BTreeSet<usize> = BTreeSet::new();
    // The stack before each line that addresses it by offset.
    let mut stack_at: BTreeMap<(BlockId, usize), Stack> = BTreeMap::new();
    let mut stack_in: Vec<Option<Stack>> = vec![None; n];
    stack_in[cfg.entry] = Some(Vec::new());
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &cfg.rpo {
            let Some(mut st) = stack_in[b].clone() else {
                continue;
            };
            for (i, l) in lifted.blocks[b].lines.iter().enumerate() {
                if ref_lines.contains(&(b, i)) || call_lines.contains_key(&(b, i)) {
                    stack_at.insert((b, i), st.clone());
                }
                let Some(&id) = ids.get(&(b, i)) else {
                    continue;
                };
                let (push, k) = kinds[id];
                let w = bytes_of(k) as usize;
                if push {
                    for j in 0..w {
                        st.push([(id, j as i32)].into_iter().collect());
                    }
                } else {
                    let base = st.len() - w;
                    for (j, byte) in st.drain(base..).enumerate() {
                        for (p, pj) in byte {
                            if kinds[p].1 != k || pj != j as i32 {
                                spoiled.insert(id);
                                spoiled.insert(p);
                            }
                            let (x, y) = (find(&mut parent, id), find(&mut parent, p));
                            parent[x] = y;
                        }
                    }
                }
                let _ = l;
            }
            for s in cfg.succs(b) {
                let new = match &stack_in[s] {
                    None => st.clone(),
                    Some(old) => old
                        .iter()
                        .zip(&st)
                        .map(|(x, y)| x.union(y).copied().collect())
                        .collect(),
                };
                if stack_in[s].as_ref() != Some(&new) {
                    stack_in[s] = Some(new);
                    changed = true;
                }
            }
        }
    }
    // What a callee reads as its arguments stays on the stack for it: the
    // pushes of the top bytes at the call stay pushes. Where the callee
    // takes them as parameters (the `full` level), they are temporaries
    // passed in the call instead, if they can be; else they stay pushes
    // too, and the call passes the stack's bytes.
    let passes = |line: &(BlockId, usize)| call_lines.get(line).and_then(|(_, a)| params.get(a));
    let mut pushed_for: BTreeSet<(BlockId, usize)> = call_lines
        .keys()
        .filter(|l| passes(l).is_none())
        .copied()
        .collect();
    let (mut next, temp_of) = loop {
        let mut spoiled = spoiled.clone();
        for line in &pushed_for {
            if let Some(st) = stack_at.get(line) {
                for byte in st.iter().rev().take(call_lines[line].0 as usize) {
                    for &(p, _) in byte {
                        spoiled.insert(p);
                    }
                }
            }
        }
        // A group of pushes and pulls becomes one temporary (P's, four)
        // when it has both, all of one kind, and nothing in it was spoiled.
        let mut groups: BTreeMap<usize, (bool, bool, bool)> = BTreeMap::new();
        for (id, kind) in kinds.iter().enumerate() {
            let r = find(&mut parent, id);
            let g = groups.entry(r).or_insert((false, false, false));
            if kind.0 {
                g.0 = true;
            } else {
                g.1 = true;
            }
            if spoiled.contains(&id) {
                g.2 = true;
            }
        }
        let mut next = lifted.temps;
        let mut temp_of: BTreeMap<usize, u32> = BTreeMap::new();
        for (&r, &(pushed, pulled, bad)) in &groups {
            if pushed && pulled && !bad {
                temp_of.insert(r, next + 1);
                next += if kinds[r].1 == P { 4 } else { 1 };
            }
        }
        // A call whose argument bytes are not all temporaries keeps them
        // pushed, and the grouping runs again.
        let mut again = false;
        for line in call_lines.keys() {
            if pushed_for.contains(line) {
                continue;
            }
            let st = stack_at.get(line);
            let all = st.is_some_and(|st| {
                st.iter().rev().take(call_lines[line].0 as usize).count()
                    == call_lines[line].0 as usize
                    && st
                        .iter()
                        .rev()
                        .take(call_lines[line].0 as usize)
                        .all(|byte| {
                            !byte.is_empty()
                                && byte.iter().all(|&(p, _)| {
                                    let r = find(&mut parent, p);
                                    temp_of.contains_key(&r) && kinds[r].1 != P
                                })
                        })
            });
            if !all {
                pushed_for.insert(*line);
                again = true;
            }
        }
        if !again {
            break (next, temp_of);
        }
    };
    let own_params = params.contains_key(&f.entry);
    if temp_of.is_empty() && ref_lines.is_empty() {
        return Ok(());
    }
    // Each line that addresses the stack, rewritten to read and write the
    // temporaries its bytes belong to. If any cannot be, the routine keeps
    // the stack as memory, as it is.
    let mut rewritten: BTreeMap<(BlockId, usize), Vec<Stmt>> = BTreeMap::new();
    // What the call that entered it left: `RTL` returns through three bytes.
    let ret_len: u32 = if f.steps.iter().any(|s| s.insn.mnemonic == RTL) {
        3
    } else {
        2
    };
    // Its arguments: the caller's bytes above the return address, by where
    // they are at entry (`arg3` is at S + 3 as the routine starts), each
    // piece read a variable of its own, read once at entry. A routine that
    // writes them passes something back that way, and keeps the stack.
    let (ranges, arg_step) = arg_ranges(lifted, &before, ret_len)?;
    let mut arg_temp: BTreeMap<u32, (u32, u32)> =
        ranges.iter().map(|(&e, &w)| (e, (0, w))).collect();
    for (t, _) in arg_temp.values_mut() {
        next += 1;
        *t = next;
    }
    for &(b, i) in &ref_lines {
        let (Some(st), Some(&d)) = (stack_at.get(&(b, i)), before.get(&(b, i))) else {
            return Err("a line that reads the stack is never reached");
        };
        let mut why = "a value read more than once has effects";
        // The temporary and byte of the value at `S + m`.
        let mut byte_at = |m: u32| -> Option<(u32, u32, u32)> {
            if m == 0 {
                why = "it reads below the stack pointer";
                return None;
            }
            if m > d as u32 {
                // Above what the routine pushed: the return address, then
                // what its caller pushed.
                let e = m - d as u32;
                if e <= ret_len {
                    why = "it reads its return address (data placed after the call)";
                    return None;
                }
                let (&s, &(t, w)) = arg_temp.range(..=e).next_back()?;
                return (e < s + w).then_some((t, w, e - s));
            }
            let idx = (d as u32 - m) as usize;
            let origins = st.get(idx)?;
            let mut which = None;
            for &(p, j) in origins {
                let r = find(&mut parent, p);
                let Some(&t) = temp_of.get(&r) else {
                    why = "a value it reads there is pushed but never pulled";
                    return None;
                };
                let k = kinds[r].1;
                if k == P {
                    why = "it reads the flags it pushed";
                    return None;
                }
                let wt = bytes_of(k) as u32;
                // A push puts its high byte on first, deeper in the stack.
                let at = (t, wt, wt - 1 - j as u32);
                if which.is_some_and(|w| w != at) {
                    why = "a byte it reads comes from different pushes on different paths";
                    return None;
                }
                which = Some(at);
            }
            which
        };
        match stack_rewrite(&lifted.blocks[b].lines[i].stmt, &mut byte_at) {
            Some(stmts) => {
                rewritten.insert((b, i), stmts);
            }
            None => return Err(why),
        }
    }
    // The calls that pass their arguments: each piece as the temporaries
    // holding its bytes, in a `stack_args` line the `full` level puts in the
    // call.
    for (line, &(_, a)) in &call_lines {
        let Some(pieces) = params.get(&a) else {
            continue;
        };
        if pushed_for.contains(line) {
            continue;
        }
        let (Some(st), Some(&d)) = (stack_at.get(line), before.get(line)) else {
            continue;
        };
        let mut byte_at = |m: u32| -> Option<(u32, u32, u32)> {
            if m == 0 || m > d as u32 {
                return None;
            }
            let mut which = None;
            for &(p, j) in st.get((d as u32 - m) as usize)? {
                let r = find(&mut parent, p);
                let t = *temp_of.get(&r)?;
                let wt = bytes_of(kinds[r].1) as u32;
                let at = (t, wt, wt - 1 - j as u32);
                if which.is_some_and(|w| w != at) {
                    return None;
                }
                which = Some(at);
            }
            which
        };
        let mut args = Vec::new();
        for &(m, w) in pieces {
            let piece = Expr::mem(
                Expr::sum(Expr::Reg(Reg::S, Width::W16), Expr::Const(m)),
                if w == 1 { Width::W8 } else { Width::W16 },
            );
            args.push(stack_read(&piece, &mut byte_at).unwrap_or(piece));
        }
        let (b, i) = *line;
        rewritten.insert(
            *line,
            vec![
                Stmt::Effect("stack_args", args),
                lifted.blocks[b].lines[i].stmt.clone(),
            ],
        );
    }
    let mut renamed: BTreeMap<(BlockId, usize), (u32, i32)> = BTreeMap::new();
    for (&line, &id) in &ids {
        let r = find(&mut parent, id);
        if let Some(&t) = temp_of.get(&r) {
            renamed.insert(line, (t, kinds[id].1));
        }
    }
    lifted.temps = next;
    const PFLAGS: [Flag; 4] = [Flag::N, Flag::V, Flag::Z, Flag::C];
    for (b, block) in lifted.blocks.iter_mut().enumerate() {
        let mut out: Vec<Line> = Vec::with_capacity(block.lines.len());
        for (i, line) in block.lines.drain(..).enumerate() {
            if let Some(stmts) = rewritten.remove(&(b, i)) {
                for stmt in stmts {
                    out.push(Line {
                        stmt,
                        step: line.step,
                        merged: line.merged.clone(),
                    });
                }
                continue;
            }
            let Some(&(t, w)) = renamed.get(&(b, i)) else {
                out.push(line);
                continue;
            };
            let step = line.step;
            let mut put = |stmt| {
                out.push(Line {
                    stmt,
                    step,
                    merged: Vec::new(),
                })
            };
            match &line.stmt {
                Stmt::Effect("push8" | "push16", args) => put(Stmt::Assign {
                    dst: Place::Temp(t),
                    value: Expr::cast(if w == 1 { Width::W8 } else { Width::W16 }, args[0].clone()),
                }),
                Stmt::Effect("PHP", _) => {
                    for (k, f) in PFLAGS.iter().enumerate() {
                        put(Stmt::Assign {
                            dst: Place::Temp(t + k as u32),
                            value: Expr::Flag(*f),
                        });
                    }
                }
                Stmt::Effect("PLP", _) => {
                    for (k, f) in PFLAGS.iter().enumerate() {
                        put(Stmt::Assign {
                            dst: Place::Flag(*f),
                            value: Expr::Temp(t + k as u32),
                        });
                    }
                }
                Stmt::Assign { dst, .. } => put(Stmt::Assign {
                    dst: dst.clone(),
                    value: Expr::Temp(t),
                }),
                _ => out.push(line),
            }
        }
        block.lines = out;
    }
    // Each argument read once, as the routine starts: its caller's bytes
    // just above S there, as a call in C leaves no return address.
    let entry_lines = &mut lifted.blocks[cfg.entry].lines;
    let step = arg_step.unwrap_or(0);
    for (i, (&e, &(t, w))) in arg_temp.iter().enumerate() {
        lifted.temp_names.insert(t, format!("arg{e}"));
        // Taken as a parameter at the `full` level.
        if own_params {
            continue;
        }
        entry_lines.insert(
            i,
            Line {
                stmt: Stmt::Assign {
                    dst: Place::Temp(t),
                    value: Expr::mem(
                        Expr::sum(Expr::Reg(Reg::S, Width::W16), Expr::Const(e - ret_len)),
                        if w == 1 { Width::W8 } else { Width::W16 },
                    ),
                },
                step,
                merged: Vec::new(),
            },
        );
    }
    Ok(())
}

/// `k` when `addr` is `S + k`: the byte `k` above the stack pointer. A
/// stack-relative operand is one byte; a larger constant plus S is an
/// address that happens to add the stack pointer (`TSX` then
/// `DEC $B200,X`), not the stack.
pub(crate) fn stack_offset(addr: &Expr) -> Option<u32> {
    match addr {
        Expr::Bin(BinOp::Add, a, b) => match (&**a, &**b) {
            (Expr::Reg(Reg::S, _), Expr::Const(k)) | (Expr::Const(k), Expr::Reg(Reg::S, _))
                if *k <= 0xFF =>
            {
                Some(*k)
            }
            _ => None,
        },
        _ => None,
    }
}

fn expr_addresses_stack(e: &Expr) -> bool {
    let mut hit = false;
    e.walk(&mut |x| {
        if let Expr::Mem { addr, .. } = x
            && stack_offset(addr).is_some()
        {
            hit = true;
        }
    });
    hit
}

fn stmt_addresses_stack(s: &Stmt) -> bool {
    let mut hit = matches!(s, Stmt::Assign { dst: Place::Mem { addr, .. }, .. } if stack_offset(addr).is_some());
    let mut probe = s.clone();
    for_each_expr_mut(&mut probe, &mut |e| hit |= expr_addresses_stack(e));
    hit
}

/// `e` with every read of the stack by offset replaced by the temporaries
/// holding those bytes. `byte_at(m)` names the byte at `S + m`: its
/// temporary, the temporary's width in bytes and which byte of it.
fn stack_read(e: &Expr, byte_at: &mut dyn FnMut(u32) -> Option<(u32, u32, u32)>) -> Option<Expr> {
    let rec = |x: &Expr, f: &mut dyn FnMut(u32) -> Option<(u32, u32, u32)>| stack_read(x, f);
    Some(match e {
        Expr::Mem { addr, width } if stack_offset(addr).is_some() => {
            let k = stack_offset(addr).unwrap();
            let n = width.bytes();
            let bytes: Vec<(u32, u32, u32)> =
                (0..n).map(|i| byte_at(k + i)).collect::<Option<_>>()?;
            // The whole of one temporary, in order.
            let (t, wt, _) = bytes[0];
            if wt == n
                && bytes
                    .iter()
                    .enumerate()
                    .all(|(i, b)| *b == (t, wt, i as u32))
            {
                return Some(Expr::Temp(t));
            }
            let mut out: Option<Expr> = None;
            for (i, (t, wt, sig)) in bytes.into_iter().enumerate() {
                let byte = if wt == 1 {
                    Expr::Temp(t)
                } else {
                    Expr::cast(
                        Width::W8,
                        Expr::bin(BinOp::Shr, Expr::Temp(t), Expr::Const(8 * sig)),
                    )
                };
                let placed = Expr::bin(BinOp::Shl, byte, Expr::Const(8 * i as u32));
                out = Some(match out {
                    None => placed,
                    Some(o) => Expr::bin(BinOp::Or, o, placed),
                });
            }
            out?
        }
        Expr::Mem { addr, width } => Expr::mem(rec(addr, byte_at)?, *width),
        Expr::Un(op, x) => Expr::un(*op, rec(x, byte_at)?),
        Expr::Bin(op, a, b) => Expr::bin(*op, rec(a, byte_at)?, rec(b, byte_at)?),
        Expr::Cast(w, x) => Expr::cast(*w, rec(x, byte_at)?),
        Expr::Signed(w, x) => Expr::Signed(*w, Box::new(rec(x, byte_at)?)),
        Expr::Call(n, args) => Expr::Call(
            n,
            args.iter()
                .map(|a| rec(a, byte_at))
                .collect::<Option<_>>()?,
        ),
        _ => e.clone(),
    })
}

/// A statement that addresses the stack by offset, rewritten over the
/// temporaries (`stack_read`): a write to part of a temporary sets just
/// those bits of it.
fn stack_rewrite(
    s: &Stmt,
    byte_at: &mut dyn FnMut(u32) -> Option<(u32, u32, u32)>,
) -> Option<Vec<Stmt>> {
    if let Stmt::Assign {
        dst: Place::Mem { addr, width },
        value,
    } = s
        && let Some(k) = stack_offset(addr)
    {
        let value = stack_read(value, byte_at)?;
        let n = width.bytes();
        let bytes: Vec<(u32, u32, u32)> = (0..n).map(|i| byte_at(k + i)).collect::<Option<_>>()?;
        let (t, wt, _) = bytes[0];
        if wt == n
            && bytes
                .iter()
                .enumerate()
                .all(|(i, b)| *b == (t, wt, i as u32))
        {
            return Some(vec![Stmt::Assign {
                dst: Place::Temp(t),
                value: Expr::cast(if n == 1 { Width::W8 } else { Width::W16 }, value),
            }]);
        }
        // Byte by byte; a value read more than once must not have effects.
        if n > 1 && value.has_effects() {
            return None;
        }
        let mut out = Vec::new();
        for (i, (t, wt, sig)) in bytes.into_iter().enumerate() {
            let byte = Expr::cast(
                Width::W8,
                Expr::bin(BinOp::Shr, value.clone(), Expr::Const(8 * i as u32)),
            );
            let v = if wt == 1 {
                byte
            } else {
                let keep = 0xFFFF & !(0xFF << (8 * sig));
                Expr::bin(
                    BinOp::Or,
                    Expr::bin(BinOp::And, Expr::Temp(t), Expr::Const(keep)),
                    Expr::bin(BinOp::Shl, byte, Expr::Const(8 * sig)),
                )
            };
            out.push(Stmt::Assign {
                dst: Place::Temp(t),
                value: v,
            });
        }
        return Some(out);
    }
    let mut out = s.clone();
    let mut ok = true;
    for_each_expr_mut(&mut out, &mut |e| match stack_read(e, byte_at) {
        Some(n) => *e = n,
        None => ok = false,
    });
    ok.then_some(vec![out])
}

fn simplify_stmt(s: &mut Stmt) {
    // What the destination keeps: casts inside modular arithmetic that it
    // truncates anyway say nothing.
    let keep = match s {
        Stmt::Assign { dst, .. } => match dst {
            Place::Reg(Reg::A, Width::W8) => Some(Width::W8),
            Place::Reg(r, _) => Some(storage(*r)),
            Place::Mem { width, .. } if *width != Width::W24 => Some(*width),
            _ => None,
        },
        _ => None,
    };
    if let Stmt::Assign { dst, value } = s {
        if let Place::Mem { addr, .. } = dst {
            *addr = simplify(addr.clone());
        }
        let v = simplify(value.clone());
        *value = match keep {
            Some(w) => narrow(v, w),
            None => v,
        };
        return;
    }
    for_each_expr_mut(s, &mut |e| *e = simplify(e.clone()));
}

/// `e` with the casts removed that cannot matter when the result is
/// truncated to `w`: the low bits of a sum, difference, bitwise operation
/// or left shift depend only on the low bits of its operands.
pub fn narrow(e: Expr, w: Width) -> Expr {
    match e {
        // `LDA #$E000; XBA` folds to 0xE000E0: the store keeps 0x00E0.
        Expr::Const(c) => Expr::Const(c & w.mask()),
        Expr::Cast(cw, x) if cw >= w => narrow(*x, w),
        Expr::Bin(op @ (BinOp::Add | BinOp::Sub | BinOp::And | BinOp::Or | BinOp::Xor), a, b) => {
            Expr::bin(op, narrow(*a, w), narrow(*b, w))
        }
        Expr::Bin(BinOp::Shl, a, b) => Expr::bin(BinOp::Shl, narrow(*a, w), *b),
        e => e,
    }
}

/// Whether `!e` simplifies to a test without a `!` in front of anything
/// longer than a name.
fn negates_plainly(e: &Expr) -> bool {
    match e {
        Expr::Bin(BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, ..) => {
            true
        }
        Expr::Bin(BinOp::LAnd | BinOp::LOr, a, b) => negates_plainly(a) && negates_plainly(b),
        Expr::Un(UnOp::LNot, _) | Expr::Flag(_) => true,
        _ => false,
    }
}

/// Tidy an expression: comparisons for negated comparisons, sign tests for
/// sign-bit masks.
pub fn simplify(e: Expr) -> Expr {
    let e = match e {
        Expr::Mem { addr, width } => Expr::mem(simplify(*addr), width),
        Expr::Un(op, x) => Expr::un(op, simplify(*x)),
        Expr::Bin(op, a, b) => Expr::bin(op, simplify(*a), simplify(*b)),
        Expr::Cast(w, x) => Expr::cast(w, narrow(simplify(*x), w)),
        Expr::Signed(w, x) => Expr::Signed(w, Box::new(simplify(*x))),
        Expr::Call(n, args) => Expr::Call(n, args.into_iter().map(simplify).collect()),
        e => e,
    };
    match e {
        Expr::Un(UnOp::LNot, x) => match *x {
            Expr::Bin(op, a, b) => {
                let flipped = match op {
                    BinOp::Eq => Some(BinOp::Ne),
                    BinOp::Ne => Some(BinOp::Eq),
                    BinOp::Lt => Some(BinOp::Ge),
                    BinOp::Ge => Some(BinOp::Lt),
                    BinOp::Le => Some(BinOp::Gt),
                    BinOp::Gt => Some(BinOp::Le),
                    _ => None,
                };
                match flipped {
                    Some(f) => simplify(Expr::Bin(f, a, b)),
                    // De Morgan, where each side turns without a `!`:
                    // `!(a == 0 || b == 0)` is `a != 0 && b != 0`.
                    None if matches!(op, BinOp::LAnd | BinOp::LOr)
                        && negates_plainly(&a)
                        && negates_plainly(&b) =>
                    {
                        let other = if op == BinOp::LAnd {
                            BinOp::LOr
                        } else {
                            BinOp::LAnd
                        };
                        Expr::Bin(
                            other,
                            Box::new(simplify(Expr::un(UnOp::LNot, *a))),
                            Box::new(simplify(Expr::un(UnOp::LNot, *b))),
                        )
                    }
                    None => Expr::un(UnOp::LNot, Expr::Bin(op, a, b)),
                }
            }
            Expr::Signed(w, inner) => Expr::un(UnOp::LNot, Expr::Signed(w, inner)),
            x => Expr::un(UnOp::LNot, x),
        },
        // (e & sign) != 0  →  (sN)e < 0
        Expr::Bin(op @ (BinOp::Ne | BinOp::Eq), a, b) if matches!(*b, Expr::Const(0)) => {
            if let Expr::Bin(BinOp::And, inner, mask) = &*a
                && let Expr::Const(m) = **mask
                && let Some(w) = [Width::W8, Width::W16].into_iter().find(|w| w.sign() == m)
            {
                let signed = Expr::Signed(w, inner.clone());
                return Expr::Bin(
                    if op == BinOp::Ne {
                        BinOp::Lt
                    } else {
                        BinOp::Ge
                    },
                    Box::new(signed),
                    Box::new(Expr::Const(0)),
                );
            }
            // A boolean compared with zero is the boolean.
            if a.is_boolean() {
                return if op == BinOp::Ne {
                    *a
                } else {
                    simplify(Expr::un(UnOp::LNot, *a))
                };
            }
            Expr::Bin(op, a, b)
        }
        e => e,
    }
}

/// Renumber temporaries through `map`.
pub fn rename_temps(lifted: &mut Lifted, map: &BTreeMap<u32, u32>) {
    lifted.temp_names = std::mem::take(&mut lifted.temp_names)
        .into_iter()
        .filter_map(|(t, n)| map.get(&t).map(|&to| (to, n)))
        .collect();
    fn expr(e: &mut Expr, map: &BTreeMap<u32, u32>) {
        match e {
            Expr::Temp(t) => {
                if let Some(n) = map.get(t) {
                    *t = *n;
                }
            }
            Expr::Mem { addr, .. } => expr(addr, map),
            Expr::Un(_, x) | Expr::Cast(_, x) | Expr::Signed(_, x) => expr(x, map),
            Expr::Bin(_, a, b) => {
                expr(a, map);
                expr(b, map);
            }
            Expr::Call(_, args) => args.iter_mut().for_each(|a| expr(a, map)),
            _ => {}
        }
    }
    let stmt = |s: &mut Stmt| {
        if let Stmt::Assign {
            dst: Place::Temp(t),
            ..
        }
        | Stmt::Invoke {
            ret: Some(Place::Temp(t)),
            ..
        } = s
            && let Some(n) = map.get(t)
        {
            *t = *n;
        }
        for_each_expr_mut(s, &mut |e| expr(e, map));
    };
    for b in &mut lifted.blocks {
        for l in &mut b.lines {
            stmt(&mut l.stmt);
        }
        if let Some(c) = &mut b.cond {
            expr(c, map);
        }
        if let Some((s, _)) = &mut b.switch {
            expr(s, map);
        }
        if let Some(x) = &mut b.exit {
            for s in x.before.iter_mut().chain(x.after.iter_mut()) {
                stmt(s);
            }
            for (_, e) in &mut x.outs {
                expr(e, map);
            }
            if let Some(e) = &mut x.ret {
                expr(e, map);
            }
        }
    }
}
