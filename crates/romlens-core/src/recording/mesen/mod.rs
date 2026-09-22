//! Recording from Mesen: the Lua recorder Romlens ships, the raw stream it
//! writes, and `rec pack`, which turns a stream into a `.romrec`.

pub mod pack;
pub mod stream;

pub use pack::{PackError, PackOptions, PackReport, WramMode, pack};

/// The recorder script, as `romlens rec script` writes it.
pub const RECORDER_SCRIPT: &str = include_str!("mesen_recorder.lua");
