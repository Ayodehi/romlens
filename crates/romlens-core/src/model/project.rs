//! The project overlay: everything a person adds on top of a ROM. Read-only
//! input to the analyzer; mutated only through [`Command`]s so every change
//! has an inverse.

use std::collections::BTreeMap;

use crate::error::ProjectError;
use crate::memory::address::{FileOffset, SnesAddress};
use crate::memory::map::MappingMode;
use crate::model::command::{Command, UndoEntry};
use crate::model::comment::{Comment, CommentKind};
use crate::model::label::{Label, LabelSource, validate_label_name};
use crate::model::region::{OverrideKind, RegionOverride};
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

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub rom: RomIdentity,
    /// User and imported labels by canonical address.
    pub labels: BTreeMap<SnesAddress, Label>,
    pub comments: BTreeMap<(SnesAddress, CommentKind), Comment>,
    /// Sorted, non-overlapping.
    pub region_overrides: Vec<RegionOverride>,
    pub flag_overrides: BTreeMap<FileOffset, FlagOverride>,
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
            settings: Settings::default(),
        }
    }

    /// The one address every mirror of a ROM byte is stored under; RAM and
    /// hardware addresses are kept as written.
    pub fn canonical(rom: &RomImage, addr: SnesAddress) -> SnesAddress {
        rom.file_offset_for(addr)
            .and_then(|off| rom.snes_address_for(off))
            .unwrap_or(addr)
    }

    pub fn label_at(&self, addr: SnesAddress) -> Option<&Label> {
        self.labels.get(&addr)
    }

    pub fn comment_at(&self, addr: SnesAddress, kind: CommentKind) -> Option<&Comment> {
        self.comments.get(&(addr, kind))
    }

    pub fn region_override_at(&self, off: FileOffset) -> Option<&RegionOverride> {
        self.region_overrides
            .iter()
            .find(|r| off.0 >= r.start.0 && off.0 < r.end())
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
                    start: r.start,
                    len: start - r.start.0,
                    kind: r.kind,
                });
            }
            if r.end() > end {
                kept.push(RegionOverride {
                    start: FileOffset(end),
                    len: r.end() - end,
                    kind: r.kind,
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

    /// Apply a command and return how to undo it.
    pub fn apply(&mut self, rom: &RomImage, cmd: Command) -> Result<UndoEntry, ProjectError> {
        let title = cmd.menu_title().to_owned();
        let inverse = match &cmd {
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
                                source: LabelSource::User,
                            },
                        );
                    }
                    None => {
                        self.labels.remove(&address);
                    }
                }
                vec![Command::SetLabel {
                    address,
                    name: previous.map(|l| l.name),
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
                });
                let mut inv = vec![Command::ClearRegionOverride {
                    start: *start,
                    len: *len,
                }];
                inv.extend(touched.into_iter().map(|r| Command::MarkRegion {
                    start: r.start,
                    len: r.len,
                    kind: r.kind,
                }));
                inv
            }
            Command::ClearRegionOverride { start, len } => {
                Self::check_range(rom, start.0, *len)?;
                let touched = self.cut_overrides(start.0, start.0 + len);
                touched
                    .into_iter()
                    .map(|r| Command::MarkRegion {
                        start: r.start,
                        len: r.len,
                        kind: r.kind,
                    })
                    .collect()
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
        Ok(UndoEntry {
            done: cmd,
            inverse,
            title,
        })
    }

    /// The override kind covering an offset, if any.
    pub fn override_kind_at(&self, off: u32) -> Option<OverrideKind> {
        self.region_override_at(FileOffset(off)).map(|r| r.kind)
    }
}
