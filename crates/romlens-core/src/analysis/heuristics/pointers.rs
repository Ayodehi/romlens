//! Runs of addresses that land inside the ROM.
//!
//! The signal is consistency, not any one value: a table of pointers shares a
//! bank and every entry resolves, while arbitrary bytes read as addresses land
//! in RAM, in open bus, or outside the image about as often as not.

use crate::analysis::heuristics::{Heuristic, WindowHit};
use crate::memory::address::SnesAddress;
use crate::model::region::{BankRule, DataKind, RegionKind};
use crate::rom::image::RomImage;

/// Entries that must resolve in a row before a window counts.
pub const MIN_RUN: usize = 8;

/// How much of a bank's 16-bit range lands in the ROM: a half in a LoROM
/// bank (`$8000`-`$FFFF`), all of it in a HiROM bank like `$C0`. Sampled a
/// page at a time.
fn resolving_share(rom: &RomImage, bank: u8) -> f32 {
    let hits = (0..=255u16)
        .filter(|hi| {
            rom.file_offset_for(SnesAddress::new(bank, hi << 8))
                .is_some()
        })
        .count();
    hits as f32 / 256.0
}

/// How much of the 24-bit range lands in the ROM, sampled.
fn long_resolving_share(rom: &RomImage) -> f32 {
    let mut hits = 0;
    for bank in 0..=255u8 {
        for hi in (0..=255u16).step_by(16) {
            hits += rom
                .file_offset_for(SnesAddress::new(bank, hi << 8))
                .is_some() as u32;
        }
    }
    hits as f32 / (256.0 * 16.0)
}

/// How many 24-bit entries share the most common bank: a table of long
/// pointers mostly points into one or two banks; code or data read as
/// addresses scatters.
fn same_bank_share(bytes: &[u8]) -> f32 {
    let banks: Vec<u8> = bytes.as_chunks::<3>().0.iter().map(|t| t[2]).collect();
    if banks.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    for &b in &banks {
        counts[b as usize] += 1;
    }
    *counts.iter().max().unwrap() as f32 / banks.len() as f32
}

/// Whether 16-bit words look like a table of pointers by their shape: at
/// least eight distinct entries, most going up from the one before (records
/// or strings stored in order), all within 16 KB of each other (one area of
/// the bank). Repeats and filler (`$0000`, `$FFFF`) are left out: filler
/// repeats, and a repeat is not an order.
fn table_shaped(bytes: &[u8]) -> bool {
    let mut values: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_le_bytes(*p))
        .filter(|&v| v != 0 && v != 0xFFFF)
        .collect();
    values.dedup();
    if values.len() < 8 {
        return false;
    }
    let up = values.windows(2).filter(|w| w[1] > w[0]).count();
    let (lo, hi) = (values.iter().min().unwrap(), values.iter().max().unwrap());
    up as f32 / (values.len() - 1) as f32 >= 0.8 && hi - lo <= 0x4000
}

/// The longest run of consecutive 16-bit values that all map into ROM within
/// one bank, and the bank they share.
fn word_run(rom: &RomImage, bytes: &[u8], bank: u8) -> usize {
    let mut best = 0;
    let mut run = 0;
    for pair in bytes.as_chunks::<2>().0 {
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
    for triple in bytes.as_chunks::<3>().0 {
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
        // Where every value lands in the ROM (a HiROM bank), landing there
        // says nothing: then only a table's own shape counts for it.
        let words = if resolving_share(rom, bank) > 0.9 && !table_shaped(bytes) {
            0
        } else {
            word_run(rom, bytes, bank)
        };
        // Likewise for long addresses where most banks are ROM.
        let longs = if long_resolving_share(rom) > 0.25 && same_bank_share(bytes) < 0.75 {
            0
        } else {
            long_run(rom, bytes)
        };
        let (entries, kind, elements) = if longs * 3 >= words * 2 {
            (longs, DataKind::Long, longs)
        } else {
            (
                words,
                // The run was found by resolving each entry in the table's own
                // bank, so that is the rule it was proved under.
                DataKind::Pointer {
                    bank: BankRule::SameBank,
                },
                words,
            )
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
