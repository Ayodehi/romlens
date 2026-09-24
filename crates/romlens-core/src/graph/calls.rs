//! The routines around one routine in the call graph: who calls it, and
//! what it calls.
//!
//! Callers come from turning every routine's callees around, so the two
//! lists always agree: a table call, a tail call or a call only the
//! execution log saw counts the same from either end.

use std::collections::BTreeMap;

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::decompile::function::{self, Callee, Dest, Function, FunctionError, Transfer};
use crate::graph::counts::Counts;
use crate::graph::routine_name;
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::exec_log::FlowKind;
use crate::model::project::Project;
use crate::model::symbols::Symbols;
use crate::model::xref::XRefKind;
use crate::rom::image::RomImage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallNeighbourhood {
    pub entry: SnesAddress,
    pub name: String,
    /// By entry address.
    pub callers: Vec<CallLink>,
    /// By entry address.
    pub callees: Vec<CallLink>,
    /// Counts come from an execution log.
    pub counted: bool,
}

/// One routine on the other end, and every place the two meet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallLink {
    pub entry: SnesAddress,
    pub name: String,
    /// In file order.
    pub sites: Vec<CallSite>,
}

impl CallLink {
    /// The calls the log saw, over every site.
    pub fn count(&self) -> Option<u64> {
        self.sites.iter().map(|s| s.count).sum()
    }
}

/// The instruction that makes a call, in the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    pub offset: FileOffset,
    pub address: SnesAddress,
    pub how: CallHow,
    /// How many times it went to this routine, with a log.
    pub count: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CallHow {
    /// `JSR` or `JSL`.
    Call,
    /// `JSR (abs,X)` through a table the analysis resolved.
    Table,
    /// A jump or branch into another routine's entry, or a jump table's
    /// entry that is one.
    Tail,
    /// Through a pointer: only an execution log saw where it went.
    Observed,
}

/// The routine entered at `entry` with its callers and callees.
pub fn call_neighbourhood(
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    entry: SnesAddress,
) -> Result<CallNeighbourhood, FunctionError> {
    let entries = function::entries(snap);
    let f = function::discover(rom, snap, &entries, entry)?;
    let counts = project.exec_log.as_deref().map(Counts::new);
    let symbols = Symbols::new(rom, project, &snap.auto_labels);

    let group = |pairs: Vec<(SnesAddress, CallSite)>| -> Vec<CallLink> {
        let mut by: BTreeMap<SnesAddress, Vec<CallSite>> = BTreeMap::new();
        for (a, s) in pairs {
            let sites = by.entry(a).or_default();
            if !sites.iter().any(|x| x.offset == s.offset && x.how == s.how) {
                sites.push(s);
            }
        }
        by.into_iter()
            .map(|(a, mut sites)| {
                sites.sort_by_key(|s| (s.offset, s.how));
                CallLink {
                    entry: a,
                    name: routine_name(&symbols, a),
                    sites,
                }
            })
            .collect()
    };

    let own = callees_of(rom, snap, &f, counts.as_ref());
    let mut callers: Vec<(SnesAddress, CallSite)> = Vec::new();
    for &e in &entries {
        let g = if e == f.entry {
            f.clone()
        } else {
            match function::discover(rom, snap, &entries, e) {
                Ok(g) => g,
                Err(_) => continue,
            }
        };
        for (to, site) in callees_of(rom, snap, &g, counts.as_ref()) {
            if to == f.entry {
                callers.push((g.entry, site));
            }
        }
    }
    Ok(CallNeighbourhood {
        entry: f.entry,
        name: routine_name(&symbols, f.entry),
        callers: group(callers),
        callees: group(own),
        counted: counts.is_some(),
    })
}

/// Every routine `f` hands control to, one pair per site and target.
pub fn callees_of(
    rom: &RomImage,
    snap: &AnalysisSnapshot,
    f: &Function,
    counts: Option<&Counts>,
) -> Vec<(SnesAddress, CallSite)> {
    let mut out = Vec::new();
    let offset_of = |a: SnesAddress| rom.file_offset_for(a).map(|o| o.0);
    for s in &f.steps {
        let at = s.insn.file_offset;
        let site = |how: CallHow, count: Option<u64>| CallSite {
            offset: at,
            address: s.insn.address,
            how,
            count,
        };
        let runs = || counts.map(|c| c.runs(at.0));
        let flow = |kinds: &[FlowKind], to: SnesAddress| {
            counts.map(|c| c.flow(at.0, kinds, offset_of(to)))
        };
        let mut indirect = false;
        match &s.transfer {
            Transfer::Call { callee, .. } => match callee {
                Callee::Direct(a) => out.push((*a, site(CallHow::Call, runs()))),
                Callee::Table { targets, .. } => {
                    let mut seen = Vec::new();
                    for &t in targets {
                        let t = Project::canonical(rom, t);
                        if !seen.contains(&t) {
                            seen.push(t);
                            out.push((
                                t,
                                site(
                                    CallHow::Table,
                                    flow(&[FlowKind::IndirectCall, FlowKind::Call], t),
                                ),
                            ));
                        }
                    }
                }
                Callee::Indirect => indirect = true,
            },
            Transfer::Jump(d) | Transfer::Next(d) => match d {
                Dest::Tail(a) => out.push((*a, site(CallHow::Tail, runs()))),
                Dest::Unknown(_) => indirect = true,
                Dest::Local(_) => {}
            },
            Transfer::Branch { taken, .. } => {
                if let Dest::Tail(a) = taken {
                    let count = counts.map(|c| c.flow(at.0, &[FlowKind::Branch], None));
                    out.push((*a, site(CallHow::Tail, count)));
                }
            }
            Transfer::Switch { cases, .. } => {
                let mut seen = Vec::new();
                for d in cases {
                    if let Dest::Tail(a) = d
                        && !seen.contains(a)
                    {
                        seen.push(*a);
                        out.push((
                            *a,
                            site(
                                CallHow::Tail,
                                flow(&[FlowKind::IndirectJump, FlowKind::Jump], *a),
                            ),
                        ));
                    }
                }
            }
            Transfer::Return | Transfer::Halt => {}
        }
        if indirect {
            // Only what an emulator saw: the log's own transfers, and the
            // references an imported trace made.
            let mut seen: Vec<SnesAddress> = Vec::new();
            for x in snap.xrefs_from(at) {
                let local = x.to_offset.is_some_and(|o| f.index_of(o).is_some());
                if x.observed
                    && !local
                    && matches!(x.kind, XRefKind::Call | XRefKind::Jump)
                    && !seen.contains(&x.to)
                {
                    seen.push(x.to);
                }
            }
            if let Some(c) = counts {
                for fl in c.flows_from(at.0) {
                    if matches!(
                        fl.kind,
                        FlowKind::IndirectCall
                            | FlowKind::IndirectJump
                            | FlowKind::Call
                            | FlowKind::Jump
                    ) && let Some(o) = fl.to_offset
                        && let Some(a) = rom.snes_address_for(FileOffset(o))
                    {
                        let a = Project::canonical(rom, a);
                        // A jump within the routine is not a call.
                        if f.index_of(FileOffset(o)).is_none() && !seen.contains(&a) {
                            seen.push(a);
                        }
                    }
                }
            }
            for a in seen {
                let count = flow(
                    &[
                        FlowKind::IndirectCall,
                        FlowKind::IndirectJump,
                        FlowKind::Call,
                        FlowKind::Jump,
                    ],
                    a,
                );
                out.push((a, site(CallHow::Observed, count)));
            }
        }
    }
    out
}
