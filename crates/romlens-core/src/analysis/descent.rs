//! Recursive descent from the vectors and the user's code marks, tracking
//! flags along every static edge.
//!
//! Two things the straight walk cannot see are recovered by bookkeeping:
//! how far the M/X widths still rest on an assumption (`WidthTrust`), and
//! callees that add to their stacked return address to skip inline
//! arguments (`found_inline_args`, fed back through `inline_args` by a
//! second pass in `analysis::analyze`).

use std::collections::{BTreeMap, VecDeque};

use crate::analysis::control::{AnalysisControl, AnalysisPhase, Cancelled};
use crate::analysis::flow::refine;
use crate::analysis::inline::ReturnAdjust;
use crate::analysis::snapshot::{InsnRecord, Warning, WarningKind};
use crate::analysis::xrefs::{vector_slots, xref_for};
use crate::cpu65816::{
    ASSUMED_DBR, ASSUMED_WIDTHS, ASSUMED_XCE_CARRY, AddressingMode, BANK_WRAP, FlagState,
    Instruction, Mnemonic, TargetKind, decode,
};
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::project::{FlagOverride, Project};
use crate::model::region::{DataKind, OverrideKind};
use crate::model::xref::{XRef, XRefKind};
use crate::rom::image::RomImage;

pub const KIND_NONE: u8 = 0;
pub const KIND_OPCODE: u8 = 1;
pub const KIND_OPERAND: u8 = 2;
/// Inline arguments after a call: skipped by the caller, never decoded.
pub const KIND_INLINE: u8 = 3;

pub const OVR_NONE: u8 = 0;
pub const OVR_CODE: u8 = 1;
pub const OVR_WALL: u8 = 2;

/// Where a `DataSeed` came from (decides its tag and evidence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedSource {
    /// An instruction read or wrote there (0.6).
    Operand,
    /// A `JMP (abs)` / `JML [abs]` slot (gets a `PTR_` label, 0.9).
    JumpPointer,
    /// Bytes after a call that the callee skips by adjusting its return
    /// address (0.8).
    InlineArgument,
}

/// A data range an instruction pointed at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DataSeed {
    pub offset: u32,
    pub len: u32,
    pub kind: DataKind,
    pub confidence: f32,
    pub source: SeedSource,
}

/// How far the operand widths of a walk rest on an assumption. The `PLP`
/// or `XCE` that made the assumption decodes exactly; what it puts at risk
/// is every later instruction whose length depends on M or X, and the
/// alignment of the stream after the first of those.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct WidthTrust {
    /// M was assumed (a `PLP`, or an `XCE` with unknown carry).
    m: bool,
    /// X was assumed.
    x: bool,
    /// An instruction was decoded under an assumed width: the rest of the
    /// walk may be misaligned.
    uncertain: bool,
}

struct Entry {
    offset: u32,
    address: SnesAddress,
    flags: FlagState,
    depth: u16,
    /// A fresh entry point (vector, call target, user mark): checked for a
    /// suspicious first instruction.
    entry_point: bool,
    /// A user-seeded entry: defers to any walk that already covered it
    /// instead of reporting a conflict.
    soft: bool,
    trust: WidthTrust,
}

pub struct Walk<'a> {
    pub rom: &'a RomImage,
    pub project: &'a Project,
    /// Per byte: none, opcode start or operand byte.
    pub kind: Vec<u8>,
    /// Per opcode start: `width_key + 1`.
    pub wkey: Vec<u8>,
    /// Per byte: override class.
    pub ovr: Vec<u8>,
    /// Descent records with their call depth.
    pub records: Vec<(InsnRecord, u16)>,
    /// `(xref, labelable)`: a data xref through an assumed bank is not labelled.
    pub xrefs: Vec<(XRef, bool)>,
    pub warnings: Vec<Warning>,
    pub seeds: Vec<DataSeed>,
    /// Opcode starts reached with two different width states.
    pub conflicts: Vec<u32>,
    /// Callees known to skip inline arguments: entry offset → byte count
    /// (from an earlier pass; see `analysis::analyze`).
    pub inline_args: BTreeMap<u32, u16>,
    /// Callees this pass saw adding to their stacked return address.
    pub found_inline_args: BTreeMap<u32, u16>,
    worklist: VecDeque<Entry>,
    processed: u64,
}

