//! Anthropic's Messages API (`POST /v1/messages`, streamed).
//!
//! - Adaptive thinking with summaries shown, and the effort in
//!   `output_config`, on models that take them.
//! - Tools strict, with their input streamed as it is written
//!   (`eager_input_streaming`); the loop parses it strictly and answers an
//!   error for input that does not parse.
//! - Four cache breakpoints: the last tool, the system prompt, the digest,
//!   and the newest block of the newest message.
//! - A refusal is retried on a fallback model the API picks, where the model
//!   supports it (`fallbacks: "default"`).
//! - Assistant turns Anthropic made go back as it sent them, thinking and
//!   signatures and all, even after a change of Claude model: the API drops
//!   what the new model cannot read.

use serde_json::{Map, Value, json};

use super::{
    Delta, HttpRequest, Parser, Protocol, Reply, Request, Stop, StreamError, json_body,
    parse_arguments,
};
use crate::models::Thinking;
use crate::sse;
use crate::transcript::{Block, Native, Part, Role, Usage, wire_id};

pub const VERSION: &str = "2023-06-01";
pub const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

fn cached() -> Value {
    json!({"type": "ephemeral"})
}

pub fn build(r: &Request) -> HttpRequest {
    let mut body = Map::new();
    body.insert("model".into(), r.model.into());
    body.insert("max_tokens".into(), r.max_tokens.into());
    body.insert("stream".into(), true.into());

    let mut system = Vec::new();
    for text in [r.system, r.digest] {
        if !text.is_empty() {
            system.push(json!({"type": "text", "text": text, "cache_control": cached()}));
        }
    }
    if !system.is_empty() {
        body.insert("system".into(), Value::Array(system));
    }

    if !r.tools.is_empty() {
        let n = r.tools.len();
        let tools: Vec<Value> = r
            .tools
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mut o = Map::new();
                o.insert("name".into(), t.name.clone().into());
                o.insert("description".into(), t.description.clone().into());
                o.insert("input_schema".into(), t.schema.clone());
                if r.endpoint.strict {
                    o.insert("strict".into(), true.into());
                }
                o.insert("eager_input_streaming".into(), true.into());
                if i + 1 == n {
                    o.insert("cache_control".into(), cached());
                }
                Value::Object(o)
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }

    let mut messages = messages(r);
    if let Some(last) = messages.last_mut()
        && let Some(Value::Array(content)) = last.get_mut("content")
        && let Some(Value::Object(b)) = content.last_mut()
    {
        b.insert("cache_control".into(), cached());
    }
    body.insert("messages".into(), Value::Array(messages));

    if r.thinking() == Thinking::Adaptive {
        body.insert(
            "thinking".into(),
            json!({"type": "adaptive", "display": "summarized"}),
        );
    }
    if let Some(e) = r.effort() {
        body.insert("output_config".into(), json!({"effort": e}));
    }
    let mut headers = vec![
        ("content-type".to_owned(), "application/json".to_owned()),
        ("anthropic-version".to_owned(), VERSION.to_owned()),
    ];
    if r.caps.fallbacks {
        body.insert("fallbacks".into(), "default".into());
        headers.push(("anthropic-beta".to_owned(), FALLBACK_BETA.to_owned()));
    }
    HttpRequest {
        url: r.endpoint.url("v1/messages"),
        headers,
        body: json_body(&Value::Object(body)),
    }
}

fn messages(r: &Request) -> Vec<Value> {
    let mut out: Vec<(Role, Vec<Value>)> = Vec::new();
    for t in r.turns {
        let content = match t.role {
            Role::User => user_content(r, &t.blocks),
            Role::Assistant => match &t.native {
                Some(n) if n.protocol == Protocol::Anthropic => match &n.content {
                    Value::Array(a) => a.clone(),
                    _ => Vec::new(),
                },
                _ => assistant_content(&t.blocks),
            },
        };
        if content.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some((role, c)) if *role == t.role => c.extend(content),
            _ => out.push((t.role, content)),
        }
    }
    out.into_iter()
        .map(|(role, content)| {
            json!({
                "role": match role { Role::User => "user", Role::Assistant => "assistant" },
                "content": content,
            })
        })
        .collect()
}

