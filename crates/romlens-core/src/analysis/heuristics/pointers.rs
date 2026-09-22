//! Runs of addresses that land inside the ROM.
//!
//! The signal is consistency, not any one value: a table of pointers shares a
//! bank and every entry resolves, while arbitrary bytes read as addresses land
//! in RAM, in open bus, or outside the image about as often as not.

use crate::analysis::heuristics::{Heuristic, WindowHit};
use crate::memory::address::SnesAddress;
use crate::model::region::{DataKind, RegionKind};
use crate::rom::image::RomImage;

/// Entries that must resolve in a row before a window counts.
pub const MIN_RUN: usize = 8;

/// The longest run of consecutive 16-bit values that all map into ROM within
/// one bank, and the bank they share.
fn word_run(rom: &RomImage, bytes: &[u8], bank: u8) -> usize {
    let mut best = 0;
    let mut run = 0;
    for pair in bytes.chunks_exact(2) {
        let value = u16::from_le_bytes([pair[0], pair[1]]);
        if rom.file_offset_for(SnesAddress::new(bank, value)).is_some() {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

/// The same for 24-bit values, which carry their own bank.
fn long_run(rom: &RomImage, bytes: &[u8]) -> usize {
    let mut best = 0;
    let mut run = 0;
    for triple in bytes.chunks_exact(3) {
        let value = u32::from_le_bytes([triple[0], triple[1], triple[2], 0]);
        if rom.file_offset_for(SnesAddress::from_u24(value)).is_some() {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

pub struct Pointers;

impl Heuristic for Pointers {
    fn name(&self) -> &'static str {
        "pointers"
    }

    fn detail(&self, value: f32) -> String {
        format!("{value:.0} consecutive addresses resolve")
    }

    fn window(
        &self,
        rom: &RomImage,
        _profile: &super::EntropyProfile,
        start: u32,
        len: u32,
    ) -> Option<WindowHit> {
        let bytes = &rom.bytes()[start as usize..(start + len) as usize];
        // A 16-bit table's bank is the one it lives in; that is how the code
        // reading it would index, and it is the only bank we can justify.
        let bank = rom
            .snes_address_for(crate::memory::address::FileOffset(start))?
            .bank();
        let words = word_run(rom, bytes, bank);
        let longs = long_run(rom, bytes);
        let (entries, kind, elements) = if longs * 3 >= words * 2 {
            (longs, DataKind::Long, longs)
        } else {
            (words, DataKind::Pointer, words)
        };
        if entries < MIN_RUN {
            return None;
        }
        // Covering the whole window is what separates a table from two or
        // three addresses that happen to sit together.
        let coverage = (entries * kind.element_len() as usize) as f32 / len as f32;
        if coverage < 0.75 {
            return None;
        }
        Some(WindowHit {
            kind: RegionKind::Data(kind),
            score: 0.45 + 0.10 * coverage.min(1.0),
            detail_value: elements as f32,
        })
    }
}