fn apply_override(flags: &mut FlagState, o: &FlagOverride) {
    if let Some(m) = o.m {
        flags.m = m;
    }
    if let Some(x) = o.x {
        flags.x = x;
    }
    if let Some(e) = o.e {
        flags.e = e;
    }
    if let Some(dbr) = o.dbr {
        flags.dbr = Some(dbr);
    }
    if let Some(dp) = o.dp {
        flags.dp = Some(dp);
    }
}

/// Bytes an instruction reads or writes at its data target.
fn access_width(insn: &Instruction) -> u32 {
    use Mnemonic::*;
    let f = insn.flags_before;
    match insn.mnemonic {
        LDX | LDY | STX | STY | CPX | CPY => {
            if f.eff_x() {
                1
            } else {
                2
            }
        }
        PEI => 2,
        _ => {
            if f.eff_m() {
                1
            } else {
                2
            }
        }
    }
}

impl<'a> Walk<'a> {
    pub fn new(rom: &'a RomImage, project: &'a Project) -> Self {
        let n = rom.len();
        let mut ovr = vec![OVR_NONE; n];
        for r in &project.region_overrides {
            let v = match r.kind {
                OverrideKind::Code => OVR_CODE,
                _ => OVR_WALL,
            };
            let end = (r.end() as usize).min(n);
            ovr[r.start.0 as usize..end].fill(v);
        }
        Self {
            rom,
            project,
            kind: vec![KIND_NONE; n],
            wkey: vec![0; n],
            ovr,
            records: Vec::new(),
            xrefs: Vec::new(),
            warnings: Vec::new(),
            seeds: Vec::new(),
            conflicts: Vec::new(),
            inline_args: BTreeMap::new(),
            found_inline_args: BTreeMap::new(),
            worklist: VecDeque::new(),
            processed: 0,
        }
    }

    fn warn(&mut self, offset: u32, kind: WarningKind, text: String) {
        self.warnings.push(Warning {
            offset: FileOffset(offset),
            kind,
            text,
        });
    }

    /// Queue an entry if the address maps to ROM.
    fn push(
        &mut self,
        address: SnesAddress,
        flags: FlagState,
        depth: u16,
        entry_point: bool,
        trust: WidthTrust,
    ) {
        self.push_entry(address, flags, depth, entry_point, false, trust);
    }

    fn push_entry(
        &mut self,
        address: SnesAddress,
        flags: FlagState,
        depth: u16,
        entry_point: bool,
        soft: bool,
        trust: WidthTrust,
    ) {
        if let Some(off) = self.rom.file_offset_for(address) {
            self.worklist.push_back(Entry {
                offset: off.0,
                address,
                flags,
                depth,
                entry_point,
                soft,
                trust,
            });
        }
    }

