//! The immutable result of one analysis run.

use std::collections::BTreeMap;

use crate::cpu65816::{FlagState, Instruction, decode};
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::label::Label;
use crate::model::region::Region;
use crate::model::xref::XRef;
use crate::rom::image::RomImage;

/// One decoded instruction, 12 bytes: enough to re-decode it exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsnRecord {
    pub offset: u32,
    pub len: u8,
    pub opcode: u8,
    /// bit0 m, bit1 x, bit2 e, bit3 dbr known, bit4 dp known, bit5 c known, bit6 c.
    pub flags: u8,
    pub assumptions: u8,
    pub dbr: u8,
    pub dp: u16,
}

impl InsnRecord {
    pub fn from_instruction(i: &Instruction) -> Self {
        let f = i.flags_before;
        let mut flags = f.packed();
        if let Some(c) = f.c {
            flags |= 0x20 | ((c as u8) << 6);
        }
        Self {
            offset: i.file_offset.0,
            len: i.len,
            opcode: i.opcode,
            flags,
            assumptions: i.assumptions,
            dbr: f.dbr.unwrap_or(0),
            dp: f.dp.unwrap_or(0),
        }
    }

    pub fn flags_before(&self) -> FlagState {
        FlagState {
            m: self.flags & 0x01 != 0,
            x: self.flags & 0x02 != 0,
            e: self.flags & 0x04 != 0,
            dbr: (self.flags & 0x08 != 0).then_some(self.dbr),
            dp: (self.flags & 0x10 != 0).then_some(self.dp),
            c: (self.flags & 0x20 != 0).then_some(self.flags & 0x40 != 0),
        }
    }

    pub fn end(&self) -> u32 {
        self.offset + self.len as u32
    }

    pub fn contains(&self, off: u32) -> bool {
        off >= self.offset && off < self.end()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WarningKind {
    ComputedJump,
    FlagConflict,
    UnknownCarryXce,
    SuspiciousEntry,
    BankWrap,
    WalkedIntoUserData,
}

impl WarningKind {
    pub const fn name(self) -> &'static str {
        match self {
            WarningKind::ComputedJump => "computed jump",
            WarningKind::FlagConflict => "flag conflict",
            WarningKind::UnknownCarryXce => "unknown carry at XCE",
            WarningKind::SuspiciousEntry => "suspicious entry",
            WarningKind::BankWrap => "bank wrap",
            WarningKind::WalkedIntoUserData => "walked into user data",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub offset: FileOffset,
    pub kind: WarningKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnalysisStats {
    pub code_bytes: u64,
    pub data_bytes: u64,
    pub unknown_bytes: u64,
    pub instructions: u64,
    pub blocks: u64,
    pub labels: u64,
    pub xrefs: u64,
    pub conflicts: u64,
    pub warnings: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct AnalysisSnapshot {
    /// Sorted by offset, non-overlapping.
    pub instructions: Vec<InsnRecord>,
    /// Sorted, non-overlapping, covering the whole image.
    pub regions: Vec<Region>,
    pub auto_labels: BTreeMap<SnesAddress, Label>,
    /// Sorted by target then source.
    pub xrefs_by_target: Vec<XRef>,
    /// Sorted by source then target.
    pub xrefs_by_source: Vec<XRef>,
    /// Sorted by offset.
    pub warnings: Vec<Warning>,
    pub stats: AnalysisStats,
}

impl AnalysisSnapshot {
    /// The instruction record containing `off`.
    pub fn instruction_at(&self, off: FileOffset) -> Option<&InsnRecord> {
        let i = self.instructions.partition_point(|r| r.offset <= off.0);
        i.checked_sub(1)
            .map(|i| &self.instructions[i])
            .filter(|r| r.contains(off.0))
    }

    /// Index of the first record at or after `off`.
    pub fn instruction_index_from(&self, off: u32) -> usize {
        self.instructions.partition_point(|r| r.offset < off)
    }

    pub fn region_at(&self, off: FileOffset) -> Option<&Region> {
        let i = self.regions.partition_point(|r| r.start.0 <= off.0);
        i.checked_sub(1)
            .map(|i| &self.regions[i])
            .filter(|r| r.contains(off.0))
    }

    /// Regions intersecting `[start, start + len)`.
    pub fn regions_in(&self, start: u32, len: u32) -> &[Region] {
        let end = start.saturating_add(len);
        let first = self.regions.partition_point(|r| r.end() <= start);
        let last = self.regions.partition_point(|r| r.start.0 < end);
        &self.regions[first..last.max(first)]
    }

    pub fn xrefs_to(&self, target: SnesAddress) -> &[XRef] {
        let first = self.xrefs_by_target.partition_point(|x| x.to < target);
        let last = self.xrefs_by_target.partition_point(|x| x.to <= target);
        &self.xrefs_by_target[first..last]
    }

    pub fn xrefs_from(&self, from: FileOffset) -> &[XRef] {
        let first = self.xrefs_by_source.partition_point(|x| x.from < from);
        let last = self.xrefs_by_source.partition_point(|x| x.from <= from);
        &self.xrefs_by_source[first..last]
    }

    pub fn warnings_at(&self, off: FileOffset) -> &[Warning] {
        let first = self.warnings.partition_point(|w| w.offset < off);
        let last = self.warnings.partition_point(|w| w.offset <= off);
        &self.warnings[first..last]
    }

    /// Re-decode a record at its canonical address.
    pub fn decode_at(&self, rom: &RomImage, rec: &InsnRecord) -> Option<Instruction> {
        let off = FileOffset(rec.offset);
        let addr = rom.snes_address_for(off)?;
        let bytes = rom.bytes().get(rec.offset as usize..)?;
        decode(bytes, addr, off, rec.flags_before())
    }
}