fn image(r: &Request, image: &crate::transcript::ImageRef) -> Value {
    match r.attachments.bytes(image) {
        Some(b) => json!({
            "type": "image",
            "source": {"type": "base64", "media_type": image.media_type, "data": super::base64(&b)},
        }),
        None => json!({"type": "text", "text": "[a picture no longer kept]"}),
    }
}

/// A user turn: tool results first, as the API asks.
fn user_content(r: &Request, blocks: &[Block]) -> Vec<Value> {
    let mut results = Vec::new();
    let mut rest = Vec::new();
    for b in blocks {
        match b {
            Block::ToolResult {
                id,
                parts,
                is_error,
            } => {
                let content: Vec<Value> = parts
                    .iter()
                    .map(|p| match p {
                        Part::Text { text } => json!({"type": "text", "text": text}),
                        Part::Image { image: i } => image(r, i),
                    })
                    .collect();
                let mut o =
                    json!({"type": "tool_result", "tool_use_id": wire_id(id), "content": content});
                if *is_error {
                    o["is_error"] = true.into();
                }
                results.push(o);
            }
            Block::Text { text } if !text.is_empty() => {
                rest.push(json!({"type": "text", "text": text}));
            }
            Block::Image { image: i } => rest.push(image(r, i)),
            _ => {}
        }
    }
    results.extend(rest);
    results
}

fn assistant_content(blocks: &[Block]) -> Vec<Value> {
    blocks
        .iter()
        .filter_map(|b| match b {
            Block::Text { text } if !text.is_empty() => Some(json!({"type": "text", "text": text})),
            Block::ToolCall { id, name, input } => {
                Some(json!({"type": "tool_use", "id": wire_id(id), "name": name, "input": input}))
            }
            _ => None,
        })
        .collect()
}

#[derive(Default)]
struct Acc {
    start: Value,
    kind: String,
    text: String,
    signature: String,
    json: String,
    id: String,
    name: String,
}

pub struct StreamParser {
    endpoint: String,
    model: String,
    blocks: Vec<Acc>,
    usage: Usage,
    stop: Option<Stop>,
    ended: bool,
}

impl StreamParser {
    pub fn new(endpoint: &str, model: &str) -> StreamParser {
        StreamParser {
            endpoint: endpoint.into(),
            model: model.into(),
            blocks: Vec::new(),
            usage: Usage::default(),
            stop: None,
            ended: false,
        }
    }

    fn take_usage(&mut self, u: &Value) {
        let n = |k: &str| u.get(k).and_then(Value::as_u64);
        if let Some(v) = n("input_tokens") {
            self.usage.input = v;
        }
        if let Some(v) = n("output_tokens") {
            self.usage.output = v;
        }
        if let Some(v) = n("cache_read_input_tokens") {
            self.usage.cache_read = v;
        }
        if let Some(v) = n("cache_creation_input_tokens") {
            self.usage.cache_write = v;
        }
    }

    fn block(&mut self, index: &Value) -> Result<&mut Acc, StreamError> {
        let i = index
            .as_u64()
            .ok_or_else(|| StreamError::Malformed("a block without an index".into()))?
            as usize;
        self.blocks
            .get_mut(i)
            .ok_or_else(|| StreamError::Malformed(format!("a delta for block {i} before it began")))
    }
}

