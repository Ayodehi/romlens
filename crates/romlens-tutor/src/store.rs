//! Conversations on disk (docs/24, "The store"), in the app's own folder
//! and never in a project package (`12-content-policy.md` rule 8):
//!
//! ```text
//! <root>/<ROM's SHA-256>/prompts.jsonl        what was asked, for ↑
//! <root>/<ROM's SHA-256>/<id>/meta.json       title, model, mode, cost, the fixed prompt
//! <root>/<ROM's SHA-256>/<id>/transcript.json every turn, sent or not
//! <root>/<ROM's SHA-256>/<id>/pictures/<id>   the pictures its turns name
//! ```
//!
//! Each save writes the conversation's files whole, to a temporary name
//! then renamed, so a crash leaves the last good copy. Turns a rewind or a
//! compaction took out stay in the file, marked as not sent.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agent::{Mode, Session};
use crate::provider::Endpoint;
use crate::transcript::{Block, Part, Turn};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    pub title: String,
    /// Seconds since 1970.
    pub created: u64,
    pub updated: u64,
    pub endpoint: Endpoint,
    pub model: String,
    #[serde(default)]
    pub effort: Option<String>,
    pub mode: Mode,
    pub cost: f64,
    #[serde(default)]
    pub cost_cap: Option<f64>,
    pub turns: usize,
    /// Fixed for the conversation, so a resumed one sends what it sent.
    pub system: String,
    pub digest: String,
    /// The tools last sent (`agent::tools_digest`).
    #[serde(default)]
    pub tools: Option<String>,
    /// The title is the model's name for the conversation, not the start
    /// of its first question.
    #[serde(default)]
    pub titled: bool,
    /// What naming it cost, beside the turns.
    #[serde(default)]
    pub side_cost: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0} is not a saved conversation: {1}")]
    Bad(String, String),
}

pub struct Store {
    dir: PathBuf,
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A new conversation's id: when it started, and a little noise.
pub fn new_id() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("c{}-{:05x}", t.as_secs(), t.subsec_nanos() & 0xFFFFF)
}

/// The first words of the first question, as a title.
pub fn title_for(turns: &[Turn]) -> String {
    // The first question's own words: its last text block, after any
    // selection or mode lines.
    let first = turns
        .iter()
        .find_map(|t| {
            t.blocks.iter().rev().find_map(|b| match b {
                Block::Text { text } => Some(text.as_str()),
                _ => None,
            })
        })
        .unwrap_or("A conversation");
    // What the student typed first: code they pasted comes after, and a
    // bracketed note to the model before.
    let words = first
        .lines()
        .find(|l| !l.trim().is_empty() && !l.starts_with('['))
        .unwrap_or(first)
        .trim();
    if words.chars().count() > 60 {
        words.chars().take(59).chain(['…']).collect()
    } else {
        words.to_owned()
    }
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

pub(crate) fn safe(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Store {
    /// The folder for one ROM's conversations.
    pub fn new(root: &Path, rom_sha256: &str) -> Store {
        Store {
            dir: root.join(rom_sha256),
        }
    }

    fn conv(&self, id: &str) -> Result<PathBuf, StoreError> {
        if !safe(id) {
            return Err(StoreError::Bad(id.into(), "not an id".into()));
        }
        Ok(self.dir.join(id))
    }

    pub fn save(&self, s: &Session, title: &str, created: u64) -> Result<Meta, StoreError> {
        let dir = self.conv(&s.id)?;
        std::fs::create_dir_all(dir.join("pictures"))?;
        let meta = Meta {
            id: s.id.clone(),
            title: s.title.clone().unwrap_or_else(|| title.into()),
            created,
            updated: now(),
            endpoint: s.endpoint.clone(),
            model: s.model.clone(),
            effort: s.effort.clone(),
            mode: s.mode,
            cost: s.cost(),
            cost_cap: s.cost_cap,
            turns: s.turns.len(),
            system: s.system.clone(),
            digest: s.digest.clone(),
            tools: s.tools_seen.clone(),
            titled: s.title.is_some(),
            side_cost: s.side_cost,
        };
        for (id, bytes) in &s.pictures {
            let p = dir.join("pictures").join(id);
            if safe(id) && !p.exists() {
                write_atomic(&p, bytes)?;
            }
        }
        let turns = serde_json::to_vec(&s.turns).expect("turns serialise");
        write_atomic(&dir.join("transcript.json"), &turns)?;
        write_atomic(
            &dir.join("meta.json"),
            &serde_json::to_vec_pretty(&meta).expect("meta serialises"),
        )?;
        Ok(meta)
    }

    pub fn meta(&self, id: &str) -> Result<Meta, StoreError> {
        let p = self.conv(id)?.join("meta.json");
        let b = std::fs::read(&p)?;
        serde_json::from_slice(&b).map_err(|e| StoreError::Bad(id.into(), e.to_string()))
    }

    /// A saved conversation, ready to go on.
    pub fn load(&self, id: &str) -> Result<(Session, Meta), StoreError> {
        let dir = self.conv(id)?;
        let meta = self.meta(id)?;
        let b = std::fs::read(dir.join("transcript.json"))?;
        let turns: Vec<Turn> =
            serde_json::from_slice(&b).map_err(|e| StoreError::Bad(id.into(), e.to_string()))?;
        let mut s = Session::new(id, meta.endpoint.clone(), &meta.model);
        s.effort = meta.effort.clone();
        s.mode = meta.mode;
        s.cost_cap = meta.cost_cap;
        s.system = meta.system.clone();
        s.digest = meta.digest.clone();
        s.tools_seen = meta.tools.clone();
        s.title = meta.titled.then(|| meta.title.clone());
        s.side_cost = meta.side_cost;
        for t in &turns {
            for image in pictures_of(t) {
                if let Ok(bytes) = std::fs::read(dir.join("pictures").join(&image)) {
                    s.pictures.insert(image, bytes);
                }
            }
        }
        s.turns = turns;
        Ok((s, meta))
    }

    /// Every conversation, the latest first.
    pub fn list(&self) -> Vec<Meta> {
        let mut out: Vec<Meta> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| self.meta(&e.file_name().to_string_lossy()).ok())
            .collect();
        out.sort_by(|a, b| b.updated.cmp(&a.updated).then(b.id.cmp(&a.id)));
        out
    }

    pub fn delete(&self, id: &str) -> Result<(), StoreError> {
        let d = self.conv(id)?;
        if d.exists() {
            std::fs::remove_dir_all(d)?;
        }
        Ok(())
    }

    /// Adds a prompt to the history ↑ walks.
    pub fn remember_prompt(&self, text: &str) -> Result<(), StoreError> {
        if text.trim().is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)?;
        let mut line = serde_json::to_string(text).expect("a string serialises");
        line.push('\n');
        use std::io::Write;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("prompts.jsonl"))?
            .write_all(line.as_bytes())?;
        Ok(())
    }

    /// The last `n` prompts, oldest first, repeats in a row folded.
    pub fn prompts(&self, n: usize) -> Vec<String> {
        let text = std::fs::read_to_string(self.dir.join("prompts.jsonl")).unwrap_or_default();
        let mut all: Vec<String> = Vec::new();
        for l in text.lines() {
            if let Ok(p) = serde_json::from_str::<String>(l)
                && all.last() != Some(&p)
            {
                all.push(p);
            }
        }
        let skip = all.len().saturating_sub(n);
        all.split_off(skip)
    }
}

