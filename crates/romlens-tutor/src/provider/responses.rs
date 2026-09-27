//! OpenAI's Responses API (`POST /v1/responses`, streamed), which OpenAI's
//! current models need for tools. Also spoken by some local servers
//! (Ollama from 0.13.3, LM Studio) and by LiteLLM.
//!
//! - Stateless: `store: false`, with the encrypted reasoning asked for and
//!   sent back, so no conversation lives on OpenAI's servers.
//! - Reasoning with its effort and a summary.
//! - Function tools strict; calls and their outputs matched by `call_id`,
//!   pictures in outputs as `input_image`.
//! - `prompt_cache_key` routes a conversation's requests to its cache.
//! - An assistant turn this protocol made goes back as its reasoning items
//!   verbatim, and its messages and calls rebuilt without their ids (with
//!   `store: false` the server does not keep items to refer to).

use serde_json::{Map, Value, json};

use super::{
    Delta, HttpRequest, Parser, Protocol, Reply, Request, Stop, StreamError, json_body,
    parse_arguments,
};
use crate::models::Thinking;
use crate::sse;
use crate::transcript::{Block, Native, Part, Role, Usage, wire_id};

pub fn build(r: &Request) -> HttpRequest {
    let mut body = Map::new();
    body.insert("model".into(), r.model.into());
    body.insert("stream".into(), true.into());
    body.insert("store".into(), false.into());
    let instructions = [r.system, r.digest]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n\n");
    if !instructions.is_empty() {
        body.insert("instructions".into(), instructions.into());
    }
    body.insert("max_output_tokens".into(), r.max_tokens.into());
    if !r.tools.is_empty() {
        let tools: Vec<Value> = r
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.schema,
                    "strict": r.endpoint.strict,
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
        body.insert("parallel_tool_calls".into(), true.into());
    }
    if r.thinking() == Thinking::Reasoning {
        let mut reasoning = json!({"summary": "auto"});
        if let Some(e) = r.effort() {
            reasoning["effort"] = e.into();
        }
        body.insert("reasoning".into(), reasoning);
        body.insert("include".into(), json!(["reasoning.encrypted_content"]));
    }
    if !r.cache_key.is_empty() {
        body.insert("prompt_cache_key".into(), r.cache_key.into());
    }
    body.insert("input".into(), Value::Array(input(r)));
    HttpRequest {
        url: r.endpoint.url("responses"),
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: json_body(&Value::Object(body)),
    }
}

fn image_part(r: &Request, image: &crate::transcript::ImageRef) -> Value {
    match r.data_url(image) {
        Some(url) => json!({"type": "input_image", "image_url": url}),
        None => json!({"type": "input_text", "text": "[a picture no longer kept]"}),
    }
}

