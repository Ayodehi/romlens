//! The per-document object: ROM + project + analysis snapshot + line index +
//! undo stack, behind one mutex. Queries during an analysis see the previous
//! snapshot; the analysis runs on its own thread and installs its result when
//! the awaiting future is polled.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use romlens_core::analysis::heuristics::EntropyProfile;
use romlens_core::analysis::{AnalysisControl, AnalysisOptions, AnalysisSnapshot, analyze_cached};
use romlens_core::cpu65816::{self, NoSymbols};
use romlens_core::explain::Explanations;
use romlens_core::io;
use romlens_core::model::{self, Project, Symbols, UndoStack};
use romlens_core::viewmodel::asm_lines::NONE_ADDRESS;
use romlens_core::viewmodel::hex_rows::{BATCH_HEADER_LEN, BYTES_PER_ROW, ROW_STRIDE};
use romlens_core::viewmodel::region_summary;
use romlens_core::{
    FileOffset, LineIndex, SnesAddress, TextOptions, encode_lines, encode_rows, format_lines_text,
};

use crate::explain::ExplanationInfo;
use crate::records::*;
use crate::{Rom, RomlensError};

/// Receives workbench events, possibly from the analysis thread.
#[uniffi::export(with_foreign)]
pub trait WorkbenchListener: Send + Sync {
    fn on_event(&self, event: WorkbenchEvent);
}

/// Every routine's summary, and the analysis generation it was built from.
type ProgramCache = (u64, Arc<romlens_core::decompile::Program>);

/// What an analysis produces: the snapshot, its explanations and the lines.
type Analysed = (AnalysisSnapshot, Arc<Explanations>, LineIndex);

/// The line index, with or without the explanations.
fn lines_for(
    image: &romlens_core::RomImage,
    snapshot: &AnalysisSnapshot,
    project: &Project,
    explain: &Arc<Explanations>,
    show: bool,
) -> LineIndex {
    if show {
        LineIndex::build_explained(image, snapshot, project, Arc::clone(explain))
    } else {
        LineIndex::build(image, snapshot, project)
    }
}

struct Inner {
    project: Project,
    snapshot: Arc<AnalysisSnapshot>,
    /// What each hardware write does and the idioms (docs/20), for this
    /// snapshot and the project's names.
    explain: Arc<Explanations>,
    /// Explanations in the listing and the C.
    show_explanations: bool,
    lines: Arc<LineIndex>,
    undo: UndoStack,
    analysis_generation: u64,
    view_generation: u64,
    dirty: bool,
    needs_analysis: bool,
}

#[derive(uniffi::Object)]
pub struct Workbench {
    rom: Arc<Rom>,
    /// Built once and shared with every analysis. It is derived from the ROM
    /// alone, so it survives every edit; rebuilding it per command would put a
    /// linear pass over the image inside the edit loop (`16-phase2-plan.md`
    /// 2A.2). Built lazily so opening a ROM stays instant and the cost lands
    /// on the background thread with the rest of the analysis.
    entropy: OnceLock<Arc<EntropyProfile>>,
    inner: Mutex<Inner>,
    listener: Mutex<Option<Arc<dyn WorkbenchListener>>>,
    cancel: Arc<AtomicBool>,
    /// Every routine's summary for the snapshot of this analysis generation:
    /// built on the first decompile after an analysis, then shared.
    decompiler: Arc<Mutex<Option<ProgramCache>>>,
}

