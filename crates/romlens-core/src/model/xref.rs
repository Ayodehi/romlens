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
    /// A dispatcher reading its table's base.
    JumpTable,
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
            XRefKind::JumpTable => "jump table",
            XRefKind::Vector => "vector",
        }
    }

    /// The target is a slot whose *contents* are the real address: a `JMP
    /// (abs)` pointer or a dispatch table's base. The instruction's operand
    /// width says nothing about how much data is there, so these never seed a
    /// data region — the pointer and the table resolver decide the extent.
    pub const fn is_indirect(self) -> bool {
        matches!(self, XRefKind::Pointer | XRefKind::JumpTable)
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
