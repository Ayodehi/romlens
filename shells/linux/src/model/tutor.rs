//! The Tutor window's model (docs/24): one per project. It holds the core's
//! `TutorSession`, the turn streaming in, the edit cards waiting, the
//! composer's history, and the slash commands. The macOS twin is
//! `TutorModel`.
//!
//! The model is GTK-free. The document hands it what it cannot know (the
//! selection as text, the recording) and delivers the session's events to it
//! on the main loop.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use romlens_ffi::RecordingSession;
use romlens_ffi::tutor::quiz::{
    AnswerResultInfo, GivenInfo, ProgressInfo, QuizInfo, QuizPurposeInfo,
};
use romlens_ffi::tutor::session::{
    AttachmentInfo, ConversationSummaryInfo, CredentialStore, LearnerInfo, LessonInfo,
    LessonOfferInfo, ProposalInfo, TurnInfo, TutorEventInfo, TutorListener, TutorSession,
    lesson_id_from_result,
};

use super::tutor_settings::{ModePreference, TutorSettings};

/// A tool call in the log, with its result once it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRow {
    pub id: String,
    pub name: String,
    pub input: String,
    pub summary: Option<String>,
    /// Pictures its result holds.
    pub images: Vec<String>,
    pub is_error: bool,
    pub done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardState {
    Waiting,
    Accepted,
    Declined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub proposal: ProposalInfo,
    pub state: CardState,
}

/// The turn streaming in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Live {
    /// The question, shown until the transcript has it.
    pub question: String,
    pub pictures: Vec<Vec<u8>>,
    pub text: String,
    pub reasoning: String,
    pub tools: Vec<ToolRow>,
    pub cards: Vec<Card>,
    pub status: Option<String>,
    /// The lesson being written in this turn (docs/25).
    pub lesson: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub data: Vec<u8>,
    pub media_type: String,
    pub name: String,
}

/// A window the Tutor asks the shell to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sheet {
    Resume,
    Rewind,
    Model,
    Help,
    Lessons,
    Map,
    Quiz,
    Progress,
}

/// What was just earned, shown for a few seconds at the top of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Banner {
    pub points: u32,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct Command {
    pub name: &'static str,
    pub about: &'static str,
}

pub const COMMANDS: [Command; 19] = [
    Command {
        name: "/new",
        about: "Start a new conversation",
    },
    Command {
        name: "/clear",
        about: "Start a new conversation",
    },
    Command {
        name: "/resume",
        about: "Go back to an earlier conversation",
    },
    Command {
        name: "/rewind",
        about: "Go back to an earlier question, and take back the tutor's edits",
    },
    Command {
        name: "/model",
        about: "Change the provider, model or effort",
    },
    Command {
        name: "/mode",
        about: "read-only, ask or accept: what the tutor may change",
    },
    Command {
        name: "/compact",
        about: "Summarise the conversation to make room",
    },
    Command {
        name: "/cost",
        about: "What this conversation has cost",
    },
    Command {
        name: "/attach",
        about: "/attach frame: the recording's frame as a picture",
    },
    Command {
        name: "/selection",
        about: "Send the main window's selection with questions, or not",
    },
    Command {
        name: "/details",
        about: "Show or hide the tutor's thinking and tool calls",
    },
    Command {
        name: "/learn",
        about: "/learn <topic>: a lesson about it, as deep as you have got",
    },
    Command {
        name: "/explain",
        about: "Answer with lessons, or not (Explain mode)",
    },
    Command {
        name: "/lessons",
        about: "Your lessons, to read again",
    },
    Command {
        name: "/quiz",
        about: "/quiz <topic>: prove what you know, with questions Romlens checks",
    },
    Command {
        name: "/review",
        about: "A quiz on what is due for review",
    },
    Command {
        name: "/progress",
        about: "Your points, rank, achievements and reviews due",
    },
    Command {
        name: "/map",
        about: "What you have learned, concept by concept",
    },
    Command {
        name: "/help",
        about: "What the tutor can do and the keys it takes",
    },
];

/// What running a command asks of the composer.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CommandResult {
    /// Put this in the composer.
    pub composer: Option<String>,
    /// And send it.
    pub send: bool,
}

/// What an event asks of the project beyond the model.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// An edit the tutor made was applied: the listing and the analysis
    /// follow.
    pub edited: bool,
}

/// The longest side a provider takes without shrinking it itself (Claude's
/// high-resolution limit), and the most bytes sent.
pub const MAX_SIDE: u32 = 2576;
pub const MAX_BYTES: usize = 3_500_000;

thread_local! {
    /// Every open Tutor's session, so a change in Settings reaches them all.
    static OPEN: RefCell<Vec<std::sync::Weak<TutorSession>>> = const { RefCell::new(Vec::new()) };
}

/// Settings › Tutor › "Check each lesson in the background", for the Tutors
/// already open.
pub fn check_lessons_changed(on: bool) {
    OPEN.with(|o| {
        let mut o = o.borrow_mut();
        o.retain(|s| s.strong_count() > 0);
        for s in o.iter().filter_map(std::sync::Weak::upgrade) {
            s.set_check_lessons(on);
        }
    });
}

pub struct TutorModel {
    pub settings: Rc<RefCell<TutorSettings>>,
    root: std::path::PathBuf,
    session: Option<Arc<TutorSession>>,

    pub turns: Vec<TurnInfo>,
    pub live: Option<Live>,
    pub busy: bool,
    pub error: Option<String>,
    pub attachments: Vec<Attachment>,
    /// Send the main window's selection with the question.
    pub include_selection: bool,
    pub sheet: Option<Sheet>,
    pub cost: f64,
    /// The conversation's name: the model's, after the first answer.
    pub title: Option<String>,
    pub context_used: f64,
    pub model_name: Option<String>,
    pub endpoint_name: Option<String>,
    pub mode: ModePreference,
    /// Cards accepted without asking for the rest of this turn.
    accept_rest: bool,
    /// Up and down walk this; `None` when not walking.
    history: Vec<String>,
    history_index: Option<usize>,
    draft: String,
    /// A line shown under the transcript by /cost.
    pub cost_note: Option<String>,

    // Lessons (docs/25)
    /// Explain mode: questions answered with lessons.
    pub explain: bool,
    /// The step each lesson's card is at, and the predict questions shown.
    pub lesson_steps: HashMap<String, usize>,
    revealed: HashSet<String>,
    lesson_cache: HashMap<String, LessonInfo>,

