//! The project overlay: everything a person adds on top of a ROM. Read-only
//! input to the analyzer; mutated only through [`Command`]s so every change
//! has an inverse.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::ProjectError;
use crate::memory::address::{FileOffset, SnesAddress};
use crate::memory::map::MappingMode;
use crate::model::command::{Command, Origin, UndoEntry};
use crate::model::comment::{Comment, CommentKind};
use crate::model::coverage::Coverage;
use crate::model::exec_log::ExecLog;
use crate::model::label::{Label, validate_label_name};
use crate::model::region::{OverrideKind, RegionOverride, RegionParams};
use crate::model::variable::VarType;
use crate::rom::image::RomImage;
use crate::viewmodel::hex_rows::AddressStyle;

/// What identifies the ROM a project belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomIdentity {
    pub sha256: String,
    pub size: u32,
    pub mapping: MappingMode,
    pub fast_rom: bool,
    pub title: String,
}

impl RomIdentity {
    pub fn of(rom: &RomImage) -> Self {
        Self {
            sha256: rom.sha256_hex(),
            size: rom.len() as u32,
            mapping: rom.mapping(),
            fast_rom: rom.header().is_fast_rom(),
            title: rom.header().title.clone(),
        }
    }

    /// Same payload (by SHA-256).
    pub fn matches(&self, rom: &RomImage) -> bool {
        self.sha256 == rom.sha256_hex()
    }
}

/// Per-address flag pin. `None` fields keep the analyzer's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlagOverride {
    pub m: Option<bool>,
    pub x: Option<bool>,
    pub e: Option<bool>,
    pub dbr: Option<u8>,
    pub dp: Option<u16>,
}

impl FlagOverride {
    pub fn is_empty(&self) -> bool {
        *self == FlagOverride::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub address_style: AddressStyle,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            address_style: AddressStyle::Both,
        }
    }
}

/// One imported trace, as `project.json` records it.
///
/// This is provenance only. The coverage itself is stored once, merged, under
/// `traces/` — merging is a union and the split is not recoverable, so keeping
/// one file per import would mean either storing several megabytes per trace
/// or writing one import's name against all of it. What a reader needs is the
/// list of what went in, and that is this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceRecord {
    /// The file it was read from, which is also the identity for re-import.
    pub source: String,
    /// `cdl` or `usage`.
    pub format: String,
    /// What this trace contributed, before merging.
    pub executed_bytes: u64,
    pub read_bytes: u64,
}

/// One imported symbol file, as `project.json` records it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportRecord {
    pub source: String,
    /// `wla`, `nocash` or `lbl`.
    pub format: String,
    pub labels: u64,
    pub comments: u64,
    /// The file's leading comment block, kept because a licence notice has to
    /// travel with what it covers (`12-content-policy.md` rule 7).
    pub notice: String,
}

/// A recording the project refers to: where it is and how to know it is
/// still the same file. Never its contents — a recording holds the game's
/// VRAM, CGRAM and OAM (`12-content-policy.md` rule 4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecordingRef {
    /// Where it was attached from, as given.
    pub path: String,
    pub frames: u64,
    pub producer: String,
    /// The recording's length, frame count and index CRC-32, as
    /// `len:frames:crc`: what the change index keys on, and enough to tell
    /// that the file at `path` has been replaced.
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub rom: RomIdentity,
    /// User and imported labels by canonical address.
    pub labels: BTreeMap<SnesAddress, Label>,
    pub comments: BTreeMap<(SnesAddress, CommentKind), Comment>,
    /// Sorted, non-overlapping.
    pub region_overrides: Vec<RegionOverride>,
    pub flag_overrides: BTreeMap<FileOffset, FlagOverride>,
    /// Variable types by canonical address; each one's name is the label at
    /// the same address.
    pub variables: BTreeMap<SnesAddress, VarType>,
    /// Every imported trace, merged.
    ///
    /// Not a `Command`, and deliberately so: an import is not an edit with an
    /// inverse but a body of observation, and putting two megabytes of bitsets
    /// on the undo stack would be absurd. `Arc` because the analyzer clones the
    /// project on every run.
    pub coverage: Option<Arc<Coverage>>,
    /// Every imported execution log, merged. Its coverage is in `coverage`
    /// too; this keeps the relationships a code/data log cannot hold: which
    /// instruction read what, where transfers went, what DMA moved.
    pub exec_log: Option<Arc<ExecLog>>,
    /// What was imported, in import order.
    pub traces: Vec<TraceRecord>,
    /// Symbol files imported, in import order. The labels themselves live in
    /// `labels` like any other; this is provenance and the licence notice.
    pub imports: Vec<ImportRecord>,
    /// Recordings attached to the project, by reference.
    pub recordings: Vec<RecordingRef>,
    pub settings: Settings,
}