impl Workbench {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, event: WorkbenchEvent) {
        let listener = self
            .listener
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(l) = listener {
            l.on_event(event);
        }
    }

    fn make(rom: Arc<Rom>, project: Project) -> Arc<Self> {
        Arc::new(Self {
            rom,
            entropy: OnceLock::new(),
            inner: Mutex::new(Inner {
                project,
                snapshot: Arc::new(AnalysisSnapshot::default()),
                explain: Arc::new(Explanations::default()),
                show_explanations: true,
                lines: Arc::new(LineIndex::default()),
                undo: UndoStack::default(),
                analysis_generation: 0,
                view_generation: 0,
                dirty: false,
                needs_analysis: true,
            }),
            listener: Mutex::new(None),
            cancel: Arc::new(AtomicBool::new(false)),
            decompiler: Arc::new(Mutex::new(None)),
        })
    }

    fn install(&self, (snapshot, explain, lines): Analysed) -> AnalysisStats {
        let stats = snapshot.stats.into();
        let generation = {
            let mut inner = self.lock();
            inner.snapshot = Arc::new(snapshot);
            inner.explain = explain;
            inner.lines = Arc::new(lines);
            inner.analysis_generation += 1;
            inner.view_generation += 1;
            inner.needs_analysis = false;
            inner.analysis_generation
        };
        self.emit(WorkbenchEvent::SnapshotChanged {
            analysis_generation: generation,
        });
        stats
    }

    /// A job decompiling the routine entered at `snes_address`.
    fn decompile_job(
        &self,
        snes_address: u32,
        level: DecompileLevel,
    ) -> impl FnOnce() -> Result<DecompiledInfo, RomlensError> + Send + 'static {
        use romlens_core::decompile::{self, DecompileOptions};
        let image = self.rom.image.clone();
        let (project, snapshot, generation, explain) = {
            let inner = self.lock();
            (
                inner.project.clone(),
                Arc::clone(&inner.snapshot),
                inner.analysis_generation,
                inner.show_explanations,
            )
        };
        let cache = Arc::clone(&self.decompiler);
        move || {
            let opts = DecompileOptions {
                level: level.into(),
                explain,
                ..Default::default()
            };
            let cached = cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .filter(|(g, _)| *g == generation)
                .map(|(_, p)| Arc::clone(p));
            let program = match cached {
                Some(p) => p,
                None => {
                    let p = Arc::new(decompile::program(&image, &project, &snapshot, &opts));
                    *cache.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some((generation, Arc::clone(&p)));
                    p
                }
            };
            let at = Project::canonical(&image, SnesAddress::from_u24(snes_address));
            let f = match program.units.get(&at) {
                Some(u) => u.f.clone(),
                None => decompile::discover(&image, &snapshot, &program.entries, at)
                    .map_err(|e| RomlensError::BadAddress { msg: e.to_string() })?,
            };
            Ok(
                decompile::render_with(&image, &project, &snapshot, &f, &opts, Some(&program))
                    .into(),
            )
        }
    }

    fn graph_job(
        &self,
        snes_address: u32,
    ) -> impl FnOnce() -> Result<crate::graphs::RoutineGraphInfo, RomlensError> + Send + 'static
    {
        let image = self.rom.image.clone();
        let (project, snapshot, lines) = {
            let inner = self.lock();
            (
                inner.project.clone(),
                Arc::clone(&inner.snapshot),
                Arc::clone(&inner.lines),
            )
        };
        move || {
            let g = romlens_core::graph::routine_graph(
                &image,
                &project,
                &snapshot,
                SnesAddress::from_u24(snes_address),
            )
            .map_err(|e| RomlensError::BadAddress { msg: e.to_string() })?;
            Ok(crate::graphs::RoutineGraphInfo::new(g, &lines))
        }
    }

    fn calls_job(
        &self,
        snes_address: u32,
    ) -> impl FnOnce() -> Result<crate::graphs::CallNeighbourhoodInfo, RomlensError> + Send + 'static
    {
        let image = self.rom.image.clone();
        let (project, snapshot) = {
            let inner = self.lock();
            (inner.project.clone(), Arc::clone(&inner.snapshot))
        };
        move || {
            romlens_core::graph::call_neighbourhood(
                &image,
                &project,
                &snapshot,
                SnesAddress::from_u24(snes_address),
            )
            .map(Into::into)
            .map_err(|e| RomlensError::BadAddress { msg: e.to_string() })
        }
    }

    /// Rebuild the explanations and the line index after a label or
    /// comment change: the explanations name what the project names.
    fn refresh_lines(&self, inner: &mut Inner) -> u64 {
        inner.explain = Arc::new(Explanations::build(
            &self.rom.image,
            &inner.project,
            &inner.snapshot,
        ));
        self.relist(inner)
    }

    /// Rebuild the line index alone.
    fn relist(&self, inner: &mut Inner) -> u64 {
        inner.lines = Arc::new(lines_for(
            &self.rom.image,
            &inner.snapshot,
            &inner.project,
            &inner.explain,
            inner.show_explanations,
        ));
        inner.view_generation += 1;
        inner.view_generation
    }

    fn after_edit(&self, inner: &mut Inner, affects_analysis: bool) -> (u64, bool) {
        inner.dirty = true;
        if affects_analysis {
            inner.needs_analysis = true;
        }
        let generation = self.refresh_lines(inner);
        (generation, inner.dirty)
    }

    fn analysis_job(
        &self,
    ) -> impl FnOnce() -> Result<Analysed, romlens_core::analysis::Cancelled> + Send + 'static {
        let image = self.rom.image.clone();
        let (project, show) = {
            let inner = self.lock();
            (inner.project.clone(), inner.show_explanations)
        };
        let entropy = Arc::clone(
            self.entropy
                .get_or_init(|| Arc::new(EntropyProfile::build(&self.rom.image))),
        );
        self.cancel.store(false, Ordering::Relaxed);
        let cancel = Arc::clone(&self.cancel);
        let listener = self
            .listener
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        move || {
            let control = AnalysisControl {
                cancel,
                progress: Box::new(move |p| {
                    if let Some(l) = &listener {
                        l.on_event(WorkbenchEvent::AnalysisProgress {
                            phase: p.phase.into(),
                            done: p.done,
                            total: p.total,
                        });
                    }
                }),
            };
            let snapshot = analyze_cached(
                &image,
                &project,
                &control,
                AnalysisOptions::default(),
                Some(&entropy),
            )?;
            control.report(romlens_core::analysis::AnalysisPhase::Lines, 0, 1);
            let explain = Arc::new(Explanations::build(&image, &project, &snapshot));
            let lines = lines_for(&image, &snapshot, &project, &explain, show);
            control.report(romlens_core::analysis::AnalysisPhase::Lines, 1, 1);
            Ok((snapshot, explain, lines))
        }
    }
}

#[uniffi::export]
impl Workbench {
    /// A fresh, unanalyzed workbench with an empty project.
    #[uniffi::constructor]
    pub fn new(rom: Arc<Rom>) -> Arc<Self> {
        let project = Project::new(&rom.image);
        Self::make(rom, project)
    }

    /// Open the files of a `.romlens` package for this ROM.
    #[uniffi::constructor]
    pub fn with_project_files(
        rom: Arc<Rom>,
        files: HashMap<String, Vec<u8>>,
    ) -> Result<Arc<Self>, RomlensError> {
        let files: std::collections::BTreeMap<String, Vec<u8>> = files.into_iter().collect();
        let identity = io::read_identity(&files)?;
        if !identity.matches(&rom.image) {
            return Err(RomlensError::RomMismatch {
                msg: format!(
                    "project belongs to a different ROM (expected SHA-256 {}, found {})",
                    identity.sha256,
                    rom.image.sha256_hex()
                ),
            });
        }
        let project = io::from_files(&rom.image, &files)?;
        Ok(Self::make(rom, project))
    }

    pub fn set_listener(&self, listener: Option<Arc<dyn WorkbenchListener>>) {
        *self.listener.lock().unwrap_or_else(|e| e.into_inner()) = listener;
    }

    pub fn rom(&self) -> Arc<Rom> {
        Arc::clone(&self.rom)
    }

    /// Run the analyzer on a background thread. Dropping the future (a
    /// cancelled task) cancels the run.
    pub async fn analyze(&self) -> Result<AnalysisStats, RomlensError> {
        let job = self.analysis_job();
        let result = crate::future::spawn(Arc::clone(&self.cancel), job).await;
        match result {
            Ok(done) => Ok(self.install(done)),
            Err(_) => Err(RomlensError::Cancelled),
        }
    }

    /// The same on the calling thread (tests, CLI-style use).
    pub fn analyze_blocking(&self) -> Result<AnalysisStats, RomlensError> {
        let job = self.analysis_job();
        match job() {
            Ok(done) => Ok(self.install(done)),
            Err(_) => Err(RomlensError::Cancelled),
        }
    }

