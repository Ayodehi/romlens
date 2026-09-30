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
    /// Asks the student about an edit, in "ask before edits".
    pub approver: &'a dyn Approver,
    pub cancel: &'a Cancel,
}

/// An edit the tutor wants to make, as its card shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    /// The tool call's id.
    pub id: String,
    pub tool: String,
    /// What it does, in a line: "Name $80:8000 `Reset`".
    pub summary: String,
    /// Why, in the tutor's words.
    pub reason: String,
    /// What is there now, and what there would be, where it can be shown.
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Accept,
    Reject { why: Option<String> },
}

/// The student's answer to an edit card. The app's waits for a click (or
/// Esc, which rejects); the CLI's asks on the terminal.
pub trait Approver: Sync {
    fn decide(&self, p: &Proposal) -> Decision;
}

/// Accepts everything: for accept-edits runs and tests.
pub struct AcceptAll;

impl Approver for AcceptAll {
    fn decide(&self, _: &Proposal) -> Decision {
        Decision::Accept
    }
}

pub trait Tools: Sync {
    /// Fixed for a conversation (docs/24, "Everything sent is
    /// append-only").
    fn specs(&self) -> Vec<ToolSpec>;
    fn kind(&self, name: &str) -> ToolKind;
    /// Runs the call `id` (the model's id for it).
    fn run(&self, id: &str, name: &str, input: &Value, cx: &ToolContext) -> ToolOutput;
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
    fn run(&self, _: &str, name: &str, _: &Value, _: &ToolContext) -> ToolOutput {
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
    /// An edit waits for the student's answer.
    EditProposed(Proposal),
    /// It was made (or not).
    EditDecided {
        id: String,
        applied: bool,
    },
    /// Older turns were summarised to make room.
    Compacted {
        summary: String,
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
    /// The conversation mid-turn, after each reply and each round of
    /// results, for a shell to save: one waiting on a card is on disk.
    fn checkpoint(&self, _s: &Session) {}
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
    pub approver: &'a dyn Approver,
    pub cancel: &'a Cancel,
}

/// A conversation in progress.
#[derive(Clone)]
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
    /// A digest of the tools the conversation was last sent (kept in its
    /// meta), to notice a Romlens that declares others.
    pub tools_seen: Option<String>,
    /// Edits the tutor proposed before writing anything in its reply,
    /// held so their cards come after the answer: the call, and the turn
    /// that made it.
    held: Vec<(String, String, Value, usize)>,
    /// What the student decided about held edits, told to the model with
    /// the next question.
    decided: Vec<String>,
    /// The model's short name for the conversation, once it has made one
    /// (`name`).
    pub title: Option<String>,
    /// Explain mode: questions are answered with lessons (docs/25).
    pub explain: bool,
    /// What requests outside the turns cost: naming the conversation.
    pub side_cost: f64,
}

pub const MAX_ROUNDS: u32 = 60;

/// Past this share of the context, the next question compacts first.
pub const COMPACT_AT: f64 = 0.8;

const COMPACT_PROMPT: &str = "Stop here and write a summary of our conversation so far, for yourself to continue from: what the student is trying to learn or do, what you found (with every address, frame and name, cited), what changed in their project, and what is still open. Do not call tools. Write only the summary.";
const ATTEMPTS: u32 = 4;

/// What naming a conversation asks (`Session::name`).
pub const NAME_PROMPT: &str = "Name this conversation for the student's list of past conversations: two to four words saying what it is about, such as \"RESET's WRAM routine\" or \"Sharing the NMI's tail\". Do not call tools. Reply with the name alone.";

/// What checking a finished lesson asks (`Session::review`, docs/25,
/// "Checking lessons"). The lesson's id and Romlens's own findings follow.
pub const REVIEW_PROMPT: &str = "[Romlens asks you to check a lesson you wrote; the student does not see this.] Check the lesson below as a strict reviewer, one step at a time. Check every fact about this ROM with the tools (listing, read_bytes, decompile, reference), every fact about the hardware against the primer and `reference`, and every number, unit and cycle count. Code a step shows must be the ROM's. Where a step is wrong, imprecise, or claims more than the tools show, call `revise_lesson_step` with the corrected step: the same level, the same teaching and about the same length, the mistake fixed and nothing else changed. Leave correct steps alone and do not restyle them. Make no other changes. End with one line: what you corrected and why, or \"No changes.\"";

/// The most one lesson's check may cost, on top of the conversation.
pub const REVIEW_CAP: f64 = 0.5;

/// What writing a quiz's questions asks (`Session::write_quiz`, docs/28).
/// The quiz's concept, level and the questions it has follow.
pub const QUIZ_PROMPT: &str = "[Romlens asks you to write quiz questions; the student does not see this.] Write the questions asked below with `quiz_question`, one call each, about this game where the level is 3 or more. First find the facts with the tools. Every question except a written one carries a claim Romlens checks against its own data before it is asked; if a claim is refused, the reply says what is so: fix the question or drop it. Ask about understanding, not trivia, and make the explanation teach. Don't repeat a question the quiz has. End with one line: what you asked.";

/// What marking a written answer asks (`Session::grade`, docs/28).
pub const GRADE_PROMPT: &str = "[Romlens asks you to mark a student's written quiz answer; the student sees only your line.] Do not call tools. Mark the answer against what a good answer says: all of it right is 1, part of it 0.5, wrong or empty 0. Reply with one line: `CREDIT 1`, `CREDIT 0.5` or `CREDIT 0`, a colon, and one sentence to the student saying what was right or missing.";

/// The most writing one quiz's questions may cost.
pub const QUIZ_CAP: f64 = 0.10;
/// The most marking one written answer may cost.
pub const GRADE_CAP: f64 = 0.02;

/// A mark's line, `CREDIT 0.5: …`: the credit and what it says to the
/// student. A line without one is 0, and says so.
pub fn read_credit(line: &str) -> (f32, String) {
    let upper = line.to_uppercase();
    let Some(at) = upper.find("CREDIT") else {
        return (0.0, "The answer couldn't be marked.".into());
    };
    let rest = line[at + "CREDIT".len()..].trim_start();
    let num: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let credit = match num.parse::<f32>() {
        Ok(c) if c >= 0.75 => 1.0,
        Ok(c) if c >= 0.25 => 0.5,
        _ => 0.0,
    };
    let said = rest[num.len()..]
        .trim_start_matches([':', ' ', '-', '—'])
        .trim()
        .to_owned();
    (credit, said)
}

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
            tools_seen: None,
            held: Vec::new(),
            decided: Vec::new(),
            title: None,
            side_cost: 0.0,
            explain: false,
        }
    }

    pub fn caps(&self) -> Capabilities {
        capabilities(&self.model, self.endpoint.vision)
    }

    pub fn cost(&self) -> f64 {
        self.turns.iter().map(|t| t.cost).sum::<f64>() + self.side_cost
    }

    /// Whether the conversation has an answer to name it by.
    pub fn answered(&self) -> bool {
        self.turns.iter().any(|t| {
            t.role == Role::Assistant && t.blocks.iter().any(|b| matches!(b, Block::Text { .. }))
        })
    }

    /// A short name for the conversation, from the model, and what asking
    /// cost. Asked after the conversation so far, the prefix the answers
    /// used, so it is read from the cache; nothing is shown and the
    /// conversation is left as it was.
    pub fn name(&self, d: &Deps) -> Result<(String, f64), AskError> {
        let key = d.credentials.key(&self.endpoint.id);
        if key.is_none() && !self.keyless {
            return Err(AskError::NoKey(self.endpoint.id.clone()));
        }
        let quiet = |_: Event| {};
        let d = Deps {
            events: &quiet,
            ..*d
        };
        let mut turns = self.turns.clone();
        close_calls(&mut turns);
        turns.push(Turn::user_text(NAME_PROMPT));
        let reply = self.request_turns(&turns, &d.tools.specs(), key.as_deref(), &d)?;
        let cost = self
            .caps()
            .price
            .map(|p| p.cost(&reply.usage))
            .unwrap_or(0.0);
        let text: String = reply
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let name = tidy_name(&text).ok_or(AskError::Stream(StreamError::Truncated))?;
        Ok((name, cost))
    }

    /// Checks lesson `lesson` (its text, and `found`, what Romlens's own
    /// checks found) on a copy of the conversation, so the prefix is read
    /// from the cache; the copy's turns are dropped and this session is
    /// left as it was. Returns the reviewer's last words and what it cost.
    /// It runs read-only, within `REVIEW_CAP`; the tools decide what a
    /// review may call.
    pub fn review(&self, lesson: &str, found: &str, d: &Deps) -> Result<(String, f64), AskError> {
        let mut text = format!("{REVIEW_PROMPT}\n\n{lesson}");
        if !found.is_empty() {
            text.push_str(&format!("\n\nRomlens found:\n{found}"));
        }
        self.side(&text, REVIEW_CAP, d)
    }

    /// Writes a quiz's questions (docs/28) on a copy of the conversation,
    /// as `review` checks a lesson: `quiz` says what to ask, and the tools
    /// take each question. Returns the last line and the cost.
    pub fn write_quiz(&self, quiz: &str, d: &Deps) -> Result<(String, f64), AskError> {
        self.side(&format!("{QUIZ_PROMPT}\n\n{quiz}"), QUIZ_CAP, d)
    }

    /// Marks a written answer on a copy: the credit (0, 0.5 or 1), the
    /// line to the student, and the cost.
    pub fn grade(
        &self,
        question: &str,
        good: &str,
        answer: &str,
        d: &Deps,
    ) -> Result<(f32, String, f64), AskError> {
        let text = format!(
            "{GRADE_PROMPT}\n\nThe question: {question}\nWhat a good answer says: {good}\nThe student's answer: {answer}"
        );
        let (line, cost) = self.side(&text, GRADE_CAP, d)?;
        let (credit, said) = read_credit(&line);
        Ok((credit, said, cost))
    }

    /// One turn on a copy of the conversation, read-only and within `cap`
    /// of what it has cost: the prefix is read from the cache, the copy's
    /// turns are dropped, and this session is left as it was.
    fn side(&self, text: &str, cap: f64, d: &Deps) -> Result<(String, f64), AskError> {
        let mut r = self.clone();
        let before = r.cost();
        r.mode = Mode::ReadOnly;
        r.cost_cap = Some(r.cost_cap.map_or(before + cap, |c| c.min(before + cap)));
        let start = r.turns.len();
        let result = r.ask(Turn::user_text(text), d);
        let cost = r.cost() - before;
        result?;
        let last = r.turns[start..]
            .iter()
            .rev()
            .filter(|t| t.role == Role::Assistant)
            .map(Turn::text)
            .find(|t| !t.trim().is_empty())
            .unwrap_or_default();
        Ok((last.trim().to_owned(), cost))
    }

    /// Asks `question` and runs the loop until the model stops.
    pub fn ask(&mut self, mut question: Turn, d: &Deps) -> Result<Stop, AskError> {
        d.cancel.reset();
        let key = d.credentials.key(&self.endpoint.id);
        if key.is_none() && !self.keyless {
            return Err(AskError::NoKey(self.endpoint.id.clone()));
        }
        // A turn saved while its calls ran, then cut off (a crash), gets
        // their results before anything follows it.
        self.close_round("not run: the conversation was interrupted");
        if self.context_used() > COMPACT_AT {
            self.compact(d)?;
        }
        if !self.decided.is_empty() {
            let text = format!(
                "[The changes you proposed last time: {}]",
                std::mem::take(&mut self.decided).join(" ")
            );
            question.blocks.insert(0, Block::Text { text });
        }
        self.turns.push(question);
        self.held.clear();
        let tools = d.tools.specs();
        self.check_tools(&tools);
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
            d.events.checkpoint(self);
            match stop {
                Stop::Paused => continue,
                Stop::ToolUse if has_calls => {}
                _ if has_calls => {
                    // Cut off mid-round: the calls are answered, not run.
                    self.close_round("not run: the reply was cut off");
                    self.offer_held(d);
                    d.events.event(Event::Ended { stop: stop.clone() });
                    return Ok(stop);
                }
                _ => {
                    self.offer_held(d);
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
            d.events.checkpoint(self);
            if d.cancel.is_cancelled() {
                return Err(AskError::Cancelled);
            }
        }
        Err(AskError::TooManyRounds(MAX_ROUNDS))
    }

    /// Takes the conversation back to before the user turn at `index`:
    /// that turn and every one after it are no longer sent (they stay in
    /// the file). Returns the words the student asked then, for the
    /// composer. The edits made from that turn on are the shell's to take
    /// back, by `Origin::Tutor { turn >= index }`.
    pub fn rewind_to(&mut self, index: usize) -> Option<String> {
        let t = self.turns.get(index)?;
        if t.role != Role::User || !t.sent {
            return None;
        }
        let asked = t
            .blocks
            .iter()
            .rev()
            .find_map(|b| match b {
                Block::Text { text } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default();
        for t in &mut self.turns[index..] {
            t.sent = false;
        }
        Some(asked)
    }

    /// The user turns a rewind can go back to: their index and words.
    pub fn prompts(&self) -> Vec<(usize, String)> {
        self.turns
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.sent
                    && t.role == Role::User
                    && t.blocks.iter().any(|b| matches!(b, Block::Text { .. }))
            })
            .map(|(i, t)| (i, t.text()))
            .collect()
    }

    /// How full the context was at the last reply, from 0 to 1.
    pub fn context_used(&self) -> f64 {
        let last = self
            .turns
            .iter()
            .rev()
            .find(|t| t.sent && t.role == Role::Assistant)
            .map(|t| t.usage.prompt() + t.usage.output)
            .unwrap_or(0);
        last as f64 / self.caps().context.max(1) as f64
    }

    /// Summarises the conversation so far and starts again from the
    /// summary (docs/24, "Compaction"): the model writes it, the old turns
    /// stop being sent, and the next question follows the summary. Never in
    /// the middle of a tool round.
    pub fn compact(&mut self, d: &Deps) -> Result<String, AskError> {
        d.cancel.reset();
        let key = d.credentials.key(&self.endpoint.id);
        if key.is_none() && !self.keyless {
            return Err(AskError::NoKey(self.endpoint.id.clone()));
        }
        self.close_round("stopped for a summary");
        self.turns.push(Turn::user_text(COMPACT_PROMPT));
        let tools = d.tools.specs();
        let reply = self.request(&tools, key.as_deref(), d);
        // The request's own turn is not part of the conversation.
        self.turns.pop();
        let reply = reply?;
        let summary = reply
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        let cost = self
            .caps()
            .price
            .map(|p| p.cost(&reply.usage))
            .unwrap_or(0.0);
        for t in &mut self.turns {
            t.sent = false;
        }
        let mut turn = Turn::user(vec![
            Block::Note {
                text: "The conversation was summarised to make room.".into(),
            },
            Block::Text {
                text: format!("[A summary of our conversation so far]\n{summary}"),
            },
        ]);
        turn.cost = cost;
        turn.usage = reply.usage;
        self.turns.push(turn);
        d.events.event(Event::Compacted {
            summary: summary.clone(),
        });
        Ok(summary)
    }

    /// A conversation resumed under a Romlens with other tools has a new
    /// prefix, and Anthropic refuses the thinking blocks made under the old
    /// one. They are dropped once (their summaries stay shown); the model
    /// goes on without that reasoning.
    fn check_tools(&mut self, tools: &[ToolSpec]) {
        let digest = tools_digest(tools);
        if self.tools_seen.as_ref().is_some_and(|d| *d != digest) {
            for t in &mut self.turns {
                if let Some(n) = &mut t.native
                    && n.protocol == provider::Protocol::Anthropic
                    && let serde_json::Value::Array(blocks) = &mut n.content
                {
                    blocks.retain(|b| {
                        !matches!(b["type"].as_str(), Some("thinking" | "redacted_thinking"))
                    });
                }
            }
        }
        self.tools_seen = Some(digest);
    }

    /// Shows the held edits' cards, now the answer is written, and keeps
    /// what the student decides for the next question.
    fn offer_held(&mut self, d: &Deps) {
        for (id, name, input, turn) in std::mem::take(&mut self.held) {
            if d.cancel.is_cancelled() {
                self.decided.push(format!(
                    "{name} was not shown: the student stopped the reply."
                ));
                continue;
            }
            let cx = ToolContext {
                mode: self.mode,
                conversation: &self.id,
                turn,
                events: d.events,
                approver: d.approver,
                cancel: d.cancel,
            };
            let out = d.tools.run(&id, &name, &input, &cx);
            let text: Vec<&str> = out
                .parts
                .iter()
                .filter_map(|p| match p {
                    Part::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            self.decided.push(text.join(" "));
        }
    }

    /// Whether the tutor has written to the student since the question:
    /// in "ask before edits", an edit before that waits, so the student
    /// reads the explanation before a card asks them to change anything.
    fn explained_this_reply(&self) -> bool {
        self.turns
            .iter()
            .rev()
            .take_while(|t| {
                t.role == Role::Assistant
                    || t.blocks
                        .iter()
                        .all(|b| matches!(b, Block::ToolResult { .. }))
            })
            .filter(|t| t.role == Role::Assistant)
            .any(|t| {
                t.blocks
                    .iter()
                    .any(|b| matches!(b, Block::Text { text } if !text.trim().is_empty()))
            })
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
        self.request_turns(&self.turns, tools, key, d)
    }

    fn request_turns(
        &self,
        turns: &[Turn],
        tools: &[ToolSpec],
        key: Option<&str>,
        d: &Deps,
    ) -> Result<provider::Reply, AskError> {
        let sent = sendable(turns);
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
            approver: d.approver,
            cancel: d.cancel,
        };
        let explained = self.explained_this_reply();
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
            } else if d.tools.kind(name) == ToolKind::Edit
                && self.mode == Mode::AskBeforeEdits
                && !explained
            {
                ToolOutput::text(HELD)
            } else {
                d.tools.run(id, name, input, &cx)
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
        if !explained && self.mode == Mode::AskBeforeEdits {
            for (id, name, input) in &calls {
                if d.tools.kind(name) == ToolKind::Edit && input.get(INVALID_JSON).is_none() {
                    self.held
                        .push((id.clone(), name.clone(), input.clone(), turn));
                }
            }
        }
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

/// The result of an edit held until the answer is written.
pub const HELD: &str = "Proposed. The student sees it as a card when your reply ends, and you will hear what they decided with their next message. Do not call it again; go on with your reply.";

/// A short digest of a tool list.
/// A reply's last tool calls with no results yet (a card still waiting)
/// are answered, so the turns can be sent as they are.
fn close_calls(turns: &mut Vec<Turn>) {
    let Some(last) = turns.last() else { return };
    if last.role != Role::Assistant {
        return;
    }
    let results: Vec<Block> = last
        .tool_calls()
        .map(|(id, _, _)| Block::ToolResult {
            id: id.to_owned(),
            parts: vec![Part::Text {
                text: "not run".into(),
            }],
            is_error: true,
        })
        .collect();
    if !results.is_empty() {
        turns.push(Turn::user(results));
    }
}

/// The model's name as a title: its first line, without quotes, markup or
/// a closing full stop, and short.
pub fn tidy_name(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let name = line
        .trim_start_matches(['#', '*', '-', ' '])
        .trim_matches(['"', '\'', '*', '`', '“', '”', ' '])
        .trim_end_matches('.')
        .trim();
    if name.is_empty() {
        return None;
    }
    Some(if name.chars().count() > 48 {
        name.chars().take(47).chain(['…']).collect()
    } else {
        name.to_owned()
    })
}

pub fn tools_digest(tools: &[ToolSpec]) -> String {
    let bytes = serde_json::to_vec(tools).expect("tools serialise");
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{h:016x}")
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