impl Project {
    pub fn new(rom: &RomImage) -> Self {
        Self {
            rom: RomIdentity::of(rom),
            labels: BTreeMap::new(),
            comments: BTreeMap::new(),
            region_overrides: Vec::new(),
            flag_overrides: BTreeMap::new(),
            variables: BTreeMap::new(),
            coverage: None,
            exec_log: None,
            traces: Vec::new(),
            imports: Vec::new(),
            recordings: Vec::new(),
            settings: Settings::default(),
        }
    }

    /// Merge a trace in and record where it came from.
    pub fn add_trace(&mut self, record: TraceRecord, coverage: Coverage) {
        match &mut self.coverage {
            Some(existing) => Arc::make_mut(existing).union(&coverage),
            None => self.coverage = Some(Arc::new(coverage)),
        }
        self.traces.retain(|t| t.source != record.source);
        self.traces.push(record);
    }

    /// Merge an execution log in. Its coverage goes through `add_trace`
    /// like any other trace's; this keeps the log.
    pub fn add_exec_log(&mut self, log: &ExecLog) {
        match &mut self.exec_log {
            Some(existing) => Arc::make_mut(existing).merge(log),
            None => self.exec_log = Some(Arc::new(log.clone())),
        }
    }

    /// Record a symbol file that was imported. The labels are applied through
    /// `apply_batch` so they are undoable; this is only the provenance.
    pub fn add_import(&mut self, record: ImportRecord) {
        self.imports.retain(|i| i.source != record.source);
        self.imports.push(record);
    }

    /// Refer to a recording, replacing any earlier reference to the same
    /// path. Like a trace, this is not an undoable edit.
    pub fn attach_recording(&mut self, r: RecordingRef) {
        self.recordings.retain(|x| x.path != r.path);
        self.recordings.push(r);
    }

    /// Drop the reference to `path`; whether there was one.
    pub fn detach_recording(&mut self, path: &str) -> bool {
        let before = self.recordings.len();
        self.recordings.retain(|x| x.path != path);
        self.recordings.len() != before
    }

    /// The one address every mirror is stored under: a ROM byte's canonical
    /// address, `$7E` for the low 8 KB of WRAM that the system banks mirror
    /// at `$0000-$1FFF`, and bank `$00` for the hardware registers. So
    /// `STA $0094` run from bank `$80` and `STA $7E0094` name one variable.
    pub fn canonical(rom: &RomImage, addr: SnesAddress) -> SnesAddress {
        use crate::memory::map::MemoryClass;
        if let Some(a) = rom
            .file_offset_for(addr)
            .and_then(|off| rom.snes_address_for(off))
        {
            return a;
        }
        match rom.map().classify(addr) {
            MemoryClass::LowRam => SnesAddress::new(0x7E, addr.offset()),
            MemoryClass::Hardware => SnesAddress::new(0x00, addr.offset()),
            _ => addr,
        }
    }

    /// The variable spanning `addr`, with its start, when one does.
    pub fn variable_containing(&self, addr: SnesAddress) -> Option<(SnesAddress, VarType)> {
        let (&start, &ty) = self.variables.range(..=addr).next_back()?;
        (start.bank() == addr.bank() && ((addr.offset() - start.offset()) as u32) < ty.len())
            .then_some((start, ty))
    }

    pub fn label_at(&self, addr: SnesAddress) -> Option<&Label> {
        self.labels.get(&addr)
    }

    pub fn comment_at(&self, addr: SnesAddress, kind: CommentKind) -> Option<&Comment> {
        self.comments.get(&(addr, kind))
    }

