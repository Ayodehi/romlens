//! What an execution log adds to the analysis beyond coverage.
//!
//! Coverage (`model::coverage`) already carries the log's instruction starts
//! and widths into the descent, which decodes them. This is the rest, the part
//! only an execution log has:
//!
//! - **cross-references the game made**: every call, jump and branch seen,
//!   indirect ones included, and every instruction's reads and writes of ROM,
//!   so Find References on a table names the code that actually reads it;
//! - **data typed by where it went**: ROM that a DMA channel sent to CGRAM is
//!   a palette, to VRAM graphics, to OAM a sprite table;
//! - **dispatch tables read**: the ROM a `JMP (abs,X)` or `JSR (abs,X)` was
//!   seen to read its target from.

use crate::cpu65816::FlagState;
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::exec_log::{Access, ExecLog, FlowKind, MemKind};
use crate::model::project::Project;
use crate::model::region::{DataKind, RegionKind};
use crate::model::xref::{XRef, XRefKind};
use crate::rom::image::RomImage;

/// A ROM span the log says something definite about.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub start: u32,
    pub len: u32,
    pub kind: RegionKind,
    /// 0..=100
    pub confidence: u8,
    /// What was seen, for the region's evidence.
    pub what: String,
}

fn fmt(addr: u32) -> String {
    format!("${:02X}:{:04X}", addr >> 16, addr & 0xFFFF)
}

/// The canonical address and ROM offset of the ROM byte at `abs`.
fn rom_target(rom: &RomImage, abs: i32) -> Option<(SnesAddress, FileOffset)> {
    if abs < 0 || abs as usize >= rom.len() {
        return None;
    }
    let off = FileOffset(abs as u32);
    rom.snes_address_for(off).map(|a| (a, off))
}

/// Every reference the log saw an instruction in ROM make, one per
/// relationship. Targets outside ROM (RAM, registers) are left out: nothing
/// in the ROM view can select them yet, and they would be most of the log.
pub fn xrefs(rom: &RomImage, log: &ExecLog) -> Vec<XRef> {
    let mut out = Vec::new();
    let from_rom = |pc: u32| log.rom_offset(pc).map(FileOffset);
    for f in &log.flows {
        let kind = match f.kind {
            FlowKind::Call | FlowKind::IndirectCall => XRefKind::Call,
            FlowKind::Jump | FlowKind::IndirectJump => XRefKind::Jump,
            FlowKind::Branch => XRefKind::Branch,
            _ => continue,
        };
        let (Some(from), Some(to)) = (from_rom(f.from), log.rom_offset(f.to)) else {
            continue;
        };
        let Some((to, to_offset)) = rom_target(rom, to as i32) else {
            continue;
        };
        out.push(XRef {
            from,
            to: Project::canonical(rom, to),
            to_offset: Some(to_offset),
            kind,
            certain: true,
            observed: true,
        });
    }
    for a in &log.accesses {
        if a.kind != MemKind::PrgRom {
            continue;
        }
        let (Some(from), Some((to, to_offset))) = (from_rom(a.pc), rom_target(rom, a.abs)) else {
            continue;
        };
        out.push(XRef {
            from,
            to: Project::canonical(rom, to),
            to_offset: Some(to_offset),
            kind: match a.access {
                Access::Read => XRefKind::Read,
                Access::Write => XRefKind::Write,
            },
            certain: true,
            observed: true,
        });
    }
    for d in &log.dma {
        if d.kind != MemKind::PrgRom || d.to_a_bus || d.hdma {
            continue;
        }
        let (Some(from), Some((to, to_offset))) = (from_rom(d.pc), rom_target(rom, d.abs)) else {
            continue;
        };
        out.push(XRef {
            from,
            to: Project::canonical(rom, to),
            to_offset: Some(to_offset),
            kind: XRefKind::Read,
            certain: true,
            observed: true,
        });
    }
    out
}

