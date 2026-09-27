//! The undo stack lives in the core so every shell shares one history and it
//! cannot drift from the project (docs/08).

use crate::error::ProjectError;
use crate::model::command::{Command, Origin, UndoEntry};
use crate::model::project::Project;
use crate::rom::image::RomImage;

pub const UNDO_CAP: usize = 1000;

/// What a rewind did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Rewound {
    /// Entries undone from the top of the stack.
    pub undone: u32,
    /// Older entries taken back by one new entry.
    pub reverted: u32,
    /// Entries left because a later edit changed the same thing.
    pub kept: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct UndoStack {
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
}

impl UndoStack {
    pub fn push(&mut self, entry: UndoEntry) {
        self.redo.clear();
        self.undo.push(entry);
        if self.undo.len() > UNDO_CAP {
            self.undo.remove(0);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_title(&self) -> Option<&str> {
        self.undo.last().map(|e| e.title.as_str())
    }

    pub fn redo_title(&self) -> Option<&str> {
        self.redo.last().map(|e| e.title.as_str())
    }

    /// Take back the last command. `Ok(false)` when there is nothing to undo.
    pub fn undo(&mut self, project: &mut Project, rom: &RomImage) -> Result<bool, ProjectError> {
        let Some(entry) = self.undo.pop() else {
            return Ok(false);
        };
        for cmd in &entry.inverse {
            project.apply(rom, cmd.clone())?;
        }
        self.redo.push(entry);
        Ok(true)
    }

    /// Re-apply the last undone command.
    pub fn redo(&mut self, project: &mut Project, rom: &RomImage) -> Result<bool, ProjectError> {
        let Some(entry) = self.redo.pop() else {
            return Ok(false);
        };
        let redone = project.apply_batch(rom, entry.done.clone(), entry.origin.clone())?;
        self.undo.push(redone);
        Ok(true)
    }

    /// The last undoable command, for `affects_analysis` checks.
    pub fn last_undone_affects_analysis(&self) -> bool {
        self.redo.last().is_some_and(UndoEntry::affects_analysis)
    }

    /// Take back every entry `mine` claims (the tutor's edits since a turn,
    /// docs/24 `/rewind`). The ones still on top are undone. Older ones,
    /// with other edits after them, have their inverses applied as one new
    /// entry, except where a later edit changed the same thing: those are
    /// left, and their titles reported, rather than undoing the later
    /// edit's work.
    pub fn rewind(
        &mut self,
        project: &mut Project,
        rom: &RomImage,
        mine: &dyn Fn(&Origin) -> bool,
    ) -> Result<Rewound, ProjectError> {
        let mut out = Rewound::default();
        while self.undo.last().is_some_and(|e| mine(&e.origin)) {
            self.undo(project, rom)?;
            out.undone += 1;
        }
        let mut inverse: Vec<Command> = Vec::new();
        for (i, e) in self.undo.iter().enumerate().rev() {
            if !mine(&e.origin) {
                continue;
            }
            let later = &self.undo[i + 1..];
            let clash = e.done.iter().any(|c| {
                let t = c.target();
                later
                    .iter()
                    .filter(|l| !mine(&l.origin))
                    .any(|l| l.done.iter().any(|lc| lc.target().overlaps(&t)))
            });
            if clash {
                out.kept.push(e.title.clone());
            } else {
                inverse.extend(e.inverse.iter().cloned());
                out.reverted += 1;
            }
        }
        if !inverse.is_empty() {
            let mut entry = project.apply_batch(rom, inverse, Origin::User)?;
            entry.title = "Rewind the Tutor's Edits".into();
            self.push(entry);
        }
        Ok(out)
    }

    pub fn entries(&self) -> &[UndoEntry] {
        &self.undo
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}
