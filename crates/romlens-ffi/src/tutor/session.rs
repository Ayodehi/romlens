//! The tutor for a shell (docs/24, U6): one `TutorSession` per open
//! project. It holds the conversation, runs each turn on its own thread
//! and tells the shell what happens through a `TutorListener`; the keys come
//! from the shell's `CredentialStore` (the Keychain on macOS) one request at
//! a time. An edit card blocks the turn's thread until the shell calls
//! `answer`, or until Esc.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use romlens_tutor::agent::{self, Approver, Credentials, Decision, Deps, Mode, Proposal, Session};
use romlens_tutor::http::{Cancel, UreqTransport, list_models};
use romlens_tutor::models::{self, MODELS, Thinking};
use romlens_tutor::prompt;
use romlens_tutor::provider::{Delta, Endpoint, Protocol};
use romlens_tutor::store::{Meta, Store, new_id, now, title_for};
use romlens_tutor::transcript::{Block, ImageRef, Part, Role, Turn};

use super::digest::digest;
use super::tools::RomTools;
use crate::RomlensError;
use crate::graphics::RecordingSession;
use crate::workbench::Workbench;

#[uniffi::export(with_foreign)]
pub trait CredentialStore: Send + Sync {
    /// The key for an endpoint (`anthropic`, `openai`, or one the user
    /// added), or nothing.
    fn key(&self, endpoint: String) -> Option<String>;
}

