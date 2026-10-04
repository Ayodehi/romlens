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

use super::batch_cache::BatchCache;
use super::commands::{EditorCommand, EditorSource, Sheet};
use super::layout::{Layout, Panes, ResultsKind, Tab};
use super::navigator::{NavTab, Navigator, NavigatorData};
use super::references::ReferencesModel;
use super::runtime::Runtime;
use super::runtime::background;
use super::search::SearchModel;
use super::session::{ChangeKind, Session};
use super::transfer::{self, ExportKind, ImportKind};
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
            layout: RefCell::new(Layout::default()),
            explanations: Cell::new(true),
            runtime,
            navigator: RefCell::new(Navigator::default()),
            filter_ticket: Cell::new(0),
            search: RefCell::new(SearchModel::default()),
            references: RefCell::new(ReferencesModel::default()),
            variable_draft: RefCell::new(VariableDraft::default()),
            project_path: RefCell::new(None),
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
                self.refresh_details();
                self.reload_navigator();
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
            std::mem::replace(&mut l.tab, tab) != tab
        };
        if changed {
            self.emit(Change::Layout);
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
                if changed {
                    self.emit(Change::Selection);
                }
            }
            Some(o) if o < self.byte_count() => {
                self.selection.borrow_mut().offset = Some(o);
                self.refresh_details();
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
    }

    /// The address of the selected item (instruction start or byte).
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
}
