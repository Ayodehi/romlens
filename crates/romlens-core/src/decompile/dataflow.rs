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
}

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
        match t {
            CallTarget::Direct(a) => self
                .calls
                .get(a)
                .cloned()
                .unwrap_or_else(|| self.default_call.clone()),
            _ => self.default_call.clone(),
        }
    }

    fn call_defs(&self, t: &CallTarget) -> LocSet {
        match t {
            CallTarget::Direct(a) => self.call_defs.get(a).cloned().unwrap_or_else(all_fixed),
            _ => all_fixed(),
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
                    "PHP" => {
                        i.uses.extend([Loc::S, Loc::N, Loc::V, Loc::Z, Loc::C]);
                        i.defs.insert(Loc::S);
                        i.writes_mem = true;
                    }
                    "PLP" => {
                        i.uses.insert(Loc::S);
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
        }
        i
    }

    /// What a block's terminator reads.
    fn term_uses(&self, term: &Term, cond: Option<&Expr>, switch: Option<&Expr>) -> LocSet {
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
            Term::Return => i.uses.extend(self.conv.exit.iter().copied()),
            Term::Tail(a) => i.uses.extend(self.conv.call_uses(&CallTarget::Direct(*a))),
            Term::Unknown(_) => i.uses.extend(all_fixed()),
            Term::Fall(_) | Term::Goto(_) | Term::Halt => {}
        }
        i.uses
    }
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
            for i in infos[b].iter().rev() {
                for d in &i.defs {
                    live.remove(d);
                }
                live.extend(i.uses.iter().copied());
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

/// Run every clean-up to a fixed point.
pub fn clean(rom: &RomImage, f: &Function, cfg: &Cfg, lifted: &mut Lifted, conv: &Conventions) {
    let flow = Flow::new(rom, conv);
    stack_slots(f, cfg, lifted);
    for _ in 0..32 {
        let live_out = liveness(&flow, cfg, lifted);
        let mut changed = dead_code(&flow, cfg, lifted, &live_out);
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
}

/// Assignments that change nothing: `X = (u8)X` while X is 8 bits wide,
/// `A = (A & 0xFF00) | (u8)A`.
fn drop_identities(f: &Function, lifted: &mut Lifted) {
    for lb in &mut lifted.blocks {
        lb.lines.retain(|l| {
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
            for d in &info.defs {
                live.remove(d);
            }
            live.extend(info.uses.iter().copied());
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
        (Expr::Temp(t), Place::Temp(dt)) if t == dt => v.clone(),
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
            for d in &infos[i].defs {
                live.remove(d);
            }
            live.extend(infos[i].uses.iter().copied());
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
/// Only where the routine never reads or sets `S`, never addresses the
/// stack directly, and the depth agrees on every path and is back to zero
/// at every return. A push nothing in the routine pulls (an argument for a
/// callee, a bank for `PLB` in a different width) stays a push.
fn stack_slots(f: &Function, cfg: &Cfg, lifted: &mut Lifted) {
    use Mnemonic::*;
    let touches_s = f.steps.iter().any(|s| {
        matches!(s.insn.mnemonic, TSC | TSX | TCS | TXS)
            || matches!(
                s.insn.mode,
                AddressingMode::StackRelative | AddressingMode::StackRelativeIndirectIndexed
            )
    });
    if touches_s {
        return;
    }
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
    // Slots by their lowest byte's depth: which widths push and pull there.
    let mut pushes: BTreeMap<i32, BTreeSet<i32>> = BTreeMap::new();
    let mut pulls: BTreeMap<i32, BTreeSet<i32>> = BTreeMap::new();
    for (&(b, i), &d) in &before {
        let s = &lifted.blocks[b].lines[i].stmt;
        match s {
            Stmt::Effect("push8" | "push16", _) => {
                pushes.entry(d).or_default().insert(delta(s));
            }
            Stmt::Effect("PHP", _) | Stmt::Effect("PLP", _) => {
                // P stays on the real stack; its slot is never renamed.
                pushes.entry(d.min(d + delta(s))).or_default().insert(0);
            }
            Stmt::Assign { .. } if delta(s) < 0 => {
                pulls.entry(d + delta(s)).or_default().insert(-delta(s));
            }
            _ => {}
        }
    }
    let renamed: BTreeMap<i32, (u32, Width)> = pushes
        .iter()
        .filter(|(d, widths)| {
            widths.len() == 1 && !widths.contains(&0) && pulls.get(d).is_some_and(|p| p == *widths)
        })
        .enumerate()
        .map(|(k, (d, widths))| {
            let w = if widths.contains(&1) {
                Width::W8
            } else {
                Width::W16
            };
            (*d, (lifted.temps + 1 + k as u32, w))
        })
        .collect();
    // Overlapping slots (a word pushed where a byte is pulled) were refused
    // above by the width check; a slot must also not straddle another.
    for (&d, &(_, w)) in &renamed {
        if w == Width::W16 && (pushes.contains_key(&(d + 1)) || pulls.contains_key(&(d + 1))) {
            return;
        }
    }
    if renamed.is_empty() {
        return;
    }
    lifted.temps += renamed.len() as u32;
    for (&(b, i), &d) in &before {
        let line: &mut Line = &mut lifted.blocks[b].lines[i];
        match &line.stmt {
            Stmt::Effect(name @ ("push8" | "push16"), args) => {
                if let Some(&(t, w)) = renamed.get(&d) {
                    let _ = name;
                    line.stmt = Stmt::Assign {
                        dst: Place::Temp(t),
                        value: Expr::cast(w, args[0].clone()),
                    };
                }
            }
            Stmt::Assign { dst, value } if delta(&line.stmt) < 0 => {
                let slot = d + delta(&line.stmt);
                if let Some(&(t, _)) = renamed.get(&slot) {
                    let _ = value;
                    line.stmt = Stmt::Assign {
                        dst: dst.clone(),
                        value: Expr::Temp(t),
                    };
                }
            }
            _ => {}
        }
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
