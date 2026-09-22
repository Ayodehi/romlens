//! Every edit is a command with an inverse, so undo is a matter of replaying.

use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::comment::CommentKind;
use crate::model::project::FlagOverride;
use crate::model::region::OverrideKind;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `None` removes the label.
    SetLabel {
        address: SnesAddress,
        name: Option<String>,
    },
    /// `None` or an empty string removes the comment.
    SetComment {
        address: SnesAddress,
        kind: CommentKind,
        text: Option<String>,
    },
    MarkRegion {
        start: FileOffset,
        len: u32,
        kind: OverrideKind,
    },
    ClearRegionOverride {
        start: FileOffset,
        len: u32,
    },
    /// `None` removes the override.
    SetFlagOverride {
        offset: FileOffset,
        flags: Option<FlagOverride>,
    },
}

impl Command {
    /// Whether the analyzer must run again after this command.
    pub fn affects_analysis(&self) -> bool {
        matches!(
            self,
            Command::MarkRegion { .. }
                | Command::ClearRegionOverride { .. }
                | Command::SetFlagOverride { .. }
        )
    }

    /// The Edit menu's "Undo …" text.
    pub fn menu_title(&self) -> &'static str {
        match self {
            Command::SetLabel { name: Some(_), .. } => "Rename Label",
            Command::SetLabel { name: None, .. } => "Remove Label",
            Command::SetComment { text: Some(t), .. } if !t.trim().is_empty() => "Set Comment",
            Command::SetComment { .. } => "Remove Comment",
            Command::MarkRegion {
                kind: OverrideKind::Code,
                ..
            } => "Mark as Code",
            Command::MarkRegion {
                kind: OverrideKind::Data(_),
                ..
            } => "Mark as Data",
            Command::MarkRegion {
                kind: OverrideKind::Unknown,
                ..
            } => "Mark as Unknown",
            Command::ClearRegionOverride { .. } => "Clear Mark",
            Command::SetFlagOverride { flags: Some(_), .. } => "Set Flags",
            Command::SetFlagOverride { flags: None, .. } => "Remove Flags",
        }
    }
}

/// A command that was applied and how to take it back.
#[derive(Debug, Clone, PartialEq)]
pub struct UndoEntry {
    pub done: Command,
    pub inverse: Vec<Command>,
    pub title: String,
}
