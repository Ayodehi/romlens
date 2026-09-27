//! The loop (docs/24, "The loop"): send, stream, run the tool calls, send
//! every result back in one message, and go on until the model stops.
//!
//! Read-only calls in one round run side by side; any other call runs alone,
//! in order. A cost cap per conversation stops the loop before a request
//! that would start past it. Everything appended keeps the transcript
//! valid for every protocol: a round cut short (Esc, a refusal, the output
//! limit) gets an error result for each call left unanswered.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

use crate::http::{Cancel, HttpError, Transport, backoff, with_key};
use crate::models::{Capabilities, capabilities};
use crate::provider::{self, Delta, Endpoint, INVALID_JSON, Request, Stop, StreamError, ToolSpec};
use crate::sse;
use crate::transcript::{Block, ImageRef, Part, Role, Turn, sendable};

/// Where the keys come from: the Keychain in the app, the environment in
/// the CLI.
pub trait Credentials: Send + Sync {
    fn key(&self, endpoint: &str) -> Option<String>;
}

/// `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, or for an endpoint of the user's,
/// `ROMLENS_KEY_<ID>` (its id upper-cased, `-` as `_`).
pub struct EnvCredentials;

impl Credentials for EnvCredentials {
    fn key(&self, endpoint: &str) -> Option<String> {
        let var = match endpoint {
            "anthropic" => "ANTHROPIC_API_KEY".to_owned(),
            "openai" => "OPENAI_API_KEY".to_owned(),
            other => format!(
                "ROMLENS_KEY_{}",
                other.to_ascii_uppercase().replace(['-', '.', ' '], "_")
            ),
        };
        std::env::var(var).ok().filter(|k| !k.is_empty())
    }
}

/// The permission mode (docs/24, decision 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    ReadOnly,
    AskBeforeEdits,
    AcceptEdits,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::ReadOnly => "read-only",
            Mode::AskBeforeEdits => "ask before edits",
            Mode::AcceptEdits => "accept edits",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// Reads only; may run beside other reads.
    Read,
    /// Changes the project, under the mode.
    Edit,
    /// Calls another service (an image model).
    Visual,
}

/// What a tool handed back.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolOutput {
    pub parts: Vec<Part>,
    pub is_error: bool,
    /// Pictures the tool made, to keep beside the transcript.
    pub pictures: Vec<(ImageRef, Vec<u8>)>,
    /// One line for the tool log.
    pub summary: String,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> ToolOutput {
        let text = text.into();
        ToolOutput {
            summary: first_line(&text),
            parts: vec![Part::Text { text }],
            ..Default::default()
        }
    }

    pub fn error(text: impl Into<String>) -> ToolOutput {
        ToolOutput {
            is_error: true,
            ..ToolOutput::text(text)
        }
    }
}

fn first_line(s: &str) -> String {
    let l = s.lines().next().unwrap_or_default();
    if l.chars().count() > 120 {
        l.chars().take(119).chain(['…']).collect()
    } else {
        l.to_owned()
    }
}

pub struct ToolContext<'a> {
    pub mode: Mode,
    pub conversation: &'a str,
    /// The index of the assistant turn that made the call.
    pub turn: usize,
    pub events: &'a dyn Events,
}

pub trait Tools: Sync {
    /// Fixed for a conversation (docs/24, "Everything sent is
    /// append-only").
    fn specs(&self) -> Vec<ToolSpec>;
    fn kind(&self, name: &str) -> ToolKind;
    fn run(&self, name: &str, input: &Value, cx: &ToolContext) -> ToolOutput;
}

/// No tools: a plain chat.
pub struct NoTools;

impl Tools for NoTools {
    fn specs(&self) -> Vec<ToolSpec> {
        Vec::new()
    }
    fn kind(&self, _: &str) -> ToolKind {
        ToolKind::Read
    }
    fn run(&self, name: &str, _: &Value, _: &ToolContext) -> ToolOutput {
        ToolOutput::error(format!("there is no tool named {name}"))
    }
}

