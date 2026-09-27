//! The three wire protocols (docs/24, decision 2). Each builds an HTTP
//! request from the transcript and parses the event stream back into
//! deltas for the window and a finished turn for the transcript.

pub mod anthropic;
pub mod chat;
pub mod responses;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::{Capabilities, Thinking};
use crate::sse;
use crate::transcript::{Block, ImageRef, Native, Turn, Usage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// Anthropic's Messages API.
    Anthropic,
    /// OpenAI's Responses API.
    Responses,
    /// Chat Completions, as local servers and proxies speak it.
    Chat,
}

/// Where requests go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    /// `anthropic`, `openai`, or the id of one the user added.
    pub id: String,
    pub protocol: Protocol,
    /// Up to the version: `https://api.anthropic.com`,
    /// `https://api.openai.com/v1`, `http://localhost:11434/v1`.
    pub base_url: String,
    /// Whether the server takes `tool_choice` (Ollama does not).
    #[serde(default)]
    pub tool_choice: bool,
    /// Whether to mark tool schemas strict (OpenAI's own servers; some
    /// local ones reject the field).
    #[serde(default)]
    pub strict: bool,
    /// For a model not in the table: whether it sees pictures.
    #[serde(default)]
    pub vision: bool,
}

impl Endpoint {
    pub fn anthropic() -> Endpoint {
        Endpoint {
            id: "anthropic".into(),
            protocol: Protocol::Anthropic,
            base_url: "https://api.anthropic.com".into(),
            tool_choice: true,
            strict: true,
            vision: true,
        }
    }

    pub fn openai() -> Endpoint {
        Endpoint {
            id: "openai".into(),
            protocol: Protocol::Responses,
            base_url: "https://api.openai.com/v1".into(),
            tool_choice: true,
            strict: true,
            vision: true,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), path)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// A JSON Schema object whose properties are all required, with
    /// `additionalProperties: false`, so every protocol's strict mode takes
    /// it; optional values are nullable.
    pub schema: Value,
}

/// The bytes of a picture the transcript names.
pub trait Attachments {
    fn bytes(&self, image: &ImageRef) -> Option<Vec<u8>>;
}

impl Attachments for std::collections::HashMap<String, Vec<u8>> {
    fn bytes(&self, image: &ImageRef) -> Option<Vec<u8>> {
        self.get(&image.id).cloned()
    }
}

/// One request, whatever the protocol.
pub struct Request<'a> {
    pub endpoint: &'a Endpoint,
    pub model: &'a str,
    pub caps: Capabilities,
    /// Fixed for the conversation.
    pub system: &'a str,
    /// The ROM digest, fixed for the conversation.
    pub digest: &'a str,
    pub tools: &'a [ToolSpec],
    pub turns: &'a [&'a Turn],
    pub effort: Option<&'a str>,
    pub max_tokens: u32,
    /// For the provider's cache routing: the conversation's id.
    pub cache_key: &'a str,
    pub attachments: &'a dyn Attachments,
}

impl Request<'_> {
    fn effort(&self) -> Option<&str> {
        let e = self.effort.or(self.caps.default_effort)?;
        self.caps.efforts.contains(&e).then_some(e)
    }

    fn thinking(&self) -> Thinking {
        self.caps.thinking
    }

    /// A picture as a data URL, or `None` when its bytes are gone.
    fn data_url(&self, image: &ImageRef) -> Option<String> {
        let b = self.attachments.bytes(image)?;
        Some(format!("data:{};base64,{}", image.media_type, base64(&b)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// What the window shows while a turn streams.
#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    Text(String),
    Reasoning(String),
    ToolCallStarted { id: String, name: String },
    ToolArguments { id: String, text: String },
}

/// Why the model stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Stop {
    EndTurn,
    ToolUse,
    MaxTokens,
    /// Declined, with the provider's category when it gave one.
    Refusal {
        category: Option<String>,
    },
    ContextFull,
    /// The server paused a long turn; send it back to go on.
    Paused,
    Other {
        reason: String,
    },
}

/// A finished assistant turn.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub blocks: Vec<Block>,
    pub native: Native,
    pub usage: Usage,
    pub stop: Stop,
    /// The model that answered, as the provider named it.
    pub model: String,
}

impl Reply {
    pub fn into_turn(self) -> Turn {
        Turn {
            role: crate::transcript::Role::Assistant,
            blocks: self.blocks,
            native: Some(self.native),
            usage: self.usage,
            cost: 0.0,
            sent: true,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum StreamError {
    /// The provider sent an error in the stream.
    #[error("{kind}: {message}")]
    Provider { kind: String, message: String },
    #[error("the stream was not understood: {0}")]
    Malformed(String),
    #[error("the stream ended before the reply did")]
    Truncated,
}

/// Turns a protocol's events into deltas, then a reply.
pub trait Parser {
    fn event(&mut self, e: &sse::Event) -> Result<Vec<Delta>, StreamError>;
    fn finish(self: Box<Self>) -> Result<Reply, StreamError>;
}

pub fn build(r: &Request) -> HttpRequest {
    match r.endpoint.protocol {
        Protocol::Anthropic => anthropic::build(r),
        Protocol::Responses => responses::build(r),
        Protocol::Chat => chat::build(r),
    }
}

pub fn parser(endpoint: &Endpoint, model: &str) -> Box<dyn Parser> {
    match endpoint.protocol {
        Protocol::Anthropic => Box::new(anthropic::StreamParser::new(&endpoint.id, model)),
        Protocol::Responses => Box::new(responses::StreamParser::new(&endpoint.id, model)),
        Protocol::Chat => Box::new(chat::StreamParser::new(&endpoint.id, model)),
    }
}

/// Parses a whole recorded stream: what the tests and the fake transport
/// use.
pub fn parse_all(
    endpoint: &Endpoint,
    model: &str,
    bytes: &[u8],
) -> Result<(Vec<Delta>, Reply), StreamError> {
    let mut p = parser(endpoint, model);
    let mut r = sse::Reader::new();
    let mut deltas = Vec::new();
    let mut events = r.feed(bytes);
    events.extend(r.finish());
    for e in &events {
        deltas.extend(p.event(e)?);
    }
    Ok((deltas, p.finish()?))
}

/// Parses a tool call's arguments strictly. Anything but a JSON object is
/// handed back to the model as an error, not run.
pub fn parse_arguments(text: &str) -> Value {
    let t = text.trim();
    if t.is_empty() {
        return Value::Object(Default::default());
    }
    match serde_json::from_str::<Value>(t) {
        Ok(v @ Value::Object(_)) => v,
        _ => serde_json::json!({ "INVALID_JSON": text }),
    }
}

/// The key the tool loop recognises as arguments that did not parse.
pub const INVALID_JSON: &str = "INVALID_JSON";

pub fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        s.push(A[(n >> 18) as usize & 63] as char);
        s.push(A[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

fn json_body(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).expect("a JSON value serialises")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc() {
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }

    #[test]
    fn arguments_parse_strictly() {
        assert_eq!(parse_arguments(""), serde_json::json!({}));
        assert_eq!(parse_arguments("{\"a\":1}"), serde_json::json!({"a": 1}));
        assert!(parse_arguments("{\"a\":").get(INVALID_JSON).is_some());
        assert!(parse_arguments("[1]").get(INVALID_JSON).is_some());
    }
}
