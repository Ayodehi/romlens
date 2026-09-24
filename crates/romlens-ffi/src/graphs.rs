//! The Graph tab's records (docs/19): a routine's blocks and edges, its
//! callers and callees, and the layered layout the shells draw them with.

use romlens_core::LineIndex;
use romlens_core::graph::{self, BlockExit, CallHow, EdgeKind, layout::EdgeSpec, layout::NodeSize};

/// How a block leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GraphExitKind {
    /// Runs on, or jumps, to one block.
    Next,
    Branch,
    /// Through a jump table.
    Switch,
    Return,
    /// `BRK` or `STP`.
    Halt,
    /// Continues in another routine: a block with no instructions.
    Tail,
    /// Goes where the analysis cannot follow: no instructions either.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GraphEdgeKind {
    Taken,
    NotTaken,
    Jump,
    Fall,
    Case,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GraphBlockInfo {
    /// The instructions' file offsets, in order; empty for a tail call or
    /// an unknown destination.
    pub offsets: Vec<u32>,
    /// The first instruction's SNES address.
    pub address: Option<u32>,
    pub exit: GraphExitKind,
    /// For `Tail`, the routine; for `Switch`, the table.
    pub exit_target: Option<u32>,
    /// For `Tail`, the routine's name; for `Unknown`, why.
    pub exit_text: String,
    pub loop_depth: u32,
    pub loop_header: bool,
    /// How many times the first instruction ran, with an execution log.
    pub runs: Option<u64>,
    /// The listing lines the block covers (its label and comments first).
    pub first_line: Option<u32>,
    pub line_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GraphEdgeInfo {
    pub from: u32,
    pub to: u32,
    pub kind: GraphEdgeKind,
    /// For `Case`: the table entries that go this way.
    pub cases: Vec<u32>,
    /// Goes back up to a loop's header.
    pub back: bool,
    /// How many times control went this way, with an execution log.
    pub count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GraphLoopInfo {
    pub header: u32,
    pub body: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RoutineGraphInfo {
    pub name: String,
    pub entry: u32,
    /// Reverse post-order: the entry is block 0.
    pub blocks: Vec<GraphBlockInfo>,
    pub edges: Vec<GraphEdgeInfo>,
    pub loops: Vec<GraphLoopInfo>,
    pub irreducible: bool,
    pub truncated: bool,
    /// Blocks and edges carry an execution log's counts.
    pub counted: bool,
}

impl RoutineGraphInfo {
    pub fn new(g: graph::RoutineGraph, lines: &LineIndex) -> Self {
        let blocks = g
            .blocks
            .iter()
            .map(|b| {
                let (exit, exit_target, exit_text) = match &b.exit {
                    BlockExit::Next => (GraphExitKind::Next, None, String::new()),
                    BlockExit::Branch => (GraphExitKind::Branch, None, String::new()),
                    BlockExit::Switch { table } => {
                        (GraphExitKind::Switch, Some(table.as_u24()), String::new())
                    }
                    BlockExit::Return => (GraphExitKind::Return, None, String::new()),
                    BlockExit::Halt => (GraphExitKind::Halt, None, String::new()),
                    BlockExit::Tail { target, name } => {
                        (GraphExitKind::Tail, Some(target.as_u24()), name.clone())
                    }
                    BlockExit::Unknown(why) => (GraphExitKind::Unknown, None, why.clone()),
                };
                let range = b.lines(lines);
                GraphBlockInfo {
                    offsets: b.offsets.iter().map(|o| o.0).collect(),
                    address: b.address.map(|a| a.as_u24()),
                    exit,
                    exit_target,
                    exit_text,
                    loop_depth: b.loop_depth,
                    loop_header: b.loop_header,
                    runs: b.runs,
                    first_line: range.as_ref().map(|r| r.start as u32),
                    line_count: range.map(|r| r.len() as u32).unwrap_or(0),
                }
            })
            .collect();
        RoutineGraphInfo {
            name: g.name,
            entry: g.entry.as_u24(),
            blocks,
            edges: g
                .edges
                .iter()
                .map(|e| GraphEdgeInfo {
                    from: e.from as u32,
                    to: e.to as u32,
                    kind: match e.kind {
                        EdgeKind::Taken => GraphEdgeKind::Taken,
                        EdgeKind::NotTaken => GraphEdgeKind::NotTaken,
                        EdgeKind::Jump => GraphEdgeKind::Jump,
                        EdgeKind::Fall => GraphEdgeKind::Fall,
                        EdgeKind::Case => GraphEdgeKind::Case,
                    },
                    cases: e.cases.clone(),
                    back: e.back,
                    count: e.count,
                })
                .collect(),
            loops: g
                .loops
                .iter()
                .map(|l| GraphLoopInfo {
                    header: l.header as u32,
                    body: l.body.iter().map(|&b| b as u32).collect(),
                })
                .collect(),
            irreducible: g.irreducible,
            truncated: g.truncated,
            counted: g.counted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CallHowKind {
    /// `JSR` or `JSL`.
    Call,
    /// `JSR (abs,X)` through a table the analysis resolved.
    Table,
    /// A jump or branch into the routine's entry.
    Tail,
    /// Through a pointer: only an execution log saw it.
    Observed,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CallSiteInfo {
    pub file_offset: u32,
    pub address: u32,
    pub how: CallHowKind,
    pub count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CallLinkInfo {
    pub entry: u32,
    pub name: String,
    pub sites: Vec<CallSiteInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CallNeighbourhoodInfo {
    pub entry: u32,
    pub name: String,
    pub callers: Vec<CallLinkInfo>,
    pub callees: Vec<CallLinkInfo>,
    pub counted: bool,
}

fn link(c: &graph::CallLink) -> CallLinkInfo {
    CallLinkInfo {
        entry: c.entry.as_u24(),
        name: c.name.clone(),
        sites: c
            .sites
            .iter()
            .map(|s| CallSiteInfo {
                file_offset: s.offset.0,
                address: s.address.as_u24(),
                how: match s.how {
                    CallHow::Call => CallHowKind::Call,
                    CallHow::Table => CallHowKind::Table,
                    CallHow::Tail => CallHowKind::Tail,
                    CallHow::Observed => CallHowKind::Observed,
                },
                count: s.count,
            })
            .collect(),
    }
}

impl From<graph::CallNeighbourhood> for CallNeighbourhoodInfo {
    fn from(n: graph::CallNeighbourhood) -> Self {
        CallNeighbourhoodInfo {
            entry: n.entry.as_u24(),
            name: n.name.clone(),
            callers: n.callers.iter().map(link).collect(),
            callees: n.callees.iter().map(link).collect(),
            counted: n.counted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct GraphNodeSize {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct GraphEdgeSpec {
    pub from: u32,
    pub to: u32,
    /// Goes back up (a loop's way back): drawn climbing.
    pub back: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct GraphLayoutOptions {
    /// Between two boxes in a row.
    pub node_gap: f64,
    /// Between rows, where the edges run.
    pub layer_gap: f64,
    /// Between an edge passing a row and whatever is beside it.
    pub edge_gap: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct GraphPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GraphEdgePath {
    /// From the source's bottom to the target's top.
    pub points: Vec<GraphPoint>,
    pub climbs: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GraphLayoutInfo {
    /// Each box's top-left corner.
    pub nodes: Vec<GraphPoint>,
    pub rows: Vec<u32>,
    /// One per edge given, in order.
    pub edges: Vec<GraphEdgePath>,
    pub width: f64,
    pub height: f64,
}

/// Lay out boxes of the given sizes joined by `edges`, top to bottom
/// (`graph::layout`). Deterministic.
#[uniffi::export]
pub fn layout_graph(
    nodes: Vec<GraphNodeSize>,
    edges: Vec<GraphEdgeSpec>,
    options: GraphLayoutOptions,
) -> GraphLayoutInfo {
    let sizes: Vec<NodeSize> = nodes
        .iter()
        .map(|n| NodeSize {
            width: n.width,
            height: n.height,
        })
        .collect();
    let specs: Vec<EdgeSpec> = edges
        .iter()
        .map(|e| EdgeSpec {
            from: e.from as usize,
            to: e.to as usize,
            back: e.back,
        })
        .collect();
    let l = graph::layout(
        &sizes,
        &specs,
        &graph::LayoutOptions {
            node_gap: options.node_gap,
            layer_gap: options.layer_gap,
            edge_gap: options.edge_gap,
            ..graph::LayoutOptions::default()
        },
    );
    let point = |p: &graph::layout::Point| GraphPoint { x: p.x, y: p.y };
    GraphLayoutInfo {
        nodes: l.nodes.iter().map(point).collect(),
        rows: l.rows,
        edges: l
            .edges
            .iter()
            .map(|e| GraphEdgePath {
                points: e.points.iter().map(point).collect(),
                climbs: e.climbs,
            })
            .collect(),
        width: l.width,
        height: l.height,
    }
}
