//! The tutor against a real model (docs/24, U11): a few questions about a
//! fixture ROM with known answers, graded by the citations and facts each
//! answer must hold, with what they cost. Only with `ROMLENS_TUTOR_LIVE=1`,
//! and never in CI:
//!
//! ```text
//! ROMLENS_TUTOR_LIVE=1 ANTHROPIC_API_KEY=… cargo test -p romlens-ffi --test tutor_live -- --nocapture
//! ROMLENS_TUTOR_LIVE=1 ROMLENS_TUTOR_PROVIDER=openai OPENAI_API_KEY=… cargo test …
//! ROMLENS_TUTOR_LIVE=1 ROMLENS_TUTOR_PROVIDER=http://localhost:11434/v1 ROMLENS_TUTOR_MODEL=qwen3:32b cargo test …
//! ```
//!
//! `ROMLENS_TUTOR_MODEL` and `ROMLENS_TUTOR_EFFORT` pick the model and
//! effort; the default is the model table's for the provider.

use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use romlens_ffi::tutor::session::*;
use romlens_ffi::{Rom, Workbench};

struct Env;
impl CredentialStore for Env {
    fn key(&self, endpoint: String) -> Option<String> {
        use romlens_tutor::agent::Credentials;
        romlens_tutor::agent::EnvCredentials.key(&endpoint)
    }
}

struct Heard(Mutex<mpsc::Sender<TutorEventInfo>>);
impl TutorListener for Heard {
    fn on_event(&self, e: TutorEventInfo) {
        let _ = self.0.lock().unwrap().send(e);
    }
}

struct Question {
    ask: &'static str,
    /// Each must appear in the answer (case ignored).
    must: &'static [&'static str],
    mode: TutorMode,
}

const QUESTIONS: &[Question] = &[
    Question {
        ask: "What does RESET do in its first five instructions, and why does a SNES game start that way?",
        must: &["$00:8000", "XCE", "native"],
        mode: TutorMode::ReadOnly,
    },
    Question {
        ask: "Where does this ROM start a DMA, and what does it copy where?",
        must: &["$00:8035", "VRAM", "$6000"],
        mode: TutorMode::ReadOnly,
    },
    Question {
        ask: "It uses the hardware multiplier. What does it multiply, and where does the product go?",
        must: &["6", "7", "42", "$4202"],
        mode: TutorMode::ReadOnly,
    },
    Question {
        ask: "Show me RESET as C and explain the loop that clears memory.",
        must: &["$7E:0200", "16"],
        mode: TutorMode::ReadOnly,
    },
    Question {
        ask: "Name the routine RESET calls first after what it does, and give RESET a routine note.",
        must: &["$00:80A0"],
        mode: TutorMode::AcceptEdits,
    },
];

fn endpoint() -> TutorEndpointInfo {
    let p = std::env::var("ROMLENS_TUTOR_PROVIDER").unwrap_or_else(|_| "anthropic".into());
    let defaults = tutor_default_endpoints();
    match p.as_str() {
        "anthropic" => defaults[0].clone(),
        "openai" => defaults[1].clone(),
        url => TutorEndpointInfo {
            id: "local".into(),
            protocol: if std::env::var_os("ROMLENS_TUTOR_RESPONSES").is_some() {
                TutorProtocol::Responses
            } else {
                TutorProtocol::Chat
            },
            base_url: url.into(),
            tool_choice: false,
            strict: false,
            vision: false,
        },
    }
}

#[test]
fn the_tutor_answers_what_the_rom_says() {
    if std::env::var_os("ROMLENS_TUTOR_LIVE").is_none() {
        eprintln!("skipped: set ROMLENS_TUTOR_LIVE=1 and a key to talk to a real model");
        return;
    }
    let e = endpoint();
    let model = std::env::var("ROMLENS_TUTOR_MODEL")
        .ok()
        .or_else(|| tutor_default_model(e.protocol))
        .expect("name the model with ROMLENS_TUTOR_MODEL");
    let effort = std::env::var("ROMLENS_TUTOR_EFFORT").ok();
    let rom = Rom::from_bytes(
        romlens_core::fixtures::explain_lorom(),
        "explain.sfc".into(),
    )
    .unwrap();
    let wb = Workbench::new(rom);
    wb.analyze_blocking().unwrap();
    let root = std::env::temp_dir().join(format!("romlens-tutor-live-{}", std::process::id()));
    let (tx, rx) = mpsc::channel();
    let t = TutorSession::new(
        Arc::clone(&wb),
        root.to_string_lossy().into_owned(),
        Arc::new(Env),
        Arc::new(Heard(Mutex::new(tx))),
    );

    let mut passed = 0;
    let mut rows = Vec::new();
    for q in QUESTIONS {
        t.new_conversation(e.clone(), model.clone(), effort.clone(), q.mode, Some(1.0))
            .unwrap();
        let start = Instant::now();
        Arc::clone(&t).send(q.ask.into(), Vec::new(), None).unwrap();
        let mut text = String::new();
        let mut tools = 0;
        let end = loop {
            match rx
                .recv_timeout(Duration::from_secs(600))
                .expect("the model answers within ten minutes")
            {
                TutorEventInfo::TextDelta { text: d } => text.push_str(&d),
                TutorEventInfo::Discarded => text.clear(),
                TutorEventInfo::ToolFinished { .. } => tools += 1,
                e @ (TutorEventInfo::Ended { .. } | TutorEventInfo::Failed { .. }) => break e,
                _ => {}
            }
        };
        let lower = text.to_lowercase();
        let missing: Vec<&str> = q
            .must
            .iter()
            .copied()
            .filter(|m| !lower.contains(&m.to_lowercase()))
            .collect();
        let ok = missing.is_empty() && matches!(end, TutorEventInfo::Ended { .. });
        passed += ok as usize;
        rows.push(format!(
            "{} {:>5.1}s {:>2} tools ${:.4}  {}{}",
            if ok { "✓" } else { "✗" },
            start.elapsed().as_secs_f64(),
            tools,
            t.cost(),
            q.ask,
            if ok {
                String::new()
            } else {
                format!("\n      missing {missing:?}; {end:?}")
            }
        ));
    }
    let edited = wb.label_at(0x80A0).map(|l| l.name).unwrap_or_default();
    let note = wb.routine_note(0x8000);
    println!("\n{} on {} ({}):", model, e.id, e.base_url);
    for r in &rows {
        println!("  {r}");
    }
    println!("  the routine at $00:80A0 is now {edited:?}; RESET's note: {note:?}");
    println!(
        "  {passed} of {} answered with what the ROM says",
        QUESTIONS.len()
    );
    let _ = std::fs::remove_dir_all(root);
    assert!(
        passed * 2 >= QUESTIONS.len(),
        "fewer than half the answers held the ROM's facts"
    );
}
