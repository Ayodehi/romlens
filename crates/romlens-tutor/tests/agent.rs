//! The loop on recorded replies (docs/24, U2): tool rounds, retries,
//! cancelling, the cap, and a transcript every protocol still takes after
//! a round cut short.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use romlens_tutor::agent::{
    AcceptAll, AskError, Credentials, Deps, Event, Session, ToolContext, ToolKind, ToolOutput,
    Tools,
};
use romlens_tutor::http::{Cancel, FakeTransport, HttpError};
use romlens_tutor::provider::{Endpoint, Protocol, Stop, ToolSpec};
use romlens_tutor::transcript::{Block, Role, Turn};
use serde_json::{Value, json};

fn stream(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/streams/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

struct Key;
impl Credentials for Key {
    fn key(&self, _: &str) -> Option<String> {
        Some("sk-test".into())
    }
}

struct NoKey;
impl Credentials for NoKey {
    fn key(&self, _: &str) -> Option<String> {
        None
    }
}

#[derive(Default)]
struct Fake {
    ran: Mutex<Vec<(String, Value)>>,
    runs: AtomicU32,
    cancel_on_run: Option<Cancel>,
}

impl Tools for Fake {
    fn specs(&self) -> Vec<ToolSpec> {
        ["disassemble", "rom_info", "labels", "set_label"]
            .iter()
            .map(|n| ToolSpec {
                name: (*n).into(),
                description: String::new(),
                schema: json!({"type": "object", "properties": {}, "required": [], "additionalProperties": false}),
            })
            .collect()
    }
    fn kind(&self, name: &str) -> ToolKind {
        if name == "set_label" {
            ToolKind::Edit
        } else {
            ToolKind::Read
        }
    }
    fn run(&self, _: &str, name: &str, input: &Value, _: &ToolContext) -> ToolOutput {
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.ran.lock().unwrap().push((name.into(), input.clone()));
        if let Some(c) = &self.cancel_on_run {
            c.cancel();
        }
        ToolOutput::text(format!("{name} ran\nmore"))
    }
}

fn events() -> (Mutex<Vec<Event>>,) {
    (Mutex::new(Vec::new()),)
}

fn session() -> Session {
    let mut s = Session::new("conv-1", Endpoint::anthropic(), "claude-opus-5");
    s.system = "You are the tutor.".into();
    s.retry_unit = std::time::Duration::ZERO;
    s
}

#[test]
fn a_tool_round_then_an_answer() {
    let t = FakeTransport::new(vec![
        Ok(stream("anthropic_tool_round.sse")),
        Ok(stream("anthropic_text.sse")),
    ]);
    let tools = Fake::default();
    let (log,) = events();
    let on = |e: Event| log.lock().unwrap().push(e);
    let cancel = Cancel::new();
    let mut s = session();
    let stop = s
        .ask(
            Turn::user_text("What does RESET do?"),
            &Deps {
                transport: &t,
                credentials: &Key,
                tools: &tools,
                events: &on,
                approver: &AcceptAll,
                cancel: &cancel,
            },
        )
        .unwrap();
    assert_eq!(stop, Stop::EndTurn);
    assert_eq!(s.turns.len(), 4);
    assert_eq!(
        tools.ran.lock().unwrap()[0],
        (
            "disassemble".into(),
            json!({"address": "$00:8000", "count": 24})
        )
    );
    assert!(s.turns[3].text().contains("native mode"));

    let sent = t.sent();
    assert_eq!(sent.len(), 2);
    assert!(
        sent[0]
            .headers
            .contains(&("x-api-key".into(), "sk-test".into()))
    );
    let second: Value = serde_json::from_slice(&sent[1].body).unwrap();
    let m = second["messages"].as_array().unwrap();
    assert_eq!(m.len(), 3);
    assert_eq!(m[2]["content"][0]["type"], "tool_result");
    assert_eq!(
        m[2]["content"][0]["content"][0]["text"],
        "disassemble ran\nmore"
    );

    // The first request's prefix is the second's: the cache holds.
    let first: Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(first["system"], second["system"]);
    assert_eq!(first["tools"], second["tools"]);

    let log = log.lock().unwrap();
    assert!(log.contains(&Event::ToolFinished {
        id: "toolu_01A".into(),
        name: "disassemble".into(),
        summary: "disassemble ran".into(),
        is_error: false
    }));
    let costs: Vec<f64> = log
        .iter()
        .filter_map(|e| match e {
            Event::Cost { total, .. } => Some(*total),
            _ => None,
        })
        .collect();
    // 120 in, 9,000 written, 87 out; then 40 in, 9,000 read, 300 written, 20 out.
    let first_cost = (120.0 * 5.0 + 9000.0 * 6.25 + 87.0 * 25.0) / 1e6;
    let second_cost = (40.0 * 5.0 + 9000.0 * 0.5 + 300.0 * 6.25 + 20.0 * 25.0) / 1e6;
    assert!((costs[0] - first_cost).abs() < 1e-12);
    assert!((costs[1] - first_cost - second_cost).abs() < 1e-12);
    assert!((s.cost() - first_cost - second_cost).abs() < 1e-12);
    assert_eq!(
        log.last(),
        Some(&Event::Ended {
            stop: Stop::EndTurn
        })
    );
}

#[test]
fn overload_is_retried() {
    let t = FakeTransport::new(vec![
        Err(HttpError::Status {
            status: 529,
            message: "Overloaded".into(),
            retry_after: Some(0),
        }),
        Ok(stream("anthropic_error.sse")),
        Ok(stream("anthropic_text.sse")),
    ]);
    let (log,) = events();
    let on = |e: Event| log.lock().unwrap().push(e);
    let cancel = Cancel::new();
    let mut s = session();
    let deps = Deps {
        transport: &t,
        credentials: &Key,
        tools: &Fake::default(),
        events: &on,
        approver: &AcceptAll,
        cancel: &cancel,
    };
    assert_eq!(s.ask(Turn::user_text("hi"), &deps).unwrap(), Stop::EndTurn);
    assert_eq!(t.sent().len(), 3);
    let retries = log
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, Event::Retrying { .. }))
        .count();
    assert_eq!(retries, 2);

    let t = FakeTransport::new(vec![Err(HttpError::Status {
        status: 401,
        message: "invalid x-api-key".into(),
        retry_after: None,
    })]);
    let deps = Deps {
        transport: &t,
        credentials: &Key,
        tools: &Fake::default(),
        events: &on,
        approver: &AcceptAll,
        cancel: &cancel,
    };
    let e = session().ask(Turn::user_text("hi"), &deps).unwrap_err();
    assert_eq!(e.to_string(), "HTTP 401: invalid x-api-key");
    assert_eq!(t.sent().len(), 1);
}