/// What the shell hears.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Delta(Delta),
    /// A request is being sent (the first, or the next after a round).
    Requesting {
        round: u32,
    },
    Retrying {
        attempt: u32,
        wait: Duration,
        why: String,
    },
    ToolStarted {
        id: String,
        name: String,
        input: Value,
    },
    ToolFinished {
        id: String,
        name: String,
        summary: String,
        is_error: bool,
    },
    /// After each reply: that reply's cost and the conversation's.
    Cost {
        turn: f64,
        total: f64,
        priced: bool,
    },
    /// A partial reply was thrown away (a retry after streaming began, or
    /// Esc); the window drops what it showed of it.
    Discarded,
    Ended {
        stop: Stop,
    },
}

pub trait Events: Sync {
    fn event(&self, e: Event);
}

impl<F: Fn(Event) + Sync> Events for F {
    fn event(&self, e: Event) {
        self(e)
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AskError {
    #[error("no key for {0}: add one in Settings")]
    NoKey(String),
    #[error("stopped")]
    Cancelled,
    #[error("the conversation reached its cost cap of ${0:.2}")]
    CapReached(f64),
    #[error("{0}")]
    Http(HttpError),
    #[error("{0}")]
    Stream(StreamError),
    #[error("more than {0} rounds of tools in one answer")]
    TooManyRounds(u32),
}

pub struct Deps<'a> {
    pub transport: &'a dyn Transport,
    pub credentials: &'a dyn Credentials,
    pub tools: &'a dyn Tools,
    pub events: &'a dyn Events,
    pub cancel: &'a Cancel,
}

/// A conversation in progress.
pub struct Session {
    pub id: String,
    pub turns: Vec<Turn>,
    pub pictures: HashMap<String, Vec<u8>>,
    pub endpoint: Endpoint,
    pub model: String,
    pub effort: Option<String>,
    pub mode: Mode,
    /// Fixed once the conversation starts.
    pub system: String,
    pub digest: String,
    pub cost_cap: Option<f64>,
    /// Endpoints that need no key (a local server).
    pub keyless: bool,
    /// The step retries wait in when the server names no time.
    pub retry_unit: Duration,
}

pub const MAX_ROUNDS: u32 = 60;
const ATTEMPTS: u32 = 4;

impl Session {
    pub fn new(id: &str, endpoint: Endpoint, model: &str) -> Session {
        // Only the two providers' own endpoints must have a key.
        let keyless = !matches!(endpoint.id.as_str(), "anthropic" | "openai");
        Session {
            id: id.into(),
            turns: Vec::new(),
            pictures: HashMap::new(),
            endpoint,
            model: model.into(),
            effort: None,
            mode: Mode::AskBeforeEdits,
            system: String::new(),
            digest: String::new(),
            cost_cap: None,
            keyless,
            retry_unit: Duration::from_secs(1),
        }
    }

    pub fn caps(&self) -> Capabilities {
        capabilities(&self.model, self.endpoint.vision)
    }

    pub fn cost(&self) -> f64 {
        self.turns.iter().map(|t| t.cost).sum()
    }