fn stop(reason: &str, details: Option<&Value>) -> Stop {
    match reason {
        "end_turn" | "stop_sequence" => Stop::EndTurn,
        "tool_use" => Stop::ToolUse,
        "max_tokens" => Stop::MaxTokens,
        "refusal" => Stop::Refusal {
            category: details
                .and_then(|d| d.get("category"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        "model_context_window_exceeded" => Stop::ContextFull,
        "pause_turn" => Stop::Paused,
        other => Stop::Other {
            reason: other.into(),
        },
    }
}

impl Parser for StreamParser {
    fn event(&mut self, e: &sse::Event) -> Result<Vec<Delta>, StreamError> {
        let v: Value = serde_json::from_str(&e.data)
            .map_err(|err| StreamError::Malformed(format!("{err}: {}", e.data)))?;
        let kind = v.get("type").and_then(Value::as_str).unwrap_or_default();
        let mut out = Vec::new();
        match kind {
            "message_start" => {
                let m = &v["message"];
                if let Some(model) = m.get("model").and_then(Value::as_str) {
                    self.model = model.into();
                }
                self.take_usage(&m["usage"]);
            }
            "content_block_start" => {
                let b = &v["content_block"];
                let i = v["index"].as_u64().unwrap_or(self.blocks.len() as u64) as usize;
                while self.blocks.len() <= i {
                    self.blocks.push(Acc::default());
                }
                let acc = &mut self.blocks[i];
                acc.start = b.clone();
                acc.kind = b["type"].as_str().unwrap_or_default().into();
                match acc.kind.as_str() {
                    "text" => acc.text = b["text"].as_str().unwrap_or_default().into(),
                    "thinking" => {
                        acc.text = b["thinking"].as_str().unwrap_or_default().into();
                        acc.signature = b["signature"].as_str().unwrap_or_default().into();
                    }
                    "tool_use" => {
                        acc.id = b["id"].as_str().unwrap_or_default().into();
                        acc.name = b["name"].as_str().unwrap_or_default().into();
                        out.push(Delta::ToolCallStarted {
                            id: acc.id.clone(),
                            name: acc.name.clone(),
                        });
                    }
                    _ => {}
                }
                if !acc.text.is_empty() {
                    out.push(match acc.kind.as_str() {
                        "thinking" => Delta::Reasoning(acc.text.clone()),
                        _ => Delta::Text(acc.text.clone()),
                    });
                }
            }
            "content_block_delta" => {
                let d = &v["delta"];
                let acc = self.block(&v["index"])?;
                match d["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        let t = d["text"].as_str().unwrap_or_default();
                        acc.text.push_str(t);
                        out.push(Delta::Text(t.into()));
                    }
                    "thinking_delta" => {
                        let t = d["thinking"].as_str().unwrap_or_default();
                        acc.text.push_str(t);
                        out.push(Delta::Reasoning(t.into()));
                    }
                    "signature_delta" => {
                        acc.signature
                            .push_str(d["signature"].as_str().unwrap_or_default());
                    }
                    "input_json_delta" => {
                        let t = d["partial_json"].as_str().unwrap_or_default();
                        acc.json.push_str(t);
                        out.push(Delta::ToolArguments {
                            id: acc.id.clone(),
                            text: t.into(),
                        });
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                let d = &v["delta"];
                if let Some(r) = d.get("stop_reason").and_then(Value::as_str) {
                    let details = d.get("stop_details").or_else(|| v.get("stop_details"));
                    self.stop = Some(stop(r, details));
                }
                if let Some(u) = v.get("usage") {
                    self.take_usage(u);
                }
            }
            "message_stop" => self.ended = true,
            "error" => {
                let err = &v["error"];
                return Err(StreamError::Provider {
                    kind: err["type"].as_str().unwrap_or("error").into(),
                    message: err["message"].as_str().unwrap_or_default().into(),
                });
            }
            _ => {}
        }
        Ok(out)
    }

    fn finish(self: Box<Self>) -> Result<Reply, StreamError> {
        let stop = match (self.stop, self.ended) {
            (Some(s), _) => s,
            (None, _) => return Err(StreamError::Truncated),
        };
        let mut blocks = Vec::new();
        let mut native = Vec::new();
        for acc in self.blocks {
            match acc.kind.as_str() {
                // The API refuses an empty text block sent back.
                "text" if acc.text.is_empty() => {}
                "text" => {
                    native.push(json!({"type": "text", "text": acc.text}));
                    blocks.push(Block::Text { text: acc.text });
                }
                "thinking" => {
                    native.push(json!({"type": "thinking", "thinking": acc.text, "signature": acc.signature}));
                    if !acc.text.is_empty() {
                        blocks.push(Block::Reasoning { summary: acc.text });
                    }
                }
                "tool_use" => {
                    let input = parse_arguments(&acc.json);
                    native.push(
                        json!({"type": "tool_use", "id": acc.id, "name": acc.name, "input": input}),
                    );
                    blocks.push(Block::ToolCall {
                        id: acc.id,
                        name: acc.name,
                        input,
                    });
                }
                "" => {}
                _ => native.push(acc.start),
            }
        }
        Ok(Reply {
            blocks,
            native: Native {
                protocol: Protocol::Anthropic,
                endpoint: self.endpoint,
                model: self.model.clone(),
                content: Value::Array(native),
            },
            usage: self.usage,
            stop,
            model: self.model,
        })
    }
}
