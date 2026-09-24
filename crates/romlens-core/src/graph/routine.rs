//! A routine's control-flow graph as a picture needs it: the blocks in the
//! order they are reached, the edges with how control takes them, which
//! edges go back up to a loop, and what an execution log saw.

use std::collections::BTreeSet;
use std::ops::Range;

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::decompile::cfg::{BlockId, Cfg, Term};
use crate::decompile::function::{self, Function, FunctionError};
use crate::graph::counts::Counts;
use crate::graph::routine_name;
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::exec_log::FlowKind;
use crate::model::project::Project;
use crate::model::symbols::Symbols;
use crate::rom::image::RomImage;
use crate::viewmodel::asm_lines::{LineIndex, LineKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineGraph {
    pub entry: SnesAddress,
    pub name: String,
    /// Reverse post-order from the entry, so the entry is block 0.
    pub blocks: Vec<GraphBlock>,
    pub edges: Vec<GraphEdge>,
    /// Outermost first.
    pub loops: Vec<GraphLoop>,
    /// Some cycle has more than one way in.
    pub irreducible: bool,
    /// The routine is longer than discovery follows (`function::MAX_INSNS`).
    pub truncated: bool,
    /// Blocks and edges carry an execution log's counts.
    pub counted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphBlock {
    /// The instructions' file offsets, in order; empty for a block that
    /// stands for somewhere outside the routine.
    pub offsets: Vec<FileOffset>,
    /// The first instruction's address.
    pub address: Option<SnesAddress>,
    pub exit: BlockExit,
    /// How many loops it is in.
    pub loop_depth: u32,
    pub loop_header: bool,
    /// How many times the first instruction ran, with a log.
    pub runs: Option<u64>,
}

impl GraphBlock {
    /// The listing lines the block covers: its label and comment lines,
    /// then every line down to its last instruction. `None` for a block
    /// with no instructions.
    pub fn lines(&self, idx: &LineIndex) -> Option<Range<usize>> {
        let first = *self.offsets.first()?;
        let last = *self.offsets.last()?;
        let mut start = idx.line_for_offset(first.0)?;
        let end = idx.line_for_offset(last.0)? + 1;
        while start > 0 {
            let l = idx.lines[start - 1];
            if l.offset == first.0
                && matches!(l.kind, LineKind::Label | LineKind::Comment | LineKind::Note)
            {
                start -= 1;
            } else {
                break;
            }
        }
        Some(start..end.max(start + 1))
    }
}

/// How control leaves a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockExit {
    /// Runs on, or jumps, to one block.
    Next,
    Branch,
    /// Through a jump table.
    Switch {
        table: SnesAddress,
    },
    Return,
    /// `BRK` or `STP`.
    Halt,
    /// Continues in another routine (this block has no instructions).
    Tail {
        target: SnesAddress,
        name: String,
    },
    /// Goes where the analysis cannot follow (no instructions either).
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeKind {
    /// A conditional branch, taken.
    Taken,
    /// A conditional branch, not taken.
    NotTaken,
    /// `JMP`, `BRA`, `BRL`, `JML`.
    Jump,
    /// Runs on into the next block.
    Fall,
    /// A jump table's entry.
    Case,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    pub from: usize,
    pub to: usize,
    pub kind: EdgeKind,
    /// For `Case`: the table entries that go this way.
    pub cases: Vec<u32>,
    /// It goes back up to a block already on the way here: a loop.
    pub back: bool,
    /// How many times control went this way, with a log.
    pub count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLoop {
    pub header: usize,
    /// Sorted, header included.
    pub body: Vec<usize>,
}

/// The graph of the routine entered at `entry`.
pub fn routine_graph(
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    entry: SnesAddress,
) -> Result<RoutineGraph, FunctionError> {
    let entries = function::entries(snap);
    let f = function::discover(rom, snap, &entries, entry)?;
    let counts = project.exec_log.as_deref().map(Counts::new);
    Ok(build(rom, project, snap, &f, counts.as_ref()))
}

pub(crate) fn build(
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    f: &Function,
    counts: Option<&Counts>,
) -> RoutineGraph {
    let symbols = Symbols::new(rom, project, &snap.auto_labels);
    let cfg = Cfg::build(f);
    let n = cfg.blocks.len();
    // Only what the entry reaches; `rpo` leaves the rest out.
    let mut node = vec![usize::MAX; n];
    for (i, &b) in cfg.rpo.iter().enumerate() {
        node[b] = i;
    }
    let back = back_edges(&cfg);
    let first_offset = |b: BlockId| {
        (!cfg.blocks[b].is_stub()).then(|| f.steps[cfg.blocks[b].steps.start].insn.file_offset)
    };
    let last_offset = |b: BlockId| {
        let s = &cfg.blocks[b].steps;
        (!s.is_empty()).then(|| f.steps[s.end - 1].insn.file_offset)
    };

    let mut blocks = Vec::with_capacity(cfg.rpo.len());
    for &b in &cfg.rpo {
        let block = &cfg.blocks[b];
        let offsets: Vec<FileOffset> = block
            .steps
            .clone()
            .map(|i| f.steps[i].insn.file_offset)
            .collect();
        let exit = match &block.term {
            Term::Fall(_) | Term::Goto(_) => BlockExit::Next,
            Term::Branch { .. } => BlockExit::Branch,
            Term::Switch { table, .. } => BlockExit::Switch { table: *table },
            Term::Return => BlockExit::Return,
            Term::Halt => BlockExit::Halt,
            Term::Tail(a) => BlockExit::Tail {
                target: *a,
                name: routine_name(&symbols, *a),
            },
            Term::Unknown(why) => BlockExit::Unknown(why.clone()),
        };
        let loop_depth = cfg.loops.iter().filter(|l| l.body.contains(&b)).count() as u32;
        blocks.push(GraphBlock {
            address: block.steps.clone().next().map(|i| f.steps[i].insn.address),
            offsets,
            exit,
            loop_depth,
            loop_header: cfg.loops.iter().any(|l| l.header == b),
            runs: counts.and_then(|c| first_offset(b).map(|o| c.runs(o.0))),
        });
    }

    let mut edges = Vec::new();
    for &b in &cfg.rpo {
        let from = node[b];
        let last = last_offset(b);
        // Whatever leaves the last instruction and does not branch: every
        // time it ran.
        let straight = || counts.and_then(|c| last.map(|o| c.runs(o.0)));
        let mut push = |to: BlockId, kind: EdgeKind, cases: Vec<u32>, count: Option<u64>| {
            edges.push(GraphEdge {
                from,
                to: node[to],
                kind,
                cases,
                back: back.contains(&(b, to)),
                count,
            });
        };
        match &cfg.blocks[b].term {
            Term::Fall(t) => {
                // A jump out of the routine ends at a stub as a fall.
                let jump = last.is_some_and(|o| {
                    f.index_of(o)
                        .is_some_and(|i| matches!(f.steps[i].transfer, function::Transfer::Jump(_)))
                });
                push(
                    *t,
                    if jump { EdgeKind::Jump } else { EdgeKind::Fall },
                    Vec::new(),
                    straight(),
                );
            }
            Term::Goto(t) => push(*t, EdgeKind::Jump, Vec::new(), straight()),
            Term::Branch { taken, fall } => {
                let at = last.map(|o| o.0);
                let flow = |k: FlowKind| counts.and_then(|c| at.map(|o| c.flow(o, &[k], None)));
                push(*taken, EdgeKind::Taken, Vec::new(), flow(FlowKind::Branch));
                push(
                    *fall,
                    EdgeKind::NotTaken,
                    Vec::new(),
                    flow(FlowKind::BranchNotTaken),
                );
            }
            Term::Switch { cases, .. } => {
                let mut seen: Vec<BlockId> = Vec::new();
                for &t in cases {
                    if seen.contains(&t) {
                        continue;
                    }
                    seen.push(t);
                    let which: Vec<u32> = cases
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| **c == t)
                        .map(|(i, _)| i as u32)
                        .collect();
                    let count = counts.and_then(|c| {
                        let at = last?.0;
                        let to = first_offset(t).map(|o| o.0);
                        // A case that leaves the routine has no offset here.
                        to.map(|to| c.flow(at, &[FlowKind::IndirectJump, FlowKind::Jump], Some(to)))
                    });
                    push(t, EdgeKind::Case, which, count);
                }
            }
            Term::Return | Term::Halt | Term::Tail(_) | Term::Unknown(_) => {}
        }
    }

    let loops = cfg
        .loops
        .iter()
        .map(|l| {
            let mut body: Vec<usize> = l
                .body
                .iter()
                .map(|&b| node[b])
                .filter(|&i| i != usize::MAX)
                .collect();
            body.sort_unstable();
            GraphLoop {
                header: node[l.header],
                body,
            }
        })
        .collect();

    RoutineGraph {
        entry: f.entry,
        name: routine_name(&symbols, f.entry),
        blocks,
        edges,
        loops,
        irreducible: cfg.irreducible,
        truncated: f.truncated,
        counted: counts.is_some(),
    }
}

/// The edges a depth-first walk from the entry finds going to a block still
/// on its stack: every loop's way back up, reducible or not.
fn back_edges(cfg: &Cfg) -> BTreeSet<(BlockId, BlockId)> {
    let n = cfg.blocks.len();
    let mut out = BTreeSet::new();
    if n == 0 {
        return out;
    }
    let succs: Vec<Vec<BlockId>> = (0..n).map(|b| cfg.succs(b)).collect();
    let mut state = vec![0u8; n];
    let mut stack = vec![(cfg.entry, 0usize)];
    state[cfg.entry] = 1;
    while let Some(&mut (b, ref mut next)) = stack.last_mut() {
        if let Some(&s) = succs[b].get(*next) {
            *next += 1;
            match state[s] {
                0 => {
                    state[s] = 1;
                    stack.push((s, 0));
                }
                1 => {
                    out.insert((b, s));
                }
                _ => {}
            }
        } else {
            state[b] = 2;
            stack.pop();
        }
    }
    out
}