    /// Asks `question` and runs the loop until the model stops.
    pub fn ask(&mut self, question: Turn, d: &Deps) -> Result<Stop, AskError> {
        d.cancel.reset();
        let key = d.credentials.key(&self.endpoint.id);
        if key.is_none() && !self.keyless {
            return Err(AskError::NoKey(self.endpoint.id.clone()));
        }
        self.turns.push(question);
        let tools = d.tools.specs();
        for round in 0..MAX_ROUNDS {
            if let Some(cap) = self.cost_cap
                && self.cost() >= cap
            {
                return Err(AskError::CapReached(cap));
            }
            d.events.event(Event::Requesting { round });
            let reply = match self.request(&tools, key.as_deref(), d) {
                Ok(r) => r,
                Err(e) => {
                    self.close_round("stopped before it ran");
                    return Err(e);
                }
            };
            let stop = reply.stop.clone();
            let mut turn = reply.into_turn();
            turn.cost = self
                .caps()
                .price
                .map(|p| p.cost(&turn.usage))
                .unwrap_or(0.0);
            d.events.event(Event::Cost {
                turn: turn.cost,
                total: self.cost() + turn.cost,
                priced: self.caps().price.is_some(),
            });
            let has_calls = turn.tool_calls().next().is_some();
            self.turns.push(turn);
            match stop {
                Stop::Paused => continue,
                Stop::ToolUse if has_calls => {}
                _ if has_calls => {
                    // Cut off mid-round: the calls are answered, not run.
                    self.close_round("not run: the reply was cut off");
                    d.events.event(Event::Ended { stop: stop.clone() });
                    return Ok(stop);
                }
                _ => {
                    d.events.event(Event::Ended { stop: stop.clone() });
                    return Ok(stop);
                }
            }
            if d.cancel.is_cancelled() {
                self.close_round("stopped by the user");
                return Err(AskError::Cancelled);
            }
            let results = self.run_tools(d);
            self.turns.push(Turn::user(results));
            if d.cancel.is_cancelled() {
                return Err(AskError::Cancelled);
            }
        }
        Err(AskError::TooManyRounds(MAX_ROUNDS))
    }

    /// Answers every call of the last turn left without a result.
    fn close_round(&mut self, why: &str) {
        let Some(last) = self.turns.last() else {
            return;
        };
        if last.role != Role::Assistant {
            return;
        }
        let results: Vec<Block> = last
            .tool_calls()
            .map(|(id, _, _)| Block::ToolResult {
                id: id.into(),
                parts: vec![Part::Text { text: why.into() }],
                is_error: true,
            })
            .collect();
        if !results.is_empty() {
            self.turns.push(Turn::user(results));
        }
    }

    fn request(
        &self,
        tools: &[ToolSpec],
        key: Option<&str>,
        d: &Deps,
    ) -> Result<provider::Reply, AskError> {
        let sent = sendable(&self.turns);
        let caps = self.caps();
        let r = Request {
            endpoint: &self.endpoint,
            model: &self.model,
            caps,
            system: &self.system,
            digest: &self.digest,
            tools,
            turns: &sent,
            effort: self.effort.as_deref(),
            max_tokens: caps.max_output.min(64_000),
            cache_key: &self.id,
            attachments: &self.pictures,
        };
        let http = with_key(provider::build(&r), self.endpoint.protocol, key);
        let mut attempt = 1;
        loop {
            match self.stream(&http, d) {
                Ok(reply) => return Ok(reply),
                Err((e, streamed)) => {
                    if streamed {
                        d.events.event(Event::Discarded);
                    }
                    let retry = match &e {
                        AskError::Http(h) => h.retryable(),
                        AskError::Stream(StreamError::Provider { kind, .. }) => matches!(
                            kind.as_str(),
                            "overloaded_error" | "api_error" | "rate_limit_error" | "server_error"
                        ),
                        AskError::Stream(StreamError::Truncated) => true,
                        _ => false,
                    };
                    if !retry || attempt >= ATTEMPTS {
                        return Err(e);
                    }
                    let wait = match &e {
                        AskError::Http(h) => backoff(attempt, Some(h), self.retry_unit),
                        _ => backoff(attempt, None, self.retry_unit),
                    };
                    d.events.event(Event::Retrying {
                        attempt,
                        wait,
                        why: e.to_string(),
                    });
                    if sleep(wait, d.cancel) {
                        return Err(AskError::Cancelled);
                    }
                    attempt += 1;
                }
            }
        }
    }