    // Quizzes (docs/28)
    pub quiz: Option<QuizInfo>,
    pub quiz_error: Option<String>,
    pub progress: Option<ProgressInfo>,
    pub banner: Option<Banner>,
}

pub fn message(e: &dyn std::fmt::Display) -> String {
    e.to_string()
}

impl TutorModel {
    pub fn new(settings: Rc<RefCell<TutorSettings>>, root: std::path::PathBuf) -> Self {
        let mode = settings.borrow().stored().mode;
        Self {
            settings,
            root,
            session: None,
            turns: Vec::new(),
            live: None,
            busy: false,
            error: None,
            attachments: Vec::new(),
            include_selection: true,
            sheet: None,
            cost: 0.0,
            title: None,
            context_used: 0.0,
            model_name: None,
            endpoint_name: None,
            mode,
            accept_rest: false,
            history: Vec::new(),
            history_index: None,
            draft: String::new(),
            cost_note: None,
            explain: false,
            lesson_steps: HashMap::new(),
            revealed: HashSet::new(),
            lesson_cache: HashMap::new(),
            quiz: None,
            quiz_error: None,
            progress: None,
            banner: None,
        }
    }

    /// Conversations live in the app's own folder, never in a project
    /// (`12-content-policy.md` rule 8): `$XDG_DATA_HOME/romlens/Tutor`.
    pub fn default_root() -> std::path::PathBuf {
        #[cfg(test)]
        {
            static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            std::env::temp_dir().join(format!("romlens-tutor-test-{}-{n}", std::process::id()))
        }
        #[cfg(not(test))]
        crate::config::data_dir().join("Tutor")
    }

    // MARK: The session

    pub fn session(&self) -> Option<&Arc<TutorSession>> {
        self.session.as_ref()
    }

    /// Made on first use, so a project that never opens the tutor costs
    /// nothing.
    pub fn ensure_session(
        &mut self,
        workbench: Arc<romlens_ffi::Workbench>,
        credentials: Arc<dyn CredentialStore>,
        listener: Arc<dyn TutorListener>,
        utc_offset: i32,
    ) -> Arc<TutorSession> {
        if let Some(s) = &self.session {
            return Arc::clone(s);
        }
        let _ = std::fs::create_dir_all(&self.root);
        let s = TutorSession::new(
            workbench,
            self.root.to_string_lossy().into_owned(),
            credentials,
            listener,
        );
        s.set_check_lessons(self.settings.borrow().stored().check_lessons);
        // Streaks count the student's own days.
        s.set_utc_offset(utc_offset);
        self.history = s.prompt_history(500);
        OPEN.with(|o| o.borrow_mut().push(Arc::downgrade(&s)));
        self.session = Some(Arc::clone(&s));
        s
    }

    /// Starts a conversation on the default provider. Returns false, with
    /// the reason in `error`, if it cannot.
    pub fn new_conversation(&mut self) -> bool {
        let Some(s) = self.session.clone() else {
            return false;
        };
        let (e, model, effort, cap) = {
            let st = self.settings.borrow();
            let e = st.default_endpoint();
            (e.clone(), st.model(&e), st.effort(&e), st.stored().cost_cap)
        };
        let Some(model) = model.filter(|m| !m.is_empty()) else {
            self.error = Some(format!("Choose a model for {} in Settings.", e.name));
            return false;
        };
        match s.new_conversation(e.info(), model, effort, self.mode.mode(), cap) {
            Ok(_) => {
                // Explain mode carries over to the next conversation.
                s.set_explain(self.explain);
                self.refresh();
                true
            }
            Err(e) => {
                self.error = Some(message(&e));
                false
            }
        }
    }

    /// Starts a conversation unless one is open.
    pub fn start_if_needed(&mut self) -> bool {
        match &self.session {
            Some(s) if s.conversation_id().is_some() => true,
            Some(_) => self.new_conversation(),
            None => false,
        }
    }

    pub fn resume(&mut self, id: &str) {
        let Some(s) = self.session.clone() else {
            return;
        };
        match s.resume(id.to_owned()) {
            Ok(_) => self.refresh(),
            Err(e) => self.error = Some(message(&e)),
        }
    }

    /// Back to before a question: the conversation, the tutor's edits, or
    /// both. The question's words come back for the composer.
    pub fn rewind(
        &mut self,
        index: u32,
        what: romlens_ffi::tutor::session::RewindWhat,
    ) -> Result<romlens_ffi::tutor::session::RewindResultInfo, String> {
        let s = self.session.clone().ok_or("No conversation to rewind.")?;
        let r = s.rewind(index, what).map_err(|e| message(&e))?;
        self.refresh();
        Ok(r)
    }

    /// The provider, model and effort from the next question on: for the open
    /// conversation, or the default for the next one.
    pub fn use_model(
        &mut self,
        e: &super::tutor_settings::Endpoint,
        model: &str,
        effort: Option<String>,
    ) {
        {
            let mut st = self.settings.borrow_mut();
            st.edit(|s| {
                s.models.insert(e.id.clone(), model.to_owned());
                match &effort {
                    Some(f) => s.efforts.insert(e.id.clone(), f.clone()),
                    None => s.efforts.remove(&e.id),
                };
            });
        }
        match &self.session {
            Some(s) if s.conversation_id().is_some() => {
                if let Err(err) = s.set_model(e.info(), model.to_owned(), effort) {
                    self.error = Some(message(&err));
                }
            }
            _ => {
                let id = e.id.clone();
                self.settings.borrow_mut().edit(|s| s.endpoint = id);
            }
        }
        self.refresh();
    }

    pub fn delete_conversation(&mut self, id: &str) {
        if let Some(s) = &self.session
            && let Err(e) = s.delete_conversation(id.to_owned())
        {
            self.error = Some(message(&e));
        }
    }

    pub fn conversations(&self) -> Vec<ConversationSummaryInfo> {
        self.session
            .as_ref()
            .map_or_else(Vec::new, |s| s.conversations())
    }

    pub fn refresh(&mut self) {
        let Some(s) = self.session.clone() else {
            return;
        };
        self.turns = s.transcript();
        self.explain = s.explain();
        self.lesson_cache.clear();
        self.title = s.title();
        self.cost = s.cost();
        self.context_used = s.context_used();
        self.model_name = s.model();
        self.endpoint_name = s.endpoint().map(|e| {
            self.settings
                .borrow()
                .endpoint(&e.id)
                .map_or(e.id, |x| x.name)
        });
        self.mode = ModePreference::from_mode(s.mode());
    }