#[test]
fn a_key_is_needed_except_locally() {
    let t = FakeTransport::new(vec![
        Ok(stream("chat_ollama.sse")),
        Ok(stream("chat_ollama.sse")),
    ]);
    let on = |_: Event| {};
    let cancel = Cancel::new();
    let deps = Deps {
        transport: &t,
        credentials: &NoKey,
        tools: &Fake::default(),
        events: &on,
        approver: &AcceptAll,
        cancel: &cancel,
    };
    assert_eq!(
        session().ask(Turn::user_text("hi"), &deps).unwrap_err(),
        AskError::NoKey("anthropic".into())
    );
    let local = Endpoint {
        id: "ollama".into(),
        protocol: Protocol::Chat,
        base_url: "http://localhost:11434/v1".into(),
        tool_choice: false,
        strict: false,
        vision: false,
    };
    let mut s = Session::new("c", local, "qwen3:32b");
    s.cost_cap = Some(0.0);
    // A local model costs nothing, but a zero cap still stops it.
    assert_eq!(
        s.ask(Turn::user_text("hi"), &deps).unwrap_err(),
        AskError::CapReached(0.0)
    );
    assert!(t.sent().is_empty());
}

#[test]
fn a_round_cut_short_is_answered_not_run() {
    let t = FakeTransport::new(vec![Ok(stream("anthropic_cut_round.sse"))]);
    let tools = Fake::default();
    let on = |_: Event| {};
    let cancel = Cancel::new();
    let mut s = session();
    let deps = Deps {
        transport: &t,
        credentials: &Key,
        tools: &tools,
        events: &on,
        approver: &AcceptAll,
        cancel: &cancel,
    };
    assert_eq!(
        s.ask(Turn::user_text("hi"), &deps).unwrap(),
        Stop::MaxTokens
    );
    assert_eq!(tools.runs.load(Ordering::SeqCst), 0);
    let last = s.turns.last().unwrap();
    assert_eq!(last.role, Role::User);
    assert!(
        matches!(&last.blocks[0], Block::ToolResult { is_error: true, id, .. } if id == "toolu_01A")
    );
}

#[test]
fn esc_during_the_tools_leaves_a_whole_transcript() {
    let cancel = Cancel::new();
    let t = FakeTransport::new(vec![Ok(stream("anthropic_tool_round.sse"))]);
    let tools = Fake {
        cancel_on_run: Some(cancel.clone()),
        ..Default::default()
    };
    let on = |_: Event| {};
    let mut s = session();
    let deps = Deps {
        transport: &t,
        credentials: &Key,
        tools: &tools,
        events: &on,
        approver: &AcceptAll,
        cancel: &cancel,
    };
    assert_eq!(
        s.ask(Turn::user_text("hi"), &deps).unwrap_err(),
        AskError::Cancelled
    );
    assert_eq!(s.turns.len(), 3);
    assert!(matches!(s.turns[2].blocks[0], Block::ToolResult { .. }));
    // The next question goes on from there.
    let t = FakeTransport::new(vec![Ok(stream("anthropic_text.sse"))]);
    let tools = Fake::default();
    let deps = Deps {
        transport: &t,
        credentials: &Key,
        tools: &tools,
        events: &on,
        approver: &AcceptAll,
        cancel: &cancel,
    };
    assert_eq!(
        s.ask(Turn::user_text("go on"), &deps).unwrap(),
        Stop::EndTurn
    );
    let body: Value = serde_json::from_slice(&t.sent()[0].body).unwrap();
    let m = body["messages"].as_array().unwrap();
    // The results and the new question make one user message.
    assert_eq!(m.len(), 3);
    assert_eq!(m[2]["content"][0]["type"], "tool_result");
    assert_eq!(m[2]["content"][1]["text"], "go on");
}