    pub fn cancel_analysis(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn needs_analysis(&self) -> bool {
        self.lock().needs_analysis
    }

    pub fn analysis_generation(&self) -> u64 {
        self.lock().analysis_generation
    }

    pub fn view_generation(&self) -> u64 {
        self.lock().view_generation
    }

    pub fn stats(&self) -> AnalysisStats {
        self.lock().snapshot.stats.into()
    }

    // ---- lines -----------------------------------------------------------

    pub fn line_count(&self) -> u32 {
        self.lock().lines.len() as u32
    }

    /// Flat asm-line batch; layout in the core's `viewmodel::asm_lines`.
    pub fn asm_lines(&self, start_line: u32, count: u32) -> Vec<u8> {
        let inner = self.lock();
        encode_lines(
            &self.rom.image,
            &inner.snapshot,
            &inner.project,
            &inner.lines,
            start_line,
            count,
        )
    }

    pub fn asm_lines_text(&self, start_line: u32, count: u32, style: AddressStyle) -> String {
        let inner = self.lock();
        format_lines_text(
            &self.rom.image,
            &inner.snapshot,
            &inner.project,
            &inner.lines,
            start_line,
            count,
            TextOptions {
                style: style.into(),
                verbose: false,
            },
        )
    }

    /// Hex rows with the header spans, plus region codes in the span lane
    /// (`0x80 | kind << 4 | confidence4`, kind 1 code, 2 data) where no span
    /// applies.
    pub fn hex_rows(&self, start_row: u32, count: u32) -> Vec<u8> {
        let mut batch = encode_rows(&self.rom.image, &self.rom.index, start_row, count);
        let inner = self.lock();
        let rows = u32::from_le_bytes(batch[4..8].try_into().unwrap()) as usize;
        let first = start_row * BYTES_PER_ROW as u32;
        let regions = inner
            .snapshot
            .regions_in(first, rows as u32 * BYTES_PER_ROW as u32);
        for r in regions {
            let code = match r.kind {
                model::RegionKind::Unknown => continue,
                model::RegionKind::Code => 1u8,
                model::RegionKind::Data(_) => 2u8,
            };
            let conf = (r.confidence * 15.0).round().clamp(0.0, 15.0) as u8;
            let value = 0x80 | (code << 4) | conf;
            let lo = r.start.0.max(first);
            let hi = r.end().min(first + rows as u32 * BYTES_PER_ROW as u32);
            for b in lo..hi {
                let row = ((b - first) / BYTES_PER_ROW as u32) as usize;
                let i = ((b - first) % BYTES_PER_ROW as u32) as usize;
                let at = BATCH_HEADER_LEN + row * ROW_STRIDE as usize + 28 + i;
                if batch[at] == 0 {
                    batch[at] = value;
                }
            }
        }
        batch
    }

    pub fn line_for_offset(&self, file_offset: u32) -> Option<u32> {
        self.lock()
            .lines
            .line_for_offset(file_offset)
            .map(|l| l as u32)
    }

    pub fn offset_for_line(&self, line: u32) -> Option<u32> {
        self.lock().lines.offset_for_line(line as usize)
    }

    pub fn line_numbers_for_bytes(&self, start: u32, len: u32) -> Vec<u32> {
        self.lock().lines.line_numbers_for_bytes(start, len)
    }

    pub fn item_range(&self, line: u32) -> Option<ByteRange> {
        self.lock()
            .lines
            .item_range(line as usize)
            .map(|(start, len)| ByteRange { start, len })
    }

    // ---- instructions and regions ---------------------------------------

    pub fn instruction_at(&self, file_offset: u32) -> Option<InstructionInfo> {
        let inner = self.lock();
        let rec = inner.snapshot.instruction_at(FileOffset(file_offset))?;
        let insn = inner.snapshot.decode_at(&self.rom.image, rec)?;
        let symbols = Symbols::new(&self.rom.image, &inner.project, &inner.snapshot.auto_labels);
        Some(instruction_info(&self.rom.image, &insn, &symbols))
    }

    /// What the instruction at `file_offset` does to the hardware, and the
    /// idioms it is part of (docs/20).
    pub fn explain_at(&self, file_offset: u32) -> ExplanationInfo {
        let inner = self.lock();
        let off = FileOffset(file_offset);
        let register = match inner.explain.write_at(off) {
            Some(e) => Some(crate::explain::store_info(e)),
            None => inner
                .snapshot
                .instruction_at(off)
                .and_then(|r| inner.snapshot.decode_at(&self.rom.image, r))
                .and_then(|insn| {
                    let r = cpu65816::register_for(&insn)?;
                    use cpu65816::Mnemonic::*;
                    let (store, wide) = match insn.mnemonic {
                        STA | STZ => (true, !insn.flags_before.m),
                        STX | STY => (true, !insn.flags_before.x),
                        LDX | LDY | CPX | CPY => (false, !insn.flags_before.x),
                        _ => (false, !insn.flags_before.m),
                    };
                    crate::explain::access_info(r.address, if wide { 2 } else { 1 }, store)
                }),
        };
        ExplanationInfo {
            register,
            idioms: inner
                .explain
                .idioms_at(off)
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }

    /// Whether the listing and the C carry the explanations.
    pub fn show_explanations(&self) -> bool {
        self.lock().show_explanations
    }

    pub fn set_show_explanations(&self, show: bool) {
        let generation = {
            let mut inner = self.lock();
            if inner.show_explanations == show {
                return;
            }
            inner.show_explanations = show;
            self.relist(&mut inner)
        };
        self.emit(WorkbenchEvent::ViewChanged {
            view_generation: generation,
        });
    }

    /// Instructions from `file_offset`: the analysis's, or with `flags` a raw
    /// linear decode (the tutor's hypothesis tool).
    pub fn disassemble(
        &self,
        file_offset: u32,
        count: u32,
        flags: Option<FlagState>,
    ) -> Vec<InstructionInfo> {
        let inner = self.lock();
        let symbols = Symbols::new(&self.rom.image, &inner.project, &inner.snapshot.auto_labels);
        let image = &self.rom.image;
        let mut out = Vec::with_capacity(count as usize);
        match flags {
            Some(f) => {
                let mut flags: cpu65816::FlagState = f.into();
                let mut pos = file_offset;
                let mut history = Vec::new();
                for _ in 0..count {
                    let Some(addr) = image.snes_address_for(FileOffset(pos)) else {
                        break;
                    };
                    let Some(mut insn) = cpu65816::decode(
                        &image.bytes()[pos as usize..],
                        addr,
                        FileOffset(pos),
                        flags,
                    ) else {
                        break;
                    };
                    romlens_core::analysis::flow::refine(&history, &mut insn);
                    out.push(instruction_info(image, &insn, &symbols));
                    flags = insn.flags_after;
                    pos += insn.len as u32;
                    history.push(insn);
                    if history.len() > 2 {
                        history.remove(0);
                    }
                }
            }
            None => {
                let start = inner.snapshot.instruction_index_from(file_offset);
                for rec in inner.snapshot.instructions[start..]
                    .iter()
                    .take(count as usize)
                {
                    if let Some(insn) = inner.snapshot.decode_at(image, rec) {
                        out.push(instruction_info(image, &insn, &symbols));
                    }
                }
            }
        }
        out
    }

    pub fn region_at(&self, file_offset: u32) -> Option<RegionInfo> {
        self.lock()
            .snapshot
            .region_at(FileOffset(file_offset))
            .map(RegionInfo::from)
    }

    pub fn regions(&self, start: u32, len: u32) -> Vec<RegionInfo> {
        self.lock()
            .snapshot
            .regions_in(start, len)
            .iter()
            .map(RegionInfo::from)
            .collect()
    }

    /// Every region, for the navigator.
    ///
    /// Deprecated in Phase 2 and kept only so a shell built against 0.2.0
    /// still links: a trace import can take a 3 MB ROM past forty thousand
    /// regions, and returning all of them as generated records to be filtered
    /// shell-side is exactly the shape `10-ffi-spike.md` measured as the
    /// expensive one. Use `regions_of_kind` or `region_map`.
    pub fn regions_summary(&self) -> Vec<RegionInfo> {
        self.lock()
            .snapshot
            .regions
            .iter()
            .map(RegionInfo::from)
            .collect()
    }

    /// Regions of one kind, largest first, at most `limit` of them.
    ///
    /// The navigator wants "the big code blocks", not every region, and the
    /// core is where that question should be answered: a shell that asked for
    /// everything and sorted it would pay for the whole list to cross the FFI
    /// before discarding it.
    pub fn regions_of_kind(&self, kind: RegionKind, limit: u32) -> Vec<RegionInfo> {
        let inner = self.lock();
        let mut out: Vec<&model::Region> = inner
            .snapshot
            .regions
            .iter()
            .filter(|r| {
                matches!(
                    (kind, r.kind),
                    (RegionKind::Unknown, model::RegionKind::Unknown)
                        | (RegionKind::Code, model::RegionKind::Code)
                        | (RegionKind::Data, model::RegionKind::Data(_))
                )
            })
            .collect();
        out.sort_by(|a, b| b.len.cmp(&a.len).then(a.start.cmp(&b.start)));
        out.truncate(limit as usize);
        // Back into address order: the navigator lists them, it does not rank
        // them, and "the ten biggest, in order" is what a reader can navigate.
        out.sort_by_key(|r| r.start);
        out.iter().map(|r| RegionInfo::from(*r)).collect()
    }

    /// The whole-ROM overview strip, reduced to `buckets` columns.
    ///
    /// A flat batch, like the hex rows: the shell draws one rect per column
    /// from a buffer it walks once, and the reduction happens here so every
    /// shell gets the same answer. The layout is documented on
    /// `viewmodel::region_summary::encode_summary`.
    pub fn region_map(&self, buckets: u32) -> Vec<u8> {
        let entropy = Arc::clone(
            self.entropy
                .get_or_init(|| Arc::new(EntropyProfile::build(&self.rom.image))),
        );
        let inner = self.lock();
        let len = self.rom.image.len() as u32;
        let map = region_summary::summarize(
            &inner.snapshot,
            len,
            buckets,
            Some(&entropy),
            inner.project.coverage.as_deref(),
        );
        region_summary::encode_summary(&map, len)
    }

    // ---- labels, xrefs, comments, warnings --------------------------------

    pub fn labels(&self) -> Vec<LabelInfo> {
        let inner = self.lock();
        let symbols = Symbols::new(&self.rom.image, &inner.project, &inner.snapshot.auto_labels);
        symbols
            .all_labels()
            .into_iter()
            .map(|l| label_info(&self.rom.image, l))
            .collect()
    }

    /// Labels whose ROM bytes fall in `[start, start + len)`.
    pub fn labels_in(&self, start: u32, len: u32) -> Vec<LabelInfo> {
        self.labels()
            .into_iter()
            .filter(|l| {
                l.file_offset
                    .is_some_and(|o| o >= start && o < start.saturating_add(len))
            })
            .collect()
    }

    pub fn label_at(&self, snes_address: u32) -> Option<LabelInfo> {
        let inner = self.lock();
        let symbols = Symbols::new(&self.rom.image, &inner.project, &inner.snapshot.auto_labels);
        symbols
            .label_at(SnesAddress::from_u24(snes_address))
            .map(|l| label_info(&self.rom.image, l))
    }

    // ---- variables -----------------------------------------------------------

    /// Name `snes_address` and give it a type, as one undo step titled
    /// "Define Variable". Replaces a variable or label already there.
    pub fn define_variable(
        &self,
        snes_address: u32,
        name: String,
        ty: VarTypeInfo,
    ) -> Result<(), RomlensError> {
        let address = SnesAddress::from_u24(snes_address);
        self.apply_user_batch(vec![
            model::Command::SetLabel {
                address,
                name: Some(name),
            },
            model::Command::SetVariable {
                address,
                ty: Some(ty.into()),
            },
        ])
    }

    /// Remove the variable at `snes_address`, name and type, as one step.
    pub fn remove_variable(&self, snes_address: u32) -> Result<(), RomlensError> {
        let address = SnesAddress::from_u24(snes_address);
        self.apply_user_batch(vec![
            model::Command::SetLabel {
                address,
                name: None,
            },
            model::Command::SetVariable { address, ty: None },
        ])
    }

    /// Every variable, by address.
    pub fn variables(&self) -> Vec<VariableInfo> {
        let inner = self.lock();
        inner
            .project
            .variables
            .iter()
            .map(|(a, t)| variable_info(&self.rom.image, &inner.project, *a, *t))
            .collect()
    }

    /// The variable spanning `snes_address`, if any: the one an instruction's
    /// operand falls in.
    pub fn variable_containing(&self, snes_address: u32) -> Option<VariableInfo> {
        let inner = self.lock();
        let addr = Project::canonical(&self.rom.image, SnesAddress::from_u24(snes_address));
        inner
            .project
            .variable_containing(addr)
            .map(|(a, t)| variable_info(&self.rom.image, &inner.project, a, t))
    }

    /// The canonical address of `snes_address`: WRAM's own for a low-RAM
    /// mirror, bank `$00` for a register, the canonical mirror for ROM.
    pub fn canonical_address(&self, snes_address: u32) -> u32 {
        Project::canonical(&self.rom.image, SnesAddress::from_u24(snes_address)).as_u24()
    }

    /// Pseudo-C for the routine entered at `snes_address`, off the calling
    /// thread. The first call after an analysis works out every routine's
    /// summary; later ones reuse it.
    pub async fn decompile(
        &self,
        snes_address: u32,
        level: DecompileLevel,
    ) -> Result<DecompiledInfo, RomlensError> {
        let job = self.decompile_job(snes_address, level);
        // Its own flag: dropping this future must not cancel an analysis.
        crate::future::spawn(Arc::new(AtomicBool::new(false)), job).await
    }

    /// The same on the calling thread.
    pub fn decompile_blocking(
        &self,
        snes_address: u32,
        level: DecompileLevel,
    ) -> Result<DecompiledInfo, RomlensError> {
        self.decompile_job(snes_address, level)()
    }

    /// The control-flow graph of the routine entered at `snes_address`
    /// (docs/19), off the calling thread; each block names the listing
    /// lines it covers.
    pub async fn routine_graph(
        &self,
        snes_address: u32,
    ) -> Result<crate::graphs::RoutineGraphInfo, RomlensError> {
        let job = self.graph_job(snes_address);
        crate::future::spawn(Arc::new(AtomicBool::new(false)), job).await
    }

    pub fn routine_graph_blocking(
        &self,
        snes_address: u32,
    ) -> Result<crate::graphs::RoutineGraphInfo, RomlensError> {
        self.graph_job(snes_address)()
    }

    /// The routine entered at `snes_address` with its callers and callees,
    /// off the calling thread.
    pub async fn call_neighbourhood(
        &self,
        snes_address: u32,
    ) -> Result<crate::graphs::CallNeighbourhoodInfo, RomlensError> {
        let job = self.calls_job(snes_address);
        crate::future::spawn(Arc::new(AtomicBool::new(false)), job).await
    }

    pub fn call_neighbourhood_blocking(
        &self,
        snes_address: u32,
    ) -> Result<crate::graphs::CallNeighbourhoodInfo, RomlensError> {
        self.calls_job(snes_address)()
    }

    /// The entry of the routine whose instructions include `file_offset`.
    pub fn function_containing(&self, file_offset: u32) -> Option<u32> {
        let inner = self.lock();
        let entries = romlens_core::decompile::entries(&inner.snapshot);
        romlens_core::decompile::containing(
            &self.rom.image,
            &inner.snapshot,
            &entries,
            romlens_core::FileOffset(file_offset),
        )
        .map(|f| f.entry.as_u24())
    }

    pub fn xrefs_to(&self, snes_address: u32) -> Vec<XRefInfo> {
        let inner = self.lock();
        let addr = Project::canonical(&self.rom.image, SnesAddress::from_u24(snes_address));
        inner
            .snapshot
            .xrefs_to(addr)
            .iter()
            .map(|x| xref_info(&self.rom.image, x))
            .collect()
    }

    pub fn xrefs_from(&self, file_offset: u32) -> Vec<XRefInfo> {
        let inner = self.lock();
        inner
            .snapshot
            .xrefs_from(FileOffset(file_offset))
            .iter()
            .map(|x| xref_info(&self.rom.image, x))
            .collect()
    }

    pub fn comments(&self) -> Vec<CommentInfo> {
        self.lock()
            .project
            .comments
            .values()
            .map(CommentInfo::from)
            .collect()
    }

    pub fn comment_at(&self, snes_address: u32, kind: CommentKind) -> Option<CommentInfo> {
        let inner = self.lock();
        let addr = Project::canonical(&self.rom.image, SnesAddress::from_u24(snes_address));
        inner
            .project
            .comment_at(addr, kind.into())
            .map(CommentInfo::from)
    }

    pub fn warnings(&self) -> Vec<WarningInfo> {
        self.lock()
            .snapshot
            .warnings
            .iter()
            .map(WarningInfo::from)
            .collect()
    }

    pub fn warnings_at(&self, file_offset: u32) -> Vec<WarningInfo> {
        self.lock()
            .snapshot
            .warnings_at(FileOffset(file_offset))
            .iter()
            .map(WarningInfo::from)
            .collect()
    }

    pub fn flag_override_at(&self, file_offset: u32) -> Option<FlagOverride> {
        self.lock()
            .project
            .flag_overrides
            .get(&FileOffset(file_offset))
            .copied()
            .map(Into::into)
    }

    pub fn region_override_at(&self, file_offset: u32) -> Option<ByteRange> {
        self.lock()
            .project
            .region_override_at(FileOffset(file_offset))
            .map(|r| ByteRange {
                start: r.start.0,
                len: r.len,
            })
    }

    /// The preview options of the override covering `file_offset`.
    pub fn region_params_at(&self, file_offset: u32) -> Option<RegionParamsInfo> {
        self.lock()
            .project
            .region_override_at(FileOffset(file_offset))
            .map(|r| r.params.into())
    }

    /// The preview for the typed range containing `file_offset`.
    pub fn preview_at(&self, file_offset: u32) -> Option<crate::graphics::PreviewInfo> {
        let inner = self.lock();
        romlens_core::viewmodel::preview::preview_at(
            &self.rom.image,
            &inner.snapshot,
            &inner.project,
            file_offset,
        )
        .map(Into::into)
    }

    // ---- editing -----------------------------------------------------------

    /// Apply a command; the inverse goes on the undo stack.
    pub fn execute(&self, command: Command) -> Result<(), RomlensError> {
        let cmd: model::Command = command.into();
        let affects = cmd.affects_analysis();
        let (generation, dirty) = {
            let mut inner = self.lock();
            let entry = inner.project.apply(&self.rom.image, cmd)?;
            inner.undo.push(entry);
            self.after_edit(&mut inner, affects)
        };
        self.emit(WorkbenchEvent::ProjectChanged { dirty });
        self.emit(WorkbenchEvent::ViewChanged {
            view_generation: generation,
        });
        Ok(())
    }

    pub fn undo(&self) -> Result<bool, RomlensError> {
        let (done, generation, dirty) = {
            let mut inner = self.lock();
            let affects = inner
                .undo
                .entries()
                .last()
                .is_some_and(model::UndoEntry::affects_analysis);
            let Inner { project, undo, .. } = &mut *inner;
            let done = undo.undo(project, &self.rom.image)?;
            if !done {
                return Ok(false);
            }
            let (g, d) = self.after_edit(&mut inner, affects);
            (done, g, d)
        };
        self.emit(WorkbenchEvent::ProjectChanged { dirty });
        self.emit(WorkbenchEvent::ViewChanged {
            view_generation: generation,
        });
        Ok(done)
    }

    pub fn redo(&self) -> Result<bool, RomlensError> {
        let (done, generation, dirty) = {
            let mut inner = self.lock();
            let affects = inner.undo.last_undone_affects_analysis();
            let Inner { project, undo, .. } = &mut *inner;
            let done = undo.redo(project, &self.rom.image)?;
            if !done {
                return Ok(false);
            }
            let (g, d) = self.after_edit(&mut inner, affects);
            (done, g, d)
        };
        self.emit(WorkbenchEvent::ProjectChanged { dirty });
        self.emit(WorkbenchEvent::ViewChanged {
            view_generation: generation,
        });
        Ok(done)
    }

    pub fn can_undo(&self) -> bool {
        self.lock().undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.lock().undo.can_redo()
    }

    pub fn undo_title(&self) -> Option<String> {
        self.lock().undo.undo_title().map(str::to_owned)
    }

    pub fn redo_title(&self) -> Option<String> {
        self.lock().undo.redo_title().map(str::to_owned)
    }

    // ---- project files -----------------------------------------------------

    /// The five package files, ready to write.
    pub fn project_files(&self) -> HashMap<String, Vec<u8>> {
        let inner = self.lock();
        io::to_files(&self.rom.image, &inner.project)
            .into_iter()
            .collect()
    }

    /// Replace the project with the given files (a Revert).
    pub fn load_project_files(&self, files: HashMap<String, Vec<u8>>) -> Result<(), RomlensError> {
        let files: std::collections::BTreeMap<String, Vec<u8>> = files.into_iter().collect();
        let identity = io::read_identity(&files)?;
        if !identity.matches(&self.rom.image) {
            return Err(RomlensError::RomMismatch {
                msg: "project belongs to a different ROM".into(),
            });
        }
        let project = io::from_files(&self.rom.image, &files)?;
        let generation = {
            let mut inner = self.lock();
            inner.project = project;
            inner.undo.clear();
            inner.dirty = false;
            inner.needs_analysis = true;
            self.refresh_lines(&mut inner)
        };
        self.emit(WorkbenchEvent::ProjectChanged { dirty: false });
        self.emit(WorkbenchEvent::ViewChanged {
            view_generation: generation,
        });
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.lock().dirty
    }

    pub fn mark_saved(&self) {
        self.lock().dirty = false;
        self.emit(WorkbenchEvent::ProjectChanged { dirty: false });
    }

    /// Write the package to a directory (the CLI path; the shell writes
    /// through `NSFileWrapper`).
    pub fn save_project(&self, path: String) -> Result<(), RomlensError> {
        let files = {
            let inner = self.lock();
            io::to_files(&self.rom.image, &inner.project)
        };
        io::write_package(std::path::Path::new(&path), &files)?;
        self.mark_saved();
        Ok(())
    }

    pub fn open_project_into(&self, path: String) -> Result<(), RomlensError> {
        let files = io::read_package(std::path::Path::new(&path))?;
        self.load_project_files(files.into_iter().collect())
    }

    pub fn rom_identity(&self) -> RomIdentityInfo {
        (&self.lock().project.rom).into()
    }

    // ---- export, resolve, search -------------------------------------------

    pub fn export_asar(&self, start: Option<u32>, len: Option<u32>) -> String {
        let inner = self.lock();
        let range = match (start, len) {
            (Some(s), Some(l)) => Some((s, l)),
            _ => None,
        };
        let mut out = String::new();
        io::export_asar(
            &self.rom.image,
            &inner.snapshot,
            &inner.project,
            io::AsarOptions {
                range,
                comments: true,
            },
            &mut out,
        );
        out
    }

    pub fn export_symbols(&self, include_auto: bool) -> String {
        let inner = self.lock();
        let mut out = String::new();
        io::export_symbols(
            &self.rom.image,
            &inner.snapshot,
            &inner.project,
            include_auto,
            &mut out,
        );
        out
    }

    pub fn resolve_any(&self, text: String) -> Result<ResolvedAny, RomlensError> {
        Ok(self.rom.image.resolve_any(&text)?.into())
    }

    pub fn search_bytes(
        &self,
        pattern: String,
        start: u32,
        len: u32,
        max: u32,
    ) -> Result<Vec<u32>, RomlensError> {
        let pattern = romlens_core::parse_pattern(&pattern)?;
        Ok(romlens_core::search_bytes(
            self.rom.image.bytes(),
            &pattern,
            start,
            len,
            max,
        ))
    }

    // ---- importing ---------------------------------------------------------

    /// Import a Mesen2 CDL, a bsnes-plus usage map or a Mesen execution log.
    ///
    /// Not a `Command`: an import is a body of observation rather than an edit
    /// with an inverse, and putting two megabytes of bitsets on the undo stack
    /// would be absurd. It marks the project dirty and re-analyzes like one.
    pub fn import_trace(
        &self,
        source: String,
        bytes: Vec<u8>,
    ) -> Result<ImportResult, RomlensError> {
        let io::import::Trace {
            format,
            coverage,
            exec_log,
        } = io::import::read(&bytes, &self.rom.image, None)?;
        if coverage.is_empty() && exec_log.as_ref().is_none_or(|l| l.is_empty()) {
            return Err(RomlensError::Project {
                msg: format!("{source} records nothing for this ROM"),
            });
        }
        let result = ImportResult {
            source: source.clone(),
            format: format.name().to_owned(),
            labels_added: 0,
            labels_replaced: 0,
            comments_added: 0,
            kept_user: 0,
            rewritten: Vec::new(),
            skipped: Vec::new(),
            notice: String::new(),
            detail: exec_log.as_ref().map(exec_log_summary).unwrap_or_default(),
            executed_bytes: coverage.executed.count(),
            read_bytes: coverage.read.count(),
            has_widths: coverage.flags.recorded,
        };
        let (generation, dirty) = {
            let mut inner = self.lock();
            inner.project.add_trace(
                model::TraceRecord {
                    source,
                    format: format.name().to_owned(),
                    executed_bytes: result.executed_bytes,
                    read_bytes: result.read_bytes,
                },
                coverage,
            );
            if let Some(log) = &exec_log {
                inner.project.add_exec_log(log);
            }
            inner.undo.clear();
            self.after_edit(&mut inner, true)
        };
        self.emit(WorkbenchEvent::ProjectChanged { dirty });
        self.emit(WorkbenchEvent::ViewChanged {
            view_generation: generation,
        });
        Ok(result)
    }

    /// Merge an execution log from a live session: the whole log on
    /// connecting, then each delta as it arrives. Like `import_trace`, but
    /// it keeps the undo history, since it runs every second while the game
    /// plays, and one source name covers the whole session. Returns how many
    /// instructions the project's log holds now that it did not before.
    pub fn merge_live_log(&self, source: String, bytes: Vec<u8>) -> Result<u64, RomlensError> {
        let log = io::import::exec_log::read(&bytes, self.rom.image.bytes())?;
        if log.is_empty() {
            return Ok(0);
        }
        let coverage = log.to_coverage(self.rom.image.bytes());
        let (added, generation, dirty) = {
            let mut inner = self.lock();
            let before = inner.project.exec_log.as_ref().map_or(0, |l| l.insns.len());
            let (executed_bytes, read_bytes) = (coverage.executed.count(), coverage.read.count());
            inner.project.add_trace(
                model::TraceRecord {
                    source,
                    format: io::TraceFormat::ExecLog.name().to_owned(),
                    executed_bytes,
                    read_bytes,
                },
                coverage,
            );
            inner.project.add_exec_log(&log);
            let after = inner.project.exec_log.as_ref().map_or(0, |l| l.insns.len());
            let (generation, dirty) = self.after_edit(&mut inner, true);
            ((after - before) as u64, generation, dirty)
        };
        self.emit(WorkbenchEvent::ProjectChanged { dirty });
        self.emit(WorkbenchEvent::ViewChanged {
            view_generation: generation,
        });
        Ok(added)
    }

    /// Import a symbol file. One undo entry, all or nothing.
    pub fn import_symbols(
        &self,
        source: String,
        text: String,
    ) -> Result<ImportResult, RomlensError> {
        use romlens_core::io::import::symbols;
        let file = symbols::read(&text, None)?;
        let format = symbols::detect(&text);
        let (result, generation, dirty) = {
            let mut inner = self.lock();
            let plan = symbols::plan(&self.rom.image, &inner.project, &file);
            let result = ImportResult {
                source: source.clone(),
                format: format.name().to_owned(),
                labels_added: plan.labels_added as u32,
                labels_replaced: plan.replaced as u32,
                comments_added: plan.comments_added as u32,
                kept_user: plan.kept_user.len() as u32,
                rewritten: file
                    .rewritten
                    .iter()
                    .map(|(from, to)| format!("{from} -> {to}"))
                    .collect(),
                skipped: file.skipped.clone(),
                notice: file.notice.clone(),
                detail: String::new(),
                executed_bytes: 0,
                read_bytes: 0,
                has_widths: false,
            };
            if !plan.commands.is_empty() {
                let entry = inner.project.apply_batch(
                    &self.rom.image,
                    plan.commands,
                    model::Origin::Import(source.clone()),
                )?;
                inner.undo.push(entry);
            }
            inner.project.add_import(model::ImportRecord {
                source,
                format: format.name().to_owned(),
                labels: file.labels.len() as u64,
                comments: file.comments.len() as u64,
                notice: file.notice,
            });
            let (generation, dirty) = self.after_edit(&mut inner, true);
            (result, generation, dirty)
        };
        self.emit(WorkbenchEvent::ProjectChanged { dirty });
        self.emit(WorkbenchEvent::ViewChanged {
            view_generation: generation,
        });
        Ok(result)
    }

    /// Refer to a recording from the project: its path and fingerprint,
    /// never its contents. Not an undoable edit, like a trace import; it
    /// marks the project dirty and changes no analysis.
    pub fn attach_recording(&self, reference: crate::graphics::RecordingRefInfo) {
        let dirty = {
            let mut inner = self.lock();
            inner
                .project
                .attach_recording(model::project::RecordingRef {
                    path: reference.path,
                    frames: reference.frames,
                    producer: reference.producer,
                    fingerprint: reference.fingerprint,
                });
            inner.dirty = true;
            inner.dirty
        };
        self.emit(WorkbenchEvent::ProjectChanged { dirty });
    }

    /// Stop referring to the recording at `path`; the file is left alone.
    pub fn detach_recording(&self, path: String) -> bool {
        let (removed, dirty) = {
            let mut inner = self.lock();
            let removed = inner.project.detach_recording(&path);
            inner.dirty |= removed;
            (removed, inner.dirty)
        };
        if removed {
            self.emit(WorkbenchEvent::ProjectChanged { dirty });
        }
        removed
    }

    /// The recordings the project refers to, in the order attached.
    pub fn recordings(&self) -> Vec<crate::graphics::RecordingRefInfo> {
        self.lock()
            .project
            .recordings
            .iter()
            .map(|r| crate::graphics::RecordingRefInfo {
                path: r.path.clone(),
                frames: r.frames,
                producer: r.producer.clone(),
                fingerprint: r.fingerprint.clone(),
            })
            .collect()
    }

    /// What has been imported, for the inspector and the licence notices.
    pub fn imports(&self) -> Vec<ImportResult> {
        let inner = self.lock();
        let mut out: Vec<ImportResult> = inner
            .project
            .traces
            .iter()
            .map(|t| ImportResult {
                source: t.source.clone(),
                format: t.format.clone(),
                labels_added: 0,
                labels_replaced: 0,
                comments_added: 0,
                kept_user: 0,
                rewritten: Vec::new(),
                skipped: Vec::new(),
                notice: String::new(),
                detail: String::new(),
                executed_bytes: t.executed_bytes,
                read_bytes: t.read_bytes,
                has_widths: false,
            })
            .collect();
        out.extend(inner.project.imports.iter().map(|i| ImportResult {
            source: i.source.clone(),
            format: i.format.clone(),
            labels_added: i.labels as u32,
            labels_replaced: 0,
            comments_added: i.comments as u32,
            kept_user: 0,
            rewritten: Vec::new(),
            skipped: Vec::new(),
            notice: i.notice.clone(),
            detail: String::new(),
            executed_bytes: 0,
            read_bytes: 0,
            has_widths: false,
        }));
        out
    }

    /// Search for hex bytes or text, with context around each hit.
    ///
    /// `text` matches the pattern's own bytes instead of parsing it as hex,
    /// and `ignore_case` matches ASCII letters in either case. The case mask
    /// is applied inside the search rather than over its results, or `max`
    /// would be spent on candidates before the first real hit.
    pub fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>, RomlensError> {
        let SearchQuery {
            pattern,
            text,
            ignore_case,
            start,
            len,
            max,
            context,
        } = query;
        let bytes = self.rom.image.bytes();
        let (offsets, width) = if text || ignore_case {
            let (p, mask) = romlens_core::pattern_from_text(&pattern, ignore_case)?;
            let n = p.len() as u32;
            (
                romlens_core::search_masked(bytes, &p, &mask, start, len, max),
                n,
            )
        } else {
            let p = romlens_core::parse_pattern(&pattern)?;
            let n = p.len() as u32;
            (romlens_core::search_bytes(bytes, &p, start, len, max), n)
        };
        let inner = self.lock();
        let n = bytes.len() as u32;
        Ok(offsets
            .into_iter()
            .map(|offset| {
                let from = offset.saturating_sub(context);
                let to = (offset + width + context).min(n);
                SearchHit {
                    file_offset: offset,
                    snes_address: self
                        .rom
                        .image
                        .snes_address_for(FileOffset(offset))
                        .map(|a| a.to_string()),
                    len: width,
                    context: bytes[from as usize..to as usize].to_vec(),
                    match_start: offset - from,
                    region_kind: inner
                        .snapshot
                        .region_at(FileOffset(offset))
                        .map_or_else(|| "unknown".to_owned(), |r| r.kind.name().to_owned()),
                }
            })
            .collect())
    }

    /// The raw text of one instruction without labels (for copy).
    pub fn plain_text(&self, file_offset: u32) -> Option<String> {
        let inner = self.lock();
        let rec = inner.snapshot.instruction_at(FileOffset(file_offset))?;
        let insn = inner.snapshot.decode_at(&self.rom.image, rec)?;
        Some(cpu65816::format_instruction(&insn, &NoSymbols).text)
    }
}

/// Find the ROM a project belongs to: hints by hash, then beside the package.
#[uniffi::export]
pub fn locate_rom(
    identity: RomIdentityInfo,
    package_dir: String,
    hints: Vec<String>,
) -> Option<String> {
    let identity: model::RomIdentity = identity.into();
    let hints: Vec<std::path::PathBuf> = hints.into_iter().map(Into::into).collect();
    io::locate_rom(&identity, std::path::Path::new(&package_dir), &hints)
        .map(|p| p.to_string_lossy().into_owned())
}

#[uniffi::export]
pub fn hardware_register(address: u32) -> Option<HardwareRegisterInfo> {
    model::hardware_register(address as u16).map(HardwareRegisterInfo::from)
}

#[uniffi::export]
pub fn all_hardware_registers() -> Vec<HardwareRegisterInfo> {
    model::all_hardware_registers()
        .into_iter()
        .map(HardwareRegisterInfo::from)
        .collect()
}

#[uniffi::export]
pub fn asm_line_stride() -> u16 {
    romlens_core::ASM_LINE_STRIDE
}

#[uniffi::export]
pub fn asm_batch_header_len() -> u16 {
    romlens_core::ASM_BATCH_HEADER_LEN as u16
}

/// The sentinel the line batch and `line_numbers_for_bytes` use for "none".
#[uniffi::export]
pub fn asm_none_address() -> u32 {
    NONE_ADDRESS
}

#[uniffi::export]
pub fn validate_label_name(name: String, address: Option<u32>) -> Result<(), RomlensError> {
    Ok(model::validate_label_name(
        &name,
        address.map(SnesAddress::from_u24),
    )?)
}

/// Read a package directory into its files (the CLI-style open).
#[uniffi::export]
pub fn read_project_package(path: String) -> Result<HashMap<String, Vec<u8>>, RomlensError> {
    Ok(io::read_package(std::path::Path::new(&path))?
        .into_iter()
        .collect())
}

/// The ROM identity stored in a package's files, before the ROM is loaded.
#[uniffi::export]
pub fn project_identity(files: HashMap<String, Vec<u8>>) -> Result<RomIdentityInfo, RomlensError> {
    let files: std::collections::BTreeMap<String, Vec<u8>> = files.into_iter().collect();
    Ok((&io::read_identity(&files)?).into())
}

/// What an execution log holds, for the import summary.
fn exec_log_summary(log: &romlens_core::model::exec_log::ExecLog) -> String {
    let mixed = log.insns.iter().filter(|i| i.mixed_widths()).count();
    format!(
        "{} instructions ({} in more than one width), {} access runs, {} transfers, {} DMA runs",
        log.insns.len(),
        mixed,
        log.accesses.len(),
        log.flows.len(),
        log.dma.len()
    )
}

fn variable_info(
    rom: &romlens_core::RomImage,
    project: &Project,
    address: SnesAddress,
    ty: model::VarType,
) -> VariableInfo {
    use romlens_core::memory::map::MemoryClass;
    let memory = match rom.map().classify(address) {
        MemoryClass::Wram | MemoryClass::LowRam => "WRAM",
        MemoryClass::Sram => "SRAM",
        MemoryClass::Hardware => "register",
        MemoryClass::Rom => "ROM",
        MemoryClass::OpenBus => "unmapped",
    };
    VariableInfo {
        address: address.as_u24(),
        name: project
            .labels
            .get(&address)
            .map(|l| l.name.clone())
            .unwrap_or_default(),
        width: ty.width.into(),
        count: ty.count,
        len: ty.len(),
        description: ty.describe(),
        memory: memory.to_owned(),
    }
}

impl Workbench {
    /// Apply `commands` as one undo step of the user's.
    fn apply_user_batch(&self, commands: Vec<model::Command>) -> Result<(), RomlensError> {
        let affects = commands.iter().any(model::Command::affects_analysis);
        let (generation, dirty) = {
            let mut inner = self.lock();
            let entry =
                inner
                    .project
                    .apply_batch(&self.rom.image, commands, model::Origin::User)?;
            inner.undo.push(entry);
            self.after_edit(&mut inner, affects)
        };
        self.emit(WorkbenchEvent::ProjectChanged { dirty });
        self.emit(WorkbenchEvent::ViewChanged {
            view_generation: generation,
        });
        Ok(())
    }
}

/// `snes.h`, which every decompiled routine includes.
#[uniffi::export]
pub fn snes_header() -> String {
    romlens_core::decompile::snes_h()
}
