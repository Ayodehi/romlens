//! Comments: one line after the instruction, or a block above it.

use crate::memory::address::SnesAddress;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CommentKind {
    Line,
    Block,
}

impl CommentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            CommentKind::Line => "line",
            CommentKind::Block => "block",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub address: SnesAddress,
    pub kind: CommentKind,
    pub text: String,
}