#[test]
fn reads_run_together_and_bad_arguments_are_refused() {
    let lite = Endpoint {
        id: "litellm".into(),
        protocol: Protocol::Chat,
        base_url: "http://localhost:4000/v1".into(),
        tool_choice: true,
        strict: false,
        vision: true,
    };
    let bad = String::from_utf8(stream("chat_litellm.sse"))
        .unwrap()
        .replace("\\\"$00:8000\\\"}", "$00:8000}");
    let t = FakeTransport::new(vec![Ok(bad.into_bytes()), Ok(stream("chat_ollama.sse"))]);
    let tools = Fake::default();
    let (log,) = events();
    let on = |e: Event| log.lock().unwrap().push(e);
    let cancel = Cancel::new();
    let mut s = Session::new("c", lite, "claude-opus-5");
    s.retry_unit = std::time::Duration::ZERO;
    let deps = Deps {
        transport: &t,
        credentials: &NoKey,
        tools: &tools,
        events: &on,
        approver: &AcceptAll,
        cancel: &cancel,
    };
    // The second reply asks for a tool again; with no third reply the loop
    // stops on the transport's error.
    let _ = s.ask(Turn::user_text("hi"), &deps);
    let ran = tools.ran.lock().unwrap().clone();
    let names: Vec<&str> = ran.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["rom_info", "disassemble"]);
    let started = log
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, Event::ToolStarted { .. }))
        .count();
    assert_eq!(started, 3);
    let results = &s.turns[2].blocks;
    assert!(matches!(
        &results[1],
        Block::ToolResult { is_error: true, .. }
    ));
}

#[test]
fn rewind_gives_back_the_prompt_and_sends_less() {
    let t = FakeTransport::new(vec![
        Ok(stream("anthropic_text.sse")),
        Ok(stream("anthropic_text.sse")),
        Ok(stream("anthropic_text.sse")),
    ]);
    let on = |_: Event| {};
    let cancel = Cancel::new();
    let tools = Fake::default();
    let deps = Deps {
        transport: &t,
        credentials: &Key,
        tools: &tools,
        events: &on,
        approver: &AcceptAll,
        cancel: &cancel,
    };
    let mut s = session();
    s.ask(Turn::user_text("first"), &deps).unwrap();
    s.ask(Turn::user_text("second"), &deps).unwrap();
    let prompts = s.prompts();
    assert_eq!(
        prompts.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(s.rewind_to(prompts[1].0).as_deref(), Some("second"));
    assert_eq!(s.prompts().len(), 1);
    assert_eq!(s.rewind_to(1), None, "an assistant turn is not a prompt");
    s.ask(Turn::user_text("second, again"), &deps).unwrap();
    let body: Value = serde_json::from_slice(&t.sent()[2].body).unwrap();
    let m = body["messages"].as_array().unwrap();
    assert_eq!(m.len(), 3);
    assert_eq!(m[2]["content"][0]["text"], "second, again");
    // The rewound turns are still in the transcript, unsent.
    assert_eq!(s.turns.len(), 6);
}

#[test]
fn compaction_starts_again_from_a_summary() {
    let summary = String::from_utf8(stream("anthropic_text.sse"))
        .unwrap()
        .replace(
            "RESET at `$00:8000` masks interrupts and enters native mode.",
            "The student asked about RESET at `$00:8000`.",
        );
    let t = FakeTransport::new(vec![
        Ok(stream("anthropic_text.sse")),
        Ok(summary.into_bytes()),
        Ok(stream("anthropic_text.sse")),
    ]);
    let (log,) = events();
    let on = |e: Event| log.lock().unwrap().push(e);
    let cancel = Cancel::new();
    let tools = Fake::default();
    let deps = Deps {
        transport: &t,
        credentials: &Key,
        tools: &tools,
        events: &on,
        approver: &AcceptAll,
        cancel: &cancel,
    };
    let mut s = session();
    s.ask(Turn::user_text("What does RESET do?"), &deps)
        .unwrap();
    let got = s.compact(&deps).unwrap();
    assert_eq!(got, "The student asked about RESET at `$00:8000`.");
    s.ask(Turn::user_text("And then?"), &deps).unwrap();
    let sent = t.sent();
    let asked: Value = serde_json::from_slice(&sent[1].body).unwrap();
    let last = asked["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert!(
        last["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Stop here and write a summary")
    );
    // After it, only the summary and the new question go.
    let after: Value = serde_json::from_slice(&sent[2].body).unwrap();
    let m = after["messages"].as_array().unwrap();
    assert_eq!(m.len(), 1);
    assert!(
        m[0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("The student asked about RESET")
    );
    assert_eq!(m[0]["content"][1]["text"], "And then?");
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, Event::Compacted { .. }))
    );
    assert!(s.cost() > 0.0, "the summary's cost is counted");
}
