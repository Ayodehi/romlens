//! Graphs to draw (docs/19): a routine's control-flow graph, the routines
//! around it in the call graph, and a layered layout for either.
//!
//! Both graphs come from the decompiler's routine discovery
//! (`decompile::function`) and blocks (`decompile::cfg`); with an execution
//! log they carry what the game was seen to do.

pub mod calls;
pub mod counts;
pub mod layout;
pub mod routine;

pub use calls::{CallHow, CallLink, CallNeighbourhood, CallSite, call_neighbourhood};
pub use counts::Counts;
pub use layout::{Layout, LayoutEdge, LayoutOptions, NodeSize, layout};
pub use routine::{
    BlockExit, EdgeKind, GraphBlock, GraphEdge, GraphLoop, RoutineGraph, routine_graph,
};

use crate::memory::address::SnesAddress;
use crate::model::symbols::Symbols;

/// A routine's name as the listing shows it: its label, or the analyzer's
/// `SUB_` form when it has none.
pub fn routine_name(symbols: &Symbols<'_>, entry: SnesAddress) -> String {
    symbols
        .label_at(entry)
        .map(|l| l.name.clone())
        .unwrap_or_else(|| format!("SUB_{:06X}", entry.as_u24()))
}
