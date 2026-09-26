//! The annotation model: labels, comments, regions, cross-references, the
//! project overlay, commands and undo.

pub mod command;
pub mod comment;
pub mod coverage;
pub mod exec_log;
pub mod hardware;
pub mod label;
pub mod project;
pub mod region;
pub mod source_map;
pub mod spc_log;
pub mod symbols;
pub mod undo;
pub mod variable;
pub mod xref;

pub use command::{Command, Origin, UndoEntry};
pub use comment::{Comment, CommentKind};
pub use coverage::{BitSet, Coverage, ObservedFlags};
pub use hardware::{
    Access, HardwareRegister, all_hardware_registers, hardware_register, is_system_bank,
};
pub use label::{Label, LabelSource, validate_label_name};
pub use project::{FlagOverride, ImportRecord, Project, RomIdentity, Settings, TraceRecord};
pub use region::{
    BankRule, DataKind, Evidence, OverrideKind, Region, RegionKind, RegionOverride, RegionParams,
    TableElem,
};
pub use symbols::Symbols;
pub use undo::{UNDO_CAP, UndoStack};
pub use variable::{VarType, VarWidth};
pub use xref::{XRef, XRefKind};
