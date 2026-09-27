//! Chat Completions (`POST /v1/chat/completions`, streamed), which every
//! local server and proxy speaks: Ollama, LM Studio, vLLM, LiteLLM.
//!
//! - Tool calls arrive as fragments keyed by index; a server that gives no
//!   id gets one of ours.
//! - Reasoning is read from whichever field the server uses: `reasoning`
//!   (Ollama, vLLM, LM Studio), `reasoning_content` (LiteLLM, DeepSeek), or
//!   LiteLLM's `thinking_blocks`, which alone are sent back, and only to the
//!   endpoint that made them.
//! - A tool's message holds text only, so pictures in a tool's result follow
//!   in a user message after the round's results.
//! - A model that does not see pictures is told one was left out.

use serde_json::{Map, Value, json};

use super::{Delta, HttpRequest, Parser, Protocol, Reply, Request, Stop, StreamError, json_body};
use crate::sse;
use crate::transcript::{Block, ImageRef, Native, Part, Role, Usage, wire_id};

pub fn build(r: &Request) -> HttpRequest {
    let mut body = Map::new();
    body.insert("model".into(), r.model.into());
    body.insert("stream".into(), true.into());
    body.insert("stream_options".into(), json!({"include_usage": true}));
    body.insert("max_tokens".into(), r.max_tokens.into());
    let mut messages = Vec::new();
    let system = [r.system, r.digest]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n\n");
    if !system.is_empty() {
        messages.push(json!({"role": "system", "content": system}));
    }
    messages.extend(conversation(r));
    body.insert("messages".into(), Value::Array(messages));
    if !r.tools.is_empty() {
        let tools: Vec<Value> = r
            .tools
            .iter()
            .map(|t| {
                let mut f =
                    json!({"name": t.name, "description": t.description, "parameters": t.schema});
                if r.endpoint.strict && t.strict {
                    f["strict"] = true.into();
                }
                json!({"type": "function", "function": f})
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
        if r.endpoint.tool_choice {
            body.insert("tool_choice".into(), "auto".into());
        }
    }
    if let Some(e) = r.effort() {
        body.insert("reasoning_effort".into(), e.into());
    }
    HttpRequest {
        url: r.endpoint.url("chat/completions"),
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: json_body(&Value::Object(body)),
    }
}

fn picture(r: &Request, image: &ImageRef) -> Value {
    if !r.caps.vision {
        return json!({"type": "text", "text": "[a picture was left out: this model does not see pictures]"});
    }
    match r.data_url(image) {
        Some(url) => json!({"type": "image_url", "image_url": {"url": url}}),
        None => json!({"type": "text", "text": "[a picture no longer kept]"}),
    }
}

fn conversation(r: &Request) -> Vec<Value> {
    let mut out = Vec::new();
    for t in r.turns {
        match t.role {
            Role::User => {
                let mut content = Vec::new();
                let mut later = Vec::new();
                for b in &t.blocks {
                    match b {
                        Block::ToolResult {
                            id,
                            parts,
                            is_error,
                        } => {
                            let mut text = String::new();
                            if *is_error {
                                text.push_str("Error: ");
                            }
                            for p in parts {
                                match p {
                                    Part::Text { text: t } => text.push_str(t),
                                    Part::Image { image } => {
                                        text.push_str("[picture follows]");
                                        later.push(picture(r, image));
                                    }
                                }
                            }
                            out.push(json!({"role": "tool", "tool_call_id": wire_id(id), "content": text}));
                        }
                        Block::Text { text } if !text.is_empty() => {
                            content.push(json!({"type": "text", "text": text}));
                        }
                        Block::Image { image } => content.push(picture(r, image)),
                        _ => {}
                    }
                }
                if !later.is_empty() {
                    let mut c = vec![
                        json!({"type": "text", "text": "The pictures from those tool results:"}),
                    ];
                    c.extend(later);
                    out.push(json!({"role": "user", "content": c}));
                }
                if !content.is_empty() {
                    let only_text = content.iter().all(|c| c["type"] == "text");
                    let content = if only_text {
                        Value::String(
                            content
                                .iter()
                                .filter_map(|c| c["text"].as_str())
                                .collect::<Vec<_>>()
                                .join("\n\n"),
                        )
                    } else {
                        Value::Array(content)
                    };
                    out.push(json!({"role": "user", "content": content}));
                }
            }
            Role::Assistant => {
                let text = t.text();
                let calls: Vec<Value> = t
                    .tool_calls()
                    .map(|(id, name, input)| {
                        json!({"id": wire_id(id), "type": "function",
                               "function": {"name": name, "arguments": input.to_string()}})
                    })
                    .collect();
                if text.is_empty() && calls.is_empty() {
                    continue;
                }
                let mut m = json!({"role": "assistant", "content": if text.is_empty() { Value::Null } else { text.into() }});
                if !calls.is_empty() {
                    m["tool_calls"] = Value::Array(calls);
                }
                if let Some(n) = &t.native
                    && n.protocol == Protocol::Chat
                    && n.endpoint == r.endpoint.id
                    && let Some(tb) = n.content.get("thinking_blocks")
                {
                    m["thinking_blocks"] = tb.clone();
                }
                out.push(m);
            }
        }
    }
    out
}

#[derive(Default)]
struct Call {
    id: String,
    name: String,
    arguments: String,
}

pub struct StreamParser {
    endpoint: String,
    model: String,
    text: String,
    reasoning: String,
    calls: Vec<(u64, Call)>,
    thinking_blocks: Vec<Value>,
    usage: Usage,
    finish: Option<String>,
    done: bool,
}

impl StreamParser {
    pub fn new(endpoint: &str, model: &str) -> StreamParser {
        StreamParser {
            endpoint: endpoint.into(),
            model: model.into(),
            text: String::new(),
            reasoning: String::new(),
            calls: Vec::new(),
            thinking_blocks: Vec::new(),
            usage: Usage::default(),
            finish: None,
            done: false,
        }
    }
}

impl Parser for StreamParser {
    fn event(&mut self, e: &sse::Event) -> Result<Vec<Delta>, StreamError> {
        if e.data.trim() == "[DONE]" {
            self.done = true;
            return Ok(Vec::new());
        }
        let v: Value = serde_json::from_str(&e.data)
            .map_err(|err| StreamError::Malformed(format!("{err}: {}", e.data)))?;
        if let Some(err) = v.get("error") {
            return Err(StreamError::Provider {
                kind: err["type"]
                    .as_str()
                    .or(err["code"].as_str())
                    .unwrap_or("error")
                    .into(),
                message: err["message"].as_str().unwrap_or_default().into(),
            });
        }
        if let Some(m) = v["model"].as_str() {
            self.model = m.into();
        }
        let u = &v["usage"];
        if u.is_object() {
            let n = |v: &Value| v.as_u64().unwrap_or(0);
            let read = n(&u["prompt_tokens_details"]["cached_tokens"])
                .max(n(&u["cache_read_input_tokens"]));
            let write = n(&u["cache_creation_input_tokens"]);
            self.usage = Usage {
                input: n(&u["prompt_tokens"]).saturating_sub(read + write),
                output: n(&u["completion_tokens"]),
                cache_read: read,
                cache_write: write,
            };
        }
        let mut out = Vec::new();
        for choice in v["choices"].as_array().into_iter().flatten() {
            let d = &choice["delta"];
            if let Some(t) = d["content"].as_str().filter(|t| !t.is_empty()) {
                self.text.push_str(t);
                out.push(Delta::Text(t.into()));
            }
            for key in ["reasoning", "reasoning_content"] {
                if let Some(t) = d[key].as_str().filter(|t| !t.is_empty()) {
                    self.reasoning.push_str(t);
                    out.push(Delta::Reasoning(t.into()));
                }
            }
            if let Some(tb) = d["thinking_blocks"].as_array() {
                self.thinking_blocks.extend(tb.iter().cloned());
            }
            for tc in d["tool_calls"].as_array().into_iter().flatten() {
                let index = tc["index"].as_u64().unwrap_or(self.calls.len() as u64);
                let at = match self.calls.iter().position(|(i, _)| *i == index) {
                    Some(at) => at,
                    None => {
                        self.calls.push((index, Call::default()));
                        self.calls.len() - 1
                    }
                };
                let call = &mut self.calls[at].1;
                let fresh = call.name.is_empty();
                if let Some(id) = tc["id"].as_str().filter(|s| !s.is_empty()) {
                    call.id = id.into();
                }
                if let Some(n) = tc["function"]["name"].as_str() {
                    call.name.push_str(n);
                }
                if call.id.is_empty() {
                    call.id = format!("call_{index}");
                }
                if fresh && !call.name.is_empty() {
                    out.push(Delta::ToolCallStarted {
                        id: call.id.clone(),
                        name: call.name.clone(),
                    });
                }
                // Ollama's own API gives an object; the OpenAI form a string.
                let args = &tc["function"]["arguments"];
                let piece = match args {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                };
                if !piece.is_empty() {
                    call.arguments.push_str(&piece);
                    out.push(Delta::ToolArguments {
                        id: call.id.clone(),
                        text: piece,
                    });
                }
            }
            if let Some(f) = choice["finish_reason"].as_str() {
                self.finish = Some(f.into());
            }
        }
        Ok(out)
    }

    fn finish(self: Box<Self>) -> Result<Reply, StreamError> {
        if self.finish.is_none() && !self.done {
            return Err(StreamError::Truncated);
        }
        let mut blocks = Vec::new();
        if !self.reasoning.is_empty() {
            blocks.push(Block::Reasoning {
                summary: self.reasoning,
            });
        }
        if !self.text.is_empty() {
            blocks.push(Block::Text { text: self.text });
        }
        let calls = !self.calls.is_empty();
        for (_, c) in self.calls {
            blocks.push(Block::ToolCall {
                id: c.id,
                name: c.name,
                input: super::parse_arguments(&c.arguments),
            });
        }
        let stop = match self.finish.as_deref() {
            Some("length") => Stop::MaxTokens,
            Some("content_filter") => Stop::Refusal {
                category: Some("content_filter".into()),
            },
            _ if calls => Stop::ToolUse,
            Some("stop") | None => Stop::EndTurn,
            Some("tool_calls") => Stop::ToolUse,
            Some(r) => Stop::Other { reason: r.into() },
        };
        let content = if self.thinking_blocks.is_empty() {
            json!({})
        } else {
            json!({"thinking_blocks": self.thinking_blocks})
        };
        Ok(Reply {
            blocks,
            native: Native {
                protocol: Protocol::Chat,
                endpoint: self.endpoint,
                model: self.model.clone(),
                content,
            },
            usage: self.usage,
            stop,
            model: self.model,
        })
    }
}
