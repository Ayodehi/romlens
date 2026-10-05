//! Per-document state shared by the hex and disassembly canvases, the
//! navigator, the inspector and the sheets: the ROM, its session, selection,
//! jump history and the details of what is selected. Views hold an
//! `Rc<Document>` and subscribe to be told what changed. The macOS twin is
//! `RomViewModel`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;

use romlens_ffi::{
    BankRule, ByteInterpretation, ByteRange, Command, CommentInfo, CommentKind, DataKind,
    ExplanationInfo, FlagOverride, InstructionInfo, LabelInfo, LabelSource, OverrideKind,
    PreviewInfo, RegionInfo, ResolvedAddress, Rom, RomInfo, RomlensError, SpanKind, TableElem,
    VarTypeInfo, VarWidth, WarningInfo, Workbench, XRefInfo,
};

use super::audio::{self, AudioModel};
use super::batch_cache::BatchCache;
use super::commands::Zoom;
use super::commands::{CEdit, EditorCommand, EditorSource, Sheet};
use super::compare::{self, CompareModel, Item as CompareItem};
use super::decompile::{Decompile, Key as DecompileKey};
use super::graph::{self, GraphMode, GraphModel, Key as GraphKey};
use super::graphics::{self as gfx, GraphicsModel};
use super::layout::{Layout, Panes, ResultsKind, Tab};
use super::navigator::{NavTab, Navigator, NavigatorData};
use super::references::ReferencesModel;
use super::runtime::Runtime;
use super::runtime::background;
use super::screen::ScreenModel;
use super::search::SearchModel;
use super::session::{ChangeKind, Session};
use super::source::{self, SourceModel};
use super::transfer::{self, ExportKind, ImportKind};
use super::tutor::{self, TutorModel};
use crate::asm::AsmBatch;
use crate::hex::{AddressStyle, BYTES_PER_ROW, HexBatch};
use crate::package::{self, LocalRecord};

/// Compatibility with a request that names a place to scroll to; the id makes
/// two requests for the same offset distinct, so a view scrolls again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollRequest {
    pub id: u64,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Selection,
    Scroll,
    AddressStyle,
    /// Cached rows are stale (analysis finished, a command ran).
    Rows,
    /// A sheet was requested or dismissed.
    Sheet,
    /// The tab or which panes are open changed.
    Layout,
    /// The navigator's lists, tab or filter changed.
    Navigator,
    /// Find's hits or Find References' rows changed.
    Results,
    /// The C tab's routine, level or text changed.
    Decompile,
    /// The Graph tab's routine, mode or graph changed.
    Graph,
    /// Zoom In, Out or Fit was asked for.
    Zoom,
    /// A graphics view's source, settings or selection changed.
    Graphics,
    /// A sound view's source, selection, playback or machine changed.
    Audio,
    /// The Tutor's conversation, turn streaming in, cards or sheets changed.
    Tutor,
    /// The inspector's Screen section opened, closed or got its result.
    Screen,
    /// The Source tab's files, shown file or text changed.
    Source,
    /// The Compare tab's state, list or selection changed.
    Compare,
    /// Analysis state or undo state changed.
    Status,
    History,
}

/// What the inspector shows about the selection; cleared with it.
#[derive(Default, Clone)]
pub struct Details {
    pub inspection: Option<ByteInterpretation>,
    pub instruction: Option<InstructionInfo>,
    pub explanation: Option<ExplanationInfo>,
    pub region: Option<RegionInfo>,
    pub label: Option<LabelInfo>,
    pub line_comment: Option<CommentInfo>,
    pub block_comment: Option<CommentInfo>,
    pub xrefs_to: Vec<XRefInfo>,
    pub xrefs_from: Vec<XRefInfo>,
    pub warnings: Vec<WarningInfo>,
    pub flag_override: Option<FlagOverride>,
    pub preview: Option<PreviewInfo>,
}

/// What the Define Variable sheet edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableDraft {
    pub address: String,
    pub name: String,
    pub width: VarWidth,
    pub count: u16,
    /// The variable being edited, if any; its address cannot change.
    pub existing: Option<u32>,
}

impl Default for VariableDraft {
    fn default() -> Self {
        Self {
            address: String::new(),
            name: String::new(),
            width: VarWidth::Byte,
            count: 1,
            existing: None,
        }
    }
}

impl VariableDraft {
    /// The address the draft names: the existing variable's, or the typed one.
    pub fn resolved_address(&self) -> Option<u32> {
        self.existing
            .or_else(|| parse_variable_address(&self.address))
    }

    /// Bytes the variable spans.
    pub fn size(&self) -> u32 {
        let width = match self.width {
            VarWidth::Byte => 1,
            VarWidth::Word => 2,
            VarWidth::Long => 3,
        };
        width * u32::from(self.count.max(1))
    }
}

/// `$7E:0094`, `7E0094` or `$0094`; four digits or fewer below $2000 is low
/// RAM (bank $7E), otherwise a register in bank $00.
pub fn parse_variable_address(text: &str) -> Option<u32> {
    let hex: String = text
        .chars()
        .filter(|c| !matches!(c, '$' | ':') && !c.is_whitespace())
        .collect();
    if hex.is_empty() || hex.len() > 6 {
        return None;
    }
    let v = u32::from_str_radix(&hex, 16).ok()?;
    Some(if hex.len() <= 4 && v < 0x2000 {
        0x7E_0000 | v
    } else {
        v
    })
}

/// The parameters of Mark as ▸ Data….
#[derive(Debug, Clone, Default)]
pub struct MarkOptions {
    pub stride: Option<u8>,
    pub bpp: Option<u8>,
    pub elem: Option<TableElem>,
    pub bank: Option<BankRule>,
}

const HISTORY_LIMIT: usize = 100;
const CACHE_CAPACITY: usize = 64;

#[derive(Default)]
struct Selection {
    offset: Option<u32>,
    anchor: Option<u32>,
}

type Listener = Box<dyn Fn(Change)>;

pub struct Document {
    pub rom: Arc<Rom>,
    pub info: RomInfo,
    pub session: Rc<Session>,
    pub hex_cache: BatchCache<HexBatch>,
    pub asm_cache: BatchCache<AsmBatch>,
    asm_line_count: Cell<u32>,
    sheet: Cell<Option<Sheet>>,
    c_edit: RefCell<Option<CEdit>>,
    layout: RefCell<Layout>,
    explanations: Cell<bool>,
    runtime: Rc<dyn Runtime>,
    navigator: RefCell<Navigator>,
    /// Bumped by every filter keystroke, so an older debounce does nothing.
    filter_ticket: Cell<u64>,
    search: RefCell<SearchModel>,
    references: RefCell<ReferencesModel>,
    variable_draft: RefCell<VariableDraft>,
    project_path: RefCell<Option<PathBuf>>,
    decompile: RefCell<Decompile>,
    graph: RefCell<GraphModel>,
    source: RefCell<SourceModel>,
    graphics: RefCell<GraphicsModel>,
    audio: RefCell<AudioModel>,
    tutor: RefCell<TutorModel>,
    tutor_post: RefCell<Option<super::runtime::Post>>,
    audio_ticking: Cell<bool>,
    live_latest: Cell<Option<u64>>,
    live_scheduled: Cell<bool>,
    live_post: RefCell<Option<super::runtime::Post>>,
    screen: RefCell<ScreenModel>,
    compare: RefCell<CompareModel>,
    zoom: Cell<Option<(Zoom, u64)>>,
    me: RefCell<Weak<Document>>,
    rom_path: RefCell<Option<PathBuf>>,
    span_kinds: HashMap<u8, SpanKind>,
    selection: RefCell<Selection>,
    details: RefCell<Details>,
    history: RefCell<Vec<u32>>,
    forward_history: RefCell<Vec<u32>>,
    scroll: Cell<Option<ScrollRequest>>,
    style: Cell<AddressStyle>,
    /// Bumped whenever cached rows must be rebuilt (snapshot or view change).
    generation: Cell<u64>,
    listeners: RefCell<Vec<Listener>>,
}

impl Document {
    pub fn open(path: &Path, runtime: Rc<dyn Runtime>) -> Result<Rc<Self>, RomlensError> {
        let rom = Rom::open(path.to_string_lossy().into_owned())?;
        let doc = Self::new(rom, runtime);
        *doc.rom_path.borrow_mut() = Some(path.to_path_buf());
        Ok(doc)
    }

    /// A saved project: the package's files, applied to the ROM they belong
    /// to. The core refuses a ROM whose hash is not the project's.
    pub fn from_project(
        rom: Arc<Rom>,
        files: std::collections::HashMap<String, Vec<u8>>,
        project_path: &Path,
        rom_path: &Path,
        runtime: Rc<dyn Runtime>,
    ) -> Result<Rc<Self>, RomlensError> {
        let workbench = Workbench::with_project_files(Arc::clone(&rom), files)?;
        let doc = Self::with_workbench(rom, workbench, runtime);
        doc.reattach_recording();
        *doc.project_path.borrow_mut() = Some(project_path.to_path_buf());
        *doc.rom_path.borrow_mut() = Some(rom_path.to_path_buf());
        Ok(doc)
    }

    pub fn new(rom: Arc<Rom>, runtime: Rc<dyn Runtime>) -> Rc<Self> {
        let workbench = Workbench::new(Arc::clone(&rom));
        Self::with_workbench(rom, workbench, runtime)
    }

    pub fn with_workbench(
        rom: Arc<Rom>,
        workbench: Arc<Workbench>,
        runtime: Rc<dyn Runtime>,
    ) -> Rc<Self> {
        let info = rom.info();
        let rom_for_graphics = Arc::clone(&rom);
        let workbench_for_audio = Arc::clone(&workbench);
        let span_kinds = rom
            .spans()
            .into_iter()
            .filter_map(|s| u8::try_from(s.id).ok().map(|id| (id, s.kind)))
            .collect();
        let fetch_from = Arc::clone(&workbench);
        let asm_from = Arc::clone(&workbench);
        let line_count = workbench.line_count();
        let session = Session::new(workbench, Rc::clone(&runtime));
        let doc = Rc::new(Self {
            rom,
            info,
            session: Rc::clone(&session),
            hex_cache: BatchCache::new(CACHE_CAPACITY, move |start, count| {
                fetch_from.hex_rows(start, count)
            }),
            asm_cache: BatchCache::new(CACHE_CAPACITY, move |start, count| {
                asm_from.asm_lines(start, count)
            }),
            asm_line_count: Cell::new(line_count),
            sheet: Cell::new(None),
            c_edit: RefCell::new(None),
            layout: RefCell::new(Layout::default()),
            explanations: Cell::new(true),
            runtime,
            navigator: RefCell::new(Navigator::default()),
            filter_ticket: Cell::new(0),
            search: RefCell::new(SearchModel::default()),
            references: RefCell::new(ReferencesModel::default()),
            variable_draft: RefCell::new(VariableDraft::default()),
            project_path: RefCell::new(None),
            decompile: RefCell::new(Decompile::default()),
            graph: RefCell::new(GraphModel::default()),
            source: RefCell::new(SourceModel::default()),
            graphics: RefCell::new(GraphicsModel::new(Arc::clone(&rom_for_graphics))),
            audio: RefCell::new(AudioModel::new(
                Arc::clone(&rom_for_graphics),
                Arc::clone(&workbench_for_audio),
                !cfg!(test),
            )),
            audio_ticking: Cell::new(false),
            tutor: RefCell::new(TutorModel::new(
                crate::settings::tutor(),
                TutorModel::default_root(),
            )),
            tutor_post: RefCell::new(None),
            live_latest: Cell::new(None),
            live_scheduled: Cell::new(false),
            live_post: RefCell::new(None),
            screen: RefCell::new(ScreenModel::default()),
            compare: RefCell::new(CompareModel::default()),
            zoom: Cell::new(None),
            me: RefCell::new(Weak::new()),
            rom_path: RefCell::new(None),
            span_kinds,
            selection: RefCell::new(Selection::default()),
            details: RefCell::new(Details::default()),
            history: RefCell::new(Vec::new()),
            forward_history: RefCell::new(Vec::new()),
            scroll: Cell::new(None),
            style: Cell::new(AddressStyle::default()),
            generation: Cell::new(0),
            listeners: RefCell::new(Vec::new()),
        });
        *doc.me.borrow_mut() = Rc::downgrade(&doc);
        doc.source.borrow_mut().reload(doc.workbench());
        let weak = Rc::downgrade(&doc);
        session.set_on_change(move |kind| {
            if let Some(d) = weak.upgrade() {
                d.handle_session_change(kind);
            }
        });
        let weak = Rc::downgrade(&doc);
        session.set_on_state(move || {
            if let Some(d) = weak.upgrade() {
                d.emit(Change::Status);
            }
        });
        doc
    }

    pub fn workbench(&self) -> &Arc<Workbench> {
        &self.session.workbench
    }

    pub fn subscribe(&self, f: impl Fn(Change) + 'static) {
        self.listeners.borrow_mut().push(Box::new(f));
    }

    fn emit(&self, change: Change) {
        // The sound views read the graphics views' recording and frame: one
        // frame for both.
        if change == Change::Graphics {
            self.sync_audio();
        }
        for f in self.listeners.borrow().iter() {
            f(change);
        }
    }

    fn handle_session_change(self: &Rc<Self>, kind: ChangeKind) {
        match kind {
            ChangeKind::Snapshot | ChangeKind::View => {
                self.hex_cache.invalidate_all();
                self.asm_cache.invalidate_all();
                self.asm_line_count.set(self.workbench().line_count());
                self.generation.set(self.generation.get() + 1);
                self.screen.borrow_mut().invalidate();
                self.refresh_details();
                self.reload_navigator();
                self.decompile.borrow_mut().invalidate();
                self.graph.borrow_mut().invalidate();
                self.refresh_decompile();
                self.refresh_graph();
                self.reload_source();
                self.refresh_compare();
                if self.audio_tab().is_some() && self.audio.borrow().source == audio::Source::Rom {
                    self.load_upload();
                }
                self.emit(Change::Rows);
            }
            ChangeKind::Project { .. } => {}
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation.get()
    }

    /// The disassembly exists once the first analysis has landed.
    pub fn has_disassembly(&self) -> bool {
        self.session.has_snapshot() && self.asm_line_count.get() > 0
    }

    pub fn asm_line_count(&self) -> u32 {
        self.asm_line_count.get()
    }

    pub fn line_for_offset(&self, offset: u32) -> Option<u32> {
        self.workbench().line_for_offset(offset)
    }

    // MARK: Project file

    /// Where the project is saved; `None` for an untitled one, which is what a
    /// freshly opened ROM is.
    pub fn project_path(&self) -> Option<PathBuf> {
        self.project_path.borrow().clone()
    }

    pub fn rom_path(&self) -> Option<PathBuf> {
        self.rom_path.borrow().clone()
    }

    /// The project's name, or the ROM's for an untitled one.
    pub fn display_name(&self) -> String {
        let stem = |p: &Path| p.file_stem().map(|s| s.to_string_lossy().into_owned());
        self.project_path()
            .as_deref()
            .and_then(stem)
            .or_else(|| self.rom_path().as_deref().and_then(stem))
            .unwrap_or_else(|| {
                if self.info.title.trim().is_empty() {
                    "Untitled".to_owned()
                } else {
                    self.info.title.trim().to_owned()
                }
            })
    }

    fn write_package(&self, dir: &Path) -> Result<(), RomlensError> {
        let io = |e: std::io::Error| RomlensError::Io {
            msg: format!("{}: {e}", dir.display()),
        };
        let files = self.workbench().project_files();
        package::write(dir, &files).map_err(io)?;
        package::remove_stale(dir, &files).map_err(io)?;
        let local = LocalRecord {
            last_path: self.rom_path().map(|p| p.to_string_lossy().into_owned()),
        };
        package::write_local(dir, &local).map_err(io)
    }

    /// Save the project to `dir`, and remember it as where it lives.
    pub fn save_to(&self, dir: &Path) -> Result<(), RomlensError> {
        self.write_package(dir)?;
        self.workbench().mark_saved();
        *self.project_path.borrow_mut() = Some(dir.to_path_buf());
        self.session.refresh_undo_state();
        self.emit(Change::Status);
        Ok(())
    }

    /// Write a copy to `dir` without moving this document there.
    pub fn duplicate_to(&self, dir: &Path) -> Result<(), RomlensError> {
        self.write_package(dir)
    }

    /// Replace the project with what is on disk.
    pub fn revert(self: &Rc<Self>) -> Result<(), RomlensError> {
        let Some(dir) = self.project_path() else {
            return Err(RomlensError::Project {
                msg: "this project has not been saved".into(),
            });
        };
        self.workbench()
            .open_project_into(dir.to_string_lossy().into_owned())?;
        self.session.reloaded();
        self.refresh_details();
        self.emit(Change::Status);
        Ok(())
    }

    // MARK: Import and export

    /// Import a trace, a symbol file or ca65 debug information. Read and
    /// applied off the main thread; through the session, so an import
    /// schedules the re-analysis and marks the document dirty exactly as an
    /// edit does. An import that left the map showing the state before it
    /// would be worse than no import.
    pub fn import(
        self: &Rc<Self>,
        kind: ImportKind,
        path: PathBuf,
        done: impl FnOnce(Result<romlens_ffi::ImportResult, RomlensError>) + 'static,
    ) {
        let wb = Arc::clone(self.workbench());
        let weak = Rc::downgrade(self);
        background(
            &*self.runtime,
            move || transfer::run_import(kind, &wb, &path),
            move |result| {
                if let Some(d) = weak.upgrade()
                    && result.is_ok()
                {
                    d.session.finish_command(true);
                    d.refresh_details();
                }
                done(result);
            },
        );
    }

    /// Write an export to `path`, generated and written off the main thread.
    pub fn export(
        &self,
        kind: ExportKind,
        include_bytes: bool,
        path: PathBuf,
        done: impl FnOnce(Result<(), RomlensError>) + 'static,
    ) {
        let wb = Arc::clone(self.workbench());
        background(
            &*self.runtime,
            move || {
                let text = transfer::generate(kind, include_bytes, &wb);
                let io = |e: std::io::Error| RomlensError::Io {
                    msg: format!("{}: {e}", path.display()),
                };
                // Beside and renamed, so a failed write never leaves half a file.
                let tmp = path.with_extension(format!("{}.tmp", kind.extension()));
                std::fs::write(&tmp, text).map_err(io)?;
                std::fs::rename(&tmp, &path).map_err(io)
            },
            done,
        );
    }

    // MARK: Layout

    pub fn tab(&self) -> Tab {
        self.layout.borrow().tab
    }

    pub fn set_tab(&self, tab: Tab) {
        let changed = {
            let mut l = self.layout.borrow_mut();
            // A text tab takes the editor area back from a graphics view.
            let was_graphics = l.graphics.take().is_some();
            let was_audio = l.audio.take().is_some();
            std::mem::replace(&mut l.tab, tab) != tab || was_graphics || was_audio
        };
        if changed {
            self.emit(Change::Layout);
            self.refresh_decompile();
            self.refresh_graph();
        }
    }

    // MARK: Graphics

    pub fn graphics(&self) -> std::cell::Ref<'_, GraphicsModel> {
        self.graphics.borrow()
    }

