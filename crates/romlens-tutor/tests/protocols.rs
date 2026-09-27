//! The three protocols on recorded streams, and one conversation carried
//! across them (docs/24, U1). The streams in `streams/` follow the
//! providers' documented event shapes; nothing here touches the network.

use std::collections::HashMap;

use romlens_tutor::models::capabilities;
use romlens_tutor::provider::{
    self, Delta, Endpoint, Protocol, Request, Stop, StreamError, ToolSpec,
};
use romlens_tutor::transcript::{Block, ImageRef, Part, Turn, Usage, sendable};
use serde_json::{Value, json};

fn stream(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/streams/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn tools() -> Vec<ToolSpec> {
    ["rom_info", "disassemble"]
        .iter()
        .map(|n| ToolSpec {
            name: (*n).into(),
            description: format!("{n} does its thing"),
            schema: json!({"type": "object", "properties": {}, "required": [], "additionalProperties": false}),
            strict: *n == "disassemble",
        })
        .collect()
}

fn local() -> Endpoint {
    Endpoint {
        id: "ollama".into(),
        protocol: Protocol::Chat,
        base_url: "http://localhost:11434/v1/".into(),
        tool_choice: false,
        strict: false,
        vision: false,
    }
}

fn body(
    endpoint: &Endpoint,
    model: &str,
    turns: &[Turn],
    pictures: &HashMap<String, Vec<u8>>,
) -> (provider::HttpRequest, Value) {
    let tools = tools();
    let sent = sendable(turns);
    let r = Request {
        endpoint,
        model,
        caps: capabilities(model, endpoint.vision),
        system: "You are the tutor.",
        digest: "ROM: SUPER MARIOWORLD",
        tools: &tools,
        turns: &sent,
        effort: None,
        max_tokens: 64_000,
        cache_key: "conv-1",
        attachments: pictures,
    };
    let h = provider::build(&r);
    let v = serde_json::from_slice(&h.body).unwrap();
    (h, v)
}

fn picture() -> (ImageRef, HashMap<String, Vec<u8>>) {
    let i = ImageRef {
        id: "a1".into(),
        media_type: "image/png".into(),
    };
    let mut m = HashMap::new();
    m.insert("a1".to_owned(), b"PNG".to_vec());
    (i, m)
}

#[test]
fn anthropic_reads_a_tool_round() {
    let (deltas, reply) = provider::parse_all(
        &Endpoint::anthropic(),
        "claude-opus-5",
        &stream("anthropic_tool_round.sse"),
    )
    .unwrap();
    assert_eq!(reply.stop, Stop::ToolUse);
    assert_eq!(
        reply.usage,
        Usage {
            input: 120,
            output: 87,
            cache_read: 0,
            cache_write: 9000
        }
    );
    assert_eq!(
        reply.blocks,
        vec![
            Block::Reasoning {
                summary: "RESET is at the vector; read it first.".into()
            },
            Block::Text {
                text: "Let me look at `$00:8000`.".into()
            },
            Block::ToolCall {
                id: "toolu_01A".into(),
                name: "disassemble".into(),
                input: json!({"address": "$00:8000", "count": 24}),
            },
        ]
    );
    // The thinking goes back with its signature.
    assert_eq!(
        reply.native.content[0],
        json!({"type": "thinking", "thinking": "RESET is at the vector; read it first.", "signature": "EqQBCgIYAhIM1gbcDa9GJwZA"})
    );
    assert!(deltas.contains(&Delta::ToolCallStarted {
        id: "toolu_01A".into(),
        name: "disassemble".into()
    }));
    let text: String = deltas
        .iter()
        .filter_map(|d| match d {
            Delta::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Let me look at `$00:8000`.");
}

#[test]
fn anthropic_refusals_and_errors() {
    let (_, r) = provider::parse_all(
        &Endpoint::anthropic(),
        "claude-opus-5",
        &stream("anthropic_refusal.sse"),
    )
    .unwrap();
    assert_eq!(
        r.stop,
        Stop::Refusal {
            category: Some("cyber".into())
        }
    );
    assert_eq!(r.usage.cache_read, 9000);
    let e = provider::parse_all(
        &Endpoint::anthropic(),
        "claude-opus-5",
        &stream("anthropic_error.sse"),
    )
    .unwrap_err();
    assert_eq!(
        e,
        StreamError::Provider {
            kind: "overloaded_error".into(),
            message: "Overloaded".into()
        }
    );
    let full = stream("anthropic_tool_round.sse");
    let cut = &full[..full.len() / 2];
    assert_eq!(
        provider::parse_all(&Endpoint::anthropic(), "claude-opus-5", cut).unwrap_err(),
        StreamError::Truncated
    );
}

/// A question, the model's tool round, and the results: what the loop
/// builds before its second request.
fn round(first: provider::Reply, pictures: bool) -> Vec<Turn> {
    let (image, _) = picture();
    let q = Turn::user(vec![
        Block::Image {
            image: image.clone(),
        },
        Block::Text {
            text: "What does RESET do?".into(),
        },
    ]);
    let calls: Vec<(String, String)> = first
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::ToolCall { id, name, .. } => Some((id.clone(), name.clone())),
            _ => None,
        })
        .collect();
    let a = first.into_turn();
    let results = Turn::user(
        calls
            .into_iter()
            .map(|(id, _)| Block::ToolResult {
                id,
                parts: if pictures {
                    vec![
                        Part::Text {
                            text: "SEI; CLC; XCE".into(),
                        },
                        Part::Image {
                            image: image.clone(),
                        },
                    ]
                } else {
                    vec![Part::Text {
                        text: "SEI; CLC; XCE".into(),
                    }]
                },
                is_error: false,
            })
            .collect(),
    );
    vec![q, a, results]
}

#[test]
fn anthropic_sends_back_what_it_sent_and_caches_the_prefix() {
    let (_, reply) = provider::parse_all(
        &Endpoint::anthropic(),
        "claude-opus-5",
        &stream("anthropic_tool_round.sse"),
    )
    .unwrap();
    let native = reply.native.content.clone();
    let turns = round(reply, true);
    let (_, pics) = picture();
    let (h, v) = body(&Endpoint::anthropic(), "claude-opus-5", &turns, &pics);
    assert_eq!(h.url, "https://api.anthropic.com/v1/messages");
    assert!(
        h.headers
            .contains(&("anthropic-version".into(), "2023-06-01".into()))
    );
    assert!(h.headers.contains(&(
        "anthropic-beta".into(),
        "server-side-fallback-2026-07-01".into()
    )));
    assert_eq!(v["fallbacks"], "default");
    assert_eq!(
        v["thinking"],
        json!({"type": "adaptive", "display": "summarized"})
    );
    assert_eq!(v["output_config"], json!({"effort": "high"}));
    assert_eq!(v["stream"], true);
    // Breakpoints: the last tool, both system blocks, the newest block.
    assert!(v["tools"][0].get("cache_control").is_none());
    assert_eq!(v["tools"][1]["cache_control"]["type"], "ephemeral");
    // Only the tools asked to be strict are, and only on endpoints that
    // take it.
    assert_eq!(v["tools"][1]["strict"], true);
    assert!(v["tools"][0].get("strict").is_none());
    assert_eq!(v["tools"][1]["eager_input_streaming"], true);
    assert_eq!(v["system"].as_array().unwrap().len(), 2);
    assert!(
        v["system"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["cache_control"].is_object())
    );
    let m = v["messages"].as_array().unwrap();
    assert_eq!(m.len(), 3);
    assert_eq!(m[0]["content"][0]["type"], "image");
    assert_eq!(m[0]["content"][0]["source"]["data"], "UE5H");
    // The assistant turn verbatim.
    assert_eq!(m[1]["content"], native);
    let result = &m[2]["content"][0];
    assert_eq!(result["type"], "tool_result");
    assert_eq!(result["tool_use_id"], "toolu_01A");
    assert_eq!(result["content"][1]["type"], "image");
    assert_eq!(result["cache_control"]["type"], "ephemeral");
    // Only the one breakpoint in the messages.
    let marks = serde_json::to_string(m)
        .unwrap()
        .matches("cache_control")
        .count();
    assert_eq!(marks, 1);
}

#[test]
fn responses_reads_reasoning_calls_and_usage() {
    let (deltas, r) = provider::parse_all(
        &Endpoint::openai(),
        "gpt-6-astra",
        &stream("responses_tool_round.sse"),
    )
    .unwrap();
    assert_eq!(r.stop, Stop::ToolUse);
    assert_eq!(r.model, "gpt-6-astra-2026-08-01");
    assert_eq!(
        r.usage,
        Usage {
            input: 904,
            output: 60,
            cache_read: 4096,
            cache_write: 0
        }
    );
    assert_eq!(
        r.blocks,
        vec![
            Block::Reasoning {
                summary: "Reading the reset vector.".into()
            },
            Block::ToolCall {
                id: "call_Ab1".into(),
                name: "rom_info".into(),
                input: json!({})
            },
        ]
    );
    assert_eq!(r.native.content[0]["encrypted_content"], "gAAAAABo-secret");
    assert!(deltas.contains(&Delta::ToolArguments {
        id: "call_Ab1".into(),
        text: "{}".into()
    }));

    let (_, t) = provider::parse_all(
        &Endpoint::openai(),
        "gpt-6-astra",
        &stream("responses_text.sse"),
    )
    .unwrap();
    assert_eq!(t.stop, Stop::MaxTokens);
    assert_eq!(t.text_len(), "RESET sets native mode at `$00:8003`.".len());
}

trait TextLen {
    fn text_len(&self) -> usize;
}

impl TextLen for provider::Reply {
    fn text_len(&self) -> usize {
        self.blocks
            .iter()
            .map(|b| match b {
                Block::Text { text } => text.len(),
                _ => 0,
            })
            .sum()
    }
}

#[test]
fn responses_is_stateless_and_replays_its_reasoning() {
    let (_, reply) = provider::parse_all(
        &Endpoint::openai(),
        "gpt-6-astra",
        &stream("responses_tool_round.sse"),
    )
    .unwrap();
    let turns = round(reply, true);
    let (_, pics) = picture();
    let (h, v) = body(&Endpoint::openai(), "gpt-6-astra", &turns, &pics);
    assert_eq!(h.url, "https://api.openai.com/v1/responses");
    assert_eq!(v["store"], false);
    assert_eq!(v["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(
        v["reasoning"],
        json!({"summary": "auto", "effort": "medium"})
    );
    assert_eq!(v["prompt_cache_key"], "conv-1");
    assert_eq!(
        v["instructions"],
        "You are the tutor.\n\nROM: SUPER MARIOWORLD"
    );
    assert_eq!(v["tools"][0]["type"], "function");
    assert_eq!(v["tools"][0]["strict"], false);
    assert_eq!(v["tools"][1]["strict"], true);
    let input = v["input"].as_array().unwrap();
    assert_eq!(input[0]["role"], "user");
    assert_eq!(
        input[0]["content"][0]["image_url"],
        "data:image/png;base64,UE5H"
    );
    assert_eq!(input[1]["type"], "reasoning");
    assert_eq!(input[1]["encrypted_content"], "gAAAAABo-secret");
    assert_eq!(
        input[2],
        json!({"type": "function_call", "call_id": "call_Ab1", "name": "rom_info", "arguments": "{}"})
    );
    assert_eq!(input[3]["type"], "function_call_output");
    assert_eq!(input[3]["call_id"], "call_Ab1");
    assert_eq!(input[3]["output"][1]["type"], "input_image");
    assert_eq!(input.len(), 4);
}

#[test]
fn chat_reads_ollama_and_litellm() {
    let (_, o) = provider::parse_all(&local(), "qwen3:32b", &stream("chat_ollama.sse")).unwrap();
    assert_eq!(o.stop, Stop::ToolUse);
    assert_eq!(
        o.blocks,
        vec![
            Block::Reasoning {
                summary: "The user wants RESET.".into()
            },
            Block::Text {
                text: "Looking.".into()
            },
            Block::ToolCall {
                id: "call_0".into(),
                name: "disassemble".into(),
                input: json!({"address": "$00:8000", "count": 8})
            },
        ]
    );
    assert_eq!(o.usage.input, 900);

    let lite = Endpoint {
        id: "litellm".into(),
        ..local()
    };
    let (_, l) = provider::parse_all(&lite, "claude-opus-5", &stream("chat_litellm.sse")).unwrap();
    let calls: Vec<_> = l
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::ToolCall { id, input, .. } => Some((id.clone(), input.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls,
        vec![
            ("toolu_X".to_owned(), json!({})),
            ("toolu_Y".to_owned(), json!({"from": "$00:8000"}))
        ]
    );
    assert_eq!(
        l.usage,
        Usage {
            input: 1000,
            output: 50,
            cache_read: 11000,
            cache_write: 0
        }
    );
    assert_eq!(
        l.native.content["thinking_blocks"][0]["signature"],
        "sig123"
    );
}

#[test]
fn chat_sends_pictures_after_the_results_and_thinking_only_home() {
    let lite = Endpoint {
        id: "litellm".into(),
        vision: true,
        ..local()
    };
    let (_, reply) = provider::parse_all(&lite, "some-local", &stream("chat_litellm.sse")).unwrap();
    let turns = round(reply, true);
    let (_, pics) = picture();

    let (h, v) = body(&lite, "some-local", &turns, &pics);
    assert_eq!(h.url, "http://localhost:11434/v1/chat/completions");
    assert!(v.get("tool_choice").is_none());
    assert!(v["tools"][0]["function"].get("strict").is_none());
    let m = v["messages"].as_array().unwrap();
    assert_eq!(m[0]["role"], "system");
    assert_eq!(m[1]["content"][0]["type"], "image_url");
    assert_eq!(m[2]["tool_calls"].as_array().unwrap().len(), 2);
    assert_eq!(m[2]["thinking_blocks"][0]["signature"], "sig123");
    assert_eq!(m[3]["role"], "tool");
    assert_eq!(m[3]["content"], "SEI; CLC; XCE[picture follows]");
    assert_eq!(m[4]["role"], "tool");
    assert_eq!(m[5]["role"], "user");
    assert_eq!(m[5]["content"][1]["type"], "image_url");
    assert_eq!(m[5]["content"][2]["type"], "image_url");

    // Another endpoint is not sent LiteLLM's thinking, and a model without
    // vision is told what it missed.
    let (_, v) = body(&local(), "some-local", &turns, &pics);
    let m = v["messages"].as_array().unwrap();
    assert!(m[2].get("thinking_blocks").is_none());
    assert!(
        m[1]["content"]
            .as_str()
            .unwrap()
            .contains("does not see pictures")
    );
}

#[test]
fn a_conversation_moves_between_providers() {
    let (_, claude) = provider::parse_all(
        &Endpoint::anthropic(),
        "claude-opus-5",
        &stream("anthropic_tool_round.sse"),
    )
    .unwrap();
    let mut turns = round(claude, false);
    let (_, pics) = picture();

    // To OpenAI: no Anthropic thinking, the call and its result by id.
    let (_, v) = body(&Endpoint::openai(), "gpt-6-astra", &turns, &pics);
    let s = serde_json::to_string(&v).unwrap();
    assert!(!s.contains("EqQBCgIYAhIM1gbcDa9GJwZA"));
    let input = v["input"].as_array().unwrap();
    assert_eq!(input[1]["type"], "message");
    assert_eq!(input[1]["content"][0]["text"], "Let me look at `$00:8000`.");
    assert_eq!(input[2]["call_id"], "toolu_01A");
    assert_eq!(input[3]["call_id"], "toolu_01A");

    // OpenAI answers; then back to Claude, whose own turn goes back as it
    // was, and OpenAI's as plain text.
    let (_, gpt) = provider::parse_all(
        &Endpoint::openai(),
        "gpt-6-astra",
        &stream("responses_text.sse"),
    )
    .unwrap();
    turns.push(gpt.into_turn());
    turns.push(Turn::user_text("And after that?"));
    let (_, v) = body(&Endpoint::anthropic(), "claude-sonnet-5", &turns, &pics);
    let m = v["messages"].as_array().unwrap();
    assert_eq!(m[1]["content"][0]["signature"], "EqQBCgIYAhIM1gbcDa9GJwZA");
    assert_eq!(m[3]["role"], "assistant");
    assert_eq!(
        m[3]["content"],
        json!([{"type": "text", "text": "RESET sets native mode at `$00:8003`."}])
    );
    assert!(!serde_json::to_string(&v).unwrap().contains("gAAAAAB"));
    assert_eq!(m.len(), 5);

    // And to a local model, everything as plain chat.
    let (_, v) = body(&local(), "qwen3:32b", &turns, &pics);
    let m = v["messages"].as_array().unwrap();
    assert_eq!(m.len(), 6);
    assert_eq!(m[2]["tool_calls"][0]["id"], "toolu_01A");
    assert_eq!(m[3]["tool_call_id"], "toolu_01A");
}

#[test]
fn haiku_is_not_asked_to_think() {
    let (_, pics) = picture();
    let (h, v) = body(
        &Endpoint::anthropic(),
        "claude-haiku-4-5",
        &[Turn::user_text("hi")],
        &pics,
    );
    assert!(v.get("thinking").is_none());
    assert!(v.get("output_config").is_none());
    assert!(v.get("fallbacks").is_none());
    assert!(!h.headers.iter().any(|(k, _)| k == "anthropic-beta"));
}
