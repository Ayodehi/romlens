//! Every edit is a command with an inverse, so undo is a matter of replaying.

use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::comment::CommentKind;
use crate::model::label::{Label, LabelSource};
use crate::model::project::FlagOverride;
use crate::model::region::OverrideKind;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `None` removes the label. The label's source comes from the batch's
    /// [`Origin`], so the same command means "the user named this" from a
    /// sheet and "a symbol file named this" from an importer.
    SetLabel {
        address: SnesAddress,
        name: Option<String>,
    },
    /// Put a label back exactly as it was, source and all.
    ///
    /// Only ever produced as the inverse of a `SetLabel`, never built by a
    /// caller. It exists because undoing an import has to restore an imported
    /// label *as imported* and a user's label as the user's, and a `SetLabel`
    /// replayed under some other origin would quietly relabel it.
    RestoreLabel {
        address: SnesAddress,
        label: Option<Label>,
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
            Command::RestoreLabel { label: Some(_), .. } => "Rename Label",
            Command::RestoreLabel { label: None, .. } => "Remove Label",
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

/// Who asked for a batch of commands. The undo stack is a *user's* history, so
/// twenty thousand labels arriving from a symbol file have to be one entry, not
/// twenty thousand; the origin is what supplies that entry's title.
///
/// `Accepted` is unused in Phase 2 and exists because the tutor's Fix mode is
/// the same call with a different origin (`16-phase2-plan.md`, 2A.4). Keeping
/// the variant now means the command model, the undo stack and the project
/// store need no rework when it lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A direct edit: a menu item, a sheet, a CLI subcommand.
    User,
    /// An importer, named by the file it read.
    Import(String),
    /// A proposal the user accepted, named by what proposed it.
    Accepted(String),
}

impl Origin {
    /// Where a label this batch creates came from.
    pub fn label_source(&self) -> LabelSource {
        match self {
            Origin::User => LabelSource::User,
            Origin::Import(source) | Origin::Accepted(source) => {
                LabelSource::Imported(source.clone())
            }
        }
    }

    /// The Edit menu's "Undo …" text. A single user command keeps the wording
    /// Phase 1 shipped, so nothing in the shell reads differently.
    pub fn title(&self, commands: &[Command]) -> String {
        match self {
            Origin::User => match commands {
                [one] => one.menu_title().to_owned(),
                other => format!("{} Changes", other.len()),
            },
            Origin::Import(source) => format!("Import from {source}"),
            Origin::Accepted(source) => format!("Accept {source}"),
        }
    }
}

/// Commands that were applied together and how to take them back. `inverse`
/// is already in undo order: the last command's inverse comes first, because
/// commands overlap (two marks over the same bytes) and the second can only be
/// unwound while the first is still in place.
#[derive(Debug, Clone, PartialEq)]
pub struct UndoEntry {
    pub done: Vec<Command>,
    pub inverse: Vec<Command>,
    pub title: String,
    pub origin: Origin,
}

impl UndoEntry {
    /// Whether the analyzer must run again after this entry.
    pub fn affects_analysis(&self) -> bool {
        self.done.iter().any(Command::affects_analysis)
    }
}
