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
    busy: bool,
    /// A name that came while a turn had the session: the conversation,
    /// the name, and what it cost.
    named: Option<(String, String, f64)>,
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
            if let Some(n) = namer {
                me.name(&n, &keys);
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
        tools
            .lessons
            .set_store(Some(romlens_tutor::lesson::LessonStore::new(
                &PathBuf::from(&root),
            )));
        Arc::new(TutorSession {
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
                busy: false,
                named: None,
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
    // A line to the student first, as an edit needs.
    test_chunk(serde_json::json!({"content": "Let me look."}), None)
        + &test_chunk(
            serde_json::json!({"tool_calls": [{"index": 0, "id": "call_9", "function": {"name": name, "arguments": arguments}}]}),
            Some("tool_calls"),
        )
        + "data: [DONE]\n\n"
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
