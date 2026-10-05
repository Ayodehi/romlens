#![allow(dead_code)] // views that use the rest land later in milestone L1
//! GTK-free state: everything a window knows that is not a widget. The macOS
//! shell keeps the same state in `RomViewModel`, `WorkbenchSession` and
//! `BatchCache`; keeping it free of GTK means `cargo test` covers it without
//! a display.

pub mod atlas;
pub mod audio;
pub mod batch_cache;
pub mod cfold;
pub mod commands;
pub mod compare;
pub mod decompile;
pub mod document;
pub mod graph;
pub mod graphics;
pub mod graphscene;
pub mod layout;
pub mod live;
pub mod markdown;
pub mod navigator;
pub mod quick;
pub mod references;
pub mod runtime;
pub mod screen;
pub mod search;
pub mod session;
pub mod sidebar;
pub mod source;
pub mod strip;
#[cfg(test)]
pub mod testing;
pub mod transfer;
pub mod tutor;
pub mod tutor_settings;
pub mod workspace;

pub use commands::{CEdit, EditorCommand, EditorSource, Sheet, Zoom};
pub use document::{Change, Details, Document, MarkOptions, ScrollRequest, VariableDraft};
pub use layout::{ResultsKind, Tab};
pub use runtime::Runtime;
