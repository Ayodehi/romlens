#![allow(dead_code)] // views that use the rest land later in milestone L1
//! GTK-free state: everything a window knows that is not a widget. The macOS
//! shell keeps the same state in `RomViewModel`, `WorkbenchSession` and
//! `BatchCache`; keeping it free of GTK means `cargo test` covers it without
//! a display.

pub mod batch_cache;
pub mod commands;
pub mod document;
pub mod layout;
pub mod navigator;
pub mod references;
pub mod runtime;
pub mod search;
pub mod session;
pub mod strip;
#[cfg(test)]
pub mod testing;
pub mod transfer;

pub use commands::{EditorCommand, EditorSource, Sheet};
pub use document::{Change, Details, Document, MarkOptions, ScrollRequest, VariableDraft};
pub use layout::{ResultsKind, Tab};
pub use runtime::Runtime;
