//! Progress (docs/28): what the student has proven, their points, streak,
//! rank and achievements. None of it is stored. It is worked out each
//! time from the lessons, the quizzes, the marks and the journal, which
//! only grow, so it cannot drift and every point has a cause.
//!
//! The journal (`<root>/Learner/journal.jsonl`) keeps the few facts that
//! have no other file: a predict question's guess, and a milestone reached
//! in a game.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::store::StoreError;

/// A fact with no other file, one JSON object a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fact {
    /// A lesson step's predict question, guessed before Show; `right` when
    /// the step's claim could check it.
    PredictAnswered {
        lesson: String,
        step: u32,
        guess: String,
        #[serde(default)]
        right: Option<bool>,
        when: u64,
    },
    /// A milestone reached in a game (`reset_named`, `routines_50`, …).
    Milestone {
        rom: String,
        rom_title: String,
        id: String,
        when: u64,
    },
}

impl Fact {
    pub fn when(&self) -> u64 {
        match self {
            Fact::PredictAnswered { when, .. } | Fact::Milestone { when, .. } => *when,
        }
    }
}

/// The journal: appended to, never rewritten.
pub struct Journal {
    path: PathBuf,
}

impl Journal {
    pub fn new(root: &Path) -> Journal {
        Journal {
            path: root.join("Learner").join("journal.jsonl"),
        }
    }

    pub fn append(&self, fact: &Fact) -> Result<(), StoreError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut line = serde_json::to_vec(fact).expect("a fact serialises");
        line.push(b'\n');
        f.write_all(&line)?;
        f.sync_data()?;
        Ok(())
    }

    /// Every fact, oldest first. A line that doesn't read (a write cut off
    /// by a crash) is skipped.
    pub fn read(&self) -> Vec<Fact> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// Removes the journal ("Reset progress").
    pub fn reset(&self) -> Result<(), StoreError> {
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_journal_keeps_facts_and_skips_a_torn_line() {
        let dir = std::env::temp_dir().join(format!("romlens-journal-{}", std::process::id()));
        let j = Journal::new(&dir);
        assert!(j.read().is_empty(), "no journal is an empty one");
        let a = Fact::PredictAnswered {
            lesson: "l1-00000".into(),
            step: 2,
            guess: "in vblank".into(),
            right: None,
            when: 10,
        };
        let b = Fact::Milestone {
            rom: "abc".into(),
            rom_title: "TEST".into(),
            id: "nmi_named".into(),
            when: 20,
        };
        j.append(&a).unwrap();
        j.append(&b).unwrap();
        // A crash in the middle of a write.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("Learner/journal.jsonl"))
            .unwrap();
        f.write_all(br#"{"kind":"milestone","rom":"ab"#).unwrap();
        assert_eq!(j.read(), vec![a, b]);
        j.reset().unwrap();
        assert!(j.read().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