    // MARK: Asking

    /// Whether the composer's `text` could be sent now.
    pub fn can_send(&self, text: &str) -> bool {
        !self.busy && !(text.trim().is_empty() && self.attachments.is_empty())
    }

    /// Ask `text`. `selection` is what the main window has selected, as text;
    /// `recording` the one it has open.
    pub fn send(
        &mut self,
        text: &str,
        selection: Option<String>,
        recording: Option<Arc<RecordingSession>>,
    ) -> bool {
        if !self.can_send(text) {
            return false;
        }
        let text = text.trim().to_owned();
        let (ready, name, image) = {
            let st = self.settings.borrow();
            let id = self
                .session
                .as_ref()
                .and_then(|s| s.endpoint())
                .map_or_else(|| st.default_endpoint().id, |e| e.id);
            let e = st.endpoint(&id).unwrap_or_else(|| st.default_endpoint());
            (
                st.ready(&e),
                e.name,
                (
                    st.image_endpoint().map(|e| e.info()),
                    st.stored().image_model.clone(),
                ),
            )
        };
        if !ready {
            self.error = Some(format!("Add a key for {name} in Settings (Ctrl+,)."));
            return false;
        }
        if !self.start_if_needed() {
            return false;
        }
        let Some(s) = self.session.clone() else {
            return false;
        };
        s.set_recording(recording);
        s.set_image_provider(image.0, image.1);
        let files: Vec<AttachmentInfo> = self
            .attachments
            .iter()
            .map(|a| AttachmentInfo {
                bytes: a.data.clone(),
                media_type: a.media_type.clone(),
            })
            .collect();
        let sel = self.include_selection.then_some(selection).flatten();
        if let Err(e) = Arc::clone(&s).send(text.clone(), files.clone(), sel) {
            self.error = Some(message(&e));
            return false;
        }
        if !text.is_empty() && self.history.last() != Some(&text) {
            self.history.push(text.clone());
        }
        self.history_index = None;
        self.attachments.clear();
        self.error = None;
        self.accept_rest = false;
        self.busy = true;
        self.live = Some(Live {
            question: text,
            pictures: files.into_iter().map(|f| f.bytes).collect(),
            ..Live::default()
        });
        self.turns = s.transcript();
        true
    }

    /// Esc: stops the turn, and a waiting card says no.
    pub fn stop(&self) {
        if let Some(s) = &self.session {
            s.cancel();
        }
    }

    /// The project is closing: every card still waiting says no, and the
    /// turn stops, so nothing goes on spending for a window that is gone.
    pub fn close(&mut self) {
        let waiting: Vec<String> = self
            .live
            .iter()
            .flat_map(|l| &l.cards)
            .filter(|c| c.state == CardState::Waiting)
            .map(|c| c.proposal.id.clone())
            .collect();
        for id in waiting {
            self.answer_card(&id, false, None, false);
        }
        self.stop();
    }

    pub fn answer_card(&mut self, id: &str, accept: bool, why: Option<String>, and_the_rest: bool) {
        self.accept_rest = accept && and_the_rest;
        if let Some(s) = &self.session {
            s.answer(id.to_owned(), accept, why);
        }
        self.set_card(
            id,
            if accept {
                CardState::Accepted
            } else {
                CardState::Declined
            },
        );
    }

    fn set_card(&mut self, id: &str, state: CardState) {
        if let Some(c) = self
            .live
            .as_mut()
            .and_then(|l| l.cards.iter_mut().find(|c| c.proposal.id == id))
        {
            c.state = state;
        }
    }

    // MARK: Events

    /// Fold an event of the session into the model. Returns what the project
    /// should do about it.
    pub fn handle(&mut self, e: TutorEventInfo) -> Effects {
        let mut effects = Effects::default();
        match e {
            TutorEventInfo::TextDelta { text } => {
                if let Some(l) = &mut self.live {
                    l.text += &text;
                }
            }
            TutorEventInfo::ReasoningDelta { text } => {
                if let Some(l) = &mut self.live {
                    l.reasoning += &text;
                }
            }
            TutorEventInfo::ToolCallStarted { id, name } => {
                if let Some(l) = &mut self.live {
                    l.status = Some(format!("Asking for {}…", tool_title(&name)));
                    if !l.tools.iter().any(|t| t.id == id) {
                        l.tools.push(tool_row(id, name, String::new()));
                    }
                }
            }
            TutorEventInfo::ToolArguments { id, text } => {
                if let Some(t) = self.tool(&id) {
                    t.input += &text;
                }
            }
            TutorEventInfo::Requesting { round } => {
                if let Some(l) = &mut self.live {
                    l.status = Some(
                        if round == 0 {
                            "Thinking…"
                        } else {
                            "Reading the results…"
                        }
                        .to_owned(),
                    );
                }
            }
            TutorEventInfo::Retrying {
                attempt,
                wait_ms,
                why,
            } => {
                if let Some(l) = &mut self.live {
                    l.status = Some(format!(
                        "Trying again in {} s ({attempt}): {why}",
                        wait_ms / 1000
                    ));
                }
            }
            TutorEventInfo::ToolStarted { id, name, input } => {
                if let Some(l) = &mut self.live {
                    match l.tools.iter_mut().find(|t| t.id == id) {
                        Some(t) => t.input = input,
                        None => l.tools.push(tool_row(id, name.clone(), input)),
                    }
                    l.status = Some(format!("{}…", tool_title(&name)));
                }
            }
            TutorEventInfo::ToolFinished {
                id,
                name,
                summary,
                is_error,
            } => {
                if let Some(t) = self.tool(&id) {
                    t.summary = Some(summary.clone());
                    t.is_error = is_error;
                    t.done = true;
                }
                // A lesson being written shows as it grows.
                if !is_error
                    && name == "begin_lesson"
                    && let Some(l) = lesson_id_from_result(summary)
                    && let Some(live) = &mut self.live
                {
                    live.lesson = Some(l);
                }
                if let Some(l) = self.live.as_ref().and_then(|l| l.lesson.clone())
                    && ["begin_lesson", "lesson_step", "end_lesson"].contains(&name.as_str())
                {
                    self.reload_lesson(&l);
                }
            }
            TutorEventInfo::EditProposed { proposal } => {
                let id = proposal.id.clone();
                if let Some(l) = &mut self.live {
                    l.cards.push(Card {
                        proposal,
                        state: CardState::Waiting,
                    });
                }
                if self.accept_rest {
                    self.answer_card(&id, true, None, true);
                }
            }
            TutorEventInfo::EditDecided { id, applied } => {
                self.set_card(
                    &id,
                    if applied {
                        CardState::Accepted
                    } else {
                        CardState::Declined
                    },
                );
                effects.edited = applied;
            }
            TutorEventInfo::Cost { total, .. } => self.cost = total,
            TutorEventInfo::Discarded => {
                if let Some(l) = &mut self.live {
                    l.text.clear();
                    l.reasoning.clear();
                }
            }
            TutorEventInfo::Compacted { .. } => {
                if let Some(l) = &mut self.live {
                    l.status = Some("Summarised to make room.".to_owned());
                }
            }
            TutorEventInfo::Named { title, total } => {
                self.title = Some(title);
                if !self.busy {
                    self.cost = total;
                }
            }
            TutorEventInfo::LessonChecked { lesson, total, .. } => {
                self.reload_lesson(&lesson);
                if !self.busy {
                    self.cost = total;
                }
            }
            TutorEventInfo::Ended { .. } => {
                self.finish(None);
                effects.edited = true;
                // A name the tutor gave, or a lesson ended, may be a milestone.
                if let Some(s) = &self.session {
                    let _ = s.check_milestones();
                }
                self.refresh_progress();
                // A lesson this turn finished is being checked now.
                let ids: Vec<String> = self.lesson_cache.keys().cloned().collect();
                for id in ids {
                    self.reload_lesson(&id);
                }
            }
            TutorEventInfo::Failed { message } => {
                self.finish(Some(message));
                effects.edited = true;
            }
            TutorEventInfo::QuizChanged { quiz } => {
                if self.quiz.as_ref().is_some_and(|q| q.id == quiz) {
                    self.reload_quiz();
                }
            }
            TutorEventInfo::GuessMarked { lesson, .. } => self.reload_lesson(&lesson),
            TutorEventInfo::Progress {
                gained,
                proven,
                unlocked,
                rank,
            } => {
                self.refresh_progress();
                self.earned(gained, proven, unlocked, rank);
            }
        }
        effects
    }