/// ROM a DMA channel sent somewhere that says what it is.
///
/// CGRAM takes only colours, so that is a palette. VRAM holds tiles and
/// tilemaps alike, and nothing in the transfer says which or at what depth;
/// graphics is the commoner answer, 4bpp the commoner depth, and the evidence
/// says both were assumed. OAM is a sprite table, the APU's ports take the
/// sound driver and its data, and WRAM is a copy whose use is decided later.
pub fn dma_spans(log: &ExecLog, rom_len: u32) -> Vec<Span> {
    let mut out = Vec::new();
    for d in &log.dma {
        if d.kind != MemKind::PrgRom || d.abs < 0 || d.to_a_bus || d.abs as u32 >= rom_len {
            continue;
        }
        let (kind, confidence, note) = match d.bbus {
            0x22 => (RegionKind::Data(DataKind::Palette), 95, ""),
            0x18 | 0x19 => (
                RegionKind::Data(DataKind::Graphics { bpp: 4 }),
                80,
                " (tiles or a tilemap; 4bpp assumed)",
            ),
            0x04 => (RegionKind::Data(DataKind::Struct), 90, " (sprite table)"),
            _ => (RegionKind::Data(DataKind::Byte), 85, ""),
        };
        let by = if d.hdma {
            "HDMA".to_owned()
        } else {
            format!("DMA from {}", fmt(d.pc))
        };
        let start = d.abs as u32;
        out.push(Span {
            start,
            len: d.len.min(rom_len - start),
            kind,
            confidence,
            what: format!("sent to {} by {by}{note}", d.destination()),
        });
    }
    out
}

/// A dispatcher's address, opcode, and the `(start, len)` runs it read.
type Dispatch = (u32, u8, Vec<(u32, u32)>);

/// Each observed `JMP (abs,X)` (`$7C`) or `JSR (abs,X)` (`$FC`) in ROM: its
/// address, its opcode, and the ROM it read its targets from, in order.
fn dispatches(rom: &RomImage, log: &ExecLog) -> Vec<Dispatch> {
    let mut sites: Vec<u32> = log
        .flows
        .iter()
        .filter(|f| f.kind.is_indirect())
        .map(|f| f.from)
        .collect();
    sites.dedup();
    let mut out = Vec::new();
    for pc in sites {
        let Some(off) = log.rom_offset(pc) else {
            continue;
        };
        let opcode = rom.bytes()[off as usize];
        if opcode != 0x7C && opcode != 0xFC {
            continue;
        }
        let first = log.accesses.partition_point(|a| a.pc < pc);
        let mut read: Vec<(u32, u32)> = log.accesses[first..]
            .iter()
            .take_while(|a| a.pc == pc)
            .filter(|a| a.access == Access::Read && a.kind == MemKind::PrgRom && a.abs >= 0)
            .map(|a| (a.abs as u32, a.len.min(rom.len() as u32 - a.abs as u32)))
            .collect();
        read.sort_unstable();
        out.push((pc, opcode, read));
    }
    out
}

/// The gaps between entries a dispatcher read that are more of its table.
fn unread_gaps(rom: &RomImage, log: &ExecLog, pc: u32, read: &[(u32, u32)]) -> Vec<(u32, u32)> {
    read.windows(2)
        .map(|w| (w[0].0 + w[0].1, w[1].0))
        .filter(|(from, to)| unread_entries(rom, log, pc, *from, *to))
        .collect()
}

/// The flags a dispatcher ran with, which its targets start with.
fn dispatch_flags(log: &ExecLog, pc: u32) -> FlagState {
    log.insn(pc)
        .and_then(|i| i.flag_states().next())
        .unwrap_or(FlagState::NATIVE_VECTOR)
}

/// The ROM each observed `JMP (abs,X)` and `JSR (abs,X)` read its target
/// from: the table entries the game actually used, and the entries between
/// them it did not, when they too name routines (a little less sure).
pub fn jump_table_spans(rom: &RomImage, log: &ExecLog) -> Vec<Span> {
    let mut out = Vec::new();
    for (pc, opcode, read) in dispatches(rom, log) {
        let by = if opcode == 0x7C {
            "JMP (abs,X)"
        } else {
            "JSR (abs,X)"
        };
        for &(start, len) in &read {
            out.push(Span {
                start,
                len,
                kind: crate::analysis::JUMP_TABLE_KIND,
                confidence: 95,
                what: format!("dispatch table: entries read by {by} at {}", fmt(pc)),
            });
        }
        for (from, to) in unread_gaps(rom, log, pc, &read) {
            out.push(Span {
                start: from,
                len: to - from,
                kind: crate::analysis::JUMP_TABLE_KIND,
                confidence: 80,
                what: format!(
                    "dispatch table: entries between those read by {by} at {}, not seen read",
                    fmt(pc)
                ),
            });
        }
    }
    out
}

/// An entry the game did not read, between ones it did: where it is, the
/// routine it names, the flags that routine starts with, and whether the
/// dispatcher calls (`JSR`) rather than jumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnreadEntry {
    pub slot: u32,
    pub target: SnesAddress,
    pub flags: FlagState,
    pub call: bool,
}