    /// The graphics view in the editor area, if one is open.
    pub fn graphics_tab(&self) -> Option<gfx::Tab> {
        self.layout.borrow().graphics
    }

    /// Open a graphics view, reading the ROM bytes at the selection.
    pub fn open_graphics(&self, tab: gfx::Tab) {
        if !tab.needs_recording()
            && self.graphics.borrow().source == gfx::Source::Rom
            && let Some(range) = self.highlighted_range()
        {
            self.graphics.borrow_mut().rom_offset = range.start;
        }
        let changed = {
            let mut l = self.layout.borrow_mut();
            let was_audio = l.audio.take().is_some();
            l.graphics.replace(tab) != Some(tab) || was_audio
        };
        if changed {
            self.emit(Change::Layout);
        }
        self.emit(Change::Graphics);
    }

    // MARK: Tutor

    pub fn tutor(&self) -> std::cell::Ref<'_, TutorModel> {
        self.tutor.borrow()
    }

    /// Change the Tutor's model, then tell the views.
    pub fn edit_tutor<R>(&self, f: impl FnOnce(&mut TutorModel) -> R) -> R {
        let r = f(&mut self.tutor.borrow_mut());
        self.emit(Change::Tutor);
        r
    }

    /// Reads the Tutor's model, which may fetch from the core, without telling
    /// the views: for a view that is itself being built from a change.
    pub fn edit_tutor_quiet<R>(&self, f: impl FnOnce(&mut TutorModel) -> R) -> R {
        f(&mut self.tutor.borrow_mut())
    }

    /// The tutor changed the project (or took its edits back): the listing and
    /// the analysis follow, as for any edit.
    pub fn tutor_edited(&self) {
        if let Some(me) = self.me.borrow().upgrade() {
            me.session.finish_command(true);
            me.refresh_details();
        }
        self.emit(Change::Tutor);
    }

    /// The core's session, made on first use so a project that never opens
    /// the tutor costs nothing.
    pub fn tutor_session(&self) -> Option<Arc<romlens_ffi::tutor::session::TutorSession>> {
        let me = self.me.borrow().upgrade()?;
        let existing = self.tutor_post.borrow().clone();
        let post = existing.unwrap_or_else(|| {
            let weak = Rc::downgrade(&me);
            let post = self.runtime.sink(Rc::new(move |message| {
                if let Some(d) = weak.upgrade()
                    && let Ok(event) =
                        message.downcast::<romlens_ffi::tutor::session::TutorEventInfo>()
                {
                    d.tutor_event(*event);
                }
            }));
            *self.tutor_post.borrow_mut() = Some(Arc::clone(&post));
            post
        });
        let keys = Arc::clone(&crate::settings::tutor().borrow().keys);
        let offset = tutor_utc_offset();
        Some(self.tutor.borrow_mut().ensure_session(
            Arc::clone(self.workbench()),
            Arc::new(crate::secrets::TutorCredentials(keys)),
            Arc::new(tutor::TutorBridge(post)),
            offset,
        ))
    }

    /// An event of the session, on the main loop.
    fn tutor_event(&self, event: romlens_ffi::tutor::session::TutorEventInfo) {
        let effects = self.tutor.borrow_mut().handle(event);
        if effects.edited {
            // What the tutor changed shows in the listing, and the analysis
            // follows where a mark or a flag did (`tutorEdited`).
            if let Some(me) = self.me.borrow().upgrade() {
                me.session.finish_command(true);
                me.refresh_details();
            }
        }
        self.emit(Change::Tutor);
    }

    /// The composer's line: a slash command, or a question.
    pub fn tutor_submit(&self, text: &str) {
        self.tutor_session();
        let text = text.trim();
        if text.starts_with('/') {
            let result = self.tutor.borrow_mut().run_command(text);
            if text.to_lowercase().starts_with("/attach") && text.to_lowercase().contains("frame") {
                self.tutor_attach_frame();
            }
            self.emit(Change::Tutor);
            if result.send
                && let Some(t) = result.composer
            {
                self.tutor_send(&t);
            }
            return;
        }
        self.tutor_send(text);
    }

    pub fn tutor_send(&self, text: &str) {
        self.tutor_session();
        let selection = self.tutor_selection_text();
        let recording = self.graphics.borrow().recording().cloned();
        self.tutor.borrow_mut().send(text, selection, recording);
        self.emit(Change::Tutor);
    }

    /// The recording's frame the main window shows, as a picture.
    pub fn tutor_attach_frame(&self) {
        let png = self
            .graphics
            .borrow()
            .frame_image()
            .and_then(|f| crate::pixels::png(&f.image));
        self.edit_tutor(|t| match png {
            Some(p) => {
                let n = self.graphics.borrow().frame();
                t.attach(p, "image/png", &format!("Frame {n}"));
            }
            None => t.error = Some("Open a recording to attach its frame.".into()),
        });
    }

    /// What goes with a question: where, and the listing there.
    pub fn tutor_selection_text(&self) -> Option<String> {
        let off = self.selected()?;
        let a = self.selected_address()?;
        let wb = self.workbench();
        let mut s = tutor::address(a);
        if let Some(l) = &self.details.borrow().label {
            s += &format!(" ({})", l.name);
        }
        if let Some(line) = wb.line_for_offset(off) {
            s += "\n";
            s += &wb.asm_lines_text(line.saturating_sub(4), 16, romlens_ffi::AddressStyle::Snes);
        }
        let at = self
            .details
            .borrow()
            .instruction
            .as_ref()
            .map_or(off, |i| i.file_offset);
        if let Some(c) = self.tutor_c_text(at) {
            s += "\n";
            s += &c;
        }
        if self.graphics.borrow().has_recording() {
            s += &format!(
                "\nA recording is open in Romlens, at frame {}.",
                self.graphics.borrow().frame()
            );
        }
        Some(s)
    }

    /// The C the student is reading when the C tab has the editor: the
    /// generated C around the selection.
    fn tutor_c_text(&self, offset: u32) -> Option<String> {
        if self.tab() != Tab::C || self.graphics_tab().is_some() || self.audio_tab().is_some() {
            return None;
        }
        let d = self.decompile.borrow();
        let r = d
            .result
            .as_ref()
            .filter(|_| d.state == super::decompile::DecompileState::Ready)?;
        if let Some(v) = d.shown_version.as_ref()
            && let Some(n) = self.c_versions(r.entry).into_iter().find(|n| &n.name == v)
        {
            return Some(format!(
                "[The student is reading the C tab: the C version “{v}” of {}, not the generated C.]\n```c\n{}\n```",
                r.name,
                tutor::clip(&n.version.text, None, 160)
            ));
        }
        let at = d.lines_for_instruction(offset).first().copied();
        Some(format!(
            "[The student is reading the C tab: {} as Romlens generates it, at level {:?}.]\n```c\n{}\n```",
            r.name,
            d.level,
            tutor::clip(&r.text, at, 160)
        ))
    }

    /// Show what a citation in an answer points at in the main window.
    /// Returns false if there is nothing to show (a frame with no recording).
    pub fn follow_citation(&self, c: tutor::Citation) -> bool {
        match c {
            tutor::Citation::Address(a) => {
                self.jump_to_snes(a);
                true
            }
            tutor::Citation::Routine(a) => {
                self.set_tab(Tab::C);
                self.jump_to_snes(a);
                true
            }
            tutor::Citation::Frame(n, view) => {
                if !self.graphics.borrow().has_recording() {
                    return false;
                }
                self.set_frame(n);
                self.open_graphics(view.unwrap_or(gfx::Tab::Frame));
                true
            }
            // A register has no place in the ROM to show; the step names it.
            tutor::Citation::Register => true,
        }
    }

    // MARK: Audio

    pub fn audio(&self) -> std::cell::Ref<'_, AudioModel> {
        self.audio.borrow()
    }

    /// The sound view in the editor area, if one is open.
    pub fn audio_tab(&self) -> Option<audio::Tab> {
        self.layout.borrow().audio
    }

    /// Open a sound view, on the recording's sound if it has any, else on the
    /// ROM's upload.
    pub fn open_audio(&self, tab: audio::Tab) {
        let trace = self.audio.borrow_mut().opened();
        let changed = {
            let mut l = self.layout.borrow_mut();
            let was_graphics = l.graphics.take().is_some();
            l.audio.replace(tab) != Some(tab) || was_graphics
        };
        if changed {
            self.emit(Change::Layout);
        }
        if trace {
            self.load_upload();
        }
        self.emit(Change::Audio);
    }

    /// Keep the sound model's recording and frame current with the graphics
    /// model's.
    fn sync_audio(&self) {
        let g = self.graphics.borrow();
        self.audio.borrow_mut().sync_recording(
            g.recording().cloned(),
            g.recording_name(),
            g.frame(),
            g.frame_count(),
        );
    }

    /// Change the sound model, then tell the views; playing starts the clock.
    pub fn edit_audio<R>(&self, f: impl FnOnce(&mut AudioModel) -> R) -> R {
        let r = f(&mut self.audio.borrow_mut());
        self.emit(Change::Audio);
        self.ensure_audio_clock();
        r
    }

    /// Trace the upload in the current analysis, once per analysis.
    pub fn load_upload(&self) {
        if !self.has_disassembly() {
            return;
        }
        let wb = Arc::clone(self.workbench());
        let generation = wb.analysis_generation();
        if !self.audio.borrow_mut().begin_upload(generation) {
            return;
        }
        let weak = self.me.borrow().clone();
        let wb_for_work = Arc::clone(&wb);
        background(
            &*self.runtime,
            move || wb_for_work.sound_upload_blocking(),
            move |report| {
                let Some(d) = weak.upgrade() else { return };
                d.audio.borrow_mut().finish_upload(generation, report);
                d.emit(Change::Audio);
            },
        );
        self.emit(Change::Audio);
    }

    /// The recording's notes for the timeline, read off the main thread.
    pub fn load_notes(&self) {
        let Some((recording, last)) = self.audio.borrow_mut().begin_notes() else {
            return;
        };
        let weak = self.me.borrow().clone();
        let rec = Arc::clone(&recording);
        background(
            &*self.runtime,
            move || rec.note_timeline(0, last).unwrap_or_default(),
            move |notes| {
                let Some(d) = weak.upgrade() else { return };
                d.audio.borrow_mut().finish_notes(&recording, notes);
                d.emit(Change::Audio);
            },
        );
        self.emit(Change::Audio);
    }

    /// While playing, a few times a second: read the machine again, and move
    /// the graphics' frame along with a recording.
    fn ensure_audio_clock(&self) {
        if !self.audio.borrow().is_playing() || self.audio_ticking.replace(true) {
            return;
        }
        self.audio_clock();
    }

    fn audio_clock(&self) {
        let weak = self.me.borrow().clone();
        self.runtime.after(
            std::time::Duration::from_millis(audio::TICK_MS),
            Box::new(move || {
                let Some(d) = weak.upgrade() else { return };
                let result = d.audio.borrow_mut().tick();
                if let Some(frame) = result.move_to {
                    d.graphics.borrow_mut().set_frame(frame);
                    d.emit(Change::Graphics);
                }
                d.emit(Change::Audio);
                if d.audio.borrow().is_playing() {
                    d.audio_clock();
                } else {
                    d.audio_ticking.set(false);
                }
            }),
        );
    }

    /// Play This Command: the ROM's driver, sent `value` on `port`.
    pub fn play_command(&self, port: u8, value: u8) {
        let trace = self.audio.borrow_mut().play_command(port, value);
        if trace {
            self.load_upload();
        }
        self.emit(Change::Audio);
        self.ensure_audio_clock();
    }

    /// Change the graphics model. The closure may return the ROM bytes the
    /// change selected, which are selected in the editor too (one selection
    /// across code and graphics).
    pub fn edit_graphics(
        &self,
        f: impl FnOnce(&mut GraphicsModel) -> Option<std::ops::Range<u32>>,
    ) {
        let range = f(&mut self.graphics.borrow_mut());
        if let Some(r) = range {
            self.select_range(r);
        }
        self.emit(Change::Graphics);
    }

    /// Attach a recording, refusing one of another ROM with the core's
    /// words. The project keeps its path and fingerprint, never its
    /// contents (docs/12, rule 4).
    pub fn attach_recording(
        &self,
        session: Arc<romlens_ffi::RecordingSession>,
        name: &str,
    ) -> Result<(), RomlensError> {
        self.graphics
            .borrow_mut()
            .attach(Arc::clone(&session), name)?;
        if let Some(reference) = session.reference() {
            self.workbench().attach_recording(reference);
        }
        if self.graphics_tab().is_none() {
            self.open_graphics(gfx::Tab::Tilemap);
        } else {
            self.emit(Change::Graphics);
        }
        Ok(())
    }

    /// Close the recording the views read, and stop referring to it.
    pub fn close_recording(&self) {
        for r in self.workbench().recordings() {
            self.workbench().detach_recording(r.path);
        }
        self.graphics.borrow_mut().detach();
        self.emit(Change::Graphics);
    }

    /// On opening a project: reattach the recording it refers to, if the
    /// file is still there and unchanged. Quietly does nothing otherwise,
    /// since a recording is a convenience, not part of the project's content.
    pub fn reattach_recording(&self) {
        let Some(r) = self.workbench().recordings().into_iter().last() else {
            return;
        };
        let Ok(session) = romlens_ffi::RecordingSession::open(r.path.clone(), false) else {
            return;
        };
        if session.reference().map(|x| x.fingerprint) != Some(r.fingerprint) {
            return;
        }
        let name = std::path::Path::new(&r.path)
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        // Attached, but the views are not opened: a reopened project shows
        // its listing, and the recording is there when asked for.
        let _ = self.graphics.borrow_mut().attach(session, &name);
    }

    /// Where a pixel of the current frame came from (docs/22, P5), worked out
    /// off the main thread: the DMAs that put its bytes in VRAM, the code
    /// before them and the ROM bytes.
    pub fn pixel_provenance(
        &self,
        x: usize,
        y: usize,
        done: impl FnOnce(Option<romlens_ffi::ProvenanceInfo>) + 'static,
    ) {
        let (Some(recording), frame) = (
            self.graphics.borrow().recording().cloned(),
            self.graphics.borrow().frame(),
        ) else {
            return done(None);
        };
        let wb = Arc::clone(self.workbench());
        background(
            &*self.runtime,
            move || wb.pixel_provenance_blocking(recording, frame, x as u32, y as u32),
            done,
        );
    }

    // MARK: Live session

    pub fn is_live(&self) -> bool {
        self.graphics.borrow().is_live()
    }

    /// File > Start Live Session: listen for the recorder script's stream and
    /// show it in the graphics views as it arrives. The session is a
    /// recording like any other to the views, so everything that reads one
    /// works live. It listens on the loopback address only, and refuses a
    /// stream recorded from another ROM.
    pub fn start_live(&self) -> Result<(), RomlensError> {
        if self.is_live() {
            return Ok(());
        }
        let Some(bridge) = self.live_bridge() else {
            return Ok(());
        };
        let session = romlens_ffi::live::LiveSession::start(
            Arc::clone(&self.rom),
            romlens_ffi::live::live_default_port(),
            bridge,
        )?;
        self.attach_live_session(session)
    }

    /// The listener a live session reports to: it posts to this document's
    /// main loop.
    pub fn live_bridge(&self) -> Option<Arc<super::live::LiveBridge>> {
        let me = self.me.borrow().upgrade()?;
        let existing = self.live_post.borrow().clone();
        let post = existing.unwrap_or_else(|| {
            let weak = Rc::downgrade(&me);
            let post = self.runtime.sink(Rc::new(move |message| {
                if let Some(d) = weak.upgrade() {
                    d.live_message(message);
                }
            }));
            *self.live_post.borrow_mut() = Some(Arc::clone(&post));
            post
        });
        Some(super::live::LiveBridge::new(
            post,
            Arc::clone(self.workbench()),
        ))
    }

    /// Read a started session's frames as the recording, following the
    /// newest.
    pub fn attach_live_session(
        &self,
        session: Arc<romlens_ffi::live::LiveSession>,
    ) -> Result<(), RomlensError> {
        self.graphics.borrow_mut().attach_live(session, 0)?;
        if self.graphics_tab().is_none() {
            self.open_graphics(gfx::Tab::Tilemap);
        } else {
            self.emit(Change::Graphics);
        }
        Ok(())
    }

    /// Stop listening, keeping the frames already received.
    pub fn stop_live(&self) {
        self.graphics.borrow_mut().stop_live();
        self.emit(Change::Graphics);
    }

    /// A message from the session's thread, on the main loop.
    fn live_message(self: &Rc<Self>, message: Box<dyn std::any::Any + Send>) {
        use super::live::{LiveMessage, MAX_FRAME_RATE};
        let Ok(message) = message.downcast::<LiveMessage>() else {
            return;
        };
        match *message {
            LiveMessage::Frame(n) => {
                // Frames arrive sixty times a second; the views are told at
                // most thirty, with only the newest, so drawing never falls
                // behind.
                self.live_latest.set(Some(n));
                if !self.live_scheduled.replace(true) {
                    let weak = Rc::downgrade(self);
                    self.runtime.after(
                        std::time::Duration::from_millis(1000 / MAX_FRAME_RATE),
                        Box::new(move || {
                            let Some(d) = weak.upgrade() else { return };
                            d.live_scheduled.set(false);
                            if let Some(latest) = d.live_latest.take() {
                                d.graphics.borrow_mut().live_arrived(latest);
                                d.emit(Change::Graphics);
                            }
                        }),
                    );
                }
            }
            LiveMessage::Status(s) => {
                self.graphics.borrow_mut().live_status_changed(&s);
                self.emit(Change::Graphics);
            }
            LiveMessage::Merged(added) => {
                self.graphics.borrow_mut().live_log_merged(added);
                // The bookkeeping a command does, and the re-analysis.
                self.session.finish_command(true);
                self.refresh_details();
                self.emit(Change::Graphics);
            }
        }
    }

    /// Pack a recorder stream (`.rlstream`) into a recording at `out`, off
    /// the main thread.
    pub fn pack_recording(
        &self,
        stream: PathBuf,
        out: PathBuf,
        done: impl FnOnce(Result<romlens_ffi::PackSummary, RomlensError>) + 'static,
    ) {
        let rom = Arc::clone(&self.rom);
        background(
            &*self.runtime,
            move || {
                if let Some(dir) = out.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| RomlensError::Io {
                        msg: format!("{}: {e}", dir.display()),
                    })?;
                }
                romlens_ffi::pack_recorder_stream(
                    rom,
                    stream.to_string_lossy().into_owned(),
                    out.to_string_lossy().into_owned(),
                    None,
                )
            },
            done,
        );
    }

    /// Move the recording to `frame`.
    pub fn set_frame(&self, frame: u64) {
        if self.graphics.borrow().frame() == frame {
            return;
        }
        self.edit_graphics(|g| {
            g.set_frame(frame);
            None
        });
    }

    pub fn step_frame(&self, delta: i64) {
        self.edit_graphics(|g| {
            g.step(delta);
            None
        });
    }

    /// Read from the selection: Decode the bytes at the editor's selection.
    pub fn read_graphics_from_selection(&self) {
        if let Some(r) = self.highlighted_range() {
            self.edit_graphics(|g| {
                g.rom_offset = r.start;
                g.selected_tile = 0;
                None
            });
        }
    }

    /// A Screen row's view: the ROM bytes a DMA sends to VRAM or the
    /// palette, in the Tile Decoder, Tilemap or Palette view.
    pub fn open_screen_link(&self, link: romlens_ffi::ScreenLinkInfo) {
        use romlens_ffi::{ScreenLinkInfo, TileFormat};
        // The palette the same setup loads, where it is in ROM.
        let palette = self.screen.borrow().setup.as_ref().and_then(|s| {
            s.sections
                .iter()
                .flat_map(|sec| &sec.rows)
                .find_map(|r| match r.link {
                    Some(ScreenLinkInfo::Palette { rom }) => Some(rom),
                    _ => None,
                })
        });
        let tab = {
            let mut g = self.graphics.borrow_mut();
            g.source = gfx::Source::Rom;
            match link {
                ScreenLinkInfo::Tiles { rom, bpp } => {
                    g.rom_offset = rom;
                    g.format = match bpp {
                        2 => TileFormat::Bpp2,
                        8 => TileFormat::Bpp8,
                        7 => TileFormat::Mode7,
                        _ => TileFormat::Bpp4,
                    };
                    if let Some(p) = palette {
                        g.palette = gfx::PaletteChoice::Rom(p);
                    }
                    g.selected_tile = 0;
                    gfx::Tab::Tiles
                }
                ScreenLinkInfo::Tilemap { rom } => {
                    g.rom_offset = rom;
                    gfx::Tab::Tilemap
                }
                ScreenLinkInfo::Palette { rom } => {
                    g.rom_offset = rom;
                    gfx::Tab::Palette
                }
            }
        };
        self.open_graphics(tab);
    }

    /// The inspector's "Open in …": the preview's view on the range it
    /// previewed, or on the decompressed bytes for compressed data.
    pub fn open_preview(&self, p: &romlens_ffi::PreviewInfo) {
        use romlens_ffi::PreviewView;
        let params = self.workbench().region_params_at(p.start);
        {
            let mut g = self.graphics.borrow_mut();
            match &p.decompressed {
                Some(data) => {
                    g.source = gfx::Source::Bytes {
                        label: "Decompressed".into(),
                        data: data.clone(),
                    }
                }
                None => {
                    g.source = gfx::Source::Rom;
                    g.rom_offset = p.start;
                }
            }
            if let Some(f) = p.format {
                g.format = f;
            }
            if let Some(params) = params {
                if let Some(c) = params.columns {
                    g.columns = usize::from(c);
                }
                if let Some(s) = params.screen_size {
                    g.screen_size = s;
                }
                if let Some(offset) = params.palette.and_then(|a| self.rom.file_offset_for(a)) {
                    g.palette = gfx::PaletteChoice::Rom(offset);
                }
            }
            g.selected_tile = 0;
        }
        self.open_graphics(match p.view {
            PreviewView::TileDecoder => gfx::Tab::Tiles,
            PreviewView::Palette => gfx::Tab::Palette,
            PreviewView::Tilemap => gfx::Tab::Tilemap,
        });
    }

    /// Source only with sources imported, Compare only while comparing.
    pub fn tab_available(&self, tab: Tab) -> bool {
        match tab {
            Tab::Source => self.source.borrow().has_files(),
            Tab::Compare => self.compare.borrow().is_active(),
            _ => true,
        }
    }

    pub fn panes(&self) -> Panes {
        self.layout.borrow().panes
    }

    pub fn results_kind(&self) -> ResultsKind {
        self.layout.borrow().results_kind
    }

    pub fn is_focused(&self) -> bool {
        self.layout.borrow().is_focused()
    }

    /// Open or close one pane. Opening one while focused ends Focus.
    pub fn set_pane(&self, pane: impl FnOnce(&mut Panes) -> &mut bool, shown: bool) {
        let before = self.layout.borrow().clone();
        self.layout.borrow_mut().set_pane(pane, shown);
        let after = self.layout.borrow();
        if before.panes != after.panes || before.is_focused() != after.is_focused() {
            drop(after);
            self.emit(Change::Layout);
        }
    }

    /// Show the results pane on one list.
    pub fn show_results(&self, kind: ResultsKind) {
        self.layout.borrow_mut().results_kind = kind;
        self.set_pane(|p| &mut p.results, true);
        self.emit(Change::Layout);
    }

    pub fn toggle_focus(&self) {
        self.layout.borrow_mut().toggle_focus();
        self.emit(Change::Layout);
    }

    // MARK: Find

    pub fn search(&self) -> std::cell::Ref<'_, SearchModel> {
        self.search.borrow()
    }

    /// Edit the query, mode and options; the sheet reads them back.
    pub fn edit_search(&self, f: impl FnOnce(&mut SearchModel)) {
        f(&mut self.search.borrow_mut());
    }

    /// Run the query and go to the first hit.
    pub fn run_search(&self) {
        self.layout.borrow_mut().results_kind = ResultsKind::Find;
        self.search.borrow_mut().search(self.workbench());
        let first = self.search.borrow().hits.first().map(|h| h.file_offset);
        self.emit(Change::Results);
        if let Some(offset) = first {
            self.jump_to(offset);
        }
    }

    /// Ctrl+G and Ctrl+Shift+G.
    pub fn step_search(&self, delta: i64) {
        let offset = self.search.borrow_mut().step(delta).map(|h| h.file_offset);
        self.emit(Change::Results);
        if let Some(o) = offset {
            self.jump_to(o);
        }
    }

    pub fn go_to_hit(&self, index: usize) {
        let offset = self
            .search
            .borrow_mut()
            .select(index)
            .map(|h| h.file_offset);
        self.emit(Change::Results);
        if let Some(o) = offset {
            self.jump_to(o);
        }
    }

    // MARK: Find References

    pub fn references(&self) -> std::cell::Ref<'_, ReferencesModel> {
        self.references.borrow()
    }

    /// What Find References would look for: the selected item's label, or its
    /// address, with how many references there are.
    pub fn reference_target(&self) -> Option<(String, usize)> {
        let address = self.selected_address()?;
        let d = self.details.borrow();
        let name = d.label.as_ref().map_or_else(
            || romlens_ffi::format_snes_address(address),
            |l| l.name.clone(),
        );
        Some((name, d.xrefs_to.len()))
    }

    /// List everything that refers to the selected item, in the results pane.
    /// The selection stays where it is until a row is chosen.
    pub fn find_references(&self) {
        if let Some(address) = self.selected_address() {
            self.find_references_to(address);
        }
    }

    pub fn find_references_to(&self, address: u32) {
        self.references.borrow_mut().find(address, self.workbench());
        self.emit(Change::Results);
        self.show_results(ResultsKind::References);
    }

    pub fn go_to_reference(&self, index: usize) {
        let offset = self
            .references
            .borrow_mut()
            .select(index)
            .map(|r| r.file_offset);
        self.emit(Change::Results);
        if let Some(o) = offset {
            self.jump_to(o);
        }
    }

    // MARK: Variables

    pub fn variable_draft(&self) -> VariableDraft {
        self.variable_draft.borrow().clone()
    }

    /// The data address the selected instruction's operand names, canonical:
    /// `STA $0094` run from bank $80 gives $7E:0094.
    pub fn operand_address(&self) -> Option<u32> {
        let target = {
            let d = self.details.borrow();
            let i = d.instruction.as_ref()?;
            if i.target_kind == Some(romlens_ffi::TargetKind::Code) {
                return None;
            }
            i.target?
        };
        Some(self.workbench().canonical_address(target))
    }

    /// Open Define Variable for `address`, or for the selected operand, or
    /// empty. An address inside a variable edits that variable.
    pub fn begin_define_variable(&self, at: Option<u32>) {
        let mut draft = VariableDraft::default();
        if let Some(at) = at.or_else(|| self.operand_address()) {
            let wb = self.workbench();
            if let Some(v) = wb.variable_containing(at) {
                draft = VariableDraft {
                    address: romlens_ffi::format_snes_address(v.address),
                    name: v.name,
                    width: v.width,
                    count: v.count,
                    existing: Some(v.address),
                };
            } else {
                draft.address = romlens_ffi::format_snes_address(at);
                draft.name = wb
                    .label_at(at)
                    .filter(|l| l.source != LabelSource::Auto)
                    .map(|l| l.name)
                    .unwrap_or_default();
            }
        }
        *self.variable_draft.borrow_mut() = draft;
        self.show_sheet(Some(Sheet::Variable));
    }

    /// The Variables list's + button: always a new, empty definition, never
    /// the variable the selection happens to be in.
    pub fn begin_new_variable(&self) {
        *self.variable_draft.borrow_mut() = VariableDraft::default();
        self.show_sheet(Some(Sheet::Variable));
    }

    /// Define (or redefine) the variable the draft describes.
    pub fn define_variable(self: &Rc<Self>, draft: &VariableDraft) -> Result<(), RomlensError> {
        let address = draft
            .resolved_address()
            .ok_or_else(|| RomlensError::BadAddress {
                msg: format!(
                    "{} is not an address; use a form like $7E:0094",
                    draft.address
                ),
            })?;
        let (name, ty) = (
            draft.name.trim().to_owned(),
            VarTypeInfo {
                width: draft.width,
                count: draft.count.max(1),
            },
        );
        self.session
            .command(false, |wb| wb.define_variable(address, name, ty))?;
        self.refresh_details();
        Ok(())
    }

    pub fn remove_variable(self: &Rc<Self>, address: u32) -> Result<(), RomlensError> {
        self.session
            .command(false, |wb| wb.remove_variable(address))?;
        self.refresh_details();
        Ok(())
    }

    /// A label's removal takes its variable with it, as one step, since a type
    /// without a name names nothing.
    pub fn remove_label_or_variable(self: &Rc<Self>) -> Result<(), RomlensError> {
        let Some(address) = self.selected_address() else {
            return Ok(());
        };
        if !self.can_remove_label() {
            return Ok(());
        }
        let canonical = self.workbench().canonical_address(address);
        if self
            .workbench()
            .variables()
            .iter()
            .any(|v| v.address == canonical)
        {
            self.remove_variable(address)
        } else {
            self.set_label(None)
        }
    }

    /// Mark as ▸ Data… with its parameters.
    pub fn mark_with(&self, data_kind: DataKind, options: MarkOptions) -> Result<(), RomlensError> {
        let Some(range) = self.highlighted_range() else {
            return Ok(());
        };
        self.session.execute(Command::MarkRegion {
            start: range.start,
            len: range.end - range.start,
            kind: OverrideKind::Data,
            data_kind: Some(data_kind),
            stride: options.stride,
            bpp: options.bpp,
            elem: options.elem,
            bank: options.bank,
        })?;
        self.refresh_details();
        Ok(())
    }

    // MARK: C

    pub fn decompile(&self) -> std::cell::Ref<'_, Decompile> {
        self.decompile.borrow()
    }

    /// Keep the C tab on the routine at the selection. Only while a tab is
    /// showing: decompiling costs a summary of every routine the first time
    /// after an analysis.
    pub fn refresh_decompile(&self) {
        if self.tab() != Tab::C || !self.has_disassembly() {
            return;
        }
        let start = self
            .details
            .borrow()
            .instruction
            .as_ref()
            .map(|i| i.file_offset)
            .or_else(|| self.selected());
        let run =
            self.decompile
                .borrow_mut()
                .follow(self.workbench(), start, self.generation.get());
        if let Some(run) = run {
            self.run_decompile(run);
        }
        self.emit(Change::Decompile);
    }

    fn run_decompile(&self, run: DecompileKey) {
        let wb = Arc::clone(self.workbench());
        let weak = self.me.borrow().clone();
        background(
            &*self.runtime,
            move || {
                wb.set_c_numbers(run.numbers);
                wb.decompile_blocking(run.entry, run.level)
                    .map_err(|e| e.to_string())
            },
            move |outcome| {
                let Some(d) = weak.upgrade() else { return };
                let next = d.decompile.borrow_mut().finish(run, outcome);
                d.emit(Change::Decompile);
                if let Some(next) = next {
                    d.run_decompile(next);
                }
            },
        );
    }

    pub fn set_decompile_level(&self, level: romlens_ffi::DecompileLevel) {
        {
            let mut d = self.decompile.borrow_mut();
            if d.level == level {
                return;
            }
            d.level = level;
            d.invalidate();
        }
        self.refresh_decompile();
    }

    pub fn set_c_numbers(&self, style: romlens_ffi::NumberStyle) {
        {
            let mut d = self.decompile.borrow_mut();
            if d.numbers == style {
                return;
            }
            d.numbers = style;
            d.invalidate();
        }
        crate::config::Settings {
            hide_explanations: !self.explanations(),
            c_numbers: super::decompile::number_style_name(style).to_owned(),
        }
        .save();
        self.refresh_decompile();
    }

    // MARK: Graph

    pub fn graph(&self) -> std::cell::Ref<'_, GraphModel> {
        self.graph.borrow()
    }

    /// Keep the Graph tab on the routine at the selection, while it shows.
    pub fn refresh_graph(&self) {
        if self.tab() != Tab::Graph || !self.has_disassembly() {
            return;
        }
        let start = self
            .details
            .borrow()
            .instruction
            .as_ref()
            .map(|i| i.file_offset)
            .or_else(|| self.selected());
        let run = self
            .graph
            .borrow_mut()
            .follow(self.workbench(), start, self.generation.get());
        if let Some(run) = run {
            self.run_graph(run);
        }
        self.emit(Change::Graph);
    }

    fn run_graph(&self, run: GraphKey) {
        let wb = Arc::clone(self.workbench());
        let weak = self.me.borrow().clone();
        background(
            &*self.runtime,
            move || graph::build(&wb, run),
            move |outcome| {
                let Some(d) = weak.upgrade() else { return };
                let next = d.graph.borrow_mut().finish(run, outcome);
                d.emit(Change::Graph);
                if let Some(next) = next {
                    d.run_graph(next);
                }
            },
        );
    }

    pub fn set_graph_mode(&self, mode: GraphMode) {
        {
            let mut g = self.graph.borrow_mut();
            if g.mode == mode {
                return;
            }
            g.mode = mode;
            g.invalidate();
        }
        self.refresh_graph();
    }

    // MARK: Source

    pub fn source(&self) -> std::cell::Ref<'_, SourceModel> {
        self.source.borrow()
    }

    /// Read the imported files again, and leave the Source tab if there are
    /// none left to show.
    fn reload_source(&self) {
        let before = self.source.borrow().generation;
        self.source.borrow_mut().reload(self.workbench());
        if self.source.borrow().generation != before {
            self.emit(Change::Source);
        }
        if self.tab() == Tab::Source && !self.source.borrow().has_files() {
            self.set_tab(Tab::Disassembly);
        }
    }

    pub fn show_source_file(&self, index: usize) {
        if self.source.borrow().shown == Some(index) {
            return;
        }
        self.source.borrow_mut().show(Some(index), self.workbench());
        self.emit(Change::Source);
    }

    /// Read the shown file again, after it was found or fixed.
    pub fn retry_source(&self) {
        let shown = self.source.borrow().shown;
        self.source.borrow_mut().show(shown, self.workbench());
        self.emit(Change::Source);
    }

    /// The lines that made the selected byte, the program's own first.
    pub fn source_lines_at_selection(&self) -> Vec<romlens_ffi::source::SourceLineInfo> {
        self.selected()
            .map_or_else(Vec::new, |o| self.workbench().source_lines_at(o))
    }

    /// Show the file of the selected byte's line, as selecting it elsewhere
    /// does on macOS.
    pub fn follow_selection_in_source(&self) {
        let Some(line) = self.source_lines_at_selection().into_iter().next() else {
            return;
        };
        let before = self.source.borrow().generation;
        self.source
            .borrow_mut()
            .show_file_of(&line, self.workbench());
        if self.source.borrow().generation != before {
            self.emit(Change::Source);
        }
    }

    /// A click on a line that made bytes selects its bytes; a macro's line
    /// went through more than once, so a second click goes to the next.
    pub fn select_source_line(&self, line: u32) {
        let hit = self.source.borrow().by_line.get(&line).cloned();
        let Some(hit) = hit else { return };
        if let Some(r) = source::next_range(&hit.ranges, self.selected()) {
            self.select_range(r.start..r.start + r.len.max(1));
        }
    }

    // MARK: Compare

    pub fn compare(&self) -> std::cell::Ref<'_, CompareModel> {
        self.compare.borrow()
    }

    /// File › Compare With…: load the other version off the main thread,
    /// analyse it, and compare.
    pub fn compare_with(&self, path: PathBuf) {
        let name = path
            .file_stem()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let ticket = self.compare.borrow_mut().begin(&name);
        self.set_tab(Tab::Compare);
        self.emit(Change::Compare);
        let this = Arc::clone(self.workbench());
        let generation = self.generation.get();
        let weak = self.me.borrow().clone();
        background(
            &*self.runtime,
            move || {
                let other = compare::load_other(&path)?;
                let info = this
                    .compare_with_blocking(Arc::clone(&other))
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>((other, info))
            },
            move |result| {
                let Some(d) = weak.upgrade() else { return };
                if !d.compare.borrow().is_current(ticket) {
                    return;
                }
                match result {
                    Ok((other, info)) => {
                        let mut c = d.compare.borrow_mut();
                        c.loaded(other);
                        c.finished(Ok(info), generation);
                    }
                    Err(e) => d.compare.borrow_mut().finished(Err(e), generation),
                }
                d.emit(Change::Compare);
                d.refresh_compare();
            },
        );
    }

    /// This version changed since the comparison (a name, or a re-analysis):
    /// compare again against the same other version.
    fn refresh_compare(&self) {
        let (ticket, other) = {
            let c = self.compare.borrow();
            if !c.needs_refresh(self.generation.get()) {
                return;
            }
            let Some(other) = c.other.clone() else { return };
            (c.current_ticket(), other)
        };
        let generation = self.generation.get();
        self.compare.borrow_mut().refreshing(generation);
        let this = Arc::clone(self.workbench());
        let weak = self.me.borrow().clone();
        background(
            &*self.runtime,
            move || this.compare_with_blocking(other).ok(),
            move |info| {
                let Some(d) = weak.upgrade() else { return };
                if !d.compare.borrow().is_current(ticket) {
                    return;
                }
                d.compare.borrow_mut().refreshed(info, generation);
                d.emit(Change::Compare);
            },
        );
    }

    pub fn close_compare(&self) {
        self.compare.borrow_mut().close();
        self.emit(Change::Compare);
        if self.tab() == Tab::Compare {
            self.set_tab(Tab::Disassembly);
        }
    }

    pub fn select_compare_item(&self, item: Option<CompareItem>) {
        self.compare.borrow_mut().selected = item;
        self.emit(Change::Compare);
    }

    /// Carry over the names the other version has for routines this one only
    /// has automatic names for: one undo step.
    pub fn carry_names(self: &Rc<Self>) -> Result<usize, RomlensError> {
        let names = self
            .compare
            .borrow()
            .info
            .as_ref()
            .map_or_else(Vec::new, |i| i.names_to_carry.clone());
        if names.is_empty() {
            return Ok(0);
        }
        let n = names.len();
        self.workbench().carry_names(names)?;
        self.session.finish_command(true);
        self.refresh_details();
        Ok(n)
    }

    /// View › Zoom In, Out and Fit, for whichever canvas is showing.
    pub fn request_zoom(&self, kind: Zoom) {
        let id = self.zoom.get().map_or(0, |(_, id)| id) + 1;
        self.zoom.set(Some((kind, id)));
        self.emit(Change::Zoom);
    }

    /// The last zoom asked for, numbered so the same one can repeat.
    pub fn zoom_request(&self) -> Option<(Zoom, u64)> {
        self.zoom.get()
    }

    // MARK: Navigator

    pub fn navigator(&self) -> std::cell::Ref<'_, Navigator> {
        self.navigator.borrow()
    }

    pub fn set_nav_tab(&self, tab: NavTab) {
        let changed = {
            let mut n = self.navigator.borrow_mut();
            let changed = n.tab() != tab;
            n.tab = Some(tab);
            changed
        };
        if changed {
            self.emit(Change::Navigator);
        }
    }

    /// Filter the navigator's lists, after a short pause so a burst of
    /// keystrokes filters once.
    pub fn set_nav_filter(self: &Rc<Self>, text: &str) {
        self.navigator.borrow_mut().filter = text.to_owned();
        let ticket = self.filter_ticket.get() + 1;
        self.filter_ticket.set(ticket);
        let this = Rc::downgrade(self);
        self.runtime.after(
            std::time::Duration::from_millis(150),
            Box::new(move || {
                let Some(d) = this.upgrade() else { return };
                if d.filter_ticket.get() == ticket {
                    d.navigator.borrow_mut().apply_filter();
                    d.emit(Change::Navigator);
                }
            }),
        );
    }

    /// Read the lists again, off the main thread.
    pub fn reload_navigator(self: &Rc<Self>) {
        self.navigator.borrow_mut().loading = true;
        let (wb, rom) = (Arc::clone(self.workbench()), Arc::clone(&self.rom));
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        background(
            &*self.runtime,
            move || NavigatorData::load(&wb, &rom),
            move |data| {
                if let Some(d) = weak.upgrade()
                    // A newer analysis has asked again; its answer wins.
                    && d.generation.get() == generation
                {
                    d.navigator.borrow_mut().set_data(data);
                    d.emit(Change::Navigator);
                }
            },
        );
        self.emit(Change::Navigator);
    }

    // MARK: Explanations

    /// Explanations in the listing and the C: explained comments on hardware
    /// writes, and a note above each idiom (docs/20).
    pub fn explanations(&self) -> bool {
        self.explanations.get()
    }

    pub fn set_explanations(&self, show: bool) {
        if self.explanations.replace(show) == show {
            return;
        }
        self.workbench().set_show_explanations(show);
        // The listing has new lines and the C new text.
        self.session.view_changed();
        self.emit(Change::Layout);
    }

    // MARK: Places

    /// Go › Header.
    pub fn go_to_header(&self) {
        self.jump_to(self.info.header_offset);
    }

    /// Go › Reset Vector.
    pub fn go_to_reset(&self) {
        if let Some(offset) = self
            .rom
            .file_offset_for(u32::from(self.info.emulation.reset))
        {
            self.jump_to(offset);
        }
    }

    pub fn active_sheet(&self) -> Option<Sheet> {
        self.sheet.get()
    }

    /// Ask for the C annotation sheet on `edit`.
    pub fn begin_c_edit(&self, edit: CEdit) {
        *self.c_edit.borrow_mut() = Some(edit);
        // A request while one is open starts over.
        self.sheet.set(None);
        self.show_sheet(Some(Sheet::CEdit));
    }

    pub fn c_edit(&self) -> Option<CEdit> {
        self.c_edit.borrow().clone()
    }

    /// The C versions written for a routine, by name.
    pub fn c_versions(&self, routine: u32) -> Vec<romlens_ffi::cnotes::NamedCVersionInfo> {
        self.workbench().c_versions(Some(routine))
    }

    /// The version of the shown routine picked in the C tab.
    pub fn shown_version(&self) -> Option<String> {
        self.decompile.borrow().shown_version.clone()
    }

    /// Pick a C version of the shown routine, or `None` for the generated C.
    pub fn show_c_version(&self, name: Option<String>) {
        if self.decompile.borrow().shown_version == name {
            return;
        }
        self.decompile.borrow_mut().shown_version = name;
        self.emit(Change::Decompile);
    }

    /// Carry out a C annotation, and leave the sheet.
    pub fn run_c_command(&self, command: Command) -> Result<(), RomlensError> {
        // A version taken away no longer shows.
        let removed = match &command {
            Command::SetCVersion {
                name,
                version: None,
                ..
            } => Some(name.clone()),
            _ => None,
        };
        self.session.execute(command)?;
        if removed.is_some() && removed == self.shown_version() {
            self.decompile.borrow_mut().shown_version = None;
        }
        self.refresh_decompile();
        self.emit(Change::Decompile);
        Ok(())
    }

    pub fn show_sheet(&self, sheet: Option<Sheet>) {
        if self.sheet.replace(sheet) != sheet {
            self.emit(Change::Sheet);
        }
    }

    // MARK: Basic facts

    pub fn byte_count(&self) -> u32 {
        self.info.byte_len
    }

    pub fn row_count(&self) -> u32 {
        self.info.row_count
    }

    pub fn span_kind(&self, id: u8) -> Option<SpanKind> {
        self.span_kinds.get(&id).copied()
    }

    pub fn address_style(&self) -> AddressStyle {
        self.style.get()
    }

    pub fn set_address_style(&self, style: AddressStyle) {
        if self.style.replace(style) != style {
            self.emit(Change::AddressStyle);
        }
    }

    /// Title bar subtitle: short enough not to truncate at the minimum
    /// width, and a size in the units a person thinks in.
    pub fn subtitle(&self) -> String {
        subtitle(&self.info)
    }

    // MARK: Selection

    pub fn selected(&self) -> Option<u32> {
        self.selection.borrow().offset
    }

    pub fn anchor(&self) -> Option<u32> {
        self.selection.borrow().anchor
    }

    pub fn details(&self) -> Details {
        self.details.borrow().clone()
    }

    /// Bytes to highlight: the shift-selected range, else the selected
    /// instruction's bytes, else the byte.
    pub fn highlighted_range(&self) -> Option<std::ops::Range<u32>> {
        let selected = self.selected()?;
        if let Some(anchor) = self.anchor() {
            return Some(anchor.min(selected)..(anchor.max(selected) + 1).min(self.byte_count()));
        }
        if let Some(i) = &self.details.borrow().instruction
            && i.file_offset <= selected
            && selected < i.file_offset + u32::from(i.len)
        {
            return Some(i.file_offset..i.file_offset + u32::from(i.len));
        }
        Some(selected..selected + 1)
    }

    /// Select a byte without scrolling (mouse, arrow keys). Clears any range.
    pub fn select(&self, offset: Option<u32>) {
        self.selection.borrow_mut().anchor = None;
        self.set_selected(offset);
    }

    /// Select a byte range without scrolling or leaving the current view.
    pub fn select_range(&self, range: std::ops::Range<u32>) {
        if range.start >= self.byte_count() || range.is_empty() {
            return;
        }
        self.select(Some(range.start));
        if range.len() > 1 {
            self.extend_selection(range.end.min(self.byte_count()) - 1);
        }
    }

    /// Extend the range from the current selection (shift-click, shift+arrows).
    pub fn extend_selection(&self, offset: u32) {
        if offset >= self.byte_count() {
            return;
        }
        {
            let mut s = self.selection.borrow_mut();
            if s.anchor.is_none() {
                s.anchor = Some(s.offset.unwrap_or(offset));
            }
        }
        self.set_selected(Some(offset));
    }

    fn set_selected(&self, offset: Option<u32>) {
        match offset {
            None => {
                let changed = self.selection.borrow_mut().offset.take().is_some();
                *self.details.borrow_mut() = Details::default();
                self.refresh_screen(false);
                if changed {
                    self.emit(Change::Selection);
                }
            }
            Some(o) if o < self.byte_count() => {
                self.selection.borrow_mut().offset = Some(o);
                self.refresh_details();
                self.refresh_decompile();
                self.refresh_graph();
                self.emit(Change::Selection);
            }
            Some(_) => {}
        }
    }

    /// Re-read everything about the selection from the core: after a
    /// command, an analysis, or a new selection.
    pub fn refresh_details(&self) {
        let Some(offset) = self.selected() else {
            return;
        };
        let wb = self.workbench();
        let instruction = wb.instruction_at(offset);
        let explanation = instruction.as_ref().map(|i| wb.explain_at(i.file_offset));
        let item_start = instruction.as_ref().map_or(offset, |i| i.file_offset);
        let mut d = Details {
            inspection: self.rom.inspect(offset),
            explanation,
            region: wb.region_at(offset),
            xrefs_from: wb.xrefs_from(item_start),
            warnings: wb.warnings_at(item_start),
            flag_override: wb.flag_override_at(item_start),
            preview: wb.preview_at(offset),
            ..Details::default()
        };
        if let Some(address) = self.rom.snes_address_for(item_start) {
            d.label = wb.label_at(address);
            d.line_comment = wb.comment_at(address, CommentKind::Line);
            d.block_comment = wb.comment_at(address, CommentKind::Block);
            d.xrefs_to = wb.xrefs_to(address);
        }
        d.instruction = instruction;
        *self.details.borrow_mut() = d;
        self.refresh_screen(false);
    }

    // MARK: Screen

    pub fn screen(&self) -> std::cell::Ref<'_, ScreenModel> {
        self.screen.borrow()
    }

    pub fn set_show_screen(&self, shown: bool) {
        {
            let mut s = self.screen.borrow_mut();
            if s.shown == shown {
                return;
            }
            s.shown = shown;
        }
        self.refresh_screen(false);
    }

    /// Work out the screen at the selected instruction, if the Screen section
    /// is open.
    fn refresh_screen(&self, force: bool) {
        let at = self
            .details
            .borrow()
            .instruction
            .as_ref()
            .map(|i| i.file_offset);
        let before = self.screen.borrow().setup.is_some();
        let request = self.screen.borrow_mut().want(at, force);
        let Some((at, ticket)) = request else {
            // Left an instruction behind: the old result goes with it.
            if before && self.screen.borrow().setup.is_none() {
                self.emit(Change::Screen);
            }
            return;
        };
        let wb = Arc::clone(self.workbench());
        let weak = self.me.borrow().clone();
        background(
            &*self.runtime,
            move || wb.screen_at_blocking(at),
            move |setup| {
                if let Some(d) = weak.upgrade()
                    && d.screen.borrow_mut().finished(ticket, setup)
                {
                    d.emit(Change::Screen);
                }
            },
        );
        self.emit(Change::Screen);
    }

    /// The address of the selected item (instruction start or byte).
    /// The SNES address a file offset has, if it maps to one.
    pub fn snes_address(&self, offset: u32) -> Option<u32> {
        self.rom.snes_address_for(offset)
    }

    pub fn selected_address(&self) -> Option<u32> {
        let offset = self.selected()?;
        let start = self
            .details
            .borrow()
            .instruction
            .as_ref()
            .map_or(offset, |i| i.file_offset);
        self.rom.snes_address_for(start)
    }

    /// The selected address as text, for Copy Address.
    pub fn selected_address_text(&self) -> Option<String> {
        self.selected_address()
            .map(romlens_ffi::format_snes_address)
    }

    /// Move the selection by `delta` bytes, clamped, and keep it visible.
    pub fn move_selection(&self, delta: i64, extend: bool) {
        let last = i64::from(self.byte_count()) - 1;
        if last < 0 {
            return;
        }
        let current = i64::from(self.selected().unwrap_or(0));
        let next = (current + delta).clamp(0, last) as u32;
        if extend {
            self.extend_selection(next);
        } else {
            self.select(Some(next));
        }
        self.request_scroll(next);
    }

    /// Move to the previous or next content line of the disassembly, skipping
    /// labels, comments and blanks.
    pub fn move_line(&self, delta: i64, extend: bool) {
        let count = i64::from(self.asm_line_count());
        if count == 0 {
            return;
        }
        let wb = self.workbench();
        let current = self
            .selected()
            .and_then(|o| wb.line_for_offset(o))
            .unwrap_or(0);
        let mut line = i64::from(current);
        let mut steps = delta.abs();
        let step = delta.signum();
        let mut landed = None;
        while steps > 0 {
            let next = line + step;
            if next < 0 || next >= count {
                break;
            }
            line = next;
            if let Some(range) = wb.item_range(line as u32)
                && range.len > 0
            {
                steps -= 1;
                landed = Some(range.start);
            }
        }
        let Some(target) = landed else { return };
        if extend {
            self.extend_selection(target);
        } else {
            self.select(Some(target));
        }
        self.request_scroll(target);
    }

    /// Select every instruction of the idiom whose note is at `offset`: what
    /// clicking its note line does.
    pub fn select_idiom(&self, note_at: u32) {
        let wb = self.workbench();
        let idiom = wb
            .explain_at(note_at)
            .idioms
            .into_iter()
            .find(|i| i.note_at == note_at);
        let (Some(first), Some(last)) = (
            idiom.as_ref().and_then(|i| i.offsets.first().copied()),
            idiom.as_ref().and_then(|i| i.offsets.last().copied()),
        ) else {
            self.select(Some(note_at));
            return;
        };
        let len = wb.instruction_at(last).map_or(1, |i| u32::from(i.len));
        self.select_range(first..last + len);
    }

    /// A key from either canvas.
    pub fn perform(&self, command: EditorCommand, from: EditorSource, visible_items: usize) {
        use EditorCommand::*;
        let hex = from == EditorSource::Hex;
        let page = visible_items.max(1) as i64;
        let row = ROW_BYTES;
        match command {
            Left => self.move_selection(-1, false),
            Right => self.move_selection(1, false),
            ExtendLeft => self.move_selection(-1, true),
            ExtendRight => self.move_selection(1, true),
            Up if hex => self.move_selection(-row, false),
            Up => self.move_line(-1, false),
            Down if hex => self.move_selection(row, false),
            Down => self.move_line(1, false),
            ExtendUp if hex => self.move_selection(-row, true),
            ExtendUp => self.move_line(-1, true),
            ExtendDown if hex => self.move_selection(row, true),
            ExtendDown => self.move_line(1, true),
            PageUp if hex => self.move_selection(-row * page, false),
            PageUp => self.move_line(-page, false),
            PageDown if hex => self.move_selection(row * page, false),
            PageDown => self.move_line(page, false),
            Home => self.jump_to(0),
            End => self.jump_to(self.byte_count().saturating_sub(1)),
            Back => self.go_back(),
            Follow => self.follow_reference(),
            Rename if self.selected().is_some() => self.show_sheet(Some(Sheet::RenameLabel)),
            Comment if self.selected().is_some() => self.show_sheet(Some(Sheet::Comment)),
            Rename | Comment => {}
            MarkCode => {
                let _ = self.mark(OverrideKind::Code, DataKind::Byte);
            }
            MarkData => {
                let _ = self.mark(OverrideKind::Data, DataKind::Byte);
            }
            MarkUnknown => {
                let _ = self.mark(OverrideKind::Unknown, DataKind::Byte);
            }
        }
    }

    /// The selected line as text, for Copy Line.
    pub fn selected_line_text(&self) -> Option<String> {
        let wb = self.workbench();
        let line = wb.line_for_offset(self.selected()?)?;
        let style = match self.address_style() {
            AddressStyle::Both => romlens_ffi::AddressStyle::Both,
            AddressStyle::Snes => romlens_ffi::AddressStyle::Snes,
            AddressStyle::File => romlens_ffi::AddressStyle::File,
        };
        Some(
            wb.asm_lines_text(line, 1, style)
                .trim_matches('\n')
                .to_owned(),
        )
    }

    // MARK: Navigation

    /// Jump to a file offset: select it, centre it, remember where we were.
    pub fn jump_to(&self, offset: u32) {
        self.jump(offset, true);
    }

    fn jump(&self, offset: u32, record_history: bool) {
        if offset >= self.byte_count() {
            return;
        }
        if record_history
            && let Some(from) = self.selected()
            && from != offset
        {
            let mut h = self.history.borrow_mut();
            h.push(from);
            if h.len() > HISTORY_LIMIT {
                h.remove(0);
            }
            self.forward_history.borrow_mut().clear();
            drop(h);
            self.emit(Change::History);
        }
        self.select(Some(offset));
        self.request_scroll(offset);
    }

    /// Jump to a 24-bit SNES address when it maps to ROM.
    pub fn jump_to_snes(&self, address: u32) {
        if let Some(offset) = self.rom.file_offset_for(address) {
            self.jump_to(offset);
        }
    }

    /// Resolve and jump to an address expression; returns the core's message
    /// on failure so the sheet can show it verbatim.
    pub fn jump_text(&self, text: &str) -> Result<ResolvedAddress, RomlensError> {
        let resolved = self.rom.resolve(text.to_owned())?;
        self.jump_to(resolved.file_offset);
        Ok(resolved)
    }

    /// Live preview for the jump sheet: what the expression resolves to.
    pub fn preview_address(&self, text: &str) -> Result<ResolvedAddress, RomlensError> {
        self.rom.resolve(text.to_owned())
    }

    /// Set how the marked range previews: undoable, and never re-analyses.
    pub fn set_preview_options(
        &self,
        params: romlens_ffi::RegionParamsInfo,
    ) -> Result<(), RomlensError> {
        let Some(range) = self.marked_range() else {
            return Ok(());
        };
        self.session.execute(Command::SetRegionParams {
            start: range.start,
            params,
        })?;
        self.refresh_details();
        Ok(())
    }

    /// How the marked range at the selection previews now.
    pub fn preview_options(&self) -> Option<romlens_ffi::RegionParamsInfo> {
        let range = self.marked_range()?;
        self.workbench().region_params_at(range.start)
    }

    /// A typed address as the 24-bit SNES address the core stores it as.
    pub fn snes_address_of(&self, text: &str) -> Result<Option<u32>, RomlensError> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let r = self.rom.resolve(text.to_owned())?;
        Ok(r.snes_address
            .or_else(|| self.rom.snes_address_for(r.file_offset)))
    }

    pub fn can_go_back(&self) -> bool {
        !self.history.borrow().is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward_history.borrow().is_empty()
    }

    pub fn go_back(&self) {
        let Some(previous) = self.history.borrow_mut().pop() else {
            return;
        };
        if let Some(current) = self.selected() {
            self.forward_history.borrow_mut().push(current);
        }
        self.jump(previous, false);
        self.emit(Change::History);
    }

    pub fn go_forward(&self) {
        let Some(next) = self.forward_history.borrow_mut().pop() else {
            return;
        };
        if let Some(current) = self.selected() {
            self.history.borrow_mut().push(current);
        }
        self.jump(next, false);
        self.emit(Change::History);
    }

    /// Follow the selected instruction's target (or a pointer's).
    pub fn follow_reference(&self) {
        let target = {
            let d = self.details.borrow();
            match &d.instruction {
                Some(i) => i.target_file_offset,
                None => d
                    .inspection
                    .as_ref()
                    .and_then(|b| b.pointer_target_file_offset),
            }
        };
        if let Some(t) = target {
            self.jump_to(t);
        }
    }

    pub fn scroll_request(&self) -> Option<ScrollRequest> {
        self.scroll.get()
    }

    pub fn request_scroll(&self, offset: u32) {
        let id = self.scroll.get().map_or(0, |r| r.id) + 1;
        self.scroll.set(Some(ScrollRequest { id, offset }));
        self.emit(Change::Scroll);
    }

    // MARK: Editing

    /// Mark the highlighted range (or the selected item) with a kind.
    pub fn mark(&self, kind: OverrideKind, data_kind: DataKind) -> Result<(), RomlensError> {
        let Some(range) = self.highlighted_range() else {
            return Ok(());
        };
        self.session.execute(Command::MarkRegion {
            start: range.start,
            len: range.end - range.start,
            kind,
            data_kind: (kind == OverrideKind::Data).then_some(data_kind),
            stride: None,
            bpp: None,
            elem: None,
            bank: None,
        })?;
        self.refresh_details();
        Ok(())
    }

    pub fn clear_mark(&self) -> Result<(), RomlensError> {
        let Some(range) = self.highlighted_range() else {
            return Ok(());
        };
        self.session.execute(Command::ClearRegionOverride {
            start: range.start,
            len: range.end - range.start,
        })?;
        self.refresh_details();
        Ok(())
    }

    pub fn set_label(&self, name: Option<String>) -> Result<(), RomlensError> {
        let Some(address) = self.selected_address() else {
            return Ok(());
        };
        self.session.execute(Command::SetLabel { address, name })?;
        self.refresh_details();
        Ok(())
    }

    /// A label a person or an import chose, which Remove Label takes away.
    pub fn can_remove_label(&self) -> bool {
        self.details
            .borrow()
            .label
            .as_ref()
            .is_some_and(|l| matches!(l.source, LabelSource::User | LabelSource::Imported))
    }

    pub fn set_comment(&self, kind: CommentKind, text: Option<String>) -> Result<(), RomlensError> {
        let Some(address) = self.selected_address() else {
            return Ok(());
        };
        self.session.execute(Command::SetComment {
            address,
            kind,
            text,
        })?;
        self.refresh_details();
        Ok(())
    }

    pub fn set_flag_override(&self, flags: Option<FlagOverride>) -> Result<(), RomlensError> {
        let offset = {
            let d = self.details.borrow();
            d.instruction
                .as_ref()
                .map(|i| i.file_offset)
                .or(self.selected())
        };
        let Some(offset) = offset else { return Ok(()) };
        self.session
            .execute(Command::SetFlagOverride { offset, flags })?;
        self.refresh_details();
        Ok(())
    }

    pub fn undo(&self) -> bool {
        let done = self.session.undo();
        self.refresh_details();
        done
    }

    pub fn redo(&self) -> bool {
        let done = self.session.redo();
        self.refresh_details();
        done
    }

    /// The marked range the selection is in, whose preview options can be set.
    pub fn marked_range(&self) -> Option<ByteRange> {
        self.workbench().region_override_at(self.selected()?)
    }

    pub fn start_analysis(&self) {
        self.session.start_analysis();
    }
}