    fn tool(&mut self, id: &str) -> Option<&mut ToolRow> {
        self.live.as_mut()?.tools.iter_mut().find(|t| t.id == id)
    }

    fn finish(&mut self, error: Option<String>) {
        self.busy = false;
        self.live = None;
        self.error = error;
        self.refresh();
    }

    // MARK: The composer

    /// Up at the first line: the prompt before. `current` is what is in the
    /// composer now; returns what to put there.
    pub fn history_up(&mut self, current: &str) -> Option<String> {
        if self.history.is_empty() {
            return None;
        }
        let i = self
            .history_index
            .unwrap_or(self.history.len())
            .checked_sub(1)?;
        if self.history_index.is_none() {
            self.draft = current.to_owned();
        }
        self.history_index = Some(i);
        Some(self.history[i].clone())
    }

    /// Down at the last line: the one after, then back to the draft.
    pub fn history_down(&mut self) -> Option<String> {
        let i = self.history_index?;
        if i + 1 < self.history.len() {
            self.history_index = Some(i + 1);
            Some(self.history[i + 1].clone())
        } else {
            self.history_index = None;
            Some(std::mem::take(&mut self.draft))
        }
    }

    pub fn walking_history(&self) -> bool {
        self.history_index.is_some()
    }

    /// Shift+Tab.
    pub fn cycle_mode(&mut self) {
        self.set_mode(self.mode.next());
    }

    pub fn set_mode(&mut self, m: ModePreference) {
        self.mode = m;
        if let Some(s) = &self.session {
            s.set_mode(m.mode());
        }
    }

    pub fn attach(&mut self, data: Vec<u8>, media_type: &str, name: &str) {
        self.attachments.push(Attachment {
            data,
            media_type: media_type.to_owned(),
            name: name.to_owned(),
        });
    }

    // MARK: Slash commands

    /// The commands the composer's text starts.
    pub fn matching_commands(text: &str) -> Vec<Command> {
        let t = text.to_lowercase();
        if !t.starts_with('/') || t.contains(' ') || t.contains('\n') {
            return Vec::new();
        }
        COMMANDS
            .iter()
            .copied()
            // /clear is /new's other name: offered only once asked for.
            .filter(|c| c.name.starts_with(&t) && (c.name != "/clear" || t.len() > 2))
            .collect()
    }

    /// Run a command line. Commands that need the project say so in their
    /// result; the rest change the model.
    pub fn run_command(&mut self, line: &str) -> CommandResult {
        let (cmd, arg) = match line.split_once(' ') {
            Some((c, a)) => (c.to_lowercase(), a.trim().to_owned()),
            None => (line.to_lowercase(), String::new()),
        };
        self.error = None;
        let mut result = CommandResult::default();
        match cmd.as_str() {
            "/new" | "/clear" => {
                if self.busy {
                    self.error = Some("Wait for the answer, or press Esc.".into());
                } else {
                    self.new_conversation();
                }
            }
            "/resume" => self.sheet = Some(Sheet::Resume),
            "/rewind" => self.sheet = Some(Sheet::Rewind),
            "/model" => self.sheet = Some(Sheet::Model),
            "/mode" => match arg.to_lowercase().as_str() {
                "read-only" | "readonly" | "read" => self.set_mode(ModePreference::ReadOnly),
                "ask" => self.set_mode(ModePreference::AskBeforeEdits),
                "accept" | "edit" | "edits" => self.set_mode(ModePreference::AcceptEdits),
                "" => self.cycle_mode(),
                _ => self.error = Some("The modes are read-only, ask and accept.".into()),
            },
            "/compact" => {
                if self.start_if_needed()
                    && !self.busy
                    && let Some(s) = self.session.clone()
                {
                    match s.compact() {
                        Ok(()) => {
                            self.busy = true;
                            self.live = Some(Live {
                                status: Some("Summarising…".into()),
                                ..Live::default()
                            });
                        }
                        Err(e) => self.error = Some(message(&e)),
                    }
                }
            }
            "/cost" => {
                self.live = None;
                self.cost_note = Some(format!("This conversation has cost ${:.4}.", self.cost));
            }
            "/selection" => self.include_selection = !self.include_selection,
            "/details" => {
                self.settings
                    .borrow_mut()
                    .edit(|s| s.show_work = !s.show_work);
            }
            "/learn" => {
                if self.busy {
                    self.error = Some("Wait for the answer, or press Esc.".into());
                } else if self.start_if_needed() {
                    self.set_explain(true);
                    if !arg.is_empty() {
                        result = CommandResult {
                            composer: Some(arg),
                            send: true,
                        };
                    }
                }
            }
            "/lessons" => self.sheet = Some(Sheet::Lessons),
            "/quiz" => {
                if arg.is_empty() {
                    self.error = Some(
                        "Name what to be quizzed on: /quiz sprites, /quiz the NMI. The map lists the concepts."
                            .into(),
                    );
                } else {
                    self.start_quiz(Some(arg), None, QuizPurposeInfo::Prove);
                }
            }
            "/review" => self.start_quiz(None, None, QuizPurposeInfo::Review),
            "/progress" => {
                if let Some(s) = &self.session {
                    let _ = s.check_milestones();
                }
                self.refresh_progress();
                self.sheet = Some(Sheet::Progress);
            }
            "/map" => self.sheet = Some(Sheet::Map),
            "/explain" => {
                if self.start_if_needed() {
                    self.set_explain(!self.explain);
                }
            }
            "/help" => self.sheet = Some(Sheet::Help),
            // Handled by the caller: they need the project.
            "/attach" => {
                if !arg.to_lowercase().starts_with("frame") {
                    self.error = Some("Try /attach frame, or paste or drop a picture.".into());
                }
            }
            _ => self.error = Some(format!("{cmd} is not a command. /help lists them.")),
        }
        result
    }

