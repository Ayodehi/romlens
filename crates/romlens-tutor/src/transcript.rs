//! A conversation as Romlens keeps it, whichever provider answered each turn
//! (docs/24). Every protocol builds its request from this and parses its
//! stream back into it.
//!
//! What a provider returned is also kept exactly as it came (`Native`), so a
//! turn goes back to the protocol that made it byte for byte: Anthropic's
//! thinking blocks with their signatures, OpenAI's encrypted reasoning. The
//! plain blocks are what the window shows and what another protocol is sent.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::Protocol;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

/// A picture in the conversation, by its attachment id; the bytes are kept
/// beside the transcript, not in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    pub id: String,
    /// `image/png`, `image/jpeg`, `image/gif` or `image/webp`.
    pub media_type: String,
}

/// Part of a tool's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text { text: String },
    Image { image: ImageRef },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    Image {
        image: ImageRef,
    },
    /// The model calls a tool. `id` is the provider's id where it gave one,
    /// else one of ours; every protocol is sent a form of it that it takes.
    ToolCall {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        parts: Vec<Part>,
        is_error: bool,
    },
    /// What the model thought, as its provider summarised it. Shown, never
    /// sent: the real reasoning is in the turn's `Native`.
    Reasoning {
        summary: String,
    },
    /// Something the window notes (a change of model, a compaction, a
    /// rewind). Never sent.
    Note {
        text: String,
    },
}

/// Tokens a turn used. `input` excludes what was read from or written to
/// the cache, whatever the provider counts in its own `input_tokens`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Usage {
    pub fn add(&mut self, o: &Usage) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
    }

    /// Everything the model read.
    pub fn prompt(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }
}

/// What a provider returned for an assistant turn, unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Native {
    pub protocol: Protocol,
    /// The endpoint that answered (`anthropic`, `openai`, or a local
    /// endpoint's id).
    pub endpoint: String,
    pub model: String,
    /// Anthropic: the `content` array. Responses: the `output` items. Chat
    /// Completions: `{"thinking_blocks": […]}` where a proxy returned them.
    pub content: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub role: Role,
    pub blocks: Vec<Block>,
    /// Assistant turns: what the provider returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<Native>,
    #[serde(default)]
    pub usage: Usage,
    /// False once a rewind or a compaction has taken the turn out of what
    /// is sent; it stays in the file.
    #[serde(default = "yes")]
    pub sent: bool,
}

fn yes() -> bool {
    true
}

impl Turn {
    pub fn user(blocks: Vec<Block>) -> Turn {
        Turn {
            role: Role::User,
            blocks,
            native: None,
            usage: Usage::default(),
            sent: true,
        }
    }

    pub fn user_text(text: &str) -> Turn {
        Turn::user(vec![Block::Text { text: text.into() }])
    }

    /// The calls this turn makes.
    pub fn tool_calls(&self) -> impl Iterator<Item = (&str, &str, &Value)> {
        self.blocks.iter().filter_map(|b| match b {
            Block::ToolCall { id, name, input } => Some((id.as_str(), name.as_str(), input)),
            _ => None,
        })
    }

    /// The text the model wrote.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for b in &self.blocks {
            if let Block::Text { text } = b {
                s.push_str(text);
            }
        }
        s
    }
}

/// Only what goes to a provider: sent turns, without notes, and without
/// user turns left empty by that.
pub fn sendable(turns: &[Turn]) -> Vec<&Turn> {
    turns
        .iter()
        .filter(|t| t.sent)
        .filter(|t| {
            t.role == Role::Assistant || t.blocks.iter().any(|b| !matches!(b, Block::Note { .. }))
        })
        .collect()
}

/// A tool-call id every protocol takes: letters, digits, `_` and `-`
/// (Anthropic's pattern), never empty.
pub fn wire_id(id: &str) -> String {
    let s: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() { "call".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_round_trips_as_json() {
        let t = Turn {
            role: Role::Assistant,
            blocks: vec![
                Block::Reasoning {
                    summary: "look at RESET".into(),
                },
                Block::ToolCall {
                    id: "toolu_1".into(),
                    name: "disassemble".into(),
                    input: serde_json::json!({"address": "$00:8000"}),
                },
            ],
            native: None,
            usage: Usage {
                input: 3,
                output: 4,
                cache_read: 5,
                cache_write: 6,
            },
            sent: true,
        };
        let s = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<Turn>(&s).unwrap(), t);
        assert_eq!(t.usage.prompt(), 14);
    }

    #[test]
    fn ids_are_made_safe() {
        assert_eq!(wire_id("call:1/2"), "call_1_2");
        assert_eq!(wire_id(""), "call");
        assert_eq!(wire_id("toolu_01A-b"), "toolu_01A-b");
    }

    #[test]
    fn notes_and_unsent_turns_stay_home() {
        let mut gone = Turn::user_text("old");
        gone.sent = false;
        let note = Turn::user(vec![Block::Note {
            text: "now on another model".into(),
        }]);
        let turns = vec![gone, note, Turn::user_text("new")];
        let s = sendable(&turns);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].text(), "new");
    }
}