fn pictures_of(t: &Turn) -> Vec<String> {
    let mut out = Vec::new();
    for b in &t.blocks {
        match b {
            Block::Image { image } => out.push(image.id.clone()),
            Block::ToolResult { parts, .. } => {
                for p in parts {
                    if let Part::Image { image } = p {
                        out.push(image.id.clone());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tidy_name;
    use crate::transcript::ImageRef;

    #[test]
    fn a_name_is_tidied_to_a_title() {
        assert_eq!(
            tidy_name("\"Sharing the NMI's tail.\"\n").as_deref(),
            Some("Sharing the NMI's tail")
        );
        assert_eq!(
            tidy_name("\n**RESET's WRAM routine**").as_deref(),
            Some("RESET's WRAM routine")
        );
        assert_eq!(tidy_name("# Title: x").as_deref(), Some("Title: x"));
        assert_eq!(tidy_name("  \n"), None);
        assert_eq!(
            tidy_name(&"word ".repeat(20)).map(|t| t.chars().count()),
            Some(48)
        );
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("romlens-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_conversation_comes_back_as_it_was() {
        let root = scratch("round");
        let store = Store::new(&root, "abc123");
        let mut s = Session::new(&new_id(), Endpoint::anthropic(), "claude-opus-5");
        s.system = "sys".into();
        s.digest = "dig".into();
        s.mode = Mode::AcceptEdits;
        s.pictures.insert("shot-1".into(), b"PNG".to_vec());
        s.turns.push(Turn::user(vec![
            Block::Image {
                image: ImageRef {
                    id: "shot-1".into(),
                    media_type: "image/png".into(),
                },
            },
            Block::Text {
                text: "[The student's selection in Romlens:]\n$00:8000".into(),
            },
            Block::Text {
                text:
                    "What does RESET do at the start of the game, in detail please?\n```c\n}\n```"
                        .into(),
            },
        ]));
        let mut gone = Turn::user_text("rewound");
        gone.sent = false;
        s.turns.push(gone);
        let title = title_for(&s.turns);
        assert_eq!(
            title,
            "What does RESET do at the start of the game, in detail plea…"
        );
        let meta = store.save(&s, &title, 1000).unwrap();
        assert_eq!(meta.turns, 2);
        let (back, m) = store.load(&s.id).unwrap();
        assert_eq!(back.turns, s.turns);
        assert_eq!(back.pictures, s.pictures);
        assert_eq!(
            (back.system.as_str(), back.digest.as_str(), back.mode),
            ("sys", "dig", Mode::AcceptEdits)
        );
        assert_eq!(m.created, 1000);
        assert_eq!(store.list().len(), 1);
        assert!(store.load("../etc").is_err());
        assert_eq!(back.title, None, "not named yet");

        // The model's name replaces the question's words, and stays.
        let mut named = back;
        named.title = Some("RESET's first steps".into());
        named.side_cost = 0.002;
        let meta = store.save(&named, &title, 1000).unwrap();
        assert_eq!(
            (meta.title.as_str(), meta.titled),
            ("RESET's first steps", true)
        );
        let (again, _) = store.load(&s.id).unwrap();
        assert_eq!(again.title.as_deref(), Some("RESET's first steps"));
        assert!((again.cost() - 0.002).abs() < 1e-9);
        store.delete(&s.id).unwrap();
        assert!(store.list().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn prompts_are_remembered_in_order() {
        let root = scratch("prompts");
        let store = Store::new(&root, "abc");
        for p in ["one", "two", "two", "three\nlines", ""] {
            store.remember_prompt(p).unwrap();
        }
        assert_eq!(store.prompts(10), ["one", "two", "three\nlines"]);
        assert_eq!(store.prompts(2), ["two", "three\nlines"]);
        let _ = std::fs::remove_dir_all(root);
    }
}