    // MARK: Lessons (docs/25)

    pub fn set_explain(&mut self, on: bool) {
        self.explain = on;
        if let Some(s) = &self.session {
            s.set_explain(on);
        }
    }

    pub fn lesson(&mut self, id: &str) -> Option<LessonInfo> {
        if let Some(l) = self.lesson_cache.get(id) {
            return Some(l.clone());
        }
        let l = self.session.as_ref()?.lesson(id.to_owned())?;
        self.lesson_cache.insert(id.to_owned(), l.clone());
        Some(l)
    }

    pub fn reload_lesson(&mut self, id: &str) {
        match self.session.as_ref().and_then(|s| s.lesson(id.to_owned())) {
            Some(l) => {
                self.lesson_cache.insert(id.to_owned(), l);
            }
            None => {
                self.lesson_cache.remove(id);
            }
        }
    }

    pub fn step_of(&self, lesson: &str) -> usize {
        self.lesson_steps.get(lesson).copied().unwrap_or(0)
    }

    /// Moves a lesson's card to step `i` and tells the next question where
    /// the student is. Returns the step's focus link, for the caller to point
    /// the main window at.
    pub fn show_step(&mut self, lesson: &LessonInfo, i: usize) -> Option<String> {
        let step = lesson.steps.get(i)?;
        self.lesson_steps.insert(lesson.id.clone(), i);
        if let Some(s) = &self.session {
            s.set_lesson_step(Some(lesson.id.clone()), i as u32);
        }
        step.focus.clone()
    }

    /// A guess at a step's predict question: kept, checked where it can be,
    /// and the step shown.
    pub fn answer_predict(&mut self, lesson: &str, step: usize, guess: &str) {
        if let Some(s) = self.session.clone()
            && let Err(e) = s.answer_predict(lesson.to_owned(), step as u32, guess.to_owned())
        {
            self.error = Some(message(&e));
        }
        self.reload_lesson(lesson);
        self.reveal(lesson, step);
    }

    pub fn is_revealed(&self, lesson: &str, step: usize) -> bool {
        self.revealed.contains(&format!("{lesson}#{step}"))
    }

    pub fn reveal(&mut self, lesson: &str, step: usize) {
        self.revealed.insert(format!("{lesson}#{step}"));
    }

    /// Every lesson, the latest first.
    pub fn lessons(&self) -> Vec<LessonInfo> {
        self.session.as_ref().map_or_else(Vec::new, |s| s.lessons())
    }

    /// What the student knows, for the map.
    pub fn learner(&self) -> Option<LearnerInfo> {
        self.session.as_ref().map(|s| s.learner())
    }

    /// Marks a concept known at a level (1 to 5), or clears it.
    pub fn mark_known(&mut self, concept: &str, level: Option<u8>) {
        if let Some(s) = &self.session
            && let Err(e) = s.mark_known(concept.to_owned(), level)
        {
            self.error = Some(message(&e));
        }
    }

    pub fn delete_lesson(&mut self, id: &str) {
        if let Some(s) = &self.session {
            match s.delete_lesson(id.to_owned()) {
                Ok(()) => {
                    self.lesson_cache.remove(id);
                }
                Err(e) => self.error = Some(message(&e)),
            }
        }
    }

    /// "Go deeper": the next lesson an offer names; the text to send.
    pub fn take_offer(&mut self, offer: &LessonOfferInfo) -> Option<String> {
        if self.busy {
            return None;
        }
        self.set_explain(true);
        Some(format!("Go deeper: {}", offer.title))
    }

    // MARK: Quizzes (docs/28)

    /// Starts a quiz: to prove a concept at a level (the next one to prove
    /// when none is given), to practise, or to review what is due. With a
    /// conversation open, the tutor may add questions about the game.
    pub fn start_quiz(
        &mut self,
        concept: Option<String>,
        level: Option<u8>,
        purpose: QuizPurposeInfo,
    ) {
        let Some(s) = self.session.clone() else {
            return;
        };
        let tutor =
            self.settings.borrow().stored().tutor_quiz_questions && s.conversation_id().is_some();
        match s.start_quiz(concept, level, purpose, tutor) {
            Ok(q) => {
                self.quiz = Some(q);
                self.quiz_error = None;
                self.sheet = Some(Sheet::Quiz);
            }
            Err(e) => self.error = Some(message(&e)),
        }
    }

    pub fn reload_quiz(&mut self) {
        let Some(id) = self.quiz.as_ref().map(|q| q.id.clone()) else {
            return;
        };
        self.quiz = self.session.as_ref().and_then(|s| s.quiz(id));
    }

    /// Answers a question; the result is in the quiz's results.
    pub fn answer_question(&mut self, question: &str, given: GivenInfo) {
        let (Some(q), Some(s)) = (
            self.quiz.as_ref().map(|q| q.id.clone()),
            self.session.clone(),
        ) else {
            return;
        };
        self.quiz_error = s
            .answer_question(q, question.to_owned(), given)
            .err()
            .map(|e| message(&e));
        self.reload_quiz();
    }