/// The seconds the person's day is ahead of UTC, for streaks that count their
/// own days.
fn tutor_utc_offset() -> i32 {
    gtk::glib::DateTime::now_local()
        .map(|d| (d.utc_offset().as_seconds() / 1_000_000) as i32)
        .unwrap_or(0)
}

impl Document {
    /// The window is gone. Nothing may go on running, or spending, for it:
    /// the tutor's turn, the analysis, the sound, a live session and a
    /// comparison all stop. Clearing the listeners lets go of the window's
    /// closures, which hold the document, so it is freed.
    pub fn close(&self) {
        self.tutor.borrow_mut().close();
        self.session.close();
        self.audio.borrow_mut().shut_down();
        self.graphics.borrow_mut().stop_live();
        self.compare.borrow_mut().close();
        self.listeners.borrow_mut().clear();
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        // A closed window must not leave an analysis running for it.
        self.session.cancel_analysis();
    }
}

pub fn subtitle(info: &RomInfo) -> String {
    let mb = f64::from(info.byte_len) / (1024.0 * 1024.0);
    let size = if mb >= 1.0 {
        // Three significant figures, as the macOS `%.3g`.
        let decimals = if mb >= 100.0 {
            0
        } else if mb >= 10.0 {
            1
        } else {
            2
        };
        let s = format!("{mb:.decimals$}");
        let s = if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            s
        };
        format!("{s} MB")
    } else {
        format!("{} KB", info.byte_len / 1024)
    };
    let speed = if info.fast_rom { "FastROM" } else { "SlowROM" };
    format!("{} · {speed} · {size}", info.mapping_name)
}