fn input(r: &Request) -> Vec<Value> {
    let mut out = Vec::new();
    for t in r.turns {
        match t.role {
            Role::User => {
                let mut content = Vec::new();
                for b in &t.blocks {
                    match b {
                        Block::ToolResult { id, parts, .. } => {
                            let output = if parts.iter().all(|p| matches!(p, Part::Text { .. })) {
                                Value::String(
                                    parts
                                        .iter()
                                        .map(|p| match p {
                                            Part::Text { text } => text.as_str(),
                                            Part::Image { .. } => "",
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n"),
                                )
                            } else {
                                Value::Array(
                                    parts
                                        .iter()
                                        .map(|p| match p {
                                            Part::Text { text } => {
                                                json!({"type": "input_text", "text": text})
                                            }
                                            Part::Image { image } => image_part(r, image),
                                        })
                                        .collect(),
                                )
                            };
                            out.push(json!({"type": "function_call_output", "call_id": wire_id(id), "output": output}));
                        }
                        Block::Text { text } if !text.is_empty() => {
                            content.push(json!({"type": "input_text", "text": text}));
                        }
                        Block::Image { image } => content.push(image_part(r, image)),
                        _ => {}
                    }
                }
                if !content.is_empty() {
                    out.push(json!({"type": "message", "role": "user", "content": content}));
                }
            }
            Role::Assistant => match &t.native {
                Some(n) if n.protocol == Protocol::Responses => {
                    for item in n.content.as_array().into_iter().flatten() {
                        out.extend(replay(item));
                    }
                }
                _ => {
                    for b in &t.blocks {
                        match b {
                            Block::Text { text } if !text.is_empty() => out.push(json!({
                                "type": "message", "role": "assistant",
                                "content": [{"type": "output_text", "text": text}],
                            })),
                            Block::ToolCall { id, name, input } => out.push(json!({
                                "type": "function_call", "call_id": wire_id(id), "name": name,
                                "arguments": input.to_string(),
                            })),
                            _ => {}
                        }
                    }
                }
            },
        }
    }
    out
}

/// An output item as the next request's input.
fn replay(item: &Value) -> Option<Value> {
    match item["type"].as_str()? {
        "reasoning" => Some(item.clone()),
        "message" => {
            let content: Vec<Value> = item["content"]
                .as_array()?
                .iter()
                .filter(|c| c["type"] == "output_text")
                .map(|c| json!({"type": "output_text", "text": c["text"]}))
                .collect();
            (!content.is_empty())
                .then(|| json!({"type": "message", "role": "assistant", "content": content}))
        }
        "function_call" => Some(json!({
            "type": "function_call",
            "call_id": item["call_id"],
            "name": item["name"],
            "arguments": item["arguments"],
        })),
        _ => None,
    }
}

pub struct StreamParser {
    endpoint: String,
    model: String,
    /// Items by output index, as the server finished them.
    items: Vec<Value>,
    /// Call ids by output index, for argument deltas.
    calls: Vec<(u64, String)>,
    usage: Usage,
    status: Option<String>,
    incomplete: Option<String>,
}

impl StreamParser {
    pub fn new(endpoint: &str, model: &str) -> StreamParser {
        StreamParser {
            endpoint: endpoint.into(),
            model: model.into(),
            items: Vec::new(),
            calls: Vec::new(),
            usage: Usage::default(),
            status: None,
            incomplete: None,
        }
    }

    fn set(&mut self, index: u64, item: Value) {
        let i = index as usize;
        while self.items.len() <= i {
            self.items.push(Value::Null);
        }
        self.items[i] = item;
    }

    fn finished(&mut self, resp: &Value) {
        if let Some(m) = resp["model"].as_str() {
            self.model = m.into();
        }
        self.status = resp["status"].as_str().map(str::to_owned);
        self.incomplete = resp["incomplete_details"]["reason"]
            .as_str()
            .map(str::to_owned);
        let u = &resp["usage"];
        let n = |v: &Value| v.as_u64().unwrap_or(0);
        let input = n(&u["input_tokens"]);
        let read = n(&u["input_tokens_details"]["cached_tokens"]);
        let write = n(&u["input_tokens_details"]["cache_write_tokens"]);
        self.usage = Usage {
            input: input.saturating_sub(read + write),
            output: n(&u["output_tokens"]),
            cache_read: read,
            cache_write: write,
        };
        // The final list is authoritative where the stream missed an item.
        if let Some(out) = resp["output"].as_array() {
            for (i, item) in out.iter().enumerate() {
                if self.items.get(i).is_none_or(Value::is_null) {
                    self.set(i as u64, item.clone());
                }
            }
        }
    }
}

impl Parser for StreamParser {
    fn event(&mut self, e: &sse::Event) -> Result<Vec<Delta>, StreamError> {
        let v: Value = serde_json::from_str(&e.data)
            .map_err(|err| StreamError::Malformed(format!("{err}: {}", e.data)))?;
        let kind = v["type"].as_str().unwrap_or_default();
        let mut out = Vec::new();
        match kind {
            "response.output_item.added" => {
                let item = &v["item"];
                if item["type"] == "function_call" {
                    let id = item["call_id"].as_str().unwrap_or_default().to_owned();
                    self.calls
                        .push((v["output_index"].as_u64().unwrap_or(0), id.clone()));
                    out.push(Delta::ToolCallStarted {
                        id,
                        name: item["name"].as_str().unwrap_or_default().into(),
                    });
                }
            }
            "response.output_text.delta" | "response.refusal.delta" => {
                out.push(Delta::Text(v["delta"].as_str().unwrap_or_default().into()));
            }
            "response.reasoning_summary_text.delta" => {
                out.push(Delta::Reasoning(
                    v["delta"].as_str().unwrap_or_default().into(),
                ));
            }
            "response.reasoning_summary_part.added" => {
                // Summary parts are separate paragraphs.
                if v["summary_index"].as_u64().unwrap_or(0) > 0 {
                    out.push(Delta::Reasoning("\n\n".into()));
                }
            }
            "response.function_call_arguments.delta" => {
                let at = v["output_index"].as_u64().unwrap_or(0);
                if let Some((_, id)) = self.calls.iter().find(|(i, _)| *i == at) {
                    out.push(Delta::ToolArguments {
                        id: id.clone(),
                        text: v["delta"].as_str().unwrap_or_default().into(),
                    });
                }
            }
            "response.output_item.done" => {
                self.set(v["output_index"].as_u64().unwrap_or(0), v["item"].clone());
            }
            "response.completed" | "response.incomplete" => self.finished(&v["response"]),
            "response.failed" => {
                let err = &v["response"]["error"];
                return Err(StreamError::Provider {
                    kind: err["code"].as_str().unwrap_or("failed").into(),
                    message: err["message"].as_str().unwrap_or_default().into(),
                });
            }
            "error" => {
                return Err(StreamError::Provider {
                    kind: v["code"].as_str().unwrap_or("error").into(),
                    message: v["message"].as_str().unwrap_or_default().into(),
                });
            }
            _ => {}
        }
        Ok(out)
    }

    fn finish(self: Box<Self>) -> Result<Reply, StreamError> {
        let Some(status) = self.status.clone() else {
            return Err(StreamError::Truncated);
        };
        let items: Vec<Value> = self.items.into_iter().filter(|i| !i.is_null()).collect();
        let mut blocks = Vec::new();
        let mut refused = false;
        for item in &items {
            match item["type"].as_str().unwrap_or_default() {
                "reasoning" => {
                    let summary = item["summary"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|s| s["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    if !summary.is_empty() {
                        blocks.push(Block::Reasoning { summary });
                    }
                }
                "message" => {
                    for c in item["content"].as_array().into_iter().flatten() {
                        let (text, refusal) = match c["type"].as_str() {
                            Some("output_text") => (c["text"].as_str(), false),
                            Some("refusal") => (c["refusal"].as_str(), true),
                            _ => (None, false),
                        };
                        refused |= refusal;
                        if let Some(t) = text.filter(|t| !t.is_empty()) {
                            blocks.push(Block::Text { text: t.into() });
                        }
                    }
                }
                "function_call" => blocks.push(Block::ToolCall {
                    id: item["call_id"].as_str().unwrap_or_default().into(),
                    name: item["name"].as_str().unwrap_or_default().into(),
                    input: parse_arguments(item["arguments"].as_str().unwrap_or_default()),
                }),
                _ => {}
            }
        }
        let calls = blocks.iter().any(|b| matches!(b, Block::ToolCall { .. }));
        let stop = if refused {
            Stop::Refusal { category: None }
        } else if status == "incomplete" {
            match self.incomplete.as_deref() {
                Some("max_output_tokens") => Stop::MaxTokens,
                Some("content_filter") => Stop::Refusal {
                    category: Some("content_filter".into()),
                },
                Some(r) => Stop::Other { reason: r.into() },
                None => Stop::Other {
                    reason: "incomplete".into(),
                },
            }
        } else if calls {
            Stop::ToolUse
        } else {
            Stop::EndTurn
        };
        Ok(Reply {
            blocks,
            native: Native {
                protocol: Protocol::Responses,
                endpoint: self.endpoint,
                model: self.model.clone(),
                content: Value::Array(items),
            },
            usage: self.usage,
            stop,
            model: self.model,
        })
    }
}