    /// The overrides are sorted and non-overlapping, so the only candidate is
    /// the last one starting at or before `off`. `analyze` calls this once per
    /// region and an import can add thousands of overrides at a stroke (2A.4),
    /// so the scan this replaced would have gone quadratic.
    pub fn region_override_at(&self, off: FileOffset) -> Option<&RegionOverride> {
        let i = self
            .region_overrides
            .partition_point(|r| r.start.0 <= off.0);
        self.region_overrides
            .get(i.checked_sub(1)?)
            .filter(|r| off.0 < r.end())
    }

    /// Overrides intersecting `[start, start + len)`.
    pub fn region_overrides_in(&self, start: u32, len: u32) -> Vec<RegionOverride> {
        let end = start + len;
        self.region_overrides
            .iter()
            .filter(|r| r.start.0 < end && r.end() > start)
            .copied()
            .collect()
    }

    fn check_range(rom: &RomImage, start: u32, len: u32) -> Result<(), ProjectError> {
        let end = start as u64 + len as u64;
        if len == 0 || end > rom.len() as u64 {
            return Err(ProjectError::BadRange(format!(
                "{}+{len}",
                FileOffset(start)
            )));
        }
        Ok(())
    }

    /// Remove `[start, end)` from the override list, splitting partial
    /// overlaps. Returns the originals that were touched, in full.
    fn cut_overrides(&mut self, start: u32, end: u32) -> Vec<RegionOverride> {
        let mut touched = Vec::new();
        let mut kept = Vec::with_capacity(self.region_overrides.len() + 2);
        for r in self.region_overrides.drain(..) {
            if r.start.0 >= end || r.end() <= start {
                kept.push(r);
                continue;
            }
            touched.push(r);
            if r.start.0 < start {
                kept.push(RegionOverride {
                    len: start - r.start.0,
                    ..r
                });
            }
            if r.end() > end {
                kept.push(RegionOverride {
                    start: FileOffset(end),
                    len: r.end() - end,
                    ..r
                });
            }
        }
        kept.sort_by_key(|r| r.start);
        self.region_overrides = kept;
        touched
    }

    fn insert_override(&mut self, r: RegionOverride) {
        let i = self.region_overrides.partition_point(|x| x.start < r.start);
        self.region_overrides.insert(i, r);
    }

    /// Apply one command and return the commands that take it back. Nothing
    /// outside this file calls it: an edit is always a batch, even of one, so
    /// there is a single place where a failure half-way is rolled back.
    fn apply_one(
        &mut self,
        rom: &RomImage,
        cmd: &Command,
        origin: &Origin,
    ) -> Result<Vec<Command>, ProjectError> {
        let inverse = match cmd {
            Command::SetLabel { address, name } => {
                let address = Self::canonical(rom, *address);
                if let Some(name) = name {
                    validate_label_name(name, Some(address))?;
                }
                let previous = self.labels.get(&address).cloned();
                match name {
                    Some(name) => {
                        self.labels.insert(
                            address,
                            Label {
                                address,
                                name: name.clone(),
                                source: origin.label_source(),
                            },
                        );
                    }
                    None => {
                        self.labels.remove(&address);
                    }
                }
                vec![Command::RestoreLabel {
                    address,
                    label: previous,
                }]
            }
            Command::RestoreLabel { address, label } => {
                let address = Self::canonical(rom, *address);
                let previous = match label {
                    Some(l) => self.labels.insert(address, l.clone()),
                    None => self.labels.remove(&address),
                };
                vec![Command::RestoreLabel {
                    address,
                    label: previous,
                }]
            }
            Command::SetComment {
                address,
                kind,
                text,
            } => {
                let address = Self::canonical(rom, *address);
                let key = (address, *kind);
                let previous = self.comments.get(&key).map(|c| c.text.clone());
                match text.as_deref().map(str::trim_end).filter(|t| !t.is_empty()) {
                    Some(text) => {
                        self.comments.insert(
                            key,
                            Comment {
                                address,
                                kind: *kind,
                                text: text.to_owned(),
                            },
                        );
                    }
                    None => {
                        self.comments.remove(&key);
                    }
                }
                vec![Command::SetComment {
                    address,
                    kind: *kind,
                    text: previous,
                }]
            }
            Command::MarkRegion { start, len, kind } => {
                Self::check_range(rom, start.0, *len)?;
                let touched = self.cut_overrides(start.0, start.0 + len);
                self.insert_override(RegionOverride {
                    start: *start,
                    len: *len,
                    kind: *kind,
                    params: RegionParams::default(),
                });
                let mut inv = vec![Command::ClearRegionOverride {
                    start: *start,
                    len: *len,
                }];
                inv.extend(restore(touched));
                inv
            }
            Command::ClearRegionOverride { start, len } => {
                Self::check_range(rom, start.0, *len)?;
                restore(self.cut_overrides(start.0, start.0 + len))
            }
            Command::SetRegionParams { start, params } => {
                let r = self
                    .region_overrides
                    .iter_mut()
                    .find(|r| r.start == *start)
                    .ok_or_else(|| ProjectError::NotMarked(start.to_string()))?;
                let previous = r.params;
                r.params = *params;
                vec![Command::SetRegionParams {
                    start: *start,
                    params: previous,
                }]
            }
            Command::SetVariable { address, ty } => {
                let address = Self::canonical(rom, *address);
                if let Some(ty) = ty {
                    ty.validate()?;
                    if address.offset() as u32 + ty.len() > 0x1_0000 {
                        return Err(ProjectError::InvalidVariable(format!(
                            "{} bytes from {address} run past the end of the bank",
                            ty.len()
                        )));
                    }
                }
                let previous = match ty {
                    Some(ty) => self.variables.insert(address, *ty),
                    None => self.variables.remove(&address),
                };
                vec![Command::SetVariable {
                    address,
                    ty: previous,
                }]
            }
            Command::SetFlagOverride { offset, flags } => {
                Self::check_range(rom, offset.0, 1)?;
                let previous = self.flag_overrides.get(offset).copied();
                match flags.filter(|f| !f.is_empty()) {
                    Some(f) => {
                        self.flag_overrides.insert(*offset, f);
                    }
                    None => {
                        self.flag_overrides.remove(offset);
                    }
                }
                vec![Command::SetFlagOverride {
                    offset: *offset,
                    flags: previous,
                }]
            }
        };
        Ok(inverse)
    }

