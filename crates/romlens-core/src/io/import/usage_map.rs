//! bsnes-plus usage maps.
//!
//! Verified against bsnes-plus's `snes/cpu/debugger/debugger.hpp`. The file is
//! a bare dump of the debugger's arrays with no header and no magic: a CPU
//! block of `1 << 24` bytes indexed by **24-bit bus address**, then an SMP
//! block of `1 << 16`, and so on. Detection is therefore by size alone.
//!
//! Indexing by bus address rather than file offset is the whole difficulty. A
//! ROM byte is visible at several addresses, so the fold iterates *our*
//! offsets and ORs every mirror — which is also why only the folded, one byte
//! per ROM byte form is ever stored in a project: the raw file is 16.8 MB of
//! mostly nothing.

use crate::error::ProjectError;
use crate::memory::address::FileOffset;
use crate::model::coverage::Coverage;
use crate::rom::image::RomImage;

pub const CPU_BLOCK: usize = 1 << 24;
pub const SMP_BLOCK: usize = 1 << 16;

pub const READ: u8 = 0x80;
pub const WRITE: u8 = 0x40;
pub const EXEC: u8 = 0x20;
pub const OPCODE: u8 = 0x10;
pub const FLAG_E: u8 = 0x04;
pub const FLAG_M: u8 = 0x02;
pub const FLAG_X: u8 = 0x01;

/// Whether `bytes` are a usage map. The CPU block alone, or the CPU block
/// followed by the SMP block (and possibly more), are both seen in the wild.
pub fn looks_like(bytes: &[u8]) -> bool {
    bytes.len() >= CPU_BLOCK && bytes.len() <= CPU_BLOCK + SMP_BLOCK * 4
}

/// Fold a usage map onto `rom`'s offsets.
pub fn read(bytes: &[u8], rom: &RomImage) -> Result<Coverage, ProjectError> {
    if bytes.len() < CPU_BLOCK {
        return Err(ProjectError::BadFormat(format!(
            "a bsnes-plus usage map starts with {CPU_BLOCK} bytes of CPU data; this file has {}",
            bytes.len()
        )));
    }
    let cpu = &bytes[..CPU_BLOCK];
    let mut coverage = Coverage::new(rom.len() as u32);
    coverage.flags.recorded = true;
    for off in 0..rom.len() as u32 {
        let mut seen = 0u8;
        for addr in rom.mirrors(FileOffset(off)) {
            seen |= cpu[addr.as_u24() as usize];
        }
        if seen & EXEC != 0 {
            coverage.executed.insert(off);
            if seen & OPCODE != 0 {
                // A usage map records no notion of a subroutine entry, so
                // every opcode start is an ordinary one.
                coverage.mark_opcode(off, false);
                // bsnes-plus records M and X as "16-bit" bits, so a clear bit
                // is the 8-bit case.
                if seen & FLAG_M == 0 {
                    coverage.flags.m8.insert(off);
                }
                if seen & FLAG_X == 0 {
                    coverage.flags.x8.insert(off);
                }
                // Emulation mode forces both widths to 8 regardless.
                if seen & FLAG_E != 0 {
                    coverage.flags.m8.insert(off);
                    coverage.flags.x8.insert(off);
                }
            }
        }
        if seen & READ != 0 {
            coverage.read.insert(off);
        }
        if seen & WRITE != 0 {
            coverage.written.insert(off);
        }
    }
    Ok(coverage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    fn rom() -> RomImage {
        RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap()
    }

    #[test]
    fn folds_bus_addresses_onto_offsets() {
        let rom = rom();
        let mut map = vec![0u8; CPU_BLOCK];
        // $00:8000 and $80:8000 are the same ROM byte in LoROM; marking either
        // mirror has to reach file offset 0.
        map[0x80_8000] = EXEC | OPCODE | FLAG_M;
        map[0x00_8001] = READ;
        map[0x00_8002] = EXEC | OPCODE | FLAG_E;
        map[0x00_8003] = WRITE;
        let c = read(&map, &rom).unwrap();
        assert!(c.executed.get(0) && c.opcode_start.get(0));
        assert!(!c.flags.m8.get(0), "FLAG_M set means 16-bit");
        assert!(c.flags.x8.get(0), "FLAG_X clear means 8-bit");
        assert!(c.read.get(1) && !c.executed.get(1));
        assert!(
            c.flags.m8.get(2) && c.flags.x8.get(2),
            "emulation forces both"
        );
        assert!(c.written.get(3));
    }

    #[test]
    fn detects_by_size_and_refuses_a_short_file() {
        assert!(looks_like(&vec![0u8; CPU_BLOCK]));
        assert!(looks_like(&vec![0u8; CPU_BLOCK + SMP_BLOCK]));
        assert!(!looks_like(&vec![0u8; 1024]));
        assert!(read(&[0u8; 1024], &rom()).is_err());
    }
}