    /// Seed the worklist: vectors in order, then user flag overrides and
    /// code marks.
    pub fn seed(&mut self) {
        let h = self.rom.header().clone();
        let reset = h.emulation.reset;
        if reset != 0 && reset != 0xFFFF {
            self.push(
                SnesAddress::new(0, reset),
                FlagState::RESET,
                0,
                true,
                WidthTrust::default(),
            );
        }
        for v in [
            h.native.cop,
            h.native.brk,
            h.native.abort,
            h.native.nmi,
            h.native.irq,
        ] {
            if v != 0 && v != 0xFFFF {
                self.push(
                    SnesAddress::new(0, v),
                    FlagState::NATIVE_VECTOR,
                    0,
                    true,
                    WidthTrust::default(),
                );
            }
        }
        for v in [
            h.emulation.cop,
            h.emulation.abort,
            h.emulation.nmi,
            h.emulation.irq,
        ] {
            if v != 0 && v != 0xFFFF {
                self.push(
                    SnesAddress::new(0, v),
                    FlagState::EMULATION_VECTOR,
                    0,
                    true,
                    WidthTrust::default(),
                );
            }
        }
        for v in vector_slots(self.rom) {
            self.xrefs.push((
                XRef {
                    from: v.slot,
                    to: Project::canonical(self.rom, v.target),
                    to_offset: self.rom.file_offset_for(v.target),
                    kind: XRefKind::Vector,
                    certain: true,
                },
                true,
            ));
        }
        let user_entries: Vec<u32> = self
            .project
            .flag_overrides
            .keys()
            .map(|o| o.0)
            .chain(
                self.project
                    .region_overrides
                    .iter()
                    .filter(|r| r.kind == OverrideKind::Code)
                    .map(|r| r.start.0),
            )
            .collect();
        for off in user_entries {
            if let Some(addr) = self.rom.snes_address_for(FileOffset(off)) {
                let mut flags = FlagState::NATIVE_VECTOR;
                if let Some(o) = self.project.flag_overrides.get(&FileOffset(off)) {
                    apply_override(&mut flags, o);
                }
                self.push_entry(addr, flags, 0, true, true, WidthTrust::default());
            }
        }
    }

    /// Drain the worklist.
    pub fn run(&mut self, control: &AnalysisControl) -> Result<(), Cancelled> {
        while let Some(entry) = self.worklist.pop_front() {
            self.processed += 1;
            if self.processed.is_multiple_of(4096) {
                control.check()?;
                control.report(
                    AnalysisPhase::Descent,
                    self.processed,
                    self.processed + self.worklist.len() as u64,
                );
            }
            self.walk(entry);
        }
        control.report(AnalysisPhase::Descent, self.processed, self.processed);
        Ok(())
    }

