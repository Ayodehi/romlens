//! The per-document object: ROM + project + analysis snapshot + line index +
//! undo stack, behind one mutex. Queries during an analysis see the previous
//! snapshot; the analysis runs on its own thread and installs its result when
//! the awaiting future is polled.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use romlens_core::analysis::{AnalysisControl, AnalysisSnapshot, analyze};
use romlens_core::cpu65816::{self, NoSymbols};
use romlens_core::io;
use romlens_core::model::{self, Project, Symbols, UndoStack};
use romlens_core::viewmodel::asm_lines::NONE_ADDRESS;
use romlens_core::viewmodel::hex_rows::{BATCH_HEADER_LEN, BYTES_PER_ROW, ROW_STRIDE};
use romlens_core::{
    FileOffset, LineIndex, SnesAddress, TextOptions, encode_lines, encode_rows, format_lines_text,
};

use crate::records::*;
use crate::{Rom, RomlensError};

/// Receives workbench events, possibly from the analysis thread.
#[uniffi::export(with_foreign)]
pub trait WorkbenchListener: Send + Sync {
    fn on_event(&self, event: WorkbenchEvent);
}

struct Inner {
    project: Project,
    snapshot: Arc<AnalysisSnapshot>,
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
    inner: Mutex<Inner>,
    listener: Mutex<Option<Arc<dyn WorkbenchListener>>>,
    cancel: Arc<AtomicBool>,
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
            inner: Mutex::new(Inner {
                project,
                snapshot: Arc::new(AnalysisSnapshot::default()),
                lines: Arc::new(LineIndex::default()),
                undo: UndoStack::default(),
                analysis_generation: 0,
                view_generation: 0,
                dirty: false,
                needs_analysis: true,
            }),
            listener: Mutex::new(None),
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }

    fn install(&self, snapshot: AnalysisSnapshot, lines: LineIndex) -> AnalysisStats {
        let stats = snapshot.stats.into();
        let generation = {
            let mut inner = self.lock();
            inner.snapshot = Arc::new(snapshot);
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

    /// Rebuild the line index after a label or comment change.
    fn refresh_lines(&self, inner: &mut Inner) -> u64 {
        inner.lines = Arc::new(LineIndex::build(
            &self.rom.image,
            &inner.snapshot,
            &inner.project,
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
    ) -> impl FnOnce() -> Result<(AnalysisSnapshot, LineIndex), romlens_core::analysis::Cancelled>
    + Send
    + 'static {
        let image = self.rom.image.clone();
        let project = self.lock().project.clone();
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
            let snapshot = analyze(&image, &project, &control)?;
            control.report(romlens_core::analysis::AnalysisPhase::Lines, 0, 1);
            let lines = LineIndex::build(&image, &snapshot, &project);
            control.report(romlens_core::analysis::AnalysisPhase::Lines, 1, 1);
            Ok((snapshot, lines))
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
            Ok((snapshot, lines)) => Ok(self.install(snapshot, lines)),
            Err(_) => Err(RomlensError::Cancelled),
        }
    }

    /// The same on the calling thread (tests, CLI-style use).
    pub fn analyze_blocking(&self) -> Result<AnalysisStats, RomlensError> {
        let job = self.analysis_job();
        match job() {
            Ok((snapshot, lines)) => Ok(self.install(snapshot, lines)),
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
    pub fn regions_summary(&self) -> Vec<RegionInfo> {
        self.lock()
            .snapshot
            .regions
            .iter()
            .map(RegionInfo::from)
            .collect()
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
                .is_some_and(|e| e.done.affects_analysis());
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