#[uniffi::export(with_foreign)]
pub trait TutorListener: Send + Sync {
    /// From the turn's thread.
    fn on_event(&self, event: TutorEventInfo);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TutorProtocol {
    Anthropic,
    Responses,
    Chat,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TutorEndpointInfo {
    pub id: String,
    pub protocol: TutorProtocol,
    pub base_url: String,
    pub tool_choice: bool,
    pub strict: bool,
    pub vision: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TutorMode {
    ReadOnly,
    AskBeforeEdits,
    AcceptEdits,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TutorModelInfo {
    pub id: String,
    pub name: String,
    pub protocol: TutorProtocol,
    pub context: u32,
    pub max_output: u32,
    pub vision: bool,
    pub thinks: bool,
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
    /// Dollars per million tokens.
    pub price_input: f64,
    pub price_output: f64,
    pub price_cache_read: f64,
    pub price_cache_write: f64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ConversationSummaryInfo {
    pub id: String,
    pub title: String,
    pub created: u64,
    pub updated: u64,
    pub endpoint: String,
    pub model: String,
    pub cost: f64,
    pub turns: u32,
}

/// A lesson for the window (docs/25).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LessonInfo {
    pub id: String,
    pub title: String,
    /// The ROM it used (levels 3 to 5), and whether that is the one open.
    pub rom: Option<String>,
    pub this_rom: bool,
    pub created: u64,
    pub from_level: u8,
    pub to_level: u8,
    /// "The idea", or "The idea to the hardware".
    pub level_name: String,
    pub concepts: Vec<LessonConceptInfo>,
    pub builds_on: Vec<String>,
    pub steps: Vec<LessonStepInfo>,
    pub next: Vec<LessonOfferInfo>,
    /// Ended with `end_lesson`; `false` while it is being written.
    pub finished: bool,
    /// The background check, once it has run, and whether it is running.
    pub checked: Option<LessonCheckedInfo>,
    pub checking: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LessonCheckedInfo {
    /// Steps it rewrote.
    pub changed: u32,
    pub cost: f64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LessonConceptInfo {
    pub id: String,
    pub name: String,
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LessonStepInfo {
    pub title: String,
    pub predict: Option<String>,
    pub body: String,
    /// Where the main window goes: `romlens://a/<addr>`, `romlens://c/<addr>`
    /// (the routine's C), `romlens://f/<n>?view=<view>`, or
    /// `romlens://r/<register>`.
    pub focus: Option<String>,
    /// The focus in words: `$00:8000`, `frame 12, oam`.
    pub focus_text: Option<String>,
    pub picture: Option<String>,
    /// Romlens can check a guess at the predict question (docs/28).
    pub checks_guess: bool,
    /// The student's first guess, and whether it was right when it could
    /// be checked.
    pub guessed: Option<String>,
    pub guess_right: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LessonOfferInfo {
    pub title: String,
    pub concept: String,
    pub level: u8,
}

/// What the student knows, concept by concept, for the map.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LearnerInfo {
    pub concepts: Vec<ConceptInfo>,
    /// The groups in order, and the ladder's level names.
    pub groups: Vec<String>,
    pub levels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ConceptInfo {
    pub id: String,
    pub name: String,
    pub group: String,
    pub needs: Vec<String>,
    pub line: String,
    /// 0 when not reached.
    pub level: u8,
    pub marked: bool,
    pub lesson: Option<String>,
    /// The highest level a quiz proved (docs/28), 0 for none.
    pub proven: u8,
    /// When that proof comes up for review.
    pub due: Option<u64>,
}

pub(crate) fn focus_link(f: &romlens_tutor::lesson::Focus) -> String {
    use romlens_tutor::lesson::Focus;
    match f {
        Focus::Address { start, .. } => format!("romlens://a/{start:06X}"),
        Focus::Routine { at, .. } => format!("romlens://c/{at:06X}"),
        Focus::Frame { n, view } => format!("romlens://f/{n}?view={view}"),
        Focus::Register { address } => format!("romlens://r/{address:06X}"),
    }
}

fn lesson_info(l: &romlens_tutor::lesson::Lesson, rom: &str) -> LessonInfo {
    use romlens_tutor::lesson::{concept, level_name};
    LessonInfo {
        id: l.id.clone(),
        title: l.title.clone(),
        rom: l.rom.clone(),
        this_rom: l.rom.as_deref().is_none_or(|r| r == rom),
        created: l.created,
        from_level: l.levels.0,
        to_level: l.levels.1,
        level_name: if l.levels.0 == l.levels.1 {
            level_name(l.levels.0).to_owned()
        } else {
            format!(
                "{} to {}",
                level_name(l.levels.0),
                level_name(l.levels.1).to_lowercase()
            )
        },
        concepts: l
            .concepts
            .iter()
            .map(|(id, level)| LessonConceptInfo {
                id: id.clone(),
                name: concept(id).map_or(id.clone(), |c| c.name.to_owned()),
                level: *level,
            })
            .collect(),
        builds_on: l.builds_on.clone(),
        steps: l
            .steps
            .iter()
            .map(|s| LessonStepInfo {
                title: s.title.clone(),
                predict: s.predict.clone(),
                body: s.body.clone(),
                focus: s.focus.as_ref().map(focus_link),
                focus_text: s.focus.as_ref().map(|f| f.text()),
                picture: s.picture.clone(),
                checks_guess: s.predict_answer.is_some(),
                guessed: None,
                guess_right: None,
            })
            .collect(),
        next: l
            .next
            .iter()
            .map(|o| LessonOfferInfo {
                title: o.title.clone(),
                concept: o.concept.clone(),
                level: o.level,
            })
            .collect(),
        finished: l.finished,
        checked: l.checked.as_ref().map(|c| LessonCheckedInfo {
            changed: c.changed,
            cost: c.cost,
        }),
        checking: false,
    }
}

/// The lesson a `begin_lesson` result names: "Lesson l… begun (…)".
#[uniffi::export]
pub fn lesson_id_from_result(text: String) -> Option<String> {
    let rest = text.trim().strip_prefix("Lesson ")?;
    let id = rest.split_whitespace().next()?;
    (id.starts_with('l') && id.contains('-')).then(|| id.to_owned())
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum TurnBlockInfo {
    Text {
        text: String,
    },
    Image {
        id: String,
        media_type: String,
    },
    ToolCall {
        id: String,
        name: String,
        input: String,
    },
    ToolResult {
        id: String,
        text: String,
        images: Vec<String>,
        is_error: bool,
    },
    Reasoning {
        summary: String,
    },
    Note {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TurnInfo {
    pub index: u32,
    pub user: bool,
    pub blocks: Vec<TurnBlockInfo>,
    pub model: Option<String>,
    pub cost: f64,
    pub sent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AttachmentInfo {
    pub bytes: Vec<u8>,
    /// `image/png`, `image/jpeg`, `image/gif` or `image/webp`.
    pub media_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProposalInfo {
    pub id: String,
    pub tool: String,
    pub summary: String,
    pub reason: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum TutorEventInfo {
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolCallStarted {
        id: String,
        name: String,
    },
    ToolArguments {
        id: String,
        text: String,
    },
    Requesting {
        round: u32,
    },
    Retrying {
        attempt: u32,
        wait_ms: u64,
        why: String,
    },
    ToolStarted {
        id: String,
        name: String,
        input: String,
    },
    ToolFinished {
        id: String,
        name: String,
        summary: String,
        is_error: bool,
    },
    EditProposed {
        proposal: ProposalInfo,
    },
    EditDecided {
        id: String,
        applied: bool,
    },
    Cost {
        turn: f64,
        total: f64,
        priced: bool,
    },
    /// What was shown of a reply is thrown away (a retry, or Esc).
    Discarded,
    Compacted {
        summary: String,
    },
    /// The turn is over: why the model stopped.
    Ended {
        stop: String,
    },
    /// The turn failed; the transcript is still whole.
    Failed {
        message: String,
    },
    /// The model named the conversation, after its first answer.
    Named {
        title: String,
        total: f64,
    },
    /// A finished lesson was checked in the background (docs/25): how many
    /// steps it rewrote, what the check cost, and the conversation's cost.
    LessonChecked {
        lesson: String,
        changed: u32,
        cost: f64,
        total: f64,
    },
    /// A quiz changed off the window's own calls (docs/28): the tutor wrote
    /// a question for it, or the model marked an answer.
    QuizChanged {
        quiz: String,
    },
    /// Points gained, and what else is new: levels proven ("Sprites, level
    /// 2"), achievements and milestones earned, a new rank.
    Progress {
        gained: u32,
        proven: Vec<String>,
        unlocked: Vec<String>,
        rank: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RewindWhat {
    Conversation,
    Edits,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RewindResultInfo {
    /// The question asked then, for the composer.
    pub prompt: Option<String>,
    pub edits: Option<crate::cnotes::RewoundInfo>,
}

impl From<TutorEndpointInfo> for Endpoint {
    fn from(e: TutorEndpointInfo) -> Self {
        Endpoint {
            id: e.id,
            protocol: match e.protocol {
                TutorProtocol::Anthropic => Protocol::Anthropic,
                TutorProtocol::Responses => Protocol::Responses,
                TutorProtocol::Chat => Protocol::Chat,
            },
            base_url: e.base_url,
            tool_choice: e.tool_choice,
            strict: e.strict,
            vision: e.vision,
        }
    }
}

fn protocol_info(p: Protocol) -> TutorProtocol {
    match p {
        Protocol::Anthropic => TutorProtocol::Anthropic,
        Protocol::Responses => TutorProtocol::Responses,
        Protocol::Chat => TutorProtocol::Chat,
    }
}

impl From<&Endpoint> for TutorEndpointInfo {
    fn from(e: &Endpoint) -> Self {
        TutorEndpointInfo {
            id: e.id.clone(),
            protocol: protocol_info(e.protocol),
            base_url: e.base_url.clone(),
            tool_choice: e.tool_choice,
            strict: e.strict,
            vision: e.vision,
        }
    }
}

fn mode(m: TutorMode) -> Mode {
    match m {
        TutorMode::ReadOnly => Mode::ReadOnly,
        TutorMode::AskBeforeEdits => Mode::AskBeforeEdits,
        TutorMode::AcceptEdits => Mode::AcceptEdits,
    }
}

fn mode_info(m: Mode) -> TutorMode {
    match m {
        Mode::ReadOnly => TutorMode::ReadOnly,
        Mode::AskBeforeEdits => TutorMode::AskBeforeEdits,
        Mode::AcceptEdits => TutorMode::AcceptEdits,
    }
}

/// The endpoints Romlens knows without being told: Anthropic's and
/// OpenAI's.
#[uniffi::export]
pub fn tutor_default_endpoints() -> Vec<TutorEndpointInfo> {
    vec![
        (&Endpoint::anthropic()).into(),
        (&Endpoint::openai()).into(),
    ]
}

/// The model table (docs/24, decision 9).
#[uniffi::export]
pub fn tutor_models() -> Vec<TutorModelInfo> {
    MODELS
        .iter()
        .map(|m| TutorModelInfo {
            id: m.id.into(),
            name: m.name.into(),
            protocol: protocol_info(m.protocol),
            context: m.context,
            max_output: m.max_output,
            vision: m.vision,
            thinks: m.thinking != Thinking::None,
            efforts: m.efforts.iter().map(|e| (*e).into()).collect(),
            default_effort: m.default_effort.map(Into::into),
            price_input: m.price.input,
            price_output: m.price.output,
            price_cache_read: m.price.cache_read,
            price_cache_write: m.price.cache_write,
        })
        .collect()
}

/// The models an endpoint serves, with the key given: Settings' Test
/// button. A network call; not on the main thread.
#[uniffi::export]
pub fn tutor_list_models(
    endpoint: TutorEndpointInfo,
    key: Option<String>,
) -> Result<Vec<String>, RomlensError> {
    let e: Endpoint = endpoint.into();
    list_models(
        &UreqTransport::new(),
        &e,
        key.as_deref().filter(|k| !k.is_empty()),
    )
    .map_err(err)
}

/// The model a new conversation starts on for a protocol, where there is
/// one.
#[uniffi::export]
pub fn tutor_default_model(protocol: TutorProtocol) -> Option<String> {
    let p = match protocol {
        TutorProtocol::Anthropic => Protocol::Anthropic,
        TutorProtocol::Responses => Protocol::Responses,
        TutorProtocol::Chat => Protocol::Chat,
    };
    models::default_model(p).map(Into::into)
}

fn turn_info(i: usize, t: &Turn) -> TurnInfo {
    let blocks = t
        .blocks
        .iter()
        .map(|b| match b {
            Block::Text { text } => TurnBlockInfo::Text { text: text.clone() },
            Block::Image { image } => TurnBlockInfo::Image {
                id: image.id.clone(),
                media_type: image.media_type.clone(),
            },
            Block::ToolCall { id, name, input } => TurnBlockInfo::ToolCall {
                id: id.clone(),
                name: name.clone(),
                input: input.to_string(),
            },
            Block::ToolResult {
                id,
                parts,
                is_error,
            } => TurnBlockInfo::ToolResult {
                id: id.clone(),
                text: parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                images: parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::Image { image } => Some(image.id.clone()),
                        _ => None,
                    })
                    .collect(),
                is_error: *is_error,
            },
            Block::Reasoning { summary } => TurnBlockInfo::Reasoning {
                summary: summary.clone(),
            },
            Block::Note { text } => TurnBlockInfo::Note { text: text.clone() },
        })
        .collect();
    TurnInfo {
        index: i as u32,
        user: t.role == Role::User,
        blocks,
        model: t.native.as_ref().map(|n| n.model.clone()),
        cost: t.cost,
        sent: t.sent,
    }
}

fn event_info(e: agent::Event) -> TutorEventInfo {
    use agent::Event as E;
    match e {
        E::Delta(Delta::Text(text)) => TutorEventInfo::TextDelta { text },
        E::Delta(Delta::Reasoning(text)) => TutorEventInfo::ReasoningDelta { text },
        E::Delta(Delta::ToolCallStarted { id, name }) => {
            TutorEventInfo::ToolCallStarted { id, name }
        }
        E::Delta(Delta::ToolArguments { id, text }) => TutorEventInfo::ToolArguments { id, text },
        E::Requesting { round } => TutorEventInfo::Requesting { round },
        E::Retrying { attempt, wait, why } => TutorEventInfo::Retrying {
            attempt,
            wait_ms: wait.as_millis() as u64,
            why,
        },
        E::ToolStarted { id, name, input } => TutorEventInfo::ToolStarted {
            id,
            name,
            input: input.to_string(),
        },
        E::ToolFinished {
            id,
            name,
            summary,
            is_error,
        } => TutorEventInfo::ToolFinished {
            id,
            name,
            summary,
            is_error,
        },
        E::EditProposed(p) => TutorEventInfo::EditProposed {
            proposal: ProposalInfo {
                id: p.id,
                tool: p.tool,
                summary: p.summary,
                reason: p.reason,
                before: p.before,
                after: p.after,
            },
        },
        E::EditDecided { id, applied } => TutorEventInfo::EditDecided { id, applied },
        E::Cost {
            turn,
            total,
            priced,
        } => TutorEventInfo::Cost {
            turn,
            total,
            priced,
        },
        E::Discarded => TutorEventInfo::Discarded,
        E::Compacted { summary } => TutorEventInfo::Compacted { summary },
        E::Ended { stop } => TutorEventInfo::Ended {
            stop: format!("{stop:?}"),
        },
    }
}

struct Keys(Arc<dyn CredentialStore>);

impl Credentials for Keys {
    fn key(&self, endpoint: &str) -> Option<String> {
        self.0.key(endpoint.to_owned()).filter(|k| !k.is_empty())
    }
}

/// The student's answers, by card. An answer can arrive before its card
/// is waited on: the card is shown first, then waited for.
#[derive(Default)]
struct Pending {
    answers: HashMap<String, Decision>,
}

struct Cards {
    pending: Mutex<Pending>,
    changed: Condvar,
    cancel: Cancel,
}

impl Approver for Cards {
    fn decide(&self, p: &Proposal) -> Decision {
        let mut g = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(a) = g.answers.remove(&p.id) {
                return a;
            }
            if self.cancel.is_cancelled() {
                return Decision::Reject {
                    why: Some("the student stopped the turn".into()),
                };
            }
            g = self
                .changed
                .wait_timeout(g, Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

struct State {
    /// `None` while a turn runs.
    session: Option<Session>,
    /// The transcript as it was when the turn began, for the shell while
    /// one runs.
    last: Vec<Turn>,
    title: String,
    created: u64,
    /// The mode changed since the model was last told.
    mode_changed: bool,
    /// Explain mode changed since the model was last told.
    explain_changed: bool,
    /// The lesson step the student is at, for the next question.
    at_step: Option<String>,
    busy: bool,
    /// A name that came while a turn had the session: the conversation,
    /// the name, and what it cost.
    named: Option<(String, String, f64)>,
    /// Checks' costs that came while a turn had the session.
    charged: Vec<(String, f64)>,
    /// Lessons being checked now, and whether finished lessons are checked.
    checking: std::collections::HashSet<String>,
    check_lessons: bool,
}

#[derive(uniffi::Object)]
pub struct TutorSession {
    wb: Arc<Workbench>,
    tools: Arc<RomTools>,
    store: Store,
    keys: Arc<dyn CredentialStore>,
    listener: Arc<dyn TutorListener>,
    transport: UreqTransport,
    cards: Arc<Cards>,
    state: Mutex<State>,
    quizzes: super::quiz::Quizzes,
    /// Progress as last announced, to tell the window what is new.
    announced: Mutex<Option<romlens_tutor::progress::Progress>>,
}

fn err(msg: impl ToString) -> RomlensError {
    RomlensError::Tutor {
        msg: msg.to_string(),
    }
}

/// Passes the loop's events to the shell, holding back the end until the
/// session is saved, and saves at each checkpoint.
struct Relay<'a> {
    me: &'a Arc<TutorSession>,
    ended: &'a Mutex<Option<TutorEventInfo>>,
}

impl agent::Events for Relay<'_> {
    fn event(&self, e: agent::Event) {
        match event_info(e) {
            end @ TutorEventInfo::Ended { .. } => {
                *self.ended.lock().unwrap_or_else(|e| e.into_inner()) = Some(end)
            }
            other => self.me.listener.on_event(other),
        }
    }

    fn checkpoint(&self, s: &Session) {
        self.me.tools.lessons.settle(&s.pictures);
        let mut st = self.me.lock();
        self.me.save(s, &mut st);
    }
}

impl TutorSession {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, s: &Session, st: &mut State) {
        if let Some(t) = &s.title {
            st.title = t.clone();
        } else if st.title.is_empty() || st.title == "A conversation" {
            st.title = title_for(&s.turns);
        }
        let _ = self.store.save(s, &st.title, st.created);
    }

    /// What the student has learned, for the model.
    fn learner_summary(&self) -> String {
        self.tools
            .lessons
            .store()
            .map(|s| s.learner().summary())
            .unwrap_or_default()
    }

    fn deps<'a>(&'a self, keys: &'a Keys, on: &'a dyn agent::Events) -> Deps<'a> {
        Deps {
            transport: &self.transport,
            credentials: keys,
            tools: self.tools.as_ref(),
            events: on,
            approver: self.cards.as_ref(),
            cancel: &self.cards.cancel,
        }
    }

    /// Runs `job` on the session on its own thread, then saves.
    fn run(
        self: &Arc<Self>,
        job: impl FnOnce(&mut Session, &Deps) -> Result<(), agent::AskError> + Send + 'static,
    ) -> Result<(), RomlensError> {
        let session = {
            let mut st = self.lock();
            if st.busy {
                return Err(err("the tutor is still answering"));
            }
            let s = st
                .session
                .take()
                .ok_or_else(|| err("start a conversation first"))?;
            st.last = s.turns.clone();
            st.busy = true;
            s
        };
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            let mut s = session;
            let keys = Keys(Arc::clone(&me.keys));
            // The end is told once the session is back and saved, so a
            // shell that reads the transcript on it sees the finished one.
            let ended: Mutex<Option<TutorEventInfo>> = Mutex::new(None);
            let on = Relay {
                me: &me,
                ended: &ended,
            };
            let r = job(&mut s, &me.deps(&keys, &on));
            me.tools.lessons.settle(&s.pictures);
            // The lessons this turn finished are checked after it, from a
            // copy, once the student has the session back (docs/25).
            let reviews = me.tools.lessons.take_reviews();
            let check = r.is_ok() && !reviews.is_empty() && me.lock().check_lessons;
            let reviewer = check.then(|| s.clone());
            // Named once, after the first answer, from a copy: the student
            // need not wait for it.
            let namer = (r.is_ok() && s.title.is_none() && s.answered()).then(|| s.clone());
            {
                let mut st = me.lock();
                if let Some((id, title, cost)) = st.named.take()
                    && id == s.id
                {
                    s.title = Some(title);
                    s.side_cost += cost;
                }
                for (id, cost) in std::mem::take(&mut st.charged) {
                    if id == s.id {
                        s.side_cost += cost;
                    }
                }
                if reviewer.is_some() {
                    st.checking.extend(reviews.iter().cloned());
                }
                me.save(&s, &mut st);
                st.last = s.turns.clone();
                st.session = Some(s);
                st.busy = false;
            }
            match r {
                Err(e) => me.listener.on_event(TutorEventInfo::Failed {
                    message: e.to_string(),
                }),
                Ok(()) => {
                    let end = ended.into_inner().unwrap_or_else(|e| e.into_inner());
                    me.listener.on_event(end.unwrap_or(TutorEventInfo::Ended {
                        stop: "EndTurn".into(),
                    }));
                }
            }
            // A finished lesson earns points; say so.
            me.announce();
            if let Some(n) = namer {
                me.name(&n, &keys);
            }
            if let Some(c) = reviewer {
                for id in reviews {
                    me.check(&c, &id, &keys);
                }
            }
        });
        Ok(())
    }

    /// Asks the model to name the conversation `n` is a copy of, and keeps
    /// the name: in the session if it is back, else for when it is. A
    /// failure keeps the first question's words.
    fn name(self: &Arc<Self>, n: &Session, keys: &Keys) {
        let quiet = |_: agent::Event| {};
        let Ok((title, cost)) = n.name(&self.deps(keys, &quiet)) else {
            return;
        };
        let total = {
            let mut st = self.lock();
            let busy = st.busy;
            match st.session.as_mut() {
                Some(s) if s.id == n.id => {
                    s.title = Some(title.clone());
                    s.side_cost += cost;
                    let s = s.clone();
                    self.save(&s, &mut st);
                    s.cost()
                }
                // Another conversation is open now: its file only.
                Some(_) => {
                    let mut s = n.clone();
                    s.title = Some(title.clone());
                    s.side_cost += cost;
                    let _ = self.store.save(
                        &s,
                        &title,
                        self.store.meta(&s.id).map(|m| m.created).unwrap_or(0),
                    );
                    return;
                }
                None if busy => {
                    st.named = Some((n.id.clone(), title.clone(), cost));
                    n.cost() + cost
                }
                None => return,
            }
        };
        self.listener
            .on_event(TutorEventInfo::Named { title, total });
    }

    /// Checks lesson `id`, which the conversation `c` is a copy of wrote,
    /// and corrects its steps; the check's cost goes to the conversation.
    fn check(self: &Arc<Self>, c: &Session, id: &str, keys: &Keys) {
        let Some(store) = self.tools.lessons.store() else {
            return;
        };
        let quiet = |_: agent::Event| {};
        let d = self.deps(keys, &quiet);
        let (changed, cost) = check_lesson(c, id, &store, self.tools.as_ref(), &self.wb, &d)
            .map_or((0, 0.0), |(changed, cost, _)| (changed, cost));
        let total = self.charge(c, cost);
        self.lock().checking.remove(id);
        self.listener.on_event(TutorEventInfo::LessonChecked {
            lesson: id.to_owned(),
            changed,
            cost,
            total,
        });
    }

    /// Adds what a side request cost to the conversation `c` is a copy of:
    /// to the open session, to its file if another is open, or for when
    /// the turn that has it ends. Returns the conversation's cost.
    fn charge(&self, c: &Session, cost: f64) -> f64 {
        let mut st = self.lock();
        let busy = st.busy;
        match st.session.as_mut() {
            Some(s) if s.id == c.id => {
                s.side_cost += cost;
                let s = s.clone();
                self.save(&s, &mut st);
                s.cost()
            }
            Some(_) => {
                if let Ok((mut s, meta)) = self.store.load(&c.id) {
                    s.side_cost += cost;
                    let _ = self.store.save(&s, &meta.title, meta.created);
                    s.cost()
                } else {
                    c.cost() + cost
                }
            }
            None if busy => {
                st.charged.push((c.id.clone(), cost));
                c.cost() + cost
            }
            None => c.cost() + cost,
        }
    }
}

/// Checks lesson `id` from `c`, a copy of the conversation that wrote it
/// (docs/25, "Checking lessons"): Romlens's own findings, then the model's
/// review with only reads and `revise_lesson_step`, then the check recorded
/// in the lesson. Returns the steps rewritten, the cost and the reviewer's
/// last line; `None` when the lesson is not in the store.
pub fn check_lesson(
    c: &Session,
    id: &str,
    store: &romlens_tutor::lesson::LessonStore,
    tools: &RomTools,
    wb: &Workbench,
    d: &Deps,
) -> Option<(u32, f64, String)> {
    let lesson = store.load(id).ok()?;
    let insn_at = |a: u32| {
        let r = wb.resolve_any(super::tools::addr(a)).ok()?;
        let i = wb.disassemble(r.file_offset?, 1, None).into_iter().next()?;
        (i.snes_address == a).then(|| (i.mnemonic.clone(), i.operand_text.clone()))
    };
    let found = super::lessons::mechanical(&lesson, &insn_at);
    let review = ReviewTools(tools);
    let d = Deps {
        tools: &review,
        ..*d
    };
    let (note, cost) = c
        .review(&lesson.describe(), &found.join("\n"), &d)
        .unwrap_or_else(|e| (format!("The check stopped: {e}"), 0.0));
    let changed = store.load(id).map_or(0, |l| {
        l.revisions.len().saturating_sub(lesson.revisions.len())
    }) as u32;
    let _ = store.set_checked(
        id,
        romlens_tutor::lesson::Checked {
            when: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            changed,
            cost,
            note: note.clone(),
        },
    );
    Some((changed, cost, note))
}

/// The tools a lesson's check may use: every read, and rewriting a
/// lesson's step. The list is the conversation's, so the cache holds.
struct ReviewTools<'a>(&'a RomTools);

impl agent::Tools for ReviewTools<'_> {
    fn specs(&self) -> Vec<romlens_tutor::provider::ToolSpec> {
        self.0.specs()
    }

    fn kind(&self, name: &str) -> agent::ToolKind {
        self.0.kind(name)
    }

    fn run(
        &self,
        id: &str,
        name: &str,
        input: &serde_json::Value,
        cx: &agent::ToolContext,
    ) -> agent::ToolOutput {
        let building = [
            "begin_lesson",
            "lesson_step",
            "end_lesson",
            "generate_image",
        ];
        if super::edits::NAMES.contains(&name) || building.contains(&name) {
            return agent::ToolOutput::error(format!(
                "{name} is not for a check: only read, and correct steps with revise_lesson_step"
            ));
        }
        self.0.run(id, name, input, cx)
    }
}

#[uniffi::export]
impl TutorSession {
    /// `root` is the app's folder for conversations (Application Support
    /// on macOS); the ROM's conversations are under its SHA-256.
    #[uniffi::constructor]
    pub fn new(
        workbench: Arc<Workbench>,
        root: String,
        credentials: Arc<dyn CredentialStore>,
        listener: Arc<dyn TutorListener>,
    ) -> Arc<Self> {
        let sha = workbench.rom_identity().sha256;
        let tools = RomTools::new(Arc::clone(&workbench));
        // The student's lessons, beside the conversations: one record for
        // every ROM.
        let lessons = romlens_tutor::lesson::LessonStore::new(&PathBuf::from(&root));
        let quizzes = super::quiz::Quizzes::new(&lessons);
        tools.lessons.set_store(Some(lessons));
        Arc::new(TutorSession {
            quizzes,
            announced: Mutex::new(None),
            tools: Arc::new(tools),
            store: Store::new(&PathBuf::from(root), &sha),
            wb: workbench,
            keys: credentials,
            listener,
            transport: UreqTransport::new(),
            cards: Arc::new(Cards {
                pending: Mutex::new(Pending::default()),
                changed: Condvar::new(),
                cancel: Cancel::new(),
            }),
            state: Mutex::new(State {
                session: None,
                last: Vec::new(),
                title: String::new(),
                created: 0,
                mode_changed: false,
                explain_changed: false,
                at_step: None,
                busy: false,
                named: None,
                charged: Vec::new(),
                checking: Default::default(),
                check_lessons: true,
            }),
        })
    }

    /// Starts a conversation; the one before stays saved. Returns its id.
    pub fn new_conversation(
        &self,
        endpoint: TutorEndpointInfo,
        model: String,
        effort: Option<String>,
        mode: TutorMode,
        cost_cap: Option<f64>,
    ) -> Result<String, RomlensError> {
        let mut st = self.lock();
        if st.busy {
            return Err(err("the tutor is still answering"));
        }
        let id = new_id();
        let mut s = Session::new(&id, endpoint.into(), &model);
        s.effort = effort;
        s.mode = self::mode(mode);
        s.cost_cap = cost_cap;
        s.system = prompt::system();
        // What the student has learned, as it is now, for the lessons.
        s.digest = digest(&self.wb) + &prompt::learner_section(&self.learner_summary());
        st.last = Vec::new();
        st.title = String::new();
        st.created = now();
        // The first question says the mode.
        st.mode_changed = true;
        st.session = Some(s);
        Ok(id)
    }

    /// Goes on with a saved conversation.
    pub fn resume(&self, id: String) -> Result<Vec<TurnInfo>, RomlensError> {
        let mut st = self.lock();
        if st.busy {
            return Err(err("the tutor is still answering"));
        }
        let (s, meta) = self.store.load(&id).map_err(err)?;
        st.title = meta.title;
        st.created = meta.created;
        st.mode_changed = false;
        st.explain_changed = false;
        st.last = s.turns.clone();
        let turns = s
            .turns
            .iter()
            .enumerate()
            .map(|(i, t)| turn_info(i, t))
            .collect();
        st.session = Some(s);
        Ok(turns)
    }

    pub fn conversations(&self) -> Vec<ConversationSummaryInfo> {
        self.store
            .list()
            .into_iter()
            .map(|m: Meta| ConversationSummaryInfo {
                id: m.id,
                title: m.title,
                created: m.created,
                updated: m.updated,
                endpoint: m.endpoint.id,
                model: m.model,
                cost: m.cost,
                turns: m.turns as u32,
            })
            .collect()
    }

    pub fn delete_conversation(&self, id: String) -> Result<(), RomlensError> {
        let st = self.lock();
        if st.session.as_ref().is_some_and(|s| s.id == id) || st.busy {
            return Err(err("that conversation is open"));
        }
        self.store.delete(&id).map_err(err)
    }

    /// The open conversation's id.
    pub fn conversation_id(&self) -> Option<String> {
        let st = self.lock();
        st.session.as_ref().map(|s| s.id.clone())
    }

    /// The open conversation's title: the model's name for it once made,
    /// else its first question's words; `None` before the first question.
    pub fn title(&self) -> Option<String> {
        let st = self.lock();
        let named = st.session.as_ref().and_then(|s| s.title.clone());
        named.or_else(|| (!st.title.is_empty()).then(|| st.title.clone()))
    }

    /// Every turn, the ones not sent too (rewound or compacted).
    pub fn transcript(&self) -> Vec<TurnInfo> {
        let st = self.lock();
        let turns = st.session.as_ref().map(|s| &s.turns).unwrap_or(&st.last);
        turns
            .iter()
            .enumerate()
            .map(|(i, t)| turn_info(i, t))
            .collect()
    }

    /// A picture the transcript names.
    pub fn picture(&self, id: String) -> Option<Vec<u8>> {
        let st = self.lock();
        st.session
            .as_ref()
            .and_then(|s| s.pictures.get(&id).cloned())
    }

    pub fn is_busy(&self) -> bool {
        self.lock().busy
    }

    /// Asks a question; the answer comes as events, ending in `Ended` or
    /// `Failed`. `selection` is what the main window has selected, as text.
    pub fn send(
        self: Arc<Self>,
        text: String,
        attachments: Vec<AttachmentInfo>,
        selection: Option<String>,
    ) -> Result<(), RomlensError> {
        let (question, pictures) = {
            let mut st = self.lock();
            let s = st
                .session
                .as_ref()
                .ok_or_else(|| err("start a conversation first"))?;
            let changed = st.mode_changed.then_some(s.mode);
            let summary = self.learner_summary();
            let explain = st.explain_changed.then_some((s.explain, summary.as_str()));
            let mut blocks = Vec::new();
            let mut pictures = HashMap::new();
            for a in attachments {
                let id = super::png::id(&a.bytes).replace("tool-", "shot-");
                blocks.push(Block::Image {
                    image: ImageRef {
                        id: id.clone(),
                        media_type: a.media_type,
                    },
                });
                pictures.insert(id, a.bytes);
            }
            // The context and the student's words apart, so a rewind gives
            // back just what they typed.
            if let Some(c) = prompt::context(changed, explain, selection.as_deref()) {
                blocks.push(Block::Text { text: c });
            }
            if let Some(at) = st.at_step.take() {
                blocks.push(Block::Text { text: at });
            }
            blocks.push(Block::Text { text: text.clone() });
            st.mode_changed = false;
            st.explain_changed = false;
            (Turn::user(blocks), pictures)
        };
        let _ = self.store.remember_prompt(&text);
        self.run(move |s, d| {
            s.pictures.extend(pictures);
            s.ask(question, d).map(|_| ())
        })
    }

    /// Stops the turn (Esc): a stream between chunks, a card with a no.
    pub fn cancel(&self) {
        self.cards.cancel.cancel();
        self.cards.changed.notify_all();
    }

    /// The student's answer to the card `edit_id`.
    pub fn answer(&self, edit_id: String, accept: bool, why: Option<String>) {
        let mut g = self.cards.pending.lock().unwrap_or_else(|e| e.into_inner());
        g.answers.insert(
            edit_id,
            if accept {
                Decision::Accept
            } else {
                Decision::Reject { why }
            },
        );
        self.cards.changed.notify_all();
    }

    /// Changes the provider, model or effort from the next question on.
    pub fn set_model(
        &self,
        endpoint: TutorEndpointInfo,
        model: String,
        effort: Option<String>,
    ) -> Result<(), RomlensError> {
        let mut st = self.lock();
        let s = st
            .session
            .as_mut()
            .ok_or_else(|| err("start a conversation first"))?;
        let endpoint: Endpoint = endpoint.into();
        let note = if s.endpoint.id != endpoint.id || s.model != model {
            Some(format!(
                "Now on {model} ({}); the cache starts cold.",
                endpoint.id
            ))
        } else {
            None
        };
        s.keyless = !matches!(endpoint.id.as_str(), "anthropic" | "openai");
        s.endpoint = endpoint;
        s.model = model;
        s.effort = effort;
        if let Some(text) = note {
            s.turns.push(Turn::user(vec![Block::Note { text }]));
        }
        let snapshot = s.turns.clone();
        if let Some(s) = st.session.take() {
            self.save(&s, &mut st);
            st.session = Some(s);
        }
        st.last = snapshot;
        Ok(())
    }

    pub fn set_mode(&self, mode: TutorMode) {
        let mut st = self.lock();
        if let Some(s) = st.session.as_mut()
            && s.mode != self::mode(mode)
        {
            s.mode = self::mode(mode);
            st.mode_changed = true;
        }
    }

    /// Explain mode (docs/25): questions answered with lessons. The next
    /// question tells the model, with what the student has learned.
    pub fn set_explain(&self, on: bool) {
        let mut st = self.lock();
        if let Some(s) = st.session.as_mut()
            && s.explain != on
        {
            s.explain = on;
            st.explain_changed = true;
        }
    }

    pub fn explain(&self) -> bool {
        self.lock().session.as_ref().is_some_and(|s| s.explain)
    }

    /// The lesson step the student is reading, told to the model with the
    /// next question; `None` once they leave the lesson.
    pub fn set_lesson_step(&self, lesson: Option<String>, step: u32) {
        let mut st = self.lock();
        st.at_step = lesson.and_then(|id| {
            let l = self
                .tools
                .lessons
                .open_lesson(&id)
                .or_else(|| self.tools.lessons.store()?.load(&id).ok())?;
            let s = l.steps.get(step as usize)?;
            let mut note = format!(
                "[The student is at step {} of {} of lesson {} \"{}\": \"{}\"]",
                step + 1,
                l.steps.len(),
                l.id,
                l.title,
                s.title
            );
            // What they guessed at its question, so a wrong idea can be
            // taken up (docs/28).
            if let Some(g) = self.guess(&l.id, step) {
                note.push_str(&format!("\n[The student guessed: \"{g}\"]"));
            }
            Some(note)
        });
    }

    /// A lesson, finished or still being written.
    pub fn lesson(&self, id: String) -> Option<LessonInfo> {
        let rom = self.wb.rom_identity().sha256;
        self.tools
            .lessons
            .open_lesson(&id)
            .or_else(|| self.tools.lessons.store()?.load(&id).ok())
            .map(|l| {
                let mut info = LessonInfo {
                    checking: self.lock().checking.contains(&l.id),
                    ..lesson_info(&l, &rom)
                };
                self.add_guesses(&mut info);
                info
            })
    }

    /// Whether finished lessons are checked in the background (docs/25).
    pub fn set_check_lessons(&self, on: bool) {
        self.lock().check_lessons = on;
    }

    /// Every lesson, the latest first.
    pub fn lessons(&self) -> Vec<LessonInfo> {
        let rom = self.wb.rom_identity().sha256;
        self.tools
            .lessons
            .store()
            .map(|s| s.list().iter().map(|l| lesson_info(l, &rom)).collect())
            .unwrap_or_default()
    }

    /// A picture a lesson's step shows: kept with the lesson, or still in
    /// the conversation while it is written.
    pub fn lesson_picture(&self, lesson: String, picture: String) -> Option<Vec<u8>> {
        self.tools
            .lessons
            .store()
            .and_then(|s| s.picture(&lesson, &picture))
            .or_else(|| self.picture(picture))
    }

    pub fn delete_lesson(&self, id: String) -> Result<(), RomlensError> {
        let store = self
            .tools
            .lessons
            .store()
            .ok_or_else(|| err("no lessons here"))?;
        store.delete(&id).map_err(err)
    }

    /// What the student knows, every concept on the map.
    pub fn learner(&self) -> LearnerInfo {
        use romlens_tutor::lesson::{CONCEPTS, Group, LEVELS};
        let l = self
            .tools
            .lessons
            .store()
            .map(|s| s.learner())
            .unwrap_or_default();
        LearnerInfo {
            concepts: CONCEPTS
                .iter()
                .map(|c| {
                    let r = l.concepts.get(c.id);
                    ConceptInfo {
                        id: c.id.into(),
                        name: c.name.into(),
                        group: c.group.name().into(),
                        needs: c.needs.iter().map(|n| (*n).to_owned()).collect(),
                        line: c.line.into(),
                        level: r.map_or(0, |r| r.level),
                        marked: r.is_some_and(|r| r.marked),
                        lesson: r.and_then(|r| r.lesson.clone()),
                        proven: r.map_or(0, |r| r.proven),
                        due: r.and_then(|r| r.due),
                    }
                })
                .collect(),
            groups: Group::ALL.iter().map(|g| g.name().to_owned()).collect(),
            levels: LEVELS.iter().map(|l| (*l).to_owned()).collect(),
        }
    }

    /// Marks a concept known at a level (1 to 5), or clears the mark.
    pub fn mark_known(&self, concept: String, level: Option<u8>) -> Result<(), RomlensError> {
        let store = self
            .tools
            .lessons
            .store()
            .ok_or_else(|| err("no lessons here"))?;
        store.mark_known(&concept, level).map_err(err)
    }

    pub fn mode(&self) -> TutorMode {
        let st = self.lock();
        mode_info(
            st.session
                .as_ref()
                .map(|s| s.mode)
                .unwrap_or(Mode::AskBeforeEdits),
        )
    }

    pub fn set_cost_cap(&self, cap: Option<f64>) {
        if let Some(s) = self.lock().session.as_mut() {
            s.cost_cap = cap;
        }
    }

    /// The recording open in the main window, for the recording tools.
    pub fn set_recording(&self, recording: Option<Arc<RecordingSession>>) {
        self.tools.set_recording(recording);
    }

    /// Where `generate_image` draws (an OpenAI Images endpoint or one that
    /// speaks it), or `None` for no pictures.
    pub fn set_image_provider(&self, endpoint: Option<TutorEndpointInfo>, model: String) {
        let keys = Arc::clone(&self.keys);
        self.tools
            .set_images(endpoint.map(|e| super::tools::ImageSetup {
                endpoint: e.into(),
                model,
                key: Arc::new(move |id: &str| keys.key(id.to_owned()).filter(|k| !k.is_empty())),
            }));
    }

    /// Back to before the question at turn `index`: the conversation, the
    /// tutor's edits since, or both (`/rewind`).
    pub fn rewind(&self, index: u32, what: RewindWhat) -> Result<RewindResultInfo, RomlensError> {
        let mut st = self.lock();
        if st.busy {
            return Err(err("the tutor is still answering"));
        }
        let s = st
            .session
            .as_mut()
            .ok_or_else(|| err("start a conversation first"))?;
        let id = s.id.clone();
        let edits = match what {
            RewindWhat::Edits | RewindWhat::Both => Some(self.wb.rewind_tutor_edits(id, index)?),
            RewindWhat::Conversation => None,
        };
        let prompt = match what {
            RewindWhat::Conversation | RewindWhat::Both => {
                let p = s.rewind_to(index as usize);
                if p.is_none() {
                    return Err(err(
                        "that turn is not a question the conversation can go back to",
                    ));
                }
                s.turns.push(Turn::user(vec![Block::Note {
                    text: "Rewound to here.".into(),
                }]));
                p
            }
            RewindWhat::Edits => None,
        };
        if let Some(s) = st.session.take() {
            self.save(&s, &mut st);
            st.last = s.turns.clone();
            st.session = Some(s);
        }
        Ok(RewindResultInfo { prompt, edits })
    }

    /// The questions a rewind can go back to: turn index and words.
    pub fn rewind_points(&self) -> Vec<TurnInfo> {
        let st = self.lock();
        let Some(s) = st.session.as_ref() else {
            return Vec::new();
        };
        s.prompts()
            .into_iter()
            .map(|(i, _)| turn_info(i, &s.turns[i]))
            .collect()
    }

    /// Summarises the conversation to make room (`/compact`), on its own
    /// thread.
    pub fn compact(self: Arc<Self>) -> Result<(), RomlensError> {
        self.run(|s, d| {
            s.compact(d)?;
            d.events.event(agent::Event::Ended {
                stop: romlens_tutor::provider::Stop::EndTurn,
            });
            Ok(())
        })
    }

    /// What was asked in this project, oldest first, for ↑.
    pub fn prompt_history(&self, limit: u32) -> Vec<String> {
        self.store.prompts(limit as usize)
    }

    pub fn cost(&self) -> f64 {
        let st = self.lock();
        st.session
            .as_ref()
            .map(|s| s.cost())
            .unwrap_or_else(|| st.last.iter().map(|t| t.cost).sum())
    }

    /// How full the context was at the last reply, 0 to 1.
    pub fn context_used(&self) -> f64 {
        self.lock()
            .session
            .as_ref()
            .map(|s| s.context_used())
            .unwrap_or(0.0)
    }

    pub fn model(&self) -> Option<String> {
        self.lock().session.as_ref().map(|s| s.model.clone())
    }

    pub fn endpoint(&self) -> Option<TutorEndpointInfo> {
        self.lock().session.as_ref().map(|s| (&s.endpoint).into())
    }

    pub fn effort(&self) -> Option<String> {
        self.lock().session.as_ref().and_then(|s| s.effort.clone())
    }

    /// The models an endpoint serves (a network call: not on the main
    /// thread).
    pub fn list_models(&self, endpoint: TutorEndpointInfo) -> Result<Vec<String>, RomlensError> {
        let e: Endpoint = endpoint.into();
        let key = Keys(Arc::clone(&self.keys)).key(&e.id);
        list_models(&self.transport, &e, key.as_deref()).map_err(err)
    }
}

/// For shell tests: a server on the loopback that answers each request with
/// the next of `replies` (Chat Completions streams), one connection each.
/// A request to name the conversation (`agent::NAME_PROMPT`) is answered
/// "Test conversation" without taking a reply, then and after the last.
/// Returns its base URL. Nothing leaves the machine.
#[uniffi::export]
pub fn tutor_test_server(replies: Vec<String>) -> String {
    use std::io::{BufRead, BufReader, Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let url = format!("http://{}/v1", l.local_addr().expect("bound"));
    let naming = agent::NAME_PROMPT
        .split(':')
        .next()
        .unwrap_or_default()
        .to_owned();
    std::thread::spawn(move || {
        let mut replies = std::collections::VecDeque::from(replies);
        loop {
            let Ok((s, _)) = l.accept() else { return };
            let Ok(clone) = s.try_clone() else { return };
            let mut r = BufReader::new(clone);
            let mut len = 0;
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut b = vec![0; len];
            let _ = r.read_exact(&mut b);
            let body = if String::from_utf8_lossy(&b).contains(&naming) {
                tutor_test_text_reply("\"Test conversation.\"".into())
            } else if let Some(next) = replies.pop_front() {
                next
            } else {
                // Out of replies: the connection closes unanswered.
                continue;
            };
            let mut s = s;
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

fn test_chunk(delta: serde_json::Value, finish: Option<&str>) -> String {
    format!(
        "data: {}\n\n",
        serde_json::json!({"model": "test-model", "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    )
}

/// For shell tests: a Chat Completions stream that answers `text`.
#[uniffi::export]
pub fn tutor_test_text_reply(text: String) -> String {
    test_chunk(serde_json::json!({"content": text}), None)
        + &test_chunk(serde_json::json!({}), Some("stop"))
        + "data: [DONE]\n\n"
}

/// For shell tests: a Chat Completions stream that calls tool `name` with
/// `arguments` (JSON).
#[uniffi::export]
pub fn tutor_test_call_reply(name: String, arguments: String) -> String {
    // Each call its own id, as a model gives them, so the window pairs
    // each call with its own result.
    static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let id = format!(
        "call_{}",
        CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    // A line to the student first, as an edit needs.
    test_chunk(serde_json::json!({"content": "Let me look."}), None)
        + &test_chunk(
            serde_json::json!({"tool_calls": [{"index": 0, "id": id, "function": {"name": name, "arguments": arguments}}]}),
            Some("tool_calls"),
        )
        + "data: [DONE]\n\n"
}

/// For shell tests: the picture id `draw_diagram` gives a diagram that
/// needs no ROM (`fields`, `blocks`), with its spec as JSON.
#[uniffi::export]
pub fn tutor_test_diagram_id(kind: String, spec: String) -> String {
    serde_json::from_str(&spec)
        .ok()
        .and_then(|v| super::draw::id_without_rom(&kind, &v))
        .unwrap_or_default()
}

/// For shell tests: the right answer to each question of a quiz, as the
/// window would send it. The window itself is never told them.
#[uniffi::export]
pub fn tutor_test_quiz_answers(
    session: Arc<TutorSession>,
    quiz: String,
) -> Vec<super::quiz::GivenInfo> {
    use super::quiz::GivenInfo;
    use romlens_tutor::quiz::Ask;
    let Ok(q) = session.quizzes.store.load(&quiz) else {
        return Vec::new();
    };
    q.questions
        .iter()
        .map(|x| match &x.ask {
            Ask::Choice { answer, .. } => GivenInfo::Choice {
                index: *answer as u32,
            },
            Ask::Number { answer, hex } => GivenInfo::Number {
                text: if *hex {
                    format!("${answer:X}")
                } else {
                    answer.to_string()
                },
            },
            Ask::Bits { answer, .. } => GivenInfo::Bits {
                bits: answer.clone(),
            },
            Ask::Line { answer, .. } => GivenInfo::Line {
                index: *answer as u32,
            },
            Ask::Text { .. } => GivenInfo::Text {
                text: "an answer".into(),
            },
        })
        .collect()
}

impl TutorSession {
    fn learner_store(&self) -> Result<Arc<romlens_tutor::lesson::LessonStore>, RomlensError> {
        self.tools
            .lessons
            .store()
            .ok_or_else(|| err("no learner record here"))
    }

    fn rom_title(&self) -> String {
        let t = self.wb.rom().info().title;
        if t.trim().is_empty() {
            "this game".into()
        } else {
            t.trim().to_owned()
        }
    }

    /// Tells the window what is new since progress was last worked out:
    /// points, proofs, achievements, a rank. The first time only notes it.
    fn announce(&self) {
        let Ok(store) = self.learner_store() else {
            return;
        };
        let now = self.quizzes.progress(&store);
        let mut last = self.announced.lock().unwrap_or_else(|e| e.into_inner());
        let Some(before) = last.replace(now.clone()) else {
            return;
        };
        let g = now.diff(&before);
        if g.xp == 0 && g.proofs.is_empty() && g.unlocked.is_empty() && g.rank.is_none() {
            return;
        }
        let name = |id: &str| {
            romlens_tutor::lesson::concept(id).map_or(id.to_owned(), |c| c.name.to_owned())
        };
        let title = |u: &romlens_tutor::progress::Unlocked| {
            romlens_tutor::progress::ACHIEVEMENTS
                .iter()
                .find(|a| a.id == u.id)
                .map(|a| a.title.to_owned())
                .or_else(|| romlens_tutor::progress::milestone(u.id).map(|m| m.title.to_owned()))
                .unwrap_or_else(|| u.id.to_owned())
        };
        self.listener.on_event(TutorEventInfo::Progress {
            gained: g.xp,
            proven: g
                .proofs
                .iter()
                .map(|(c, l)| format!("{}, level {l}", name(c)))
                .collect(),
            unlocked: g.unlocked.iter().map(title).collect(),
            rank: g.rank.map(str::to_owned),
        });
    }

    /// The tutor's questions for quiz `id`, written on a copy of `w`.
    fn write_quiz(self: &Arc<Self>, w: &Session, id: &str) {
        let held = crate::quiz::Held::of(&self.wb);
        let world = held.world();
        let tools = super::quiz::QuizTools {
            tools: self.tools.as_ref(),
            world: &world,
            quizzes: &self.quizzes,
            listener: self.listener.as_ref(),
            quiz: id.to_owned(),
            added: Mutex::new(0),
        };
        let keys = Keys(Arc::clone(&self.keys));
        let quiet = |_: agent::Event| {};
        let d = Deps {
            tools: &tools,
            ..self.deps(&keys, &quiet)
        };
        let brief = self
            .quizzes
            .store
            .load(id)
            .map(|q| super::quiz::brief(&q))
            .unwrap_or_default();
        let cost = w.write_quiz(&brief, &d).map_or(0.0, |(_, c)| c);
        if let Ok(mut q) = self.quizzes.store.load(id) {
            q.cost += cost;
            let _ = self.quizzes.store.save(&q);
        }
        if cost > 0.0 {
            self.charge(w, cost);
        }
        self.quizzes
            .writing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        self.listener.on_event(TutorEventInfo::QuizChanged {
            quiz: id.to_owned(),
        });
    }

    /// The model's mark for a written answer.
    fn grade(self: &Arc<Self>, quiz: &str, question: &str) {
        let writer = self
            .quizzes
            .writer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(w) = writer.or_else(|| self.lock().session.clone()) else {
            return;
        };
        let Ok(q) = self.quizzes.store.load(quiz) else {
            return;
        };
        let (Some(asked), Some(a)) = (q.question(question), q.attempt(question)) else {
            return;
        };
        let romlens_tutor::quiz::Given::Text(answer) = &a.given else {
            return;
        };
        let keys = Keys(Arc::clone(&self.keys));
        let quiet = |_: agent::Event| {};
        // No tools: a mark needs none.
        let none = super::quiz::NoTools(self.tools.as_ref());
        let d = Deps {
            tools: &none,
            ..self.deps(&keys, &quiet)
        };
        let (credit, said, cost) = w
            .grade(&asked.prompt, &asked.explanation, answer, &d)
            .unwrap_or_else(|e| (0.0, format!("The answer couldn't be marked: {e}"), 0.0));
        if let Ok(mut q) = self.quizzes.store.load(quiz) {
            let _ = q.mark(question, credit, &said);
            q.cost += cost;
            let _ = self.quizzes.store.save(&q);
        }
        if cost > 0.0 {
            self.charge(&w, cost);
        }
        self.listener.on_event(TutorEventInfo::QuizChanged {
            quiz: quiz.to_owned(),
        });
        self.announce();
    }

    /// The first guess at a lesson step's predict question.
    fn guess(&self, lesson: &str, step: u32) -> Option<String> {
        use romlens_tutor::progress::Fact;
        self.quizzes
            .journal
            .read()
            .into_iter()
            .find_map(|f| match f {
                Fact::PredictAnswered {
                    lesson: l,
                    step: s,
                    guess,
                    ..
                } if l == lesson && s == step => Some(guess),
                _ => None,
            })
    }

    /// Each step's first guess, from the journal.
    fn add_guesses(&self, info: &mut LessonInfo) {
        use romlens_tutor::progress::Fact;
        for f in self.quizzes.journal.read() {
            if let Fact::PredictAnswered {
                lesson,
                step,
                guess,
                right,
                ..
            } = f
                && lesson == info.id
                && let Some(s) = info.steps.get_mut(step as usize)
                && s.guessed.is_none()
            {
                s.guessed = Some(guess);
                s.guess_right = right;
            }
        }
    }

    fn load_quiz(&self, id: &str) -> Result<romlens_tutor::quiz::Quiz, RomlensError> {
        self.quizzes.store.load(id).map_err(err)
    }

    fn quiz_record(&self, q: &romlens_tutor::quiz::Quiz) -> super::quiz::QuizInfo {
        let writing = self
            .quizzes
            .writing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&q.id);
        super::quiz::quiz_info(q, writing)
    }
}

/// Quizzes and progress (docs/28).
#[uniffi::export]
impl TutorSession {
    /// Days are counted in local time: the offset from UTC in seconds.
    pub fn set_utc_offset(&self, seconds: i32) {
        self.quizzes.set_offset(seconds);
    }

    /// Starts a quiz of Romlens's questions: to prove a concept at a level
    /// (the next one to prove when none is given), to practise, or to
    /// review what is due.
    ///
    /// With `tutor`, and a conversation open, the tutor adds up to two
    /// questions about the game in the background (`QuizChanged` as each
    /// comes), each checked by Romlens, its cost added to the
    /// conversation's.
    pub fn start_quiz(
        self: Arc<Self>,
        concept: Option<String>,
        level: Option<u8>,
        purpose: super::quiz::QuizPurposeInfo,
        tutor: bool,
    ) -> Result<super::quiz::QuizInfo, RomlensError> {
        use romlens_tutor::quiz::Purpose;
        let store = self.learner_store()?;
        let progress = self.quizzes.progress(&store);
        let learner = store.learner();
        let purpose = match purpose {
            super::quiz::QuizPurposeInfo::Prove => Purpose::Prove,
            super::quiz::QuizPurposeInfo::Review => Purpose::Review,
            super::quiz::QuizPurposeInfo::Practice => Purpose::Practice,
        };
        let mut quiz = super::quiz::start(
            &self.wb,
            &progress,
            &|id| learner.level(id),
            concept.as_deref(),
            level,
            purpose,
            &self.wb.rom_identity().sha256,
            &self.rom_title(),
            None,
        )
        .map_err(err)?;
        quiz.conversation = self.conversation_id().unwrap_or_default();
        self.quizzes.store.save(&quiz).map_err(err)?;
        // A baseline for what the quiz will earn.
        if self
            .announced
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
        {
            self.announce();
        }
        // The conversation, as the tutor's parts of the quiz start from.
        let writer = {
            let st = self.lock();
            if st.busy { None } else { st.session.clone() }
        };
        *self
            .quizzes
            .writer
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = writer.clone();
        if let (true, Some(w)) = (tutor, writer) {
            self.quizzes
                .writing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(quiz.id.clone());
            let me = Arc::clone(&self);
            let id = quiz.id.clone();
            std::thread::spawn(move || me.write_quiz(&w, &id));
        }
        Ok(self.quiz_record(&quiz))
    }

    pub fn quiz(&self, id: String) -> Option<super::quiz::QuizInfo> {
        self.load_quiz(&id).ok().map(|q| self.quiz_record(&q))
    }

    /// Every quiz, the latest first.
    pub fn quizzes(&self) -> Vec<super::quiz::QuizInfo> {
        let mut all = self.quizzes.store.list();
        all.reverse();
        all.iter().map(|q| self.quiz_record(q)).collect()
    }

    /// Answers a question and marks it: the right answer and why come back.
    /// A written answer is marked by the model in the background: its
    /// credit is none until `QuizChanged`.
    pub fn answer_question(
        self: Arc<Self>,
        quiz: String,
        question: String,
        given: super::quiz::GivenInfo,
    ) -> Result<super::quiz::AnswerResultInfo, RomlensError> {
        let mut q = self.load_quiz(&quiz)?;
        let ask = q
            .question(&question)
            .ok_or_else(|| err(format!("{question} is not a question of this quiz")))?
            .ask
            .clone();
        let g = super::quiz::given(&ask, given).map_err(err)?;
        q.answer(&question, g).map_err(err)?;
        self.quizzes.store.save(&q).map_err(err)?;
        if q.attempt(&question).is_some_and(|a| a.credit.is_none()) {
            let me = Arc::clone(&self);
            let (quiz, question) = (quiz.clone(), question.clone());
            std::thread::spawn(move || me.grade(&quiz, &question));
        } else {
            self.announce();
        }
        super::quiz::result_info(&q, &question).ok_or_else(|| err("no answer"))
    }

    /// A question's hint; it halves what the answer earns.
    pub fn question_hint(&self, quiz: String, question: String) -> Result<String, RomlensError> {
        let mut q = self.load_quiz(&quiz)?;
        let h = q.hint(&question).map_err(err)?;
        self.quizzes.store.save(&q).map_err(err)?;
        Ok(h)
    }

    /// Ends a quiz: what it proved, and its review, now count.
    pub fn finish_quiz(&self, quiz: String) -> Result<super::quiz::QuizInfo, RomlensError> {
        let mut q = self.load_quiz(&quiz)?;
        if q.finished.is_none() {
            q.finished = Some(romlens_tutor::store::now());
            self.quizzes.store.save(&q).map_err(err)?;
        }
        self.announce();
        Ok(self.quiz_record(&q))
    }

    /// Points, rank, streak, achievements, this game's milestones and the
    /// reviews due.
    pub fn progress(&self) -> Option<super::quiz::ProgressInfo> {
        let store = self.learner_store().ok()?;
        let p = self.quizzes.progress(&store);
        Some(super::quiz::progress_info(
            &p,
            &store,
            &self.wb.rom_identity().sha256,
            &self.rom_title(),
        ))
    }

    /// Checks this game's milestones against the project now, keeps those
    /// newly met, and returns their titles.
    pub fn check_milestones(&self) -> Vec<String> {
        let got = super::quiz::record_milestones(
            &self.quizzes,
            &self.wb,
            &self.wb.rom_identity().sha256,
            &self.rom_title(),
        );
        if !got.is_empty() {
            self.announce();
        }
        got
    }

    /// A guess at a lesson step's predict question, before Show. The first
    /// guess at a step is kept and earns its points; where the step carries
    /// a claim and the guess reads as its answer, Romlens says whether it is
    /// right (`right` is none when it can't tell).
    pub fn answer_predict(
        &self,
        lesson: String,
        step: u32,
        guess: String,
    ) -> Result<super::quiz::PredictResultInfo, RomlensError> {
        let guess = guess.trim().to_owned();
        if guess.chars().count() < 3 {
            return Err(err("a guess of a few words or a number"));
        }
        let l = self
            .tools
            .lessons
            .open_lesson(&lesson)
            .or_else(|| self.tools.lessons.store()?.load(&lesson).ok())
            .ok_or_else(|| err(format!("no lesson {lesson}")))?;
        let s = l
            .steps
            .get(step as usize)
            .filter(|s| s.predict.is_some())
            .ok_or_else(|| err("that step asks nothing"))?;
        let right = s.predict_answer.as_ref().and_then(|c| {
            let alt = c.with_expected(&guess)?;
            let held = crate::quiz::Held::of(&self.wb);
            Some(crate::quiz::claims::check(&held.world(), &alt).is_ok())
        });
        let first = self.guess(&lesson, step).is_none();
        if first {
            self.quizzes
                .journal
                .append(&romlens_tutor::progress::Fact::PredictAnswered {
                    lesson: lesson.clone(),
                    step,
                    guess,
                    right,
                    when: now(),
                })
                .map_err(err)?;
            self.announce();
        }
        // The next question says what they guessed.
        self.set_lesson_step(Some(lesson), step);
        Ok(super::quiz::PredictResultInfo { right, first })
    }

    /// Removes every quiz and the journal: points, proofs, streaks and
    /// achievements start again. Lessons and marks stay.
    pub fn reset_progress(&self) -> Result<(), RomlensError> {
        self.quizzes.store.reset().map_err(err)?;
        self.quizzes.journal.reset().map_err(err)?;
        *self.announced.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn text_reply(t: &str) -> String {
        tutor_test_text_reply(t.into())
    }

    fn call_reply(name: &str, args: serde_json::Value) -> String {
        tutor_test_call_reply(name.into(), args.to_string())
    }

    fn server(replies: Vec<String>) -> String {
        tutor_test_server(replies)
    }

    struct NoKeys;
    impl CredentialStore for NoKeys {
        fn key(&self, _: String) -> Option<String> {
            None
        }
    }

    struct Heard(Mutex<mpsc::Sender<TutorEventInfo>>);
    impl TutorListener for Heard {
        fn on_event(&self, e: TutorEventInfo) {
            let _ = self.0.lock().unwrap().send(e);
        }
    }

    fn until_done(
        rx: &mpsc::Receiver<TutorEventInfo>,
        mut each: impl FnMut(&TutorEventInfo),
    ) -> TutorEventInfo {
        loop {
            let e = rx
                .recv_timeout(Duration::from_secs(20))
                .expect("the turn ends");
            each(&e);
            if matches!(
                e,
                TutorEventInfo::Ended { .. } | TutorEventInfo::Failed { .. }
            ) {
                return e;
            }
        }
    }

    fn setup(
        name: &str,
        replies: Vec<String>,
    ) -> (
        Arc<TutorSession>,
        mpsc::Receiver<TutorEventInfo>,
        TutorEndpointInfo,
        PathBuf,
        Arc<Workbench>,
    ) {
        let rom = crate::Rom::from_bytes(romlens_core::fixtures::explain_lorom(), "e.sfc".into())
            .unwrap();
        let wb = Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        let root = std::env::temp_dir().join(format!(
            "romlens-tutor-session-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let (tx, rx) = mpsc::channel();
        let t = TutorSession::new(
            Arc::clone(&wb),
            root.to_string_lossy().into_owned(),
            Arc::new(NoKeys),
            Arc::new(Heard(Mutex::new(tx))),
        );
        let local = TutorEndpointInfo {
            id: "ollama".into(),
            protocol: TutorProtocol::Chat,
            base_url: server(replies),
            tool_choice: false,
            strict: false,
            vision: false,
        };
        (t, rx, local, root, wb)
    }

    /// The right answer to a question, as the window would send it.
    fn right(q: &romlens_tutor::quiz::Question) -> super::super::quiz::GivenInfo {
        use super::super::quiz::GivenInfo;
        use romlens_tutor::quiz::Ask;
        match &q.ask {
            Ask::Choice { answer, .. } => GivenInfo::Choice {
                index: *answer as u32,
            },
            Ask::Number { answer, hex } => GivenInfo::Number {
                text: if *hex {
                    format!("${answer:X}")
                } else {
                    answer.to_string()
                },
            },
            Ask::Bits { answer, .. } => GivenInfo::Bits {
                bits: answer.clone(),
            },
            Ask::Line { answer, .. } => GivenInfo::Line {
                index: *answer as u32,
            },
            Ask::Text { .. } => GivenInfo::Text {
                text: "because".into(),
            },
        }
    }

    #[test]
    fn a_quiz_proves_a_level_with_no_model() {
        use super::super::quiz::QuizPurposeInfo;
        let (t, rx, _local, root, _wb) = setup("quiz", vec![]);
        t.set_utc_offset(0);
        let info = Arc::clone(&t)
            .start_quiz(
                Some("Sprites".into()),
                Some(1),
                QuizPurposeInfo::Prove,
                false,
            )
            .unwrap();
        assert_eq!(
            (info.concept.as_str(), info.level, info.questions.len()),
            ("sprites", 1, 5)
        );
        assert!(
            info.questions
                .iter()
                .all(|q| q.certain && q.source == "From Romlens's tables")
        );
        let quiz = t.quizzes.store.load(&info.id).unwrap();
        for q in &quiz.questions {
            let r = Arc::clone(&t)
                .answer_question(info.id.clone(), q.id.clone(), right(q))
                .unwrap();
            assert_eq!(r.credit, Some(1.0), "{}", q.prompt);
            assert!(!r.explanation.is_empty());
        }
        assert!(
            Arc::clone(&t)
                .answer_question(
                    info.id.clone(),
                    quiz.questions[0].id.clone(),
                    right(&quiz.questions[0])
                )
                .is_err()
        );
        let done = t.finish_quiz(info.id.clone()).unwrap();
        assert!(done.outcome.passed && done.finished, "{:?}", done.outcome);
        // The window hears what it earned.
        let mut proven = Vec::new();
        while let Ok(e) = rx.recv_timeout(Duration::from_millis(200)) {
            if let TutorEventInfo::Progress {
                proven: p, gained, ..
            } = e
            {
                assert!(gained > 0);
                proven.extend(p);
            }
        }
        assert_eq!(proven, vec!["Sprites, level 1".to_string()]);
        let sprites = t
            .learner()
            .concepts
            .into_iter()
            .find(|c| c.id == "sprites")
            .unwrap();
        assert_eq!(
            (sprites.level, sprites.proven),
            (1, 1),
            "a proven level is a level reached"
        );
        assert!(sprites.due.is_some());
        let p = t.progress().unwrap();
        assert!(p.xp >= 50, "{}", p.xp);
        assert_eq!(p.rank, "Reset");
        assert!(
            p.achievements
                .iter()
                .any(|a| a.id == "first_proof" && a.unlocked.is_some())
        );
        assert!(
            p.achievements
                .iter()
                .any(|a| a.game && a.rom_title.is_some()),
            "this game's milestones are listed"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_hint_halves_the_answer_and_too_few_questions_is_said() {
        use super::super::quiz::QuizPurposeInfo;
        let (t, _rx, _local, root, _wb) = setup("hint", vec![]);
        let info = Arc::clone(&t)
            .start_quiz(
                Some("ppu".into()),
                Some(2),
                QuizPurposeInfo::Practice,
                false,
            )
            .unwrap();
        // Which five questions a quiz asks depends on its seed: give the
        // first a hint.
        let mut quiz = t.quizzes.store.load(&info.id).unwrap();
        quiz.questions[0].hint = Some("Look near $2100.".into());
        t.quizzes.store.save(&quiz).unwrap();
        let q = &quiz.questions[0];

        assert!(
            t.question_hint(info.id.clone(), q.id.clone())
                .unwrap()
                .contains("$2100")
        );
        let r = Arc::clone(&t)
            .answer_question(info.id.clone(), q.id.clone(), right(q))
            .unwrap();
        assert_eq!(r.credit, Some(0.5));
        // Compression at level 4 has no questions in this game.
        let e = Arc::clone(&t)
            .start_quiz(
                Some("compression".into()),
                Some(4),
                QuizPurposeInfo::Prove,
                false,
            )
            .unwrap_err();
        assert!(
            e.to_string().contains("can't be proven in this game yet"),
            "{e}"
        );
        assert!(
            Arc::clone(&t)
                .start_quiz(None, None, QuizPurposeInfo::Review, false)
                .is_err(),
            "nothing is due"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_tutor_adds_checked_questions_and_the_model_marks_writing() {
        use super::super::quiz::{GivenInfo, QuizPurposeInfo};
        let field = |expect: &str| {
            serde_json::json!({"kind": "register_field", "register": "INIDISP", "value": 0x80,
                "field": "Forced blank", "expect": expect})
            .to_string()
        };
        let question = |claim: Option<String>, kind: &str| {
            call_reply(
                "quiz_question",
                serde_json::json!({"quiz": "", "prompt": format!("Writing $80 to INIDISP: forced blank? ({kind})"),
                    "kind": kind, "choices": ["forced blank", "display on"], "answer": 0, "bits": null,
                    "claim": claim, "explanation": "Bit 7 of INIDISP is forced blank: the screen is black and VRAM can be written.",
                    "hint": null}),
            )
        };
        let (t, rx, local, root, _wb) = setup(
            "tutorquiz",
            vec![
                // A false claim: refused, with what is so.
                question(Some(field("display on")), "choice"),
                question(Some(field("forced blank")), "choice"),
                question(None, "text"),
                text_reply("Asked two."),
                text_reply("CREDIT 1: Right, the PPU leaves VRAM alone then."),
            ],
        );
        t.new_conversation(local, "qwen3".into(), None, TutorMode::ReadOnly, None)
            .unwrap();
        let turns = t.transcript().len();
        let info = Arc::clone(&t)
            .start_quiz(
                Some("forced_blank".into()),
                Some(2),
                QuizPurposeInfo::Prove,
                true,
            )
            .unwrap();
        assert!(info.writing);
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let q = t.quiz(info.id.clone()).unwrap();
            if !q.writing {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "the writer ends");
            let _ = rx.recv_timeout(Duration::from_millis(100));
        }
        let quiz = t.quizzes.store.load(&info.id).unwrap();
        let theirs: Vec<_> = quiz
            .questions
            .iter()
            .filter(|q| q.id.contains("-t"))
            .collect();
        assert_eq!(theirs.len(), 2, "the refused one is not in the quiz");
        assert_eq!(theirs[0].source.label(), "By the tutor, checked by Romlens");
        assert!(!theirs[1].source.certain());
        assert_eq!(t.transcript().len(), turns, "the conversation is untouched");
        // The written answer, marked by the model.
        let written = theirs[1].id.clone();
        let r = Arc::clone(&t)
            .answer_question(
                info.id.clone(),
                written.clone(),
                GivenInfo::Text {
                    text: "VRAM is free then".into(),
                },
            )
            .unwrap();
        assert_eq!(r.credit, None, "marked in the background");
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let marked = loop {
            let q = t.quizzes.store.load(&info.id).unwrap();
            if let Some(c) = q.attempt(&written).and_then(|a| a.credit) {
                break (c, q.attempt(&written).unwrap().feedback.clone());
            }
            assert!(std::time::Instant::now() < deadline, "the mark comes");
            let _ = rx.recv_timeout(Duration::from_millis(100));
        };
        assert_eq!(marked.0, 1.0);
        assert!(marked.1.unwrap().contains("VRAM alone"));
        let o = t.quizzes.store.load(&info.id).unwrap().outcome();
        assert_eq!(o.certain_right, 0, "the model's mark is never certain");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn quiz_question_is_refused_outside_a_quiz_and_the_tools_stay_the_same() {
        use romlens_tutor::agent::Tools;
        let (t, _rx, _local, root, _wb) = setup("quiztool", vec![]);
        let names: Vec<String> = t.tools.specs().into_iter().map(|s| s.name).collect();
        assert!(names.iter().any(|n| n == "quiz_question"));
        let held = crate::quiz::Held::of(&t.wb);
        let world = held.world();
        let q = super::super::quiz::QuizTools {
            tools: t.tools.as_ref(),
            world: &world,
            quizzes: &t.quizzes,
            listener: t.listener.as_ref(),
            quiz: "q1-00000".into(),
            added: Mutex::new(0),
        };
        let same: Vec<String> = q.specs().into_iter().map(|s| s.name).collect();
        assert_eq!(names, same, "the writer's list is the conversation's");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_guess_is_kept_once_checked_and_told_to_the_tutor() {
        use romlens_tutor::lesson::{Lesson, Step};
        let (t, _rx, _local, root, _wb) = setup("guess", vec![]);
        let store = t.tools.lessons.store().unwrap();
        let step = |predict: Option<&str>, claim: Option<romlens_tutor::quiz::Claim>| Step {
            title: "Where".into(),
            predict: predict.map(str::to_owned),
            body: "At $2100.".into(),
            focus: None,
            picture: None,
            predict_answer: claim,
        };
        let lesson = Lesson {
            id: "l1-00000".into(),
            title: "The screen".into(),
            rom: None,
            created: 1,
            levels: (2, 2),
            concepts: vec![("forced_blank".into(), 2)],
            builds_on: vec![],
            steps: vec![
                step(
                    Some("Where is INIDISP?"),
                    Some(romlens_tutor::quiz::Claim::RegisterAddress {
                        register: "INIDISP".into(),
                        expect: 0x2100,
                    }),
                ),
                step(Some("Why black?"), None),
                step(None, None),
            ],
            next: vec![],
            conversation: String::new(),
            finished: true,
            revisions: vec![],
            checked: None,
        };
        store.save(&lesson, &[]).unwrap();
        let r = t
            .answer_predict(lesson.id.clone(), 0, " $2100 ".into())
            .unwrap();
        assert_eq!((r.right, r.first), (Some(true), true));
        let again = t
            .answer_predict(lesson.id.clone(), 0, "$2105".into())
            .unwrap();
        assert_eq!(
            (again.right, again.first),
            (Some(false), false),
            "checked, but only the first counts"
        );
        let unchecked = t
            .answer_predict(lesson.id.clone(), 1, "the PPU stops".into())
            .unwrap();
        assert_eq!(unchecked.right, None, "no claim, nothing marked");
        assert!(
            t.answer_predict(lesson.id.clone(), 2, "anything".into())
                .is_err(),
            "no question"
        );
        assert!(
            t.answer_predict(lesson.id.clone(), 1, "no".into()).is_err(),
            "too short"
        );
        let info = t.lesson(lesson.id.clone()).unwrap();
        assert_eq!(info.steps[0].guessed.as_deref(), Some("$2100"));
        assert_eq!(info.steps[0].guess_right, Some(true));
        assert!(info.steps[0].checks_guess && !info.steps[1].checks_guess);
        // The next question tells the tutor.
        t.set_lesson_step(Some(lesson.id.clone()), 1);
        let note = t.lock().at_step.clone().unwrap();
        assert!(
            note.contains("The student guessed: \"the PPU stops\""),
            "{note}"
        );
        // Points: 3 + 2 for the first, 3 for the second, and the lesson's own.
        let p = t.progress().unwrap();
        assert!(
            p.recent
                .iter()
                .any(|l| l.why.contains("predict") && l.points == 5)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_milestone_once_met_is_kept() {
        let (t, _rx, _local, root, wb) = setup("milestone", vec![]);
        assert!(t.check_milestones().is_empty());
        wb.execute(crate::records::Command::SetLabel {
            address: 0x8000,
            name: Some("Boot".into()),
        })
        .unwrap();
        // The test program's reset routine also waits for vertical blank
        // and sends DMA to VRAM: naming it meets all three.
        assert_eq!(
            t.check_milestones(),
            vec!["Where it all starts", "Round and round", "Pictures in"]
        );
        assert!(t.check_milestones().is_empty(), "once");
        wb.execute(crate::records::Command::SetLabel {
            address: 0x8000,
            name: None,
        })
        .unwrap();
        let p = t.progress().unwrap();
        let m = p
            .achievements
            .iter()
            .find(|a| a.id == "reset_named")
            .unwrap();
        assert!(m.unlocked.is_some(), "a name taken away takes nothing back");
        assert!(p.xp >= 20);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_finished_lesson_is_checked_and_corrected() {
        let (t, rx, local, root, _wb) = setup(
            "check",
            vec![
                call_reply(
                    "begin_lesson",
                    serde_json::json!({"title": "DMA", "from_level": 1, "to_level": 1,
                        "concepts": [{"id": "dma", "level": 1}], "builds_on": []}),
                ),
                call_reply(
                    "lesson_step",
                    serde_json::json!({"lesson": "", "title": "The cost", "predict": null,
                        "body": "DMA takes about 8 CPU cycles a byte.", "focus_address": null, "focus_end": null,
                        "focus_in": null, "focus_frame": null, "focus_view": null, "picture": null}),
                ),
                call_reply("end_lesson", serde_json::json!({"lesson": "", "next": []})),
                text_reply("That is DMA."),
                // The check: it tries an edit, which is refused, then
                // corrects the step.
                call_reply(
                    "set_label",
                    serde_json::json!({"address": "$00:8000", "name": "Boot", "reason": "r"}),
                ),
                call_reply(
                    "revise_lesson_step",
                    serde_json::json!({"lesson": "", "step": 1, "title": "The cost", "predict": null,
                        "body": "DMA takes 8 master cycles a byte.", "reason": "DMA's cost is in master cycles"}),
                ),
                text_reply("Corrected step 1: the units."),
            ],
        );
        t.new_conversation(local, "qwen3".into(), None, TutorMode::AcceptEdits, None)
            .unwrap();
        t.set_explain(true);
        Arc::clone(&t)
            .send("What is DMA?".into(), Vec::new(), None)
            .unwrap();
        let end = until_done(&rx, |_| {});
        assert!(matches!(end, TutorEventInfo::Ended { .. }), "{end:?}");
        let turns = t.transcript().len();
        let checked = loop {
            match rx
                .recv_timeout(Duration::from_secs(20))
                .expect("the check ends")
            {
                e @ TutorEventInfo::LessonChecked { .. } => break e,
                _ => continue,
            }
        };
        let TutorEventInfo::LessonChecked {
            lesson, changed, ..
        } = checked
        else {
            unreachable!()
        };
        assert_eq!(changed, 1);
        let store = romlens_tutor::lesson::LessonStore::new(&root);
        let l = store.load(&lesson).unwrap();
        assert_eq!(l.steps[0].body, "DMA takes 8 master cycles a byte.");
        assert_eq!(
            l.revisions[0].before.body,
            "DMA takes about 8 CPU cycles a byte."
        );
        let c = l.checked.as_ref().unwrap();
        assert_eq!(
            (c.changed, c.note.as_str()),
            (1, "Corrected step 1: the units.")
        );
        // The transcript is as the turn left it, and the edit was not made.
        assert_eq!(t.transcript().len(), turns);
        assert!(t.wb.label_at(0x008000).is_none_or(|l| l.name != "Boot"));
        let info = t.lesson(lesson).unwrap();
        assert!(info.checked.is_some() && !info.checking);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_lesson_step_carries_a_diagram_romlens_drew() {
        // The picture's id is its bytes' hash, so the scripted step can
        // name it before it is drawn.
        let spec = serde_json::json!({"kind": "blocks", "spec": {"preset": "machine", "highlight": ["ppu"]}});
        let id = tutor_test_diagram_id("blocks".into(), spec["spec"].to_string());
        assert!(id.starts_with("draw-"));
        let (t, rx, local, root, _wb) = setup(
            "diagram",
            vec![
                call_reply("draw_diagram", spec),
                call_reply(
                    "begin_lesson",
                    serde_json::json!({"title": "The machine", "from_level": 1,
                        "to_level": 1, "concepts": [{"id": "ppu", "level": 1}], "builds_on": []}),
                ),
                call_reply(
                    "lesson_step",
                    serde_json::json!({"lesson": "", "title": "Two chips", "predict": null,
                        "body": "The CPU decides; the PPU draws.", "focus_address": null, "focus_end": null,
                        "focus_in": null, "focus_frame": null, "focus_view": null, "picture": id}),
                ),
                call_reply("end_lesson", serde_json::json!({"lesson": "", "next": []})),
                text_reply("Here is the machine."),
            ],
        );
        t.new_conversation(local, "qwen3".into(), None, TutorMode::ReadOnly, None)
            .unwrap();
        t.set_explain(true);
        Arc::clone(&t)
            .send("What is inside an SNES?".into(), Vec::new(), None)
            .unwrap();
        let mut failed = Vec::new();
        let end = until_done(&rx, |e| {
            if let TutorEventInfo::ToolFinished {
                name,
                summary,
                is_error: true,
                ..
            } = e
            {
                failed.push(format!("{name}: {summary}"));
            }
        });
        assert!(matches!(end, TutorEventInfo::Ended { .. }), "{end:?}");
        assert!(failed.is_empty(), "{failed:?}");
        let lessons = t.lessons();
        assert_eq!(lessons.len(), 1);
        assert_eq!(lessons[0].steps[0].picture.as_deref(), Some(id.as_str()));
        // Kept with the lesson, so it reads again after the conversation.
        let store = romlens_tutor::lesson::LessonStore::new(&root);
        let png = store.picture(&lessons[0].id, &id).unwrap();
        assert_eq!(&png[..4], b"\x89PNG");
        assert_eq!(t.lesson_picture(lessons[0].id.clone(), id), Some(png));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn explain_mode_answers_with_a_lesson() {
        let (t, rx, local, root, _wb) = setup(
            "lesson",
            vec![
                call_reply(
                    "begin_lesson",
                    serde_json::json!({"title": "How a sprite reaches the screen", "from_level": 1,
                        "to_level": 2, "concepts": [{"id": "sprites", "level": 2}], "builds_on": []}),
                ),
                call_reply(
                    "lesson_step",
                    serde_json::json!({"lesson": "", "title": "Two chips", "predict": "Which chip draws?",
                        "body": "The CPU decides; the PPU draws.", "focus_address": null, "focus_end": null,
                        "focus_in": null, "focus_frame": null, "focus_view": null, "picture": null}),
                ),
                call_reply(
                    "end_lesson",
                    serde_json::json!({"lesson": "", "next": [{"title": "OAM", "concept": "oam", "level": 2}]}),
                ),
                text_reply("That is the idea; Next takes you through it."),
            ],
        );
        t.new_conversation(
            local.clone(),
            "qwen3".into(),
            None,
            TutorMode::ReadOnly,
            None,
        )
        .unwrap();
        t.set_explain(true);
        assert!(t.explain());
        Arc::clone(&t)
            .send(
                "How does a sprite get rendered to the screen?".into(),
                Vec::new(),
                None,
            )
            .unwrap();
        let mut failed = Vec::new();
        let end = until_done(&rx, |e| {
            if let TutorEventInfo::ToolFinished {
                name,
                summary,
                is_error: true,
                ..
            } = e
            {
                failed.push(format!("{name}: {summary}"));
            }
        });
        assert!(matches!(end, TutorEventInfo::Ended { .. }), "{end:?}");
        assert!(failed.is_empty(), "{failed:?}");
        // The model was told, with what the student had learned.
        let turns = t.transcript();
        assert!(
            matches!(&turns[0].blocks[0], TurnBlockInfo::Text { text } if text.contains("[Explain mode is on: answer with a lesson.]") && text.contains("start at level 1")),
            "{:?}",
            turns[0].blocks
        );
        // The lesson is kept, and the record has it.
        let store = romlens_tutor::lesson::LessonStore::new(&root);
        let lessons = store.list();
        assert_eq!(lessons.len(), 1);
        assert!(lessons[0].finished && lessons[0].steps.len() == 1);
        assert_eq!(lessons[0].next[0].concept, "oam");
        assert_eq!(store.learner().level("sprites"), 2);
        // For the window: the lesson, the record, the mark.
        let begun = turns
            .iter()
            .flat_map(|t| &t.blocks)
            .find_map(|b| match b {
                TurnBlockInfo::ToolResult { text, .. } => lesson_id_from_result(text.clone()),
                _ => None,
            })
            .unwrap();
        let info = t.lesson(begun.clone()).unwrap();
        assert_eq!(info.id, lessons[0].id);
        assert_eq!(info.level_name, "The idea to the hardware");
        assert_eq!(info.steps[0].predict.as_deref(), Some("Which chip draws?"));
        assert!(info.finished && info.this_rom);
        assert_eq!(t.lessons().len(), 1);
        let learner = t.learner();
        let sprites = learner.concepts.iter().find(|c| c.id == "sprites").unwrap();
        assert_eq!(
            (sprites.level, sprites.lesson.as_deref()),
            (2, Some(begun.as_str()))
        );
        assert_eq!(learner.levels[0], "The idea");
        t.mark_known("dma".into(), Some(2)).unwrap();
        assert!(
            t.learner()
                .concepts
                .iter()
                .any(|c| c.id == "dma" && c.marked && c.level == 2)
        );
        assert!(t.mark_known("nope".into(), Some(2)).is_err());
        t.set_lesson_step(Some(begun.clone()), 0);
        let at = t.lock().at_step.clone().unwrap();
        assert!(
            at.starts_with("[The student is at step 1 of 1 of lesson"),
            "{at}"
        );
        // A new conversation starts from it.
        t.new_conversation(local, "qwen3".into(), None, TutorMode::ReadOnly, None)
            .unwrap();
        let st = t.lock();
        let digest = &st.session.as_ref().unwrap().digest;
        assert!(
            digest.contains("## The student's learning so far"),
            "{digest}"
        );
        assert!(digest.contains("- sprites 2"), "{digest}");
        drop(st);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_question_answered_saved_and_resumed() {
        let (t, rx, local, root, wb) = setup(
            "ask",
            vec![
                call_reply(
                    "listing",
                    serde_json::json!({"address": "$00:8000", "lines": 4}),
                ),
                text_reply("RESET at `$00:8000` masks interrupts."),
            ],
        );
        t.new_conversation(
            local.clone(),
            "qwen3".into(),
            None,
            TutorMode::ReadOnly,
            None,
        )
        .unwrap();
        Arc::clone(&t)
            .send(
                "What does RESET do?\n```c\n}\n```".into(),
                Vec::new(),
                Some("$00:8000 SEI".into()),
            )
            .unwrap();
        let mut text = String::new();
        let mut tools = Vec::new();
        let end = until_done(&rx, |e| match e {
            TutorEventInfo::TextDelta { text: d } => text.push_str(d),
            TutorEventInfo::ToolFinished { name, is_error, .. } => {
                tools.push((name.clone(), *is_error))
            }
            _ => {}
        });
        assert_eq!(
            end,
            TutorEventInfo::Ended {
                stop: "EndTurn".into()
            }
        );
        assert_eq!(text, "Let me look.RESET at `$00:8000` masks interrupts.");
        assert_eq!(tools, [("listing".to_owned(), false)]);
        let named = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert!(
            matches!(&named, TutorEventInfo::Named { title, .. } if title == "Test conversation"),
            "{named:?}"
        );
        let turns = t.transcript();
        assert_eq!(turns.len(), 4);
        assert!(
            matches!(&turns[0].blocks[0], TurnBlockInfo::Text { text } if text.starts_with("[The mode is now: read-only.]"))
        );
        assert!(
            matches!(&turns[2].blocks[0], TurnBlockInfo::ToolResult { text, .. } if text.contains("SEI"))
        );

        // Saved: another session for the same ROM lists and resumes it.
        let (tx, _rx2) = mpsc::channel();
        let again = TutorSession::new(
            wb,
            root.to_string_lossy().into_owned(),
            Arc::new(NoKeys),
            Arc::new(Heard(Mutex::new(tx))),
        );
        let list = again.conversations();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "Test conversation");
        assert_eq!(again.resume(list[0].id.clone()).unwrap().len(), 4);
        assert_eq!(
            again.conversations()[0].title,
            "Test conversation",
            "kept on resume"
        );
        assert_eq!(
            again.prompt_history(10),
            ["What does RESET do?\n```c\n}\n```"]
        );
        let points = again.rewind_points();
        assert_eq!(points.len(), 1);
        let r = again.rewind(points[0].index, RewindWhat::Both).unwrap();
        assert_eq!(
            r.prompt.as_deref(),
            Some("What does RESET do?\n```c\n}\n```")
        );
        assert!(again.transcript().iter().take(4).all(|t| !t.sent));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_answer_before_the_wait_is_kept() {
        let cards = Cards {
            pending: Mutex::new(Pending::default()),
            changed: Condvar::new(),
            cancel: Cancel::new(),
        };
        cards
            .pending
            .lock()
            .unwrap()
            .answers
            .insert("call_1".into(), Decision::Accept);
        let p = Proposal {
            id: "call_1".into(),
            tool: "set_label".into(),
            summary: String::new(),
            reason: String::new(),
            before: None,
            after: None,
        };
        assert_eq!(cards.decide(&p), Decision::Accept);
        cards.cancel.cancel();
        assert!(
            matches!(cards.decide(&p), Decision::Reject { .. }),
            "Esc says no"
        );
    }

    /// A card never comes before the explanation: an edit called before
    /// the tutor has written anything is held, shown after the answer, and
    /// the student's decision goes with the next question.
    #[test]
    fn an_edit_waits_for_the_explanation() {
        let bare = test_chunk(
            serde_json::json!({"tool_calls": [{"index": 0, "id": "call_1", "function": {"name": "set_label", "arguments": r#"{"address":"$00:8000","name":"Reset","reason":"the vector"}"#}}]}),
            Some("tool_calls"),
        ) + "data: [DONE]\n\n";
        let (t, rx, local, root, wb) = setup(
            "explain",
            vec![
                bare,
                text_reply("RESET starts the game."),
                text_reply("Good."),
            ],
        );
        t.new_conversation(local, "qwen3".into(), None, TutorMode::AskBeforeEdits, None)
            .unwrap();
        Arc::clone(&t)
            .send("What is RESET?".into(), Vec::new(), None)
            .unwrap();
        let mut order = Vec::new();
        until_done(&rx, |e| match e {
            TutorEventInfo::TextDelta { .. } => order.push("text"),
            TutorEventInfo::ToolFinished { is_error, .. } => {
                assert!(!is_error);
                order.push("held")
            }
            TutorEventInfo::EditProposed { proposal } => {
                order.push("card");
                t.answer(proposal.id.clone(), true, None);
            }
            _ => {}
        });
        assert_eq!(order, ["held", "text", "card"]);
        assert_eq!(wb.label_at(0x8000).unwrap().name, "Reset");
        Arc::clone(&t)
            .send("Thanks".into(), Vec::new(), None)
            .unwrap();
        until_done(&rx, |_| {});
        let told = t.transcript().iter().any(|turn| {
            turn.blocks.iter().any(|b| {
                matches!(b, TurnBlockInfo::Text { text } if text.contains("[The changes you proposed last time: The student accepted: Name $00:8000 `Reset`."))
            })
        });
        assert!(told, "the decision goes with the next question");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_edit_waits_for_the_student() {
        let (t, rx, local, root, wb) = setup(
            "edit",
            vec![
                call_reply(
                    "set_label",
                    serde_json::json!({"address": "$00:8000", "name": "Reset", "reason": "the vector"}),
                ),
                text_reply("Named it."),
            ],
        );
        t.new_conversation(local, "qwen3".into(), None, TutorMode::AskBeforeEdits, None)
            .unwrap();
        Arc::clone(&t)
            .send("Name RESET".into(), Vec::new(), None)
            .unwrap();
        let end = until_done(&rx, |e| {
            if let TutorEventInfo::EditProposed { proposal } = e {
                assert_eq!(proposal.summary, "Name $00:8000 `Reset`");
                // Saved while the card waits: the question and the call.
                assert_eq!(t.conversations()[0].turns, 2);
                t.answer(proposal.id.clone(), true, None);
            }
        });
        assert_eq!(
            end,
            TutorEventInfo::Ended {
                stop: "EndTurn".into()
            }
        );
        assert_eq!(wb.label_at(0x8000).unwrap().name, "Reset");
        // /rewind of the edits takes it back.
        let r = t.rewind(0, RewindWhat::Edits).unwrap();
        assert_eq!(r.edits.unwrap().undone, 1);
        assert_eq!(wb.label_at(0x8000).unwrap().name, "RESET_008000");
        let _ = std::fs::remove_dir_all(root);
    }
}