    /// Apply a command and return how to undo it.
    pub fn apply(&mut self, rom: &RomImage, cmd: Command) -> Result<UndoEntry, ProjectError> {
        self.apply_batch(rom, vec![cmd], Origin::User)
    }

    /// Apply commands as one undoable step.
    ///
    /// The batch is all-or-nothing: if any command is refused, the ones already
    /// applied are rolled back before the error is returned, so a symbol file
    /// with one bad entry cannot leave a project half-imported. Rolling back
    /// uses the inverses just computed, which is the same path undo takes.
    pub fn apply_batch(
        &mut self,
        rom: &RomImage,
        commands: Vec<Command>,
        origin: Origin,
    ) -> Result<UndoEntry, ProjectError> {
        let title = origin.title(&commands);
        let mut inverse: Vec<Command> = Vec::with_capacity(commands.len());
        for cmd in &commands {
            match self.apply_one(rom, cmd, &origin) {
                // Prepending keeps `inverse` in undo order: last done, first
                // undone.
                Ok(inv) => {
                    inverse.splice(0..0, inv);
                }
                Err(e) => {
                    // Unwinding cannot fail: every inverse was produced by a
                    // command this project just accepted.
                    for undo in std::mem::take(&mut inverse) {
                        let _ = self.apply_one(rom, &undo, &origin);
                    }
                    return Err(e);
                }
            }
        }
        Ok(UndoEntry {
            done: commands,
            inverse,
            title,
            origin,
        })
    }

    /// The override kind covering an offset, if any.
    pub fn override_kind_at(&self, off: u32) -> Option<OverrideKind> {
        self.region_override_at(FileOffset(off)).map(|r| r.kind)
    }
}

/// The commands that put cut overrides back exactly: the mark, then its
/// preview options if it had any.
fn restore(touched: Vec<RegionOverride>) -> Vec<Command> {
    let mut out = Vec::with_capacity(touched.len());
    for r in touched {
        out.push(Command::MarkRegion {
            start: r.start,
            len: r.len,
            kind: r.kind,
        });
        if !r.params.is_default() {
            out.push(Command::SetRegionParams {
                start: r.start,
                params: r.params,
            });
        }
    }
    out
}