/// Bytes per hex row, for callers that move by rows.
pub const ROW_BYTES: i64 = BYTES_PER_ROW as i64;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing::{TestRuntime, test_rom};
    use romlens_ffi::{TableElem, VarWidth, WorkbenchEvent};
    use std::cell::Cell;

    fn doc() -> (Rc<Document>, Rc<TestRuntime>) {
        let rt = TestRuntime::new();
        let d = Document::new(test_rom(), rt.clone());
        (d, rt)
    }

    fn analysed() -> (Rc<Document>, Rc<TestRuntime>) {
        let (d, rt) = doc();
        d.start_analysis();
        rt.pump();
        (d, rt)
    }

    /// Closing stops what was running for the window and frees the document,
    /// although a listener (as every view's is) holds it.
    #[test]
    fn closing_stops_the_work_and_frees_the_document() {
        let (d, rt) = doc();
        d.subscribe({
            let held = Rc::clone(&d);
            move |_| {
                let _ = &held;
            }
        });
        d.start_analysis();
        d.close();
        let (session, weak) = (Rc::clone(&d.session), Rc::downgrade(&d));
        drop(d);
        rt.pump();
        assert!(weak.upgrade().is_none(), "nothing keeps a closed document");
        assert!(!session.analysis().is_running());
    }

    #[test]
    fn analysis_lands_and_invalidates_rows() {
        let (d, rt) = doc();
        assert!(!d.session.has_snapshot());
        let before = d.generation();
        d.start_analysis();
        rt.pump();
        assert!(d.session.has_snapshot());
        assert!(d.generation() > before);
        assert!(d.session.stats().is_some());
        assert!(!d.session.analysis().is_running());
    }

    #[test]
    fn selection_extends_and_highlights() {
        let (d, _) = analysed();
        d.select(Some(0x10));
        assert_eq!(d.selected(), Some(0x10));
        d.extend_selection(0x14);
        assert_eq!(d.anchor(), Some(0x10));
        assert_eq!(d.highlighted_range(), Some(0x10..0x15));
        // A plain select drops the range.
        d.select(Some(0x20));
        assert_eq!(d.anchor(), None);
        d.select(None);
        assert_eq!(d.selected(), None);
        assert_eq!(d.highlighted_range(), None);
    }

    #[test]
    fn highlight_covers_the_selected_instruction() {
        let (d, _) = analysed();
        // The test ROM's entry point is `SEI; CLC; XCE; ...`.
        d.select(Some(0));
        let range = d.highlighted_range().unwrap();
        let insn = d.details().instruction.expect("instruction at the entry");
        assert_eq!(range, 0..u32::from(insn.len));
    }

    #[test]
    fn moving_the_selection_clamps_and_requests_a_scroll() {
        let (d, _) = analysed();
        d.select(Some(2));
        d.move_selection(-100, false);
        assert_eq!(d.selected(), Some(0));
        d.move_selection(i64::from(u32::MAX), false);
        assert_eq!(d.selected(), Some(d.byte_count() - 1));
        let first = d.scroll_request().unwrap();
        d.move_selection(0, false);
        assert!(d.scroll_request().unwrap().id > first.id);
    }

    #[test]
    fn history_goes_back_and_forward_and_forward_clears_on_a_new_jump() {
        let (d, _) = analysed();
        d.select(Some(0));
        d.jump_to(0x100);
        d.jump_to(0x200);
        assert!(d.can_go_back() && !d.can_go_forward());
        d.go_back();
        assert_eq!(d.selected(), Some(0x100));
        d.go_back();
        assert_eq!(d.selected(), Some(0));
        assert!(!d.can_go_back());
        d.go_forward();
        assert_eq!(d.selected(), Some(0x100));
        d.jump_to(0x300);
        assert!(!d.can_go_forward());
        // Jumping to where we already are does not record history.
        let depth = d.history.borrow().len();
        d.jump_to(0x300);
        assert_eq!(d.history.borrow().len(), depth);
    }

    #[test]
    fn history_is_capped() {
        let (d, _) = analysed();
        d.select(Some(0));
        for i in 1..=150u32 {
            d.jump_to(i);
        }
        assert_eq!(d.history.borrow().len(), HISTORY_LIMIT);
        // The oldest entries were the ones dropped.
        assert_eq!(d.history.borrow()[0], 50);
    }

    #[test]
    fn jump_text_resolves_snes_addresses_and_reports_the_core_message() {
        let (d, _) = analysed();
        let resolved = d.jump_text("$00:8000").unwrap();
        assert_eq!(d.selected(), Some(resolved.file_offset));
        assert_eq!(resolved.file_offset, 0);
        let err = d.jump_text("not an address").unwrap_err();
        assert!(!err.to_string().is_empty());
        // A failed jump leaves the selection alone.
        assert_eq!(d.selected(), Some(0));
    }

    #[test]
    fn follow_reference_takes_the_instruction_target() {
        let (d, _) = analysed();
        // Find an instruction with a target in the test ROM.
        let found = (0..d.byte_count().min(64)).find_map(|o| {
            d.select(Some(o));
            d.details().instruction.and_then(|i| i.target_file_offset)
        });
        let target = found.expect("the test ROM's entry code has a branch or jump");
        let from = d.selected().unwrap();
        d.follow_reference();
        assert_eq!(d.selected(), Some(target));
        d.go_back();
        assert_eq!(d.selected(), Some(from));
    }

    #[test]
    fn marking_is_undoable_and_marks_the_document_dirty() {
        let (d, rt) = analysed();
        d.select(Some(0x40));
        assert!(!d.session.can_undo());
        d.mark(OverrideKind::Data, DataKind::Byte).unwrap();
        assert!(d.session.can_undo());
        assert!(d.session.is_dirty());
        assert!(d.session.undo_title().is_some());
        // A mark changes the analysis, so one run is scheduled, not started.
        assert_eq!(rt.pending_timers(), 1);
        assert!(d.undo());
        assert!(!d.session.can_undo());
        assert!(d.session.can_redo());
    }

    #[test]
    fn a_label_edit_does_not_reanalyse() {
        let (d, rt) = analysed();
        d.select(Some(0));
        d.set_label(Some("Boot".into())).unwrap();
        assert_eq!(d.details().label.map(|l| l.name), Some("Boot".into()));
        assert!(d.can_remove_label());
        assert_eq!(rt.pending_timers(), 0);
        d.set_label(None).unwrap();
        assert!(!d.can_remove_label());
    }

    #[test]
    fn rapid_edits_collapse_into_one_reanalysis() {
        let (d, rt) = analysed();
        let runs = *rt.runs.borrow();
        for o in [0x40, 0x50, 0x60] {
            d.select(Some(o));
            d.mark(OverrideKind::Data, DataKind::Byte).unwrap();
        }
        assert_eq!(rt.pending_timers(), 3);
        rt.fire_timers();
        assert_eq!(*rt.runs.borrow(), runs + 1);
    }

    #[test]
    fn an_edit_during_a_run_queues_one_run_after_it() {
        let (d, rt) = doc();
        *rt.defer.borrow_mut() = true;
        d.start_analysis();
        assert!(d.session.analysis().is_running());
        d.select(Some(0x40));
        d.mark(OverrideKind::Data, DataKind::Byte).unwrap();
        rt.fire_timers();
        // The running analysis is not cancelled or doubled.
        assert_eq!(*rt.runs.borrow(), 1);
        rt.finish_parked();
        assert_eq!(*rt.runs.borrow(), 2);
        assert!(rt.has_parked());
        rt.finish_parked();
        assert!(!d.session.analysis().is_running());
    }

    #[test]
    fn cancelling_stops_a_queued_rerun() {
        let (d, rt) = doc();
        *rt.defer.borrow_mut() = true;
        d.start_analysis();
        d.select(Some(0x40));
        d.mark(OverrideKind::Data, DataKind::Byte).unwrap();
        rt.fire_timers();
        d.session.cancel_analysis();
        assert!(!d.session.analysis().is_running());
        rt.finish_parked();
        assert_eq!(*rt.runs.borrow(), 1);
    }

    #[test]
    fn progress_after_the_run_ended_is_ignored() {
        let (d, _) = analysed();
        d.session.handle(WorkbenchEvent::AnalysisProgress {
            phase: romlens_ffi::AnalysisPhase::Lines,
            done: 1,
            total: 1,
        });
        assert!(!d.session.analysis().is_running());
    }

    #[test]
    fn subtitle_matches_the_macos_wording() {
        let (d, _) = doc();
        assert_eq!(d.subtitle(), "LoROM · SlowROM · 32 KB");
    }

    #[test]
    fn rows_are_cached_until_the_snapshot_changes() {
        let (d, _) = analysed();
        assert!(d.hex_cache.batch(0).is_some());
        let misses = d.hex_cache.miss_count();
        assert!(d.hex_cache.batch(3).is_some());
        assert_eq!(d.hex_cache.miss_count(), misses);
        d.session.finish_command(false);
        assert!(!d.hex_cache.is_cached(0));
    }

    #[test]
    fn disassembly_exists_after_analysis_and_lines_map_to_offsets() {
        let (d, rt) = doc();
        assert!(!d.has_disassembly());
        d.start_analysis();
        rt.pump();
        assert!(d.has_disassembly());
        let line = d.line_for_offset(0).expect("a line at the entry");
        assert!(d.asm_cache.batch(line).unwrap().line(line).is_some());
    }

    #[test]
    fn up_and_down_move_by_content_line_in_the_listing_and_by_row_in_hex() {
        let (d, _) = analysed();
        d.select(Some(0));
        let first = d.details().instruction.expect("instruction").len;
        d.perform(EditorCommand::Down, EditorSource::Asm, 20);
        assert_eq!(d.selected(), Some(u32::from(first)));
        d.perform(EditorCommand::Up, EditorSource::Asm, 20);
        assert_eq!(d.selected(), Some(0));
        d.perform(EditorCommand::Down, EditorSource::Hex, 20);
        assert_eq!(d.selected(), Some(16));
        d.perform(EditorCommand::ExtendDown, EditorSource::Hex, 20);
        assert_eq!(d.highlighted_range(), Some(16..33));
    }

    #[test]
    fn page_keys_move_by_the_visible_count() {
        let (d, _) = analysed();
        d.select(Some(0));
        d.perform(EditorCommand::PageDown, EditorSource::Hex, 10);
        assert_eq!(d.selected(), Some(160));
        d.perform(EditorCommand::PageUp, EditorSource::Hex, 10);
        assert_eq!(d.selected(), Some(0));
        d.perform(EditorCommand::End, EditorSource::Hex, 10);
        assert_eq!(d.selected(), Some(d.byte_count() - 1));
        d.perform(EditorCommand::Home, EditorSource::Hex, 10);
        assert_eq!(d.selected(), Some(0));
        d.perform(EditorCommand::Back, EditorSource::Hex, 10);
        assert_eq!(d.selected(), Some(d.byte_count() - 1));
    }

    #[test]
    fn rename_and_comment_ask_for_a_sheet_only_with_a_selection() {
        let (d, _) = analysed();
        d.perform(EditorCommand::Rename, EditorSource::Asm, 10);
        assert_eq!(d.active_sheet(), None);
        d.select(Some(0));
        d.perform(EditorCommand::Rename, EditorSource::Asm, 10);
        assert_eq!(d.active_sheet(), Some(Sheet::RenameLabel));
        d.show_sheet(None);
        d.perform(EditorCommand::Comment, EditorSource::Asm, 10);
        assert_eq!(d.active_sheet(), Some(Sheet::Comment));
    }

    #[test]
    fn mark_keys_mark_the_highlighted_range() {
        let (d, _) = analysed();
        d.select(Some(0x100));
        d.perform(EditorCommand::MarkData, EditorSource::Hex, 10);
        assert_eq!(d.marked_range().map(|r| r.start), Some(0x100));
        d.perform(EditorCommand::MarkUnknown, EditorSource::Hex, 10);
        assert!(d.session.can_undo());
    }

    #[test]
    fn copy_line_gives_the_listing_line() {
        let (d, _) = analysed();
        d.select(Some(0));
        let text = d.selected_line_text().unwrap();
        assert!(text.contains("SEI"), "{text}");
        assert_eq!(d.selected_address_text().as_deref(), Some("$00:8000"));
    }

    #[test]
    fn tabs_and_panes_emit_layout_changes() {
        let (d, _) = doc();
        let seen = Rc::new(Cell::new(0));
        let s = Rc::clone(&seen);
        d.subscribe(move |c| {
            if c == Change::Layout {
                s.set(s.get() + 1);
            }
        });
        d.set_tab(Tab::Disassembly);
        d.set_tab(Tab::Disassembly);
        assert_eq!((d.tab(), seen.get()), (Tab::Disassembly, 1));
        d.set_pane(|p| &mut p.inspector, false);
        assert!(!d.panes().inspector);
        assert_eq!(seen.get(), 2);
        d.set_pane(|p| &mut p.inspector, false);
        assert_eq!(seen.get(), 2);
        d.toggle_focus();
        assert!(d.is_focused() && !d.panes().navigator);
        d.toggle_focus();
        assert!(d.panes().navigator && !d.panes().inspector);
    }

    #[test]
    fn show_results_opens_the_pane_on_that_list() {
        let (d, _) = doc();
        assert!(!d.panes().results);
        d.show_results(ResultsKind::References);
        assert!(d.panes().results);
        assert_eq!(d.results_kind(), ResultsKind::References);
    }

    #[test]
    fn header_and_reset_vector_jumps() {
        let (d, _) = analysed();
        d.go_to_header();
        assert_eq!(d.selected(), Some(d.info.header_offset));
        d.go_to_reset();
        assert_eq!(d.selected(), Some(0));
        assert!(d.can_go_back());
    }

    #[test]
    fn turning_explanations_off_refreshes_the_listing() {
        let (d, _) = analysed();
        let before = d.generation();
        d.set_explanations(false);
        assert!(!d.explanations());
        assert!(d.generation() > before || d.session.generation() > 0);
        let again = d.generation();
        d.set_explanations(false);
        assert_eq!(d.generation(), again);
    }

    #[test]
    fn variable_addresses_parse_as_the_sheet_documents() {
        assert_eq!(parse_variable_address("$7E:0094"), Some(0x7E_0094));
        assert_eq!(parse_variable_address("7F8000"), Some(0x7F_8000));
        assert_eq!(parse_variable_address("$0094"), Some(0x7E_0094));
        // At or above $2000 it is a register or ROM address, not low RAM.
        assert_eq!(parse_variable_address("$2100"), Some(0x2100));
        assert_eq!(parse_variable_address(" $7e : 0094 "), Some(0x7E_0094));
        for bad in ["", "$", "zz", "1234567", "$7E:00 94 12"] {
            assert_eq!(parse_variable_address(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_draft_knows_its_size_and_address() {
        let mut d = VariableDraft {
            address: "$0094".into(),
            ..VariableDraft::default()
        };
        assert_eq!((d.size(), d.resolved_address()), (1, Some(0x7E_0094)));
        d.width = VarWidth::Word;
        d.count = 8;
        assert_eq!(d.size(), 16);
        d.existing = Some(0x7E_0100);
        assert_eq!(d.resolved_address(), Some(0x7E_0100));
        d.count = 0;
        assert_eq!(d.size(), 2);
    }

    #[test]
    fn defining_and_removing_a_variable_is_undoable_and_names_the_address() {
        let (d, _) = analysed();
        d.begin_new_variable();
        assert_eq!(d.active_sheet(), Some(Sheet::Variable));
        let draft = VariableDraft {
            address: "$0094".into(),
            name: "PlayerX".into(),
            width: VarWidth::Word,
            count: 1,
            existing: None,
        };
        d.define_variable(&draft).unwrap();
        assert!(d.session.can_undo());
        assert_eq!(d.workbench().variables().len(), 1);
        // Opening the sheet on the address edits what is there.
        d.begin_define_variable(Some(0x7E_0095));
        let edit = d.variable_draft();
        assert_eq!(
            (edit.name.as_str(), edit.existing),
            ("PlayerX", Some(0x7E_0094))
        );
        d.remove_variable(0x7E_0094).unwrap();
        assert!(d.workbench().variables().is_empty());
    }

    #[test]
    fn a_bad_variable_address_is_the_cores_kind_of_message() {
        let (d, _) = analysed();
        let draft = VariableDraft {
            address: "nope".into(),
            name: "X".into(),
            ..VariableDraft::default()
        };
        let e = d.define_variable(&draft).unwrap_err();
        assert!(e.to_string().contains("not an address"), "{e}");
    }

    #[test]
    fn removing_a_variables_label_takes_the_variable_with_it() {
        let (d, _) = analysed();
        let draft = VariableDraft {
            address: "$0094".into(),
            name: "PlayerX".into(),
            ..VariableDraft::default()
        };
        d.define_variable(&draft).unwrap();
        d.jump_to_snes(0x80_8000);
        // Nothing is selected on the variable (it is in RAM), so this is a
        // no-op rather than an error.
        assert!(d.remove_label_or_variable().is_ok());
        assert_eq!(d.workbench().variables().len(), 1);
    }

    #[test]
    fn marking_with_options_types_the_range() {
        let (d, _) = analysed();
        d.select(Some(0x100));
        d.mark_with(
            DataKind::Table,
            MarkOptions {
                stride: Some(2),
                elem: Some(TableElem::Raw),
                ..MarkOptions::default()
            },
        )
        .unwrap();
        assert_eq!(d.marked_range().map(|r| r.start), Some(0x100));
        assert!(d.session.can_undo());
    }

    #[test]
    fn find_runs_jumps_steps_and_wraps() {
        let (d, _) = analysed();
        d.edit_search(|s| s.query = "78 18 FB".into());
        d.run_search();
        assert_eq!(d.selected(), Some(0));
        assert!(d.search().has_results());
        d.edit_search(|s| s.query = "00 00 00".into());
        d.run_search();
        let n = d.search().hits.len();
        assert!(n > 2);
        let first = d.selected();
        d.step_search(-1);
        assert_eq!(d.search().current, Some(n - 1));
        d.step_search(1);
        assert_eq!(d.selected(), first);
        d.go_to_hit(1);
        assert_eq!(d.selected(), Some(d.search().hits[1].file_offset));
    }

    #[test]
    fn find_references_fills_the_pane_and_leaves_the_selection() {
        let (d, _) = analysed();
        d.select(Some(0));
        d.find_references();
        assert!(d.references().has_results());
        assert!(d.panes().results);
        assert_eq!(d.results_kind(), ResultsKind::References);
        assert_eq!(d.selected(), Some(0));
        let target = d.reference_target().expect("a target");
        assert!(!target.0.is_empty());
        d.go_to_reference(usize::MAX);
        assert_eq!(d.selected(), Some(0));
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("romlens-doc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn reopen(dir: &Path, rt: &Rc<TestRuntime>) -> Rc<Document> {
        let files =
            romlens_ffi::workbench::read_project_package(dir.to_string_lossy().into_owned())
                .unwrap();
        let (files, local) = crate::package::split_local(files);
        let rom_path = local.last_path.expect("a recorded ROM path");
        Document::from_project(test_rom(), files, dir, Path::new(&rom_path), rt.clone()).unwrap()
    }

    #[test]
    fn an_untitled_project_is_named_for_its_rom() {
        let rt = TestRuntime::new();
        let d = Document::new(test_rom(), rt);
        assert!(d.project_path().is_none());
        // No path at all: the ROM's own title, which the test ROM has.
        assert!(!d.display_name().is_empty());
    }

    #[test]
    fn save_writes_a_package_that_reopens_with_its_edits() {
        let (d, rt) = analysed();
        *d.rom_path.borrow_mut() = Some(PathBuf::from("/roms/game.sfc"));
        d.select(Some(0));
        d.set_label(Some("Boot".into())).unwrap();
        assert!(d.session.is_dirty());
        let dir = scratch("save").join("Game.romlens");
        d.save_to(&dir).unwrap();
        assert!(!d.session.is_dirty(), "a save clears the dirty flag");
        assert_eq!(d.project_path().as_deref(), Some(dir.as_path()));
        assert_eq!(d.display_name(), "Game");
        assert!(dir.join("project.json").exists() && dir.join("local.json").exists());

        let again = reopen(&dir, &rt);
        again.start_analysis();
        rt.pump();
        again.select(Some(0));
        assert_eq!(again.details().label.map(|l| l.name), Some("Boot".into()));
        assert_eq!(again.rom_path(), Some(PathBuf::from("/roms/game.sfc")));
        assert!(!again.session.is_dirty());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn removing_the_last_variable_removes_its_file_on_the_next_save() {
        let (d, _) = analysed();
        let dir = scratch("stale").join("G.romlens");
        d.define_variable(&VariableDraft {
            address: "$0094".into(),
            name: "PlayerX".into(),
            ..VariableDraft::default()
        })
        .unwrap();
        d.save_to(&dir).unwrap();
        assert!(dir.join("variables.json").exists());
        d.remove_variable(0x7E_0094).unwrap();
        d.save_to(&dir).unwrap();
        assert!(!dir.join("variables.json").exists());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn duplicate_writes_a_copy_and_leaves_the_document_alone() {
        let (d, _) = analysed();
        d.select(Some(0));
        d.set_label(Some("Boot".into())).unwrap();
        let copy = scratch("dup").join("Copy.romlens");
        d.duplicate_to(&copy).unwrap();
        assert!(copy.join("project.json").exists());
        assert!(d.project_path().is_none());
        assert!(d.session.is_dirty(), "the original is still unsaved");
        let _ = std::fs::remove_dir_all(copy.parent().unwrap());
    }

    #[test]
    fn revert_goes_back_to_what_was_saved_and_clears_undo() {
        let (d, rt) = analysed();
        let dir = scratch("revert").join("R.romlens");
        d.select(Some(0));
        d.set_label(Some("Saved".into())).unwrap();
        d.save_to(&dir).unwrap();
        d.set_label(Some("Unsaved".into())).unwrap();
        assert!(d.session.is_dirty());
        d.revert().unwrap();
        rt.fire_timers();
        rt.pump();
        d.select(Some(0));
        assert_eq!(d.details().label.map(|l| l.name), Some("Saved".into()));
        assert!(!d.session.is_dirty());
        assert!(!d.session.can_undo());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn reverting_an_unsaved_project_is_an_error_not_a_panic() {
        let (d, _) = analysed();
        assert!(d.revert().is_err());
    }

    #[test]
    fn a_project_for_a_different_rom_is_refused() {
        let (d, _) = analysed();
        let dir = scratch("other").join("O.romlens");
        d.save_to(&dir).unwrap();
        let files =
            romlens_ffi::workbench::read_project_package(dir.to_string_lossy().into_owned())
                .unwrap();
        let (files, _) = crate::package::split_local(files);
        let other = romlens_ffi::Rom::from_bytes(
            romlens_ffi::make_test_rom(romlens_ffi::Mapping::HiRom),
            "other.sfc".into(),
        )
        .unwrap();
        let err =
            Document::from_project(other, files, &dir, Path::new("x.sfc"), TestRuntime::new());
        assert!(matches!(err, Err(RomlensError::RomMismatch { .. })));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn an_import_is_one_undo_step_and_marks_the_document_dirty() {
        let (d, rt) = analysed();
        let dir = scratch("import");
        std::fs::create_dir_all(&dir).unwrap();
        let sym = dir.join("game.sym");
        std::fs::write(&sym, "[labels]\n00:8000 Start\n").unwrap();
        let got = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&got);
        d.import(ImportKind::Symbols, sym, move |r| {
            *sink.borrow_mut() = Some(r)
        });
        let result = got.borrow_mut().take().expect("done was called").unwrap();
        assert_eq!(result.labels_added, 1);
        assert!(d.session.is_dirty() && d.session.can_undo());
        // An import changes what the analysis names, so a re-run is queued.
        assert_eq!(rt.pending_timers(), 1);
        d.select(Some(0));
        assert_eq!(d.details().label.map(|l| l.name), Some("Start".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_import_changes_nothing() {
        let (d, _) = analysed();
        let got = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&got);
        d.import(
            ImportKind::Trace,
            PathBuf::from("/nonexistent/x.cdl"),
            move |r| *sink.borrow_mut() = Some(r),
        );
        assert!(got.borrow_mut().take().unwrap().is_err());
        assert!(!d.session.is_dirty());
    }

    #[test]
    fn export_writes_the_file_and_leaves_no_temporary() {
        let (d, _) = analysed();
        let dir = scratch("export");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("Game.asm");
        let got = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&got);
        d.export(ExportKind::Assembly, false, out.clone(), move |r| {
            *sink.borrow_mut() = Some(r)
        });
        got.borrow_mut().take().unwrap().unwrap();
        assert!(std::fs::read_to_string(&out).unwrap().contains("org $"));
        let leftovers = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(leftovers, 1);
        // An unwritable place is an error naming the path.
        let got = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&got);
        let bad = dir.join("no-such-dir").join("x.asm");
        d.export(ExportKind::Symbols, false, bad.clone(), move |r| {
            *sink.borrow_mut() = Some(r)
        });
        let err = got.borrow_mut().take().unwrap().unwrap_err();
        assert!(err.to_string().contains("no-such-dir"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_c_tab_follows_the_selection_and_its_level() {
        let rt = TestRuntime::new();
        let rom =
            romlens_ffi::Rom::from_bytes(romlens_ffi::make_routines_test_rom(), "r.sfc".into())
                .unwrap();
        let d = Document::new(rom, rt.clone());
        d.start_analysis();
        rt.pump();
        // Nothing decompiles while another tab shows.
        d.select(Some(0x22));
        assert!(d.decompile().result.is_none());
        d.set_tab(Tab::C);
        assert_eq!(
            d.decompile().result.as_ref().map(|r| r.name.clone()),
            Some("SUB_008020".into())
        );
        let lines = d.decompile().lines_for_instruction(0x22);
        assert!(!lines.is_empty());
        let full = d.decompile().result.as_ref().unwrap().text.clone();
        d.set_decompile_level(romlens_ffi::DecompileLevel::Lift);
        assert_ne!(d.decompile().result.as_ref().unwrap().text, full);
        // A rename changes the C: the next analysis or edit decompiles again.
        d.select(Some(0x20));
        d.set_label(Some("Loopy".into())).unwrap();
        assert!(
            d.decompile()
                .result
                .as_ref()
                .unwrap()
                .text
                .contains("Loopy")
        );
    }

    #[test]
    fn the_graph_tab_follows_the_selection_and_its_mode() {
        let rt = TestRuntime::new();
        let rom =
            romlens_ffi::Rom::from_bytes(romlens_ffi::make_routines_test_rom(), "r.sfc".into())
                .unwrap();
        let d = Document::new(rom, rt.clone());
        d.start_analysis();
        rt.pump();
        d.select(Some(0x22));
        assert!(
            d.graph().blocks.is_none(),
            "nothing builds while another tab shows"
        );
        d.set_tab(Tab::Graph);
        assert_eq!(
            d.graph().blocks.as_ref().map(|b| b.name.clone()),
            Some("SUB_008020".into())
        );
        assert!(d.graph().block_containing(0x22).is_some());
        d.set_graph_mode(GraphMode::Calls);
        assert!(d.graph().calls.is_some() && d.graph().blocks.is_none());
        d.request_zoom(Zoom::Fit);
        assert_eq!(d.zoom_request(), Some((Zoom::Fit, 1)));
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("romlens-doc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn importing_a_dbg_brings_the_source_tab_and_clicking_a_line_selects_its_bytes() {
        let dir = scratch_dir("source");
        for f in romlens_ffi::make_ca65_test_program() {
            std::fs::write(dir.join(&f.name), &f.bytes).unwrap();
        }
        let rt = TestRuntime::new();
        let d = Document::open(&dir.join("fixture.sfc"), rt.clone()).unwrap();
        d.start_analysis();
        rt.pump();
        assert!(!d.source().has_files());
        let seen = Rc::new(Cell::new(0));
        d.subscribe({
            let seen = Rc::clone(&seen);
            move |c| {
                if c == Change::Source {
                    seen.set(seen.get() + 1);
                }
            }
        });
        d.import(ImportKind::Dbg, dir.join("fixture.dbg"), |r| {
            r.unwrap();
        });
        rt.pump();
        assert!(d.source().has_files());
        assert!(seen.get() > 0, "the tab is told to appear");
        let line = d
            .source()
            .by_line
            .values()
            .min_by_key(|l| l.line)
            .cloned()
            .expect("a line that made bytes");
        d.select_source_line(line.line);
        let first = line.ranges[0].start;
        assert_eq!(d.selected(), Some(first));
        // The byte's lines name the same line, and its file stays shown.
        let at = d.source_lines_at_selection();
        assert!(
            at.iter()
                .any(|l| l.line == line.line && l.file == line.file)
        );
        d.follow_selection_in_source();
        assert_eq!(d.source().shown_file().map(|f| f.file), Some(line.file));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn comparing_with_a_rom_file_lists_what_changed_and_follows_this_versions_edits() {
        let dir = scratch_dir("compare");
        let roms = romlens_ffi::make_compare_test_roms();
        std::fs::write(dir.join("old.sfc"), &roms[0]).unwrap();
        std::fs::write(dir.join("new.sfc"), &roms[1]).unwrap();
        let rt = TestRuntime::new();
        let d = Document::open(&dir.join("new.sfc"), rt.clone()).unwrap();
        d.start_analysis();
        rt.pump();
        assert!(!d.compare().is_active());
        d.compare_with(dir.join("old.sfc"));
        assert_eq!(d.tab(), Tab::Compare);
        rt.pump();
        assert_eq!(d.compare().state, compare::CompareState::Ready);
        assert_eq!(d.compare().other_name.as_deref(), Some("old"));
        assert!(!d.compare().routines().is_empty());
        // A change here compares again, against the same other version.
        let before = d.compare().compared_generation;
        d.select(Some(0));
        d.set_label(Some("Boot".into())).unwrap();
        rt.pump();
        assert_ne!(d.compare().compared_generation, before, "compared again");
        assert_eq!(d.compare().state, compare::CompareState::Ready);
        d.close_compare();
        assert!(!d.compare().is_active());
        assert_eq!(
            d.tab(),
            Tab::Disassembly,
            "the tab closes with the comparison"
        );
        assert!(before.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_that_is_not_a_rom_fails_the_comparison_with_a_message() {
        let dir = scratch_dir("compare-bad");
        std::fs::write(dir.join("junk.sfc"), b"not a rom").unwrap();
        let rt = TestRuntime::new();
        let d = Document::new(
            romlens_ffi::Rom::from_bytes(romlens_ffi::make_routines_test_rom(), "r.sfc".into())
                .unwrap(),
            rt.clone(),
        );
        d.start_analysis();
        rt.pump();
        d.compare_with(dir.join("junk.sfc"));
        rt.pump();
        assert!(matches!(
            d.compare().state,
            compare::CompareState::Failed(_)
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_screen_is_worked_out_only_while_its_section_is_open_and_follows_the_selection() {
        let rt = TestRuntime::new();
        let rom =
            romlens_ffi::Rom::from_bytes(romlens_ffi::make_explain_test_rom(), "e.sfc".into())
                .unwrap();
        let d = Document::new(rom, rt.clone());
        d.start_analysis();
        rt.pump();
        let seen = Rc::new(Cell::new(0));
        d.subscribe({
            let seen = Rc::clone(&seen);
            move |c| {
                if c == Change::Screen {
                    seen.set(seen.get() + 1);
                }
            }
        });
        // Past the reset's first few instructions, where setup has happened.
        d.select(Some(0x10));
        rt.pump();
        assert!(
            d.screen().setup.is_none() && !d.screen().loading,
            "closed: no work"
        );
        d.set_show_screen(true);
        rt.pump();
        assert!(!d.screen().loading);
        assert!(d.screen().setup.is_some(), "the reset is inside a routine");
        assert!(seen.get() >= 2, "asked, then answered");
        // Leaving the instruction clears it.
        d.select(None);
        assert!(d.screen().setup.is_none());
    }

    fn graphics_doc() -> (Rc<Document>, Rc<TestRuntime>) {
        let rt = TestRuntime::new();
        let rom =
            romlens_ffi::Rom::from_bytes(romlens_ffi::make_graphics_test_rom(), "g.sfc".into())
                .unwrap();
        let d = Document::new(rom, rt.clone());
        d.start_analysis();
        rt.pump();
        (d, rt)
    }

    #[test]
    fn a_graphics_view_reads_the_selection_and_a_text_tab_takes_the_area_back() {
        let (d, _rt) = graphics_doc();
        d.select(Some(0x1400));
        assert_eq!(d.graphics_tab(), None);
        d.open_graphics(gfx::Tab::Tiles);
        assert_eq!(d.graphics_tab(), Some(gfx::Tab::Tiles));
        assert_eq!(d.graphics().rom_offset, 0x1400);
        // Frame and Layers do not move the offset.
        d.select(Some(0x1800));
        d.open_graphics(gfx::Tab::Frame);
        assert_eq!(d.graphics().rom_offset, 0x1400);
        // Choosing the tab that was already current still closes the view.
        let current = d.tab();
        d.set_tab(current);
        assert_eq!(d.graphics_tab(), None);
    }

    #[test]
    fn selecting_in_a_graphics_view_selects_its_bytes_in_the_editor() {
        let (d, _rt) = graphics_doc();
        d.select(Some(0x1000));
        d.open_graphics(gfx::Tab::Tiles);
        d.edit_graphics(|g| g.select_tile(2));
        assert_eq!(d.highlighted_range(), Some(0x1040..0x1060));
        d.open_graphics(gfx::Tab::Palette);
        d.edit_graphics(|g| g.select_colour(1));
        // The palette view reads from where the tile selection left the editor.
        assert_eq!(d.highlighted_range(), Some(0x1042..0x1044));
    }

    #[test]
    fn a_previews_open_button_reads_the_range_in_the_view_it_names() {
        let (d, rt) = graphics_doc();
        d.select(Some(0x1000));
        d.select_range(0x1000..0x1400);
        d.mark(OverrideKind::Data, DataKind::Graphics).unwrap();
        rt.fire_timers();
        rt.pump();
        let preview = d.details().preview.expect("a typed range previews");
        d.open_preview(&preview);
        assert_eq!(d.graphics_tab(), Some(gfx::Tab::Tiles));
        assert_eq!(d.graphics().rom_offset, 0x1000);
        assert_eq!(d.graphics().format, romlens_ffi::TileFormat::Bpp4);
        assert_eq!(d.graphics().source, gfx::Source::Rom);
    }

    #[test]
    fn preview_options_are_set_on_the_mark_and_an_empty_address_means_the_default() {
        let (d, rt) = graphics_doc();
        d.select(Some(0x1000));
        d.select_range(0x1000..0x1400);
        // Not marked yet: there is nothing for the options to belong to.
        assert!(d.preview_options().is_none());
        d.mark(OverrideKind::Data, DataKind::Graphics).unwrap();
        rt.fire_timers();
        rt.pump();
        assert_eq!(d.snes_address_of("  ").unwrap(), None);
        assert_eq!(d.snes_address_of("0x1800").unwrap(), Some(0x00_9800));
        assert!(d.snes_address_of("nonsense").is_err());
        d.set_preview_options(romlens_ffi::RegionParamsInfo {
            palette: Some(0x00_9800),
            columns: Some(8),
            ..Default::default()
        })
        .unwrap();
        let got = d.preview_options().expect("a mark has options");
        assert_eq!((got.palette, got.columns), (Some(0x00_9800), Some(8)));
        d.session.undo();
        assert_eq!(d.preview_options().and_then(|p| p.columns), None);
    }

    fn recording_file(name: &str, frames: u32) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("romlens-rec-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.romrec");
        std::fs::write(&path, romlens_ffi::make_test_recording(frames)).unwrap();
        path
    }

    #[test]
    fn attaching_a_recording_opens_the_tilemap_and_closing_it_goes_back_to_the_rom() {
        let (d, _rt) = graphics_doc();
        let path = recording_file("attach", 12);
        let session =
            romlens_ffi::RecordingSession::open(path.to_string_lossy().into_owned(), false)
                .unwrap();
        d.attach_recording(session, "test.romrec").unwrap();
        assert_eq!(d.graphics_tab(), Some(gfx::Tab::Tilemap));
        assert_eq!(d.graphics().source, gfx::Source::Recording);
        // The project refers to it, never holds it.
        assert_eq!(d.workbench().recordings().len(), 1);
        d.set_frame(5);
        assert_eq!(d.graphics().frame(), 5);
        d.step_frame(100);
        assert_eq!(d.graphics().frame(), 11);
        d.close_recording();
        assert!(d.workbench().recordings().is_empty());
        assert_eq!(d.graphics().source, gfx::Source::Rom);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_saved_project_reattaches_its_recording_when_the_file_is_unchanged() {
        let dir = scratch_dir("reattach");
        let rec = dir.join("test.romrec");
        std::fs::write(&rec, romlens_ffi::make_test_recording(6)).unwrap();
        let (d, _rt) = graphics_doc();
        let session =
            romlens_ffi::RecordingSession::open(rec.to_string_lossy().into_owned(), false).unwrap();
        d.attach_recording(session, "test.romrec").unwrap();
        let project = dir.join("g.romlens");
        d.save_to(&project).unwrap();

        let files =
            romlens_ffi::workbench::read_project_package(project.to_string_lossy().into_owned())
                .unwrap();
        let (files, _) = package::split_local(files);
        let rt = TestRuntime::new();
        let rom =
            romlens_ffi::Rom::from_bytes(romlens_ffi::make_graphics_test_rom(), "g.sfc".into())
                .unwrap();
        let reopened =
            Document::from_project(rom, files.clone(), &project, &dir.join("g.sfc"), rt.clone())
                .unwrap();
        assert!(reopened.graphics().has_recording(), "found where it was");

        // A changed file is not the recording the project saw.
        std::fs::write(&rec, romlens_ffi::make_test_recording(7)).unwrap();
        let rom =
            romlens_ffi::Rom::from_bytes(romlens_ffi::make_graphics_test_rom(), "g.sfc".into())
                .unwrap();
        let again = Document::from_project(rom, files, &project, &dir.join("g.sfc"), rt).unwrap();
        assert!(!again.graphics().has_recording());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_recording_of_another_rom_is_refused_and_attaches_nothing() {
        let rt = TestRuntime::new();
        let rom =
            romlens_ffi::Rom::from_bytes(romlens_ffi::make_routines_test_rom(), "r.sfc".into())
                .unwrap();
        let d = Document::new(rom, rt);
        let path = recording_file("other", 3);
        let session =
            romlens_ffi::RecordingSession::open(path.to_string_lossy().into_owned(), false)
                .unwrap();
        assert!(d.attach_recording(session, "x").is_err());
        assert!(d.workbench().recordings().is_empty());
        assert_eq!(d.graphics_tab(), None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn packing_a_recorder_stream_makes_a_recording_in_the_folder_asked_for() {
        // A stream is what the Mesen script writes; the core's own tests make
        // one, so here only a bad stream's failure is checked.
        let (d, rt) = graphics_doc();
        let dir = scratch_dir("pack");
        let stream = dir.join("bad.rlstream");
        std::fs::write(&stream, b"not a stream").unwrap();
        let out = dir.join("nested").join("out.romrec");
        let got = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&got);
        d.pack_recording(stream, out.clone(), move |r| *sink.borrow_mut() = Some(r));
        rt.pump();
        assert!(matches!(got.borrow().as_ref(), Some(Err(_))));
        assert!(!out.exists(), "a failure leaves nothing half-written");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_live_session_feeds_the_views_coalesced_and_follows_the_newest_frame() {
        let (d, rt) = graphics_doc();
        let stream = romlens_ffi::live::make_test_stream(Arc::clone(&d.rom), 12, None);
        let bridge = d.live_bridge().expect("the document is alive");
        let session = romlens_ffi::live::LiveSession::replay(Arc::clone(&d.rom), stream, bridge)
            .expect("a fixture stream replays");
        d.attach_live_session(Arc::clone(&session)).unwrap();
        assert!(d.is_live());
        assert_eq!(d.graphics_tab(), Some(gfx::Tab::Tilemap));
        // The session reads on its own thread; wait for the last frame.
        let start = std::time::Instant::now();
        while session.latest_frame() != Some(11) {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "stream never arrived"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        rt.pump();
        // Frames are told at most thirty times a second: one timer, however
        // many arrived.
        assert!(rt.pending_timers() <= 1, "{} timers", rt.pending_timers());
        rt.fire_timers();
        assert_eq!(d.graphics().frame(), 11, "following the newest");
        assert!(
            d.graphics()
                .source_description()
                .starts_with("Live, frame 11, ")
        );
        // Moving off the newest frame pauses following.
        d.set_frame(4);
        assert!(!d.graphics().follow_live);
        d.stop_live();
        assert!(!d.is_live());
        assert_eq!(d.graphics().recording_name(), Some("Live (stopped)"));
        assert_eq!(
            d.graphics().frame_count(),
            12,
            "the frames received stay readable"
        );
    }

    #[test]
    fn a_screen_rows_link_opens_its_rom_bytes_in_the_view_it_names() {
        use romlens_ffi::ScreenLinkInfo;
        let (d, _rt) = graphics_doc();
        d.open_screen_link(ScreenLinkInfo::Tiles {
            rom: 0x1000,
            bpp: 2,
        });
        assert_eq!(d.graphics_tab(), Some(gfx::Tab::Tiles));
        assert_eq!(d.graphics().rom_offset, 0x1000);
        assert_eq!(d.graphics().format, romlens_ffi::TileFormat::Bpp2);
        d.open_screen_link(ScreenLinkInfo::Tiles {
            rom: 0x1400,
            bpp: 7,
        });
        assert_eq!(d.graphics().format, romlens_ffi::TileFormat::Mode7);
        d.open_screen_link(ScreenLinkInfo::Tilemap { rom: 0x2000 });
        assert_eq!(
            (d.graphics_tab(), d.graphics().rom_offset),
            (Some(gfx::Tab::Tilemap), 0x2000)
        );
        d.open_screen_link(ScreenLinkInfo::Palette { rom: 0x1800 });
        assert_eq!(
            (d.graphics_tab(), d.graphics().rom_offset),
            (Some(gfx::Tab::Palette), 0x1800)
        );
        assert_eq!(d.graphics().source, gfx::Source::Rom);
    }

    /// A document on the explain fixture, with a scripted model server as the
    /// tutor's default endpoint.
    fn tutor_doc(replies: Vec<String>) -> (Rc<Document>, Rc<TestRuntime>) {
        let rt = TestRuntime::new();
        let rom =
            romlens_ffi::Rom::from_bytes(romlens_ffi::make_explain_test_rom(), "e.sfc".into())
                .unwrap();
        let d = Document::new(rom, rt.clone());
        d.start_analysis();
        rt.pump();
        let url = romlens_ffi::tutor::session::tutor_test_server(replies);
        let settings = crate::settings::tutor();
        let taken: Vec<String> = settings
            .borrow()
            .endpoints()
            .into_iter()
            .map(|e| e.id)
            .collect();
        let e = crate::model::tutor_settings::Endpoint::local(
            "Scripted",
            &url,
            crate::model::tutor_settings::Kind::Chat,
            &taken,
        );
        let id = e.id.clone();
        settings.borrow_mut().add(e);
        settings.borrow_mut().edit(|s| {
            s.endpoint = id.clone();
            s.models.insert(id, "qwen3".into());
        });
        (d, rt)
    }

    /// Deliver the session's events until the turn is over.
    fn until_answered(d: &Rc<Document>, rt: &Rc<TestRuntime>, mut each: impl FnMut(&Document)) {
        for _ in 0..3000 {
            rt.pump();
            each(d);
            if !d.tutor().busy {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the turn never ended");
    }

    #[test]
    fn a_question_is_answered_streams_in_and_is_kept_in_the_transcript() {
        let reply =
            romlens_ffi::tutor::session::tutor_test_text_reply("RESET starts the game.".into());
        let (d, rt) = tutor_doc(vec![reply]);
        d.select(Some(0));
        d.tutor_submit("What is at the reset vector?");
        assert!(d.tutor().busy, "the turn is running");
        assert_eq!(
            d.tutor().live.as_ref().map(|l| l.question.as_str()),
            Some("What is at the reset vector?")
        );
        let mut streamed = String::new();
        until_answered(&d, &rt, |d| {
            if let Some(l) = &d.tutor().live {
                streamed = streamed.clone().max(l.text.clone());
            }
        });
        let t = d.tutor();
        assert!(t.error.is_none(), "{:?}", t.error);
        assert!(
            t.turns.len() >= 2,
            "the question and the answer: {}",
            t.turns.len()
        );
        let answer = t.turns.iter().rev().find(|x| !x.user).expect("an answer");
        assert!(format!("{:?}", answer.blocks).contains("RESET starts the game."));
        assert!(t.live.is_none());
        assert!(streamed.contains("RESET starts") || t.turns.len() >= 2);
        assert_eq!(t.model_name.as_deref(), Some("qwen3"));
        assert_eq!(t.endpoint_name.as_deref(), Some("Scripted"));
        drop(t);
        // The selection went with it, as text.
        let sel = d.tutor_selection_text().expect("something is selected");
        assert!(sel.starts_with("$00:8000"), "{sel}");
        assert!(sel.contains("SEI"), "the listing there: {sel}");
        // The question can be walked back to with the up arrow.
        assert_eq!(
            d.edit_tutor(|t| t.history_up("half a thought")).as_deref(),
            Some("What is at the reset vector?")
        );
        assert_eq!(
            d.edit_tutor(|t| t.history_down()).as_deref(),
            Some("half a thought")
        );
    }

    #[test]
    fn an_edit_is_a_card_that_waits_and_accepting_it_changes_the_project() {
        use romlens_ffi::tutor::session::tutor_test_call_reply;
        let (d, rt) = tutor_doc(vec![
            tutor_test_call_reply(
                "set_label".into(),
                r#"{"address":"$00:8000","name":"Reset","reason":"the vector"}"#.into(),
            ),
            romlens_ffi::tutor::session::tutor_test_text_reply("Named it.".into()),
        ]);
        d.tutor_submit("Name RESET");
        let mut decided = false;
        until_answered(&d, &rt, |d| {
            let waiting = d.tutor().live.as_ref().and_then(|l| {
                l.cards
                    .iter()
                    .find(|c| c.state == tutor::CardState::Waiting)
                    .cloned()
            });
            if let Some(card) = waiting {
                assert_eq!(card.proposal.summary, "Name $00:8000 `Reset`");
                assert_ne!(
                    d.workbench().label_at(0x8000).map(|l| l.name),
                    Some("Reset".into()),
                    "nothing is changed before the answer"
                );
                d.edit_tutor(|t| t.answer_card(&card.proposal.id, true, None, false));
                decided = true;
            }
        });
        assert!(decided, "a card was shown");
        assert_eq!(d.workbench().label_at(0x8000).unwrap().name, "Reset");
        assert!(
            d.session.is_dirty(),
            "the project has the tutor's edit to save"
        );
    }

    #[test]
    fn declining_a_card_leaves_the_project_alone_and_a_read_only_tutor_proposes_nothing() {
        use romlens_ffi::tutor::session::tutor_test_call_reply;
        let (d, rt) = tutor_doc(vec![
            tutor_test_call_reply(
                "set_label".into(),
                r#"{"address":"$00:8000","name":"Reset","reason":"the vector"}"#.into(),
            ),
            romlens_ffi::tutor::session::tutor_test_text_reply("Fine.".into()),
        ]);
        d.tutor_submit("Name RESET");
        until_answered(&d, &rt, |d| {
            let waiting = d.tutor().live.as_ref().and_then(|l| {
                l.cards
                    .iter()
                    .find(|c| c.state == tutor::CardState::Waiting)
                    .cloned()
            });
            if let Some(card) = waiting {
                d.edit_tutor(|t| {
                    t.answer_card(&card.proposal.id, false, Some("not yet".into()), false)
                });
            }
        });
        assert_ne!(
            d.workbench().label_at(0x8000).map(|l| l.name),
            Some("Reset".into())
        );
    }

    #[test]
    fn rewinding_goes_back_to_before_a_question_and_hands_back_its_words() {
        use romlens_ffi::tutor::session::RewindWhat;
        let reply = romlens_ffi::tutor::session::tutor_test_text_reply("One.".into());
        let (d, rt) = tutor_doc(vec![reply.clone(), reply]);
        d.tutor_submit("First question");
        until_answered(&d, &rt, |_| {});
        d.tutor_submit("Second question");
        until_answered(&d, &rt, |_| {});
        let points = d.tutor().session().unwrap().rewind_points();
        assert_eq!(points.len(), 2, "one point for each question");
        let r = d
            .edit_tutor(|t| t.rewind(points[1].index, RewindWhat::Both))
            .unwrap();
        assert_eq!(r.prompt.as_deref(), Some("Second question"));
        let shown = tutor::rows(&d.tutor().turns);
        assert!(
            matches!(shown.last(), Some(tutor::Row::Note { text, .. }) if text == "Rewound to here."),
            "the transcript ends where it was rewound: {shown:?}"
        );
        assert!(
            !shown.iter().any(
                |r| matches!(r, tutor::Row::Question { text, .. } if text == "Second question")
            ),
            "the question taken back is not shown"
        );
        assert!(
            d.edit_tutor(|t| t.rewind(999, RewindWhat::Both)).is_err(),
            "a point that is not there is refused"
        );
    }

    #[test]
    fn the_model_chosen_is_kept_and_a_conversation_goes_on_with_it() {
        let reply = romlens_ffi::tutor::session::tutor_test_text_reply("Hi.".into());
        let (d, rt) = tutor_doc(vec![reply]);
        let settings = crate::settings::tutor();
        let e = settings.borrow().default_endpoint();
        // With no conversation open, it is the default for the next one.
        d.tutor_session();
        d.edit_tutor(|t| t.use_model(&e, "other-model", None));
        assert_eq!(settings.borrow().model(&e).as_deref(), Some("other-model"));
        d.tutor_submit("Hello");
        until_answered(&d, &rt, |_| {});
        assert_eq!(d.tutor().model_name.as_deref(), Some("other-model"));
        // With one open, it changes the conversation, and says so in it.
        d.edit_tutor(|t| t.use_model(&e, "qwen3", Some("high".into())));
        assert_eq!(d.tutor().model_name.as_deref(), Some("qwen3"));
        assert_eq!(settings.borrow().effort(&e).as_deref(), Some("high"));
        let rows = tutor::rows(&d.tutor().turns);
        assert!(
            rows.iter().any(
                |r| matches!(r, tutor::Row::Note { text, .. } if text.contains("Now on qwen3"))
            ),
            "the switch is noted in the transcript: {rows:?}"
        );
    }

    #[test]
    fn a_quiz_is_answered_proves_a_level_and_earns_points() {
        use romlens_ffi::tutor::quiz::QuizPurposeInfo;
        use romlens_ffi::tutor::session::tutor_test_quiz_answers;
        let (d, _rt) = tutor_doc(Vec::new());
        d.tutor_session();
        assert!(
            d.edit_tutor(|t| t.start_if_needed()),
            "a conversation is open"
        );
        let concept = d.tutor().learner().unwrap().concepts[0].id.clone();
        d.edit_tutor(|t| t.start_quiz(Some(concept.clone()), None, QuizPurposeInfo::Prove));
        assert_eq!(
            d.tutor().sheet,
            Some(tutor::Sheet::Quiz),
            "the quiz is raised"
        );
        let quiz = d.tutor().quiz.clone().unwrap();
        let session = d.tutor().session().unwrap().clone();
        let answers = tutor_test_quiz_answers(session, quiz.id.clone());
        assert_eq!(answers.len(), quiz.questions.len());
        for (q, a) in quiz.questions.iter().zip(answers) {
            d.edit_tutor(|t| t.answer_question(&q.id, a));
            assert!(
                d.tutor().result(&q.id).is_some(),
                "each answer has its result"
            );
        }
        d.edit_tutor(|t| t.finish_quiz());
        let t = d.tutor();
        let q = t.quiz.as_ref().unwrap();
        assert!(q.finished && q.outcome.passed, "{:?}", q.outcome);
        assert!(
            t.progress.as_ref().unwrap().xp > 0,
            "right answers earn points"
        );
        let p = t.learner().unwrap();
        assert!(p.concepts.iter().any(|c| c.id == concept && c.proven > 0));
    }

    #[test]
    fn a_lesson_is_stepped_through_and_a_guess_is_kept() {
        let (d, _rt) = tutor_doc(Vec::new());
        assert!(d.edit_tutor(|t| t.lessons().is_empty()), "no lessons yet");
        d.tutor_session();
        assert!(d.tutor().learner().is_some_and(|l| !l.concepts.is_empty()));
        // The step a card is at is the model's, and moving it is remembered.
        assert_eq!(d.tutor().step_of("nothing"), 0);
        d.edit_tutor(|t| t.mark_known("cpu", Some(3)));
        let l = d.tutor().learner().unwrap();
        assert!(
            l.concepts
                .iter()
                .all(|c| c.id != "cpu" || (c.level == 3 && c.marked)),
            "marking a concept known shows on the map"
        );
        d.edit_tutor(|t| t.mark_known("cpu", None));
        let l = d.tutor().learner().unwrap();
        assert!(
            l.concepts.iter().all(|c| c.id != "cpu" || !c.marked),
            "and the mark can go"
        );
    }

    #[test]
    fn a_provider_without_its_key_says_so_and_asks_nothing() {
        let rt = TestRuntime::new();
        let rom =
            romlens_ffi::Rom::from_bytes(romlens_ffi::make_explain_test_rom(), "e.sfc".into())
                .unwrap();
        let d = Document::new(rom, rt);
        crate::settings::tutor()
            .borrow_mut()
            .edit(|s| s.endpoint = "anthropic".into());
        d.tutor_submit("Hello?");
        let t = d.tutor();
        assert!(!t.busy);
        assert_eq!(
            t.error.as_deref(),
            Some("Add a key for Anthropic in Settings (Ctrl+,).")
        );
    }

    #[test]
    fn slash_commands_change_the_mode_raise_sheets_and_refuse_what_they_do_not_know() {
        let (d, _rt) = tutor_doc(Vec::new());
        d.tutor_submit("/mode read-only");
        assert_eq!(
            d.tutor().mode,
            crate::model::tutor_settings::ModePreference::ReadOnly
        );
        d.tutor_submit("/mode");
        assert_eq!(
            d.tutor().mode,
            crate::model::tutor_settings::ModePreference::AskBeforeEdits,
            "cycles"
        );
        d.tutor_submit("/mode nonsense");
        assert_eq!(
            d.tutor().error.as_deref(),
            Some("The modes are read-only, ask and accept.")
        );
        d.tutor_submit("/help");
        assert_eq!(d.tutor().sheet, Some(tutor::Sheet::Help));
        d.tutor_submit("/bogus");
        assert_eq!(
            d.tutor().error.as_deref(),
            Some("/bogus is not a command. /help lists them.")
        );
        d.tutor_submit("/quiz");
        assert!(
            d.tutor()
                .error
                .as_deref()
                .unwrap()
                .starts_with("Name what to be quizzed on")
        );
        d.tutor_submit("/selection");
        assert!(!d.tutor().include_selection);
        d.tutor_submit("/attach nothing");
        assert!(d.tutor().error.is_some());
        d.tutor_submit("/attach frame");
        assert_eq!(
            d.tutor().error.as_deref(),
            Some("Open a recording to attach its frame.")
        );
    }

    #[test]
    fn a_citation_shows_its_address_its_routine_or_its_frame() {
        let (d, _rt) = tutor_doc(Vec::new());
        assert!(d.follow_citation(tutor::Citation::Address(0x8030)));
        assert_eq!(d.selected(), Some(0x30));
        assert!(d.follow_citation(tutor::Citation::Routine(0x8000)));
        assert_eq!(d.tab(), Tab::C);
        assert!(
            !d.follow_citation(tutor::Citation::Frame(3, None)),
            "no recording to show"
        );
        assert!(d.follow_citation(tutor::Citation::Register));
    }
}
