//! What each routine reads, writes and returns, over the whole program.
//!
//! - **Writes** is the least fixed point: a routine writes what its own
//!   statements write, plus what its callees and tail calls write.
//! - **Reads** is what is live at its entry, and **returns** is what its
//!   callers read after it returns (only what it writes). Both start from the
//!   assumption `dataflow::Conventions::default()` makes (every register and
//!   the carry) and shrink together until nothing changes, so every step is
//!   a safe over-approximation.
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
use crate::decompile::ir::{CallTarget, Stmt};
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
}

/// Every routine and its summary.
pub struct Program {
    pub entries: BTreeSet<SnesAddress>,
    pub units: BTreeMap<SnesAddress, Unit>,
    pub summaries: BTreeMap<SnesAddress, Summary>,
}

const ROUNDS: usize = 24;

impl Program {
    pub fn build(rom: &RomImage, snap: &AnalysisSnapshot, opts: LiftOptions) -> Program {
        let entries = function::entries(snap);
        let mut units = BTreeMap::new();
        for &e in &entries {
            if let Ok(f) = function::discover(rom, snap, &entries, e) {
                let cfg = Cfg::build(&f);
                let mut lifted = lift::lift(&f, &cfg, opts);
                dataflow::stack_slots(&f, &cfg, &mut lifted);
                units.insert(f.entry, Unit { f, cfg, lifted });
            }
        }
        let open = open_routines(snap, &units);
        let preserves: BTreeMap<SnesAddress, LocSet> =
            units.iter().map(|(a, u)| (*a, preserved(u))).collect();
        let base = Conventions::default();

        // Writes, growing from nothing.
        let mut writes: BTreeMap<SnesAddress, LocSet> =
            units.keys().map(|&a| (a, LocSet::new())).collect();
        for _ in 0..ROUNDS {
            let mut changed = false;
            let conv = Conventions {
                call_defs: writes.clone(),
                ..base.clone()
            };
            let flow = Flow::new(rom, &conv);
            for (a, u) in &units {
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
                break;
            }
        }

        // Reads and returns, shrinking from the default.
        let mut reads: BTreeMap<SnesAddress, LocSet> = units
            .keys()
            .map(|&a| (a, base.default_call.clone()))
            .collect();
        let mut returns: BTreeMap<SnesAddress, LocSet> = units
            .keys()
            .map(|&a| (a, base.exit.intersection(&writes[&a]).copied().collect()))
            .collect();
        for _ in 0..ROUNDS {
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
                };
                let flow = Flow::new(rom, &conv);
                let live_out = dataflow::liveness(&flow, &u.cfg, &u.lifted);
                let mut r = dataflow::live_at_entry(&flow, &u.cfg, &u.lifted, &live_out);
                r.retain(|l| !matches!(l, Loc::Temp(_)));
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
                    let seen = if open.contains(a) {
                        base.exit.clone()
                    } else {
                        read_after[a].clone()
                    };
                    let mut r: LocSet = seen.intersection(&writes[a]).copied().collect();
                    r.retain(|l| !matches!(l, Loc::Temp(_)));
                    (*a, r)
                })
                .collect();
            let done = new_reads == reads && new_returns == returns;
            reads = new_reads;
            returns = new_returns;
            if done {
                break;
            }
        }

        let summaries = units
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
                    },
                )
            })
            .collect();
        Program {
            entries,
            units,
            summaries,
        }
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
            exit: self
                .summaries
                .get(&at)
                .map(|s| s.returns.union(&s.preserves).copied().collect())
                .unwrap_or(base.exit),
            calls,
            default_call: base.default_call,
            call_defs,
        }
    }
}

/// Registers and flags saved to a slot in the entry block before anything
/// changes them, and restored from it last in every returning block.
fn preserved(u: &Unit) -> LocSet {
    use crate::decompile::ir::{Expr, Place};
    let flow_locs = |e: &Expr| -> Option<LocSet> {
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
    let place_locs = |p: &Place| -> Option<LocSet> {
        match p {
            Place::Reg(r, w) => Some(reg_locs(*r, *w)),
            Place::Flag(f) => Some([flag_loc(*f)].into_iter().collect()),
            _ => None,
        }
    };
    // How often each temporary is written: a save must be its only write,
    // or the slot may hold something else by the time it is restored.
    let mut writes_of: BTreeMap<u32, usize> = BTreeMap::new();
    for lb in &u.lifted.blocks {
        for l in &lb.lines {
            if let Stmt::Assign {
                dst: Place::Temp(t),
                ..
            } = &l.stmt
            {
                *writes_of.entry(*t).or_default() += 1;
            }
        }
    }
    // Saved: temp -> the entry values it holds.
    let mut saved: BTreeMap<u32, LocSet> = BTreeMap::new();
    let mut defined = LocSet::new();
    let entry = &u.lifted.blocks[u.cfg.entry];
    for l in &entry.lines {
        if let Stmt::Assign {
            dst: Place::Temp(t),
            value,
        } = &l.stmt
            && let Some(locs) = flow_locs(value)
            && locs.is_disjoint(&defined)
            && writes_of.get(t) == Some(&1)
        {
            saved.insert(*t, locs);
        }
        if let Stmt::Assign { dst, .. } = &l.stmt
            && let Some(locs) = place_locs(dst)
        {
            defined.extend(locs);
        }
        if matches!(l.stmt, Stmt::Call(_) | Stmt::Asm { .. }) {
            break;
        }
    }
    if saved.is_empty() {
        return LocSet::new();
    }
    let mut out: Option<LocSet> = None;
    for (b, block) in u.cfg.blocks.iter().enumerate() {
        match block.term {
            Term::Return => {}
            Term::Halt => continue,
            // Leaving another way, nothing is known to be restored.
            Term::Tail(_) | Term::Unknown(_) => return LocSet::new(),
            _ => continue,
        }
        // Where the last definition of a location in this block is the
        // restore of its save.
        let mut here = LocSet::new();
        let mut seen = LocSet::new();
        for l in u.lifted.blocks[b].lines.iter().rev() {
            let Stmt::Assign { dst, value } = &l.stmt else {
                if matches!(l.stmt, Stmt::Call(_) | Stmt::Asm { .. }) {
                    break;
                }
                continue;
            };
            let Some(locs) = place_locs(dst) else {
                continue;
            };
            for loc in locs {
                if seen.insert(loc)
                    && let Expr::Temp(t) = value
                    && saved.get(t).is_some_and(|s| s.contains(&loc))
                {
                    here.insert(loc);
                }
            }
        }
        out = Some(match out {
            None => here,
            Some(o) => o.intersection(&here).copied().collect(),
        });
    }
    out.unwrap_or_default()
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
    let reads = name(&s.reads);
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
