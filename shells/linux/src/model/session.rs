//! Owns the core `Workbench` for one document: runs analysis, applies
//! commands, mirrors undo state, and turns core events into main-thread
//! changes. The macOS twin is `WorkbenchSession`.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use romlens_ffi::{AnalysisPhase, AnalysisStats, Command, RomlensError, Workbench, WorkbenchEvent};

use super::runtime::Runtime;

#[derive(Debug, Clone, PartialEq)]
pub enum AnalysisState {
    Idle,
    Running { fraction: f64, phase: &'static str },
    Failed(String),
}

impl AnalysisState {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Snapshot,
    View,
    Project { dirty: bool },
}

#[derive(Default)]
struct State {
    analysis: Option<AnalysisState>,
    stats: Option<AnalysisStats>,
    has_snapshot: bool,
    can_undo: bool,
    can_redo: bool,
    undo_title: Option<String>,
    redo_title: Option<String>,
    dirty: bool,
    /// Another run was asked for while one was running; start it when that
    /// one ends.
    rerun_requested: bool,
}

type Hook = Box<dyn Fn(ChangeKind)>;

pub struct Session {
    pub workbench: Arc<Workbench>,
    runtime: Rc<dyn Runtime>,
    state: RefCell<State>,
    /// Bumped by every re-analysis request, so an older debounce timer does
    /// nothing when it fires.
    debounce: Cell<u64>,
    generation: Cell<u64>,
    reanalysis_delay: Cell<Duration>,
    /// The view model drops caches here.
    on_change: RefCell<Option<Hook>>,
    /// The document counts changes here.
    on_command: RefCell<Option<Hook>>,
    /// The window repaints its status here.
    on_state: RefCell<Option<Box<dyn Fn()>>>,
}

impl Session {
    pub fn new(workbench: Arc<Workbench>, runtime: Rc<dyn Runtime>) -> Rc<Self> {
        let session = Rc::new(Self {
            workbench,
            runtime,
            state: RefCell::new(State::default()),
            debounce: Cell::new(0),
            generation: Cell::new(0),
            reanalysis_delay: Cell::new(Duration::from_millis(300)),
            on_change: RefCell::new(None),
            on_command: RefCell::new(None),
            on_state: RefCell::new(None),
        });
        let weak: Weak<Session> = Rc::downgrade(&session);
        session.runtime.attach_listener(
            &session.workbench,
            Rc::new(move |event| {
                if let Some(s) = weak.upgrade() {
                    s.handle(event);
                }
            }),
        );
        session.refresh_undo_state();
        session
    }