    fn walk(&mut self, entry: Entry) {
        let bytes = self.rom.bytes();
        let n = bytes.len() as u32;
        let mut off = entry.offset;
        let mut addr = entry.address;
        let mut flags = entry.flags;
        let mut history: Vec<Instruction> = Vec::with_capacity(3);
        let mut first = entry.entry_point;
        let mut trust = entry.trust;
        let mut ret_adjust = ReturnAdjust::default();
        loop {
            if off >= n {
                return;
            }
            if let Some(o) = self.project.flag_overrides.get(&FileOffset(off)) {
                apply_override(&mut flags, o);
            }
            if self.ovr[off as usize] == OVR_WALL {
                self.warn(
                    off,
                    WarningKind::WalkedIntoUserData,
                    format!("code reaches {} which is marked as data", addr),
                );
                return;
            }
            let key = flags.width_key() + 1;
            match self.kind[off as usize] {
                KIND_OPCODE => {
                    if self.wkey[off as usize] != key && !(entry.soft && first) {
                        self.conflicts.push(off);
                        self.warn(
                            off,
                            WarningKind::FlagConflict,
                            format!(
                                "{} reached with {} and {}",
                                addr,
                                key_name(self.wkey[off as usize] - 1),
                                key_name(key - 1)
                            ),
                        );
                    }
                    return;
                }
                KIND_OPERAND => {
                    self.conflicts.push(off);
                    self.warn(
                        off,
                        WarningKind::FlagConflict,
                        format!("{} is inside another instruction", addr),
                    );
                    return;
                }
                KIND_INLINE => {
                    self.warn(
                        off,
                        WarningKind::FlagConflict,
                        format!("{} is an inline argument of the call before it", addr),
                    );
                    return;
                }
                _ => {}
            }
            let Some(mut insn) = decode(&bytes[off as usize..], addr, FileOffset(off), flags)
            else {
                return;
            };
            refine(&history, &mut insn);
            // Decoded under an assumed width, or after something that was:
            // the length may be wrong, and so may every length after it.
            if trust.uncertain
                || (trust.m && insn.mode == AddressingMode::ImmediateM)
                || (trust.x && insn.mode == AddressingMode::ImmediateX)
            {
                trust.uncertain = true;
                insn.assumptions |= ASSUMED_WIDTHS;
            }
            // Operand bytes already decoded as opcodes: overlapping instructions.
            let end = off + insn.len as u32;
            if end > n || (off + 1..end).any(|b| self.kind[b as usize] != KIND_NONE) {
                self.conflicts.push(off);
                self.warn(
                    off,
                    WarningKind::FlagConflict,
                    format!("{} overlaps an instruction already decoded", addr),
                );
                return;
            }
            let suspicious = matches!(
                insn.mnemonic,
                Mnemonic::BRK | Mnemonic::WDM | Mnemonic::STP | Mnemonic::COP
            );
            if first {
                first = false;
                if suspicious {
                    self.warn(
                        off,
                        WarningKind::SuspiciousEntry,
                        format!("entry point {} starts with {}", addr, insn.mnemonic),
                    );
                }
            } else if suspicious {
                self.warn(
                    off,
                    WarningKind::SuspiciousFallthrough,
                    format!(
                        "code falls through into {} at {}; inline arguments or data?",
                        insn.mnemonic, addr
                    ),
                );
            }
            self.kind[off as usize] = KIND_OPCODE;
            self.wkey[off as usize] = key;
            for b in off + 1..end {
                self.kind[b as usize] = KIND_OPERAND;
            }
            self.records
                .push((InsnRecord::from_instruction(&insn), entry.depth));
            // A callee that adds to its stacked return address skips that
            // many bytes of inline arguments.
            if let Some(n) = ret_adjust.observe(&insn) {
                self.found_inline_args.insert(entry.offset, n);
            }
            if insn.assumptions & ASSUMED_XCE_CARRY != 0 {
                self.warn(
                    off,
                    WarningKind::UnknownCarryXce,
                    format!("carry unknown at XCE {}; assumed native mode", addr),
                );
            }
            if let Some(x) = xref_for(self.rom, &insn) {
                let labelable = insn.assumptions & ASSUMED_DBR == 0;
                self.xrefs.push((x, labelable));
                if x.kind != XRefKind::Pointer
                    && !x.kind.is_code()
                    && let Some(to) = x.to_offset
                    && labelable
                {
                    self.seeds.push(DataSeed {
                        offset: to.0,
                        len: access_width(&insn),
                        kind: match access_width(&insn) {
                            2 => DataKind::Word,
                            _ => DataKind::Byte,
                        },
                        confidence: 0.6,
                        source: SeedSource::Operand,
                    });
                }
            }
            let after = insn.flags_after;
            let next = insn.next_address();
            let depth = entry.depth;
            let m = insn.mnemonic;
            let target = insn.target;
            let mut stop = m.is_block_end();
            // Do the widths after this instruction still rest on an assumption?
            match m {
                Mnemonic::SEP | Mnemonic::REP => {
                    let imm = insn.operand.value() as u8;
                    if imm & 0x20 != 0 {
                        trust.m = false;
                    }
                    if imm & 0x10 != 0 {
                        trust.x = false;
                    }
                }
                Mnemonic::XCE => {
                    if insn.assumptions & ASSUMED_XCE_CARRY != 0 {
                        trust.m = true;
                        trust.x = true;
                    } else if after.e {
                        // Emulation mode forces both widths.
                        trust.m = false;
                        trust.x = false;
                    }
                }
                Mnemonic::PLP if !after.e => {
                    trust.m = true;
                    trust.x = true;
                }
                _ => {}
            }
            // Inline arguments after this call (a callee from `inline_args`).
            let mut skip = 0u32;
            if m.is_branch() {
                if let Some(t) = target {
                    self.push(t.address, after, depth, false, trust);
                }
            } else if m.is_call() {
                match insn.mode {
                    AddressingMode::AbsoluteIndexedIndirect => self.warn(
                        off,
                        WarningKind::ComputedJump,
                        format!("{}: JSR through a table; targets not followed", addr),
                    ),
                    _ => {
                        if let Some(t) = target {
                            self.push(t.address, after, depth.saturating_add(1), true, trust);
                            if let Some(k) = self
                                .rom
                                .file_offset_for(t.address)
                                .and_then(|o| self.inline_args.get(&o.0))
                            {
                                skip = *k as u32;
                            }
                        }
                    }
                }
            } else if m.is_jump() {
                match insn.mode {
                    AddressingMode::AbsoluteIndirect | AddressingMode::AbsoluteIndirectLong => {
                        let Some(t) = target else { return };
                        let long = insn.mode == AddressingMode::AbsoluteIndirectLong;
                        match self.rom.file_offset_for(t.address) {
                            Some(slot)
                                if (slot.0 as usize + if long { 3 } else { 2 }) <= bytes.len() =>
                            {
                                let s = slot.0 as usize;
                                let dest = if long {
                                    SnesAddress::from_u24(u32::from_le_bytes([
                                        bytes[s],
                                        bytes[s + 1],
                                        bytes[s + 2],
                                        0,
                                    ]))
                                } else {
                                    SnesAddress::new(
                                        addr.bank(),
                                        u16::from_le_bytes([bytes[s], bytes[s + 1]]),
                                    )
                                };
                                self.seeds.push(DataSeed {
                                    offset: slot.0,
                                    len: if long { 3 } else { 2 },
                                    kind: DataKind::Pointer,
                                    confidence: 0.9,
                                    source: SeedSource::JumpPointer,
                                });
                                self.xrefs.push((
                                    XRef {
                                        from: slot,
                                        to: Project::canonical(self.rom, dest),
                                        to_offset: self.rom.file_offset_for(dest),
                                        kind: XRefKind::Jump,
                                        certain: true,
                                    },
                                    true,
                                ));
                                self.push(dest, after, depth, true, trust);
                            }
                            _ => self.warn(
                                off,
                                WarningKind::ComputedJump,
                                format!("{}: jump through a pointer in RAM; target unknown", addr),
                            ),
                        }
                    }
                    AddressingMode::AbsoluteIndexedIndirect => self.warn(
                        off,
                        WarningKind::ComputedJump,
                        format!("{}: jump through a table; targets not followed", addr),
                    ),
                    _ => {
                        if let Some(t) = target {
                            self.push(t.address, after, depth, false, trust);
                        }
                    }
                }
            }
            if insn.assumptions & BANK_WRAP != 0 {
                self.warn(
                    off,
                    WarningKind::BankWrap,
                    format!("{} crosses the end of the bank", addr),
                );
                stop = true;
            }
            if stop {
                return;
            }
            history.push(insn);
            if history.len() > 2 {
                history.remove(0);
            }
            flags = after;
            off = end;
            addr = next;
            if skip > 0 {
                let stop_at = (end + skip).min(n);
                self.seeds.push(DataSeed {
                    offset: end,
                    len: stop_at - end,
                    kind: match skip {
                        3 => DataKind::Long,
                        2 => DataKind::Word,
                        _ => DataKind::Byte,
                    },
                    confidence: 0.8,
                    source: SeedSource::InlineArgument,
                });
                for b in end..stop_at {
                    if self.kind[b as usize] == KIND_NONE {
                        self.kind[b as usize] = KIND_INLINE;
                    }
                }
                off = stop_at;
                addr = SnesAddress::new(addr.bank(), addr.offset().wrapping_add(skip as u16));
            }
            // Stepping past $FFFF lands in the next bank's low half, which is
            // never ROM in the same way; the wrap warning above already fired.
            if self.rom.file_offset_for(addr) != Some(FileOffset(off)) {
                if let Some(a) = self.rom.snes_address_for(FileOffset(off)) {
                    addr = a;
                } else {
                    return;
                }
            }
            let _ = TargetKind::Code;
        }
    }
}

fn key_name(key: u8) -> String {
    format!("M={} X={}", (key & 1 != 0) as u8, (key & 2 != 0) as u8)
}
