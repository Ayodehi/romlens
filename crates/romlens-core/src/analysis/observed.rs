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

/// The ROM each observed `JMP (abs,X)` and `JSR (abs,X)` read its target
/// from: the table entries the game actually used.
pub fn jump_table_spans(rom: &RomImage, log: &ExecLog) -> Vec<Span> {
    let mut out = Vec::new();
    let mut sites: Vec<u32> = log
        .flows
        .iter()
        .filter(|f| f.kind.is_indirect())
        .map(|f| f.from)
        .collect();
    sites.dedup();
    for pc in sites {
        let Some(off) = log.rom_offset(pc) else {
            continue;
        };
        let opcode = rom.bytes()[off as usize];
        if opcode != 0x7C && opcode != 0xFC {
            continue;
        }
        let first = log.accesses.partition_point(|a| a.pc < pc);
        for a in log.accesses[first..].iter().take_while(|a| a.pc == pc) {
            if a.access != Access::Read || a.kind != MemKind::PrgRom || a.abs < 0 {
                continue;
            }
            out.push(Span {
                start: a.abs as u32,
                len: a.len.min(rom.len() as u32 - a.abs as u32),
                kind: crate::analysis::JUMP_TABLE_KIND,
                confidence: 95,
                what: format!(
                    "dispatch table: entries read by {} at {}",
                    if opcode == 0x7C {
                        "JMP (abs,X)"
                    } else {
                        "JSR (abs,X)"
                    },
                    fmt(pc)
                ),
            });
        }
    }
    out
}