    pub fn set_on_change(&self, f: impl Fn(ChangeKind) + 'static) {
        *self.on_change.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_on_command(&self, f: impl Fn(ChangeKind) + 'static) {
        *self.on_command.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_on_state(&self, f: impl Fn() + 'static) {
        *self.on_state.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_reanalysis_delay(&self, d: Duration) {
        self.reanalysis_delay.set(d);
    }

    // MARK: State

    pub fn analysis(&self) -> AnalysisState {
        self.state
            .borrow()
            .analysis
            .clone()
            .unwrap_or(AnalysisState::Idle)
    }

    pub fn stats(&self) -> Option<AnalysisStats> {
        self.state.borrow().stats
    }

    /// The disassembly exists once the first analysis has landed.
    pub fn has_snapshot(&self) -> bool {
        self.state.borrow().has_snapshot
    }

    pub fn can_undo(&self) -> bool {
        self.state.borrow().can_undo
    }

    pub fn can_redo(&self) -> bool {
        self.state.borrow().can_redo
    }

    pub fn undo_title(&self) -> Option<String> {
        self.state.borrow().undo_title.clone()
    }

    pub fn redo_title(&self) -> Option<String> {
        self.state.borrow().redo_title.clone()
    }

    pub fn is_dirty(&self) -> bool {
        self.state.borrow().dirty
    }

    /// Bumped on every snapshot or view change.
    pub fn generation(&self) -> u64 {
        self.generation.get()
    }

    fn set_analysis(&self, a: AnalysisState) {
        self.state.borrow_mut().analysis = Some(a);
        self.notify_state();
    }

    fn notify_state(&self) {
        if let Some(f) = self.on_state.borrow().as_ref() {
            f();
        }
    }

    // MARK: Analysis

    pub fn start_analysis(self: &Rc<Self>) {
        self.set_analysis(AnalysisState::Running {
            fraction: 0.0,
            phase: "starting",
        });
        let this = Rc::downgrade(self);
        self.runtime.analyze(
            Arc::clone(&self.workbench),
            Box::new(move |result| {
                let Some(s) = this.upgrade() else { return };
                match result {
                    Ok(stats) => {
                        s.state.borrow_mut().stats = Some(stats);
                        s.set_analysis(AnalysisState::Idle);
                    }
                    Err(RomlensError::Cancelled) => s.set_analysis(AnalysisState::Idle),
                    Err(e) => s.set_analysis(AnalysisState::Failed(e.to_string())),
                }
                s.start_requested_rerun();
            }),
        );
    }

    fn start_requested_rerun(self: &Rc<Self>) {
        let requested = std::mem::take(&mut self.state.borrow_mut().rerun_requested);
        if requested {
            self.start_analysis();
        }
    }

    pub fn cancel_analysis(&self) {
        self.state.borrow_mut().rerun_requested = false;
        self.workbench.cancel_analysis();
        self.set_analysis(AnalysisState::Idle);
    }

    /// Re-run after a short pause. A run already under way finishes first
    /// and the new one follows it: cancelling it instead meant that changes
    /// arriving faster than a run takes, as a live session's execution log
    /// does every second, could cancel every run and the numbers never moved.
    pub fn schedule_reanalysis(self: &Rc<Self>) {
        let ticket = self.debounce.get() + 1;
        self.debounce.set(ticket);
        let this = Rc::downgrade(self);
        self.runtime.after(
            self.reanalysis_delay.get(),
            Box::new(move || {
                let Some(s) = this.upgrade() else { return };
                if s.debounce.get() != ticket {
                    return;
                }
                if s.analysis().is_running() {
                    s.state.borrow_mut().rerun_requested = true;
                } else {
                    s.start_analysis();
                }
            }),
        );
    }

    // MARK: Commands

    pub fn execute(self: &Rc<Self>, command: Command) -> Result<(), RomlensError> {
        let affects = Self::affects_analysis(&command);
        self.workbench.execute(command)?;
        self.finish_command(affects);
        Ok(())
    }

    /// Run `f` against the workbench as a command: the bookkeeping an edit
    /// does (undo mirror, dirty, cache drop) without re-analysing unless the
    /// core says the analysis is stale.
    pub fn command<T>(
        self: &Rc<Self>,
        affects_analysis: bool,
        f: impl FnOnce(&Workbench) -> Result<T, RomlensError>,
    ) -> Result<T, RomlensError> {
        let out = f(&self.workbench)?;
        self.finish_command(affects_analysis);
        Ok(out)
    }

    pub fn undo(self: &Rc<Self>) -> bool {
        let before = self.workbench.needs_analysis();
        if !matches!(self.workbench.undo(), Ok(true)) {
            return false;
        }
        self.finish_command(before || self.workbench.needs_analysis());
        true
    }

    pub fn redo(self: &Rc<Self>) -> bool {
        if !matches!(self.workbench.redo(), Ok(true)) {
            return false;
        }
        self.finish_command(self.workbench.needs_analysis());
        true
    }

    pub fn affects_analysis(command: &Command) -> bool {
        match command {
            Command::SetLabel { .. }
            | Command::SetComment { .. }
            | Command::SetRegionParams { .. }
            | Command::SetVariable { .. }
            | Command::SetLocalName { .. }
            | Command::SetRoutineNote { .. }
            | Command::SetCComment { .. }
            | Command::SetCVersion { .. } => false,
            Command::MarkRegion { .. }
            | Command::ClearRegionOverride { .. }
            | Command::SetFlagOverride { .. } => true,
        }
    }

    /// The tutor changed the project through the core, not through here:
    /// refresh as after any edit, and analyse again if a mark or flag changed.
    pub fn tutor_edited(self: &Rc<Self>) {
        self.finish_command(self.workbench.needs_analysis());
    }

    pub fn finish_command(self: &Rc<Self>, affects_analysis: bool) {
        self.refresh_undo_state();
        self.generation.set(self.generation.get() + 1);
        self.fire_change(ChangeKind::View);
        self.fire_command(ChangeKind::Project {
            dirty: self.workbench.is_dirty(),
        });
        if affects_analysis || self.workbench.needs_analysis() {
            self.schedule_reanalysis();
        }
    }

    fn fire_change(&self, kind: ChangeKind) {
        if let Some(f) = self.on_change.borrow().as_ref() {
            f(kind);
        }
    }

    fn fire_command(&self, kind: ChangeKind) {
        if let Some(f) = self.on_command.borrow().as_ref() {
            f(kind);
        }
    }

    /// The project was replaced wholesale (Revert): the undo history is gone,
    /// nothing is dirty, and the analysis is stale.
    pub fn reloaded(self: &Rc<Self>) {
        self.refresh_undo_state();
        self.generation.set(self.generation.get() + 1);
        self.fire_change(ChangeKind::View);
        self.schedule_reanalysis();
    }

    /// What the listing and the C show changed without the project changing
    /// (Show Explanations): drop caches, but this is not an edit.
    pub fn view_changed(&self) {
        self.generation.set(self.generation.get() + 1);
        self.fire_change(ChangeKind::View);
    }

    pub fn refresh_undo_state(&self) {
        {
            let mut s = self.state.borrow_mut();
            s.can_undo = self.workbench.can_undo();
            s.can_redo = self.workbench.can_redo();
            s.undo_title = self.workbench.undo_title();
            s.redo_title = self.workbench.redo_title();
            s.dirty = self.workbench.is_dirty();
        }
        self.notify_state();
    }

    // MARK: Events

    pub fn handle(&self, event: WorkbenchEvent) {
        match event {
            WorkbenchEvent::SnapshotChanged { .. } => {
                {
                    let mut s = self.state.borrow_mut();
                    s.has_snapshot = true;
                    s.stats = Some(self.workbench.stats());
                }
                self.generation.set(self.generation.get() + 1);
                self.fire_change(ChangeKind::Snapshot);
                self.notify_state();
            }
            WorkbenchEvent::ViewChanged { .. } => {}
            WorkbenchEvent::ProjectChanged { dirty } => {
                self.state.borrow_mut().dirty = dirty;
                self.notify_state();
            }
            WorkbenchEvent::AnalysisProgress { phase, done, total } => {
                // Progress arrives on its own hop, so the run's last report can
                // land after the run ended and set the bar going again. Only a
                // running analysis shows progress.
                if !self.analysis().is_running() {
                    return;
                }
                let fraction = if total == 0 {
                    0.0
                } else {
                    done as f64 / total as f64
                };
                self.set_analysis(AnalysisState::Running {
                    fraction,
                    phase: phase_name(phase),
                });
            }
        }
    }
}

pub fn phase_name(phase: AnalysisPhase) -> &'static str {
    match phase {
        AnalysisPhase::Descent => "walking code",
        AnalysisPhase::Tables => "resolving jump tables",
        AnalysisPhase::Sweep => "sweeping gaps",
        AnalysisPhase::Heuristics => "scoring the rest",
        AnalysisPhase::Labels => "naming",
        AnalysisPhase::Lines => "building lines",
    }
}