    pub fn question_hint(&mut self, question: &str) -> Option<String> {
        let (q, s) = (self.quiz.as_ref()?.id.clone(), self.session.clone()?);
        let hint = s.question_hint(q, question.to_owned()).ok();
        self.reload_quiz();
        hint
    }

    pub fn finish_quiz(&mut self) {
        let (Some(q), Some(s)) = (
            self.quiz.as_ref().map(|q| q.id.clone()),
            self.session.clone(),
        ) else {
            return;
        };
        if let Ok(done) = s.finish_quiz(q) {
            self.quiz = Some(done);
        }
        self.refresh_progress();
    }

    pub fn result(&self, question: &str) -> Option<&AnswerResultInfo> {
        self.quiz
            .as_ref()?
            .results
            .iter()
            .find(|r| r.question == question)
    }

    // MARK: Progress (docs/28)

    pub fn refresh_progress(&mut self) {
        if let Some(s) = &self.session {
            self.progress = s.progress();
        }
    }

    /// Reviews due now.
    pub fn due_count(&self) -> usize {
        self.progress.as_ref().map_or(0, |p| p.due.len())
    }

    fn earned(
        &mut self,
        gained: u32,
        proven: Vec<String>,
        unlocked: Vec<String>,
        rank: Option<String>,
    ) {
        if !self.settings.borrow().stored().show_progress {
            return;
        }
        let mut lines: Vec<String> = proven.iter().map(|p| format!("Proven: {p}")).collect();
        lines.extend(unlocked.iter().map(|u| format!("Earned: {u}")));
        if let Some(r) = rank {
            lines.push(format!("New rank: {r}"));
        }
        // A right answer alone is quiet: the quiz sheet shows it.
        if !lines.is_empty() {
            self.banner = Some(Banner {
                points: gained,
                lines,
            });
        }
    }
}

/// Receives the session's events on the turn's thread and posts them to the
/// main loop, in order.
pub struct TutorBridge(pub super::runtime::Post);

impl TutorListener for TutorBridge {
    fn on_event(&self, event: TutorEventInfo) {
        (self.0)(Box::new(event));
    }
}

fn tool_row(id: String, name: String, input: String) -> ToolRow {
    ToolRow {
        id,
        name,
        input,
        summary: None,
        images: Vec::new(),
        is_error: false,
        done: false,
    }
}

/// `read_bytes` as "Read Bytes".
pub fn tool_title(name: &str) -> String {
    name.split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or_else(String::new, |f| {
                f.to_uppercase().collect::<String>() + c.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Tools whose pictures are the answer's own, shown large with a line saying
/// where they came from (docs/24, docs/26).
pub fn drawing_caption(tool: &str) -> Option<&'static str> {
    match tool {
        "generate_image" => Some("Generated by an image model, not from the ROM"),
        "draw_diagram" => Some("Drawn by Romlens from the ROM"),
        "draw_svg" => Some("Drawn by the tutor, checked by Romlens"),
        _ => None,
    }
}

/// `$00:8000`.
pub fn address(a: u32) -> String {
    format!("${:02X}:{:04X}", (a >> 16) & 0xFF, a & 0xFFFF)
}

/// At most `limit` lines of `text`, centred on line `around`.
pub fn clip(text: &str, around: Option<usize>, limit: usize) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() <= limit {
        return text.to_owned();
    }
    let centre = around.unwrap_or(0);
    let start = centre.saturating_sub(limit / 2).min(lines.len() - limit);
    let end = start + limit;
    let mut out = Vec::new();
    if start > 0 {
        out.push(format!("/* … {start} lines above left out */"));
    }
    out.extend(lines[start..end].iter().map(|l| (*l).to_owned()));
    if end < lines.len() {
        out.push(format!(
            "/* … {} lines below left out */",
            lines.len() - end
        ));
    }
    out.join("\n")
}

// MARK: The transcript

/// What the transcript shows, built from the turns.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Question {
        index: u32,
        text: String,
        images: Vec<String>,
        selection: bool,
    },
    Answer {
        index: u32,
        text: String,
        reasoning: String,
        tools: Vec<ToolRow>,
        model: Option<String>,
        cost: f64,
    },
    Note {
        index: u32,
        text: String,
    },
    /// A lesson the tutor wrote in this turn (docs/25).
    Lesson {
        index: u32,
        id: String,
    },
}

impl Row {
    pub fn key(&self) -> String {
        match self {
            Row::Question { index, .. } => format!("q{index}"),
            Row::Answer { index, .. } => format!("a{index}"),
            Row::Note { index, .. } => format!("n{index}"),
            Row::Lesson { id, .. } => format!("l{id}"),
        }
    }
}

/// The notes Romlens adds before the student's words, which the window does
/// not show as theirs.
pub fn is_note(text: &str) -> bool {
    [
        "[The mode is now",
        "[The student's selection",
        "[The changes you proposed last time",
        "[Explain mode is",
        "[The student is at step",
    ]
    .iter()
    .any(|p| text.starts_with(p))
}