/// Every such entry, so the descent walks the routines they name.
pub fn unread_entries_of(rom: &RomImage, log: &ExecLog) -> Vec<UnreadEntry> {
    let mut out = Vec::new();
    for (pc, opcode, read) in dispatches(rom, log) {
        let bank = (pc >> 16) as u8;
        for (from, to) in unread_gaps(rom, log, pc, &read) {
            for slot in (from..to).step_by(2) {
                let b = rom.bytes();
                let word = u16::from_le_bytes([b[slot as usize], b[slot as usize + 1]]);
                out.push(UnreadEntry {
                    slot,
                    target: SnesAddress::new(bank, word),
                    flags: dispatch_flags(log, pc),
                    call: opcode == 0xFC,
                });
            }
        }
    }
    out
}

/// Whether `[from, to)`, between two entries the dispatcher at `pc` read, is
/// more of its table: whole entries, a few, each the address of a routine in
/// the dispatcher's bank. A routine is one the game ran, or bytes that
/// decode as one under the flags the dispatcher ran with.
fn unread_entries(rom: &RomImage, log: &ExecLog, pc: u32, from: u32, to: u32) -> bool {
    const MOST: u32 = 32;
    let n = to.saturating_sub(from);
    if n == 0 || !n.is_multiple_of(2) || n / 2 > MOST {
        return false;
    }
    let bank = (pc >> 16) as u8;
    let flags = dispatch_flags(log, pc);
    (from..to).step_by(2).all(|e| {
        let word = u16::from_le_bytes([rom.bytes()[e as usize], rom.bytes()[e as usize + 1]]);
        let target = SnesAddress::new(bank, word);
        let Some(off) = rom.file_offset_for(target) else {
            return false;
        };
        log.insn(target.as_u24()).is_some()
            || crate::analysis::jumptable::plausible_entry(rom, off.0, flags)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use crate::memory::map::MappingMode;
    use crate::model::exec_log::{AccessRun, ExecInsn, Flow, FlowKind};

    /// `JSR ($8010,X)` at `$80:8000` and four entries at `$8010`, of which
    /// the game read the first and the last.
    fn table(entry2: u16) -> (RomImage, ExecLog) {
        let mut code = vec![0u8; 0x40];
        code[0..3].copy_from_slice(&[0xFC, 0x10, 0x80]);
        for (i, e) in [0x8020u16, 0x8024, entry2, 0x802C].iter().enumerate() {
            code[0x10 + i * 2..0x12 + i * 2].copy_from_slice(&e.to_le_bytes());
        }
        for at in [0x20, 0x24, 0x28, 0x2C] {
            code[at] = 0x60; // RTS
        }
        let rom = RomImage::from_bytes(
            fixtures::build_with_code(MappingMode::LoRom, 0x8000, false, &code, "TABLE"),
            "t.sfc",
        )
        .unwrap();
        let read = |abs: i32| AccessRun {
            pc: 0x80_8000,
            addr: 0x80_8000 + abs as u32,
            abs,
            len: 2,
            access: Access::Read,
            kind: MemKind::PrgRom,
            count: 1,
        };
        let log = ExecLog {
            rom_crc32: 0,
            rom_size: rom.len() as u32,
            insns: vec![ExecInsn {
                pc: 0x80_8000,
                abs: 0,
                kind: MemKind::PrgRom,
                states: 1 << 3,
                count: 2,
            }],
            accesses: vec![read(0x10), read(0x16)],
            flows: vec![Flow {
                from: 0x80_8000,
                to: 0x80_8020,
                kind: FlowKind::IndirectCall,
                count: 1,
            }],
            dma: Vec::new(),
        };
        (rom, log)
    }

    #[test]
    fn entries_between_those_read_join_the_table() {
        let (rom, log) = table(0x8028);
        let spans = jump_table_spans(&rom, &log);
        let gap: Vec<_> = spans.iter().filter(|s| s.confidence == 80).collect();
        assert_eq!(gap.len(), 1, "{spans:?}");
        assert_eq!((gap[0].start, gap[0].len), (0x12, 4));
        assert!(gap[0].what.contains("not seen read"), "{}", gap[0].what);

        // One of them points into RAM: not a table the game could use.
        let unread = unread_entries_of(&rom, &log);
        assert_eq!(
            unread
                .iter()
                .map(|e| (e.slot, e.target.as_u24()))
                .collect::<Vec<_>>(),
            [(0x12, 0x80_8024), (0x14, 0x80_8028)]
        );
        assert!(unread.iter().all(|e| e.call && e.flags.m && e.flags.x));

        let (rom, log) = table(0x1234);
        let spans = jump_table_spans(&rom, &log);
        assert!(spans.iter().all(|s| s.confidence == 95), "{spans:?}");
        assert!(unread_entries_of(&rom, &log).is_empty());
    }
}
