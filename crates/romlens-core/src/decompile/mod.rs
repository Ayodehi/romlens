//! Pseudo-C for one routine at a time (`docs/18-decompiler.md`).
//!
//! The stages, each in its own module: find the function's instructions
//! (`function`), split them into blocks and find the graph's dominators and
//! loops (`cfg`), then lift, clean up, structure and print.

pub mod cfg;
pub mod function;

pub use cfg::{Block, BlockId, Cfg, Loop, Term};
pub use function::{
    Callee, Dest, Function, FunctionError, Step, Transfer, containing, discover, entries,
};
