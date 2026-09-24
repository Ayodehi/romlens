//! Data flow over the lifted IR: what the `clean` level adds.
//!
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

use crate::cpu65816::{AddressingMode, Mnemonic};
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
                    "mvn" | "mvp" => {
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
    stack_slots(f, cfg, lifted);
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
        if !matches!(width, Width::W8 | Width::W16) {
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
                Expr::Reg(vr, vw) => vr == r && (vw == dw || *r != Reg::A),
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
        Place::Mem { .. } => None,
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
    for (b, lb) in lifted.blocks.iter_mut().enumerate() {
        let mut i = 0;
        while i < lb.lines.len() {
            if try_propagate(flow, &cfg.blocks[b].term, lb, i, &live_out[b]) {
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
                    | Stmt::Effect("PHP" | "mvn" | "mvp" | "BRK" | "COP", _)
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

/// Pushes and pulls that pair at one depth become temporaries.
///
/// Only where the routine never addresses the stack directly, reads or
/// sets `S` only with nothing pushed (a `TCS` that sets the stack up, a
/// `TSX` that looks at it), and the depth agrees on every path and is back
/// to zero at every return. A push nothing in the routine pulls (an
/// argument for a callee, a bank for `PLB` in a different width) stays a
/// push.
pub fn stack_slots(f: &Function, cfg: &Cfg, lifted: &mut Lifted) {
    use Mnemonic::*;
    let stack_relative = f.steps.iter().any(|s| {
        matches!(
            s.insn.mode,
            AddressingMode::StackRelative | AddressingMode::StackRelativeIndirectIndexed
        )
    });
    if stack_relative {
        return;
    }
    let moves_s = |step: usize| matches!(f.steps[step].insn.mnemonic, TSC | TSX | TCS | TXS);
    // Stack effect of a line: pushed bytes (positive) or pulled (negative).
    fn delta(s: &Stmt) -> i32 {
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
                return;
            }
            d += delta(&l.stmt);
            if d < 0 {
                return;
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
            return;
        }
        for s in cfg.succs(b) {
            match depth_in[s] {
                None => {
                    depth_in[s] = Some(d);
                    work.push(s);
                }
                Some(old) if old != d => return,
                Some(_) => {}
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
            Stmt::Assign { .. } if delta(s) < 0 => Some((false, -delta(s))),
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
    // A group of pushes and pulls becomes one temporary (P's, four) when it
    // has both, all of one kind, and nothing in it was spoiled.
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
    if temp_of.is_empty() {
        return;
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
        Expr::Cast(cw, x) if cw >= w => narrow(*x, w),
        Expr::Bin(op @ (BinOp::Add | BinOp::Sub | BinOp::And | BinOp::Or | BinOp::Xor), a, b) => {
            Expr::bin(op, narrow(*a, w), narrow(*b, w))
        }
        Expr::Bin(BinOp::Shl, a, b) => Expr::bin(BinOp::Shl, narrow(*a, w), *b),
        e => e,
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
    for b in &mut lifted.blocks {
        for l in &mut b.lines {
            if let Stmt::Assign {
                dst: Place::Temp(t),
                ..
            } = &mut l.stmt
                && let Some(n) = map.get(t)
            {
                *t = *n;
            }
            for_each_expr_mut(&mut l.stmt, &mut |e| expr(e, map));
        }
        if let Some(c) = &mut b.cond {
            expr(c, map);
        }
        if let Some((s, _)) = &mut b.switch {
            expr(s, map);
        }
    }
}
