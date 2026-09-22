//! The undo stack lives in the core so every shell shares one history and it
//! cannot drift from the project (docs/08).

use crate::error::ProjectError;
use crate::model::command::UndoEntry;
use crate::model::project::Project;
use crate::rom::image::RomImage;

pub const UNDO_CAP: usize = 1000;

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
        let redone = project.apply(rom, entry.done.clone())?;
        self.undo.push(redone);
        Ok(true)
    }

    /// The last undoable command, for `affects_analysis` checks.
    pub fn last_undone_affects_analysis(&self) -> bool {
        self.redo.last().is_some_and(|e| e.done.affects_analysis())
    }

    pub fn entries(&self) -> &[UndoEntry] {
        &self.undo
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}