pub fn rows(turns: &[TurnInfo]) -> Vec<Row> {
    use romlens_ffi::tutor::session::TurnBlockInfo as B;
    // What each tool call answered, by the call's id.
    let mut results: HashMap<&str, (String, bool, Vec<String>)> = HashMap::new();
    for t in turns.iter().filter(|t| t.user) {
        for b in &t.blocks {
            if let B::ToolResult {
                id,
                text,
                images,
                is_error,
            } = b
            {
                results.insert(
                    id,
                    (
                        text.lines().next().unwrap_or("").to_owned(),
                        *is_error,
                        images.clone(),
                    ),
                );
            }
        }
    }
    // A diagram a lesson step shows is in the lesson's card, so the answer
    // does not show it again.
    let mut in_lessons: HashSet<String> = HashSet::new();
    for t in turns.iter().filter(|t| !t.user) {
        for b in &t.blocks {
            if let B::ToolCall { name, input, .. } = b
                && name == "lesson_step"
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(input)
                && let Some(p) = v.get("picture").and_then(|p| p.as_str())
            {
                in_lessons.insert(p.to_owned());
            }
        }
    }
    let mut out = Vec::new();
    for t in turns.iter().filter(|t| t.sent) {
        if t.user {
            let (mut words, mut images, mut selection, mut notes) =
                (Vec::new(), Vec::new(), false, Vec::new());
            for b in &t.blocks {
                match b {
                    B::Text { text } => {
                        if is_note(text) {
                            selection = selection || text.contains("selection");
                        } else {
                            words.push(text.clone());
                        }
                    }
                    B::Image { id, .. } => images.push(id.clone()),
                    B::Note { text } => notes.push(text.clone()),
                    _ => {}
                }
            }
            for n in notes {
                out.push(Row::Note {
                    index: t.index,
                    text: n,
                });
            }
            if !words.is_empty() || !images.is_empty() {
                out.push(Row::Question {
                    index: t.index,
                    text: words.join("\n\n"),
                    images,
                    selection,
                });
            }
        } else {
            let (mut text, mut reasoning, mut tools, mut lessons) =
                (String::new(), String::new(), Vec::new(), Vec::new());
            for b in &t.blocks {
                match b {
                    B::Text { text: s } => text += s,
                    B::Reasoning { summary } => {
                        if !reasoning.is_empty() {
                            reasoning += "\n\n";
                        }
                        reasoning += summary;
                    }
                    B::ToolCall { id, name, input } => {
                        let r = results.get(id.as_str());
                        let mut images = r.map_or_else(Vec::new, |r| r.2.clone());
                        if name.starts_with("draw_") {
                            images.retain(|i| !in_lessons.contains(i));
                        }
                        tools.push(ToolRow {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                            summary: r.map(|r| r.0.clone()),
                            images,
                            is_error: r.is_some_and(|r| r.1),
                            done: true,
                        });
                        if name == "begin_lesson"
                            && let Some(r) = r
                            && !r.1
                            && let Some(l) = lesson_id_from_result(r.0.clone())
                        {
                            lessons.push(l);
                        }
                    }
                    _ => {}
                }
            }
            out.push(Row::Answer {
                index: t.index,
                text,
                reasoning,
                tools,
                model: t.model.clone(),
                cost: t.cost,
            });
            out.extend(
                lessons
                    .into_iter()
                    .map(|id| Row::Lesson { index: t.index, id }),
            );
        }
    }
    out
}

/// The rows as the window shows them. A reply's rounds share one footer, on
/// its last answer, with the reply's whole cost; without the thinking and tool
/// calls, rounds with nothing else are left out.
pub fn shown(rows: Vec<Row>, work: bool) -> Vec<Row> {
    let mut out = Vec::new();
    let mut reply: Vec<Row> = Vec::new();
    let close = |reply: &mut Vec<Row>, out: &mut Vec<Row>| {
        let cost: f64 = reply
            .iter()
            .map(|r| {
                if let Row::Answer { cost, .. } = r {
                    *cost
                } else {
                    0.0
                }
            })
            .sum();
        let model = reply.last().and_then(|r| {
            if let Row::Answer { model, .. } = r {
                model.clone()
            } else {
                None
            }
        });
        let kept: Vec<Row> = reply
            .drain(..)
            .filter(|r| {
                work || match r {
                    Row::Answer { text, tools, .. } => {
                        !text.is_empty()
                            || tools
                                .iter()
                                .any(|t| drawing_caption(&t.name).is_some() && !t.images.is_empty())
                    }
                    _ => true,
                }
            })
            .collect();
        let n = kept.len();
        for (i, r) in kept.into_iter().enumerate() {
            out.push(match r {
                Row::Answer {
                    index,
                    text,
                    reasoning,
                    tools,
                    ..
                } => {
                    let last = i + 1 == n;
                    Row::Answer {
                        index,
                        text,
                        reasoning,
                        tools,
                        model: if last { model.clone() } else { None },
                        cost: if last { cost } else { 0.0 },
                    }
                }
                other => other,
            });
        }
    };
    for r in rows {
        if matches!(r, Row::Answer { .. }) {
            reply.push(r);
        } else {
            close(&mut reply, &mut out);
            out.push(r);
        }
    }
    close(&mut reply, &mut out);
    out
}

/// What a citation in an answer points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Citation {
    /// An address, in the listing.
    Address(u32),
    /// A routine's C, at the instruction.
    Routine(u32),
    /// A frame, and the graphics view to show it in.
    Frame(u64, Option<crate::model::graphics::Tab>),
    /// A register: nothing in the ROM to show; the step names it.
    Register,
}

