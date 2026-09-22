//! Cross-references: one instruction (or vector slot) pointing at an address.

use crate::memory::address::{FileOffset, SnesAddress};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum XRefKind {
    Call,
    Jump,
    Branch,
    Read,
    Write,
    ReadWrite,
    Pointer,
    Vector,
}

impl XRefKind {
    pub const fn name(self) -> &'static str {
        match self {
            XRefKind::Call => "call",
            XRefKind::Jump => "jump",
            XRefKind::Branch => "branch",
            XRefKind::Read => "read",
            XRefKind::Write => "write",
            XRefKind::ReadWrite => "read/write",
            XRefKind::Pointer => "pointer",
            XRefKind::Vector => "vector",
        }
    }

    pub const fn is_code(self) -> bool {
        matches!(
            self,
            XRefKind::Call | XRefKind::Jump | XRefKind::Branch | XRefKind::Vector
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XRef {
    /// The referencing instruction or vector slot.
    pub from: FileOffset,
    /// Canonical target address.
    pub to: SnesAddress,
    /// The target's file offset when it is in ROM.
    pub to_offset: Option<FileOffset>,
    pub kind: XRefKind,
    pub certain: bool,
}