    /// One try. The flag says whether any of the reply was shown.
    fn stream(
        &self,
        http: &provider::HttpRequest,
        d: &Deps,
    ) -> Result<provider::Reply, (AskError, bool)> {
        let mut parser = provider::parser(&self.endpoint, &self.model);
        let mut reader = sse::Reader::new();
        let mut failed: Option<StreamError> = None;
        let mut streamed = false;
        let take = |events: Vec<sse::Event>,
                    parser: &mut Box<dyn provider::Parser>,
                    failed: &mut Option<StreamError>,
                    streamed: &mut bool| {
            for e in events {
                if failed.is_some() {
                    return;
                }
                match parser.event(&e) {
                    Ok(deltas) => {
                        for dl in deltas {
                            *streamed = true;
                            d.events.event(Event::Delta(dl));
                        }
                    }
                    Err(err) => *failed = Some(err),
                }
            }
        };
        let sent = d.transport.post(http, d.cancel, &mut |bytes| {
            let events = reader.feed(bytes);
            take(events, &mut parser, &mut failed, &mut streamed);
        });
        match sent {
            Err(HttpError::Cancelled) => return Err((AskError::Cancelled, streamed)),
            Err(e) => return Err((AskError::Http(e), streamed)),
            Ok(()) => {}
        }
        take(reader.finish(), &mut parser, &mut failed, &mut streamed);
        if let Some(e) = failed {
            return Err((AskError::Stream(e), streamed));
        }
        parser.finish().map_err(|e| (AskError::Stream(e), streamed))
    }

    fn run_tools(&mut self, d: &Deps) -> Vec<Block> {
        let turn = self.turns.len() - 1;
        let calls: Vec<(String, String, Value)> = self.turns[turn]
            .tool_calls()
            .map(|(i, n, v)| (i.to_owned(), n.to_owned(), v.clone()))
            .collect();
        let cx = ToolContext {
            mode: self.mode,
            conversation: &self.id,
            turn,
            events: d.events,
        };
        let run = |(id, name, input): &(String, String, Value)| -> ToolOutput {
            d.events.event(Event::ToolStarted {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            });
            let out = if let Some(raw) = input.get(INVALID_JSON) {
                ToolOutput::error(format!(
                    "{{\"{INVALID_JSON}\": {raw}}}: the arguments were not a JSON object; call again"
                ))
            } else if d.cancel.is_cancelled() {
                ToolOutput::error("stopped by the user")
            } else {
                d.tools.run(name, input, &cx)
            };
            d.events.event(Event::ToolFinished {
                id: id.clone(),
                name: name.clone(),
                summary: out.summary.clone(),
                is_error: out.is_error,
            });
            out
        };
        let all_read = calls
            .iter()
            .all(|(_, n, _)| d.tools.kind(n) == ToolKind::Read);
        let outs: Vec<ToolOutput> = if all_read && calls.len() > 1 {
            std::thread::scope(|s| {
                let hs: Vec<_> = calls.iter().map(|c| s.spawn(|| run(c))).collect();
                hs.into_iter()
                    .map(|h| {
                        h.join()
                            .unwrap_or_else(|_| ToolOutput::error("the tool failed"))
                    })
                    .collect()
            })
        } else {
            calls.iter().map(run).collect()
        };
        let mut blocks = Vec::new();
        for ((id, _, _), out) in calls.into_iter().zip(outs) {
            for (image, bytes) in out.pictures {
                self.pictures.insert(image.id.clone(), bytes);
            }
            blocks.push(Block::ToolResult {
                id,
                parts: out.parts,
                is_error: out.is_error,
            });
        }
        blocks
    }
}

/// Sleeps in short steps; true when cancelled meanwhile.
fn sleep(total: Duration, cancel: &Cancel) -> bool {
    let step = Duration::from_millis(50);
    let mut left = total;
    while !left.is_zero() {
        if cancel.is_cancelled() {
            return true;
        }
        let s = left.min(step);
        std::thread::sleep(s);
        left -= s;
    }
    cancel.is_cancelled()
}