/// Parse a `romlens://` link: `a/<addr>`, `c/<addr>`, `f/<n>?view=<view>`,
/// `r/<register>`.
pub fn parse_citation(url: &str) -> Option<Citation> {
    let rest = url.strip_prefix("romlens://")?;
    let (kind, tail) = rest.split_once('/')?;
    let (value, query) = tail.split_once('?').map_or((tail, ""), |(v, q)| (v, q));
    match kind {
        "a" => u32::from_str_radix(value, 16).ok().map(Citation::Address),
        "c" => u32::from_str_radix(value, 16).ok().map(Citation::Routine),
        "f" => {
            let view = query
                .split('&')
                .find_map(|p| p.strip_prefix("view="))
                .and_then(crate::model::graphics::Tab::from_id);
            value.parse().ok().map(|n| Citation::Frame(n, view))
        }
        "r" => Some(Citation::Register),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use romlens_ffi::tutor::session::TurnBlockInfo as B;

    fn turn(index: u32, user: bool, blocks: Vec<B>) -> TurnInfo {
        TurnInfo {
            index,
            user,
            blocks,
            model: user
                .then_some(String::new())
                .map_or(Some("m".into()), |_| None),
            cost: if user { 0.0 } else { 0.01 },
            sent: true,
        }
    }

    #[test]
    fn the_transcript_shows_questions_without_romlens_notes_and_answers_with_their_tools() {
        let turns = vec![
            turn(
                0,
                true,
                vec![
                    B::Text {
                        text: "[The student's selection: $00:8000]".into(),
                    },
                    B::Image {
                        id: "shot-1".into(),
                        media_type: "image/png".into(),
                    },
                    B::Text {
                        text: "What is this?".into(),
                    },
                ],
            ),
            turn(
                1,
                false,
                vec![
                    B::Reasoning {
                        summary: "thinking".into(),
                    },
                    B::ToolCall {
                        id: "c1".into(),
                        name: "listing".into(),
                        input: "{}".into(),
                    },
                ],
            ),
            turn(
                2,
                true,
                vec![B::ToolResult {
                    id: "c1".into(),
                    text: "first line\nsecond".into(),
                    images: vec![],
                    is_error: false,
                }],
            ),
            turn(
                3,
                false,
                vec![B::Text {
                    text: "It is RESET.".into(),
                }],
            ),
        ];
        let r = rows(&turns);
        assert_eq!(r.len(), 3, "the tool result turn shows as nothing: {r:?}");
        match &r[0] {
            Row::Question {
                text,
                images,
                selection,
                ..
            } => {
                assert_eq!(text, "What is this?");
                assert_eq!(images, &["shot-1"]);
                assert!(selection, "it went with the selection");
            }
            other => panic!("{other:?}"),
        }
        match &r[1] {
            Row::Answer {
                tools,
                reasoning,
                text,
                ..
            } => {
                assert_eq!((reasoning.as_str(), text.as_str()), ("thinking", ""));
                assert_eq!(
                    tools[0].summary.as_deref(),
                    Some("first line"),
                    "the first line of the result"
                );
            }
            other => panic!("{other:?}"),
        }
        // Without the work, the round that only called a tool is left out and
        // the one footer carries the whole reply's cost.
        let quiet = shown(r.clone(), false);
        assert_eq!(quiet.len(), 2);
        match &quiet[1] {
            Row::Answer {
                text, cost, model, ..
            } => {
                assert_eq!(text, "It is RESET.");
                assert!((cost - 0.02).abs() < 1e-9, "both rounds' cost: {cost}");
                assert!(model.is_some());
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            shown(r, true).len(),
            3,
            "with the work, nothing is left out"
        );
    }

    #[test]
    fn a_picture_the_answer_drew_keeps_its_round_even_when_the_work_is_hidden() {
        let turns = vec![
            turn(
                0,
                false,
                vec![B::ToolCall {
                    id: "c".into(),
                    name: "draw_diagram".into(),
                    input: "{}".into(),
                }],
            ),
            turn(
                1,
                true,
                vec![B::ToolResult {
                    id: "c".into(),
                    text: "ok".into(),
                    images: vec!["draw-1".into()],
                    is_error: false,
                }],
            ),
        ];
        let r = shown(rows(&turns), false);
        assert_eq!(r.len(), 1);
        // A lesson's diagram is in its card, not in the answer.
        let with_lesson = vec![
            turn(
                0,
                false,
                vec![
                    B::ToolCall {
                        id: "l".into(),
                        name: "lesson_step".into(),
                        input: r#"{"picture":"draw-1"}"#.into(),
                    },
                    B::ToolCall {
                        id: "c".into(),
                        name: "draw_diagram".into(),
                        input: "{}".into(),
                    },
                ],
            ),
            turn(
                1,
                true,
                vec![B::ToolResult {
                    id: "c".into(),
                    text: "ok".into(),
                    images: vec!["draw-1".into()],
                    is_error: false,
                }],
            ),
        ];
        let r = rows(&with_lesson);
        match &r[0] {
            Row::Answer { tools, .. } => assert!(tools[1].images.is_empty()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_note_from_romlens_is_a_note_and_the_bracketed_context_is_not_the_students() {
        assert!(is_note("[The mode is now read-only.]"));
        assert!(is_note("[The student is at step 2]"));
        assert!(!is_note("What is the mode?"));
        let turns = vec![turn(
            0,
            true,
            vec![B::Note {
                text: "Now on m.".into(),
            }],
        )];
        assert!(matches!(rows(&turns).as_slice(), [Row::Note { .. }]));
    }

    #[test]
    fn a_citation_is_an_address_a_routines_c_a_frame_or_a_register() {
        assert_eq!(
            parse_citation("romlens://a/008000"),
            Some(Citation::Address(0x8000))
        );
        assert_eq!(
            parse_citation("romlens://c/80841C"),
            Some(Citation::Routine(0x80841C))
        );
        assert_eq!(
            parse_citation("romlens://f/12"),
            Some(Citation::Frame(12, None))
        );
        assert_eq!(
            parse_citation("romlens://f/12?view=oam"),
            Some(Citation::Frame(12, Some(crate::model::graphics::Tab::Oam)))
        );
        assert_eq!(
            parse_citation("romlens://r/002100"),
            Some(Citation::Register)
        );
        assert_eq!(parse_citation("romlens://x/1"), None);
        assert_eq!(parse_citation("https://example.com"), None);
        assert_eq!(parse_citation("romlens://a/zz"), None);
    }

    #[test]
    fn tools_have_readable_names_and_pictures_say_where_they_came_from() {
        assert_eq!(tool_title("read_bytes"), "Read Bytes");
        assert_eq!(tool_title("rom_info"), "Rom Info");
        assert!(
            drawing_caption("generate_image")
                .unwrap()
                .contains("not from the ROM")
        );
        assert!(drawing_caption("draw_diagram").unwrap().contains("Romlens"));
        assert_eq!(drawing_caption("listing"), None);
        assert_eq!(address(0x80841C), "$80:841C");
    }

    #[test]
    fn long_text_is_clipped_around_a_line_and_says_what_it_left_out() {
        let text: Vec<String> = (0..300).map(|i| format!("line {i}")).collect();
        let text = text.join("\n");
        let c = clip(&text, Some(150), 40);
        assert!(c.contains("line 150") && !c.contains("line 10\n"));
        assert!(
            c.starts_with("/* … 130 lines above left out */"),
            "{}",
            &c[..40]
        );
        assert!(c.ends_with("lines below left out */"));
        assert_eq!(clip("short", Some(0), 40), "short");
        let top = clip(&text, None, 40);
        assert!(top.starts_with("line 0") && top.contains("260 lines below"));
        let end = clip(&text, Some(299), 40);
        assert!(end.contains("line 299") && !end.contains("below"));
    }

    #[test]
    fn typing_a_slash_lists_the_commands_that_start_it() {
        let names = |t: &str| {
            TutorModel::matching_commands(t)
                .iter()
                .map(|c| c.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names("/"),
            COMMANDS
                .iter()
                .map(|c| c.name)
                .filter(|n| *n != "/clear")
                .collect::<Vec<_>>()
        );
        assert_eq!(names("/re"), ["/resume", "/rewind", "/review"]);
        assert_eq!(names("/Mod"), ["/model", "/mode"]);
        assert_eq!(
            names("/c"),
            ["/compact", "/cost"],
            "/clear is offered only once asked for"
        );
        assert_eq!(names("/cl"), ["/clear"]);
        assert!(names("hello").is_empty());
        assert!(names("/model gpt").is_empty(), "past the command, no list");
    }
}
