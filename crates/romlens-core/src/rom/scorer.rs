//! Mapping detection by scoring the candidate header slots.
//!
//! Each slot earns points for the properties a real header has; the best
//! slot wins, ties go to the slot whose map-mode nibble matches, then to
//! LoROM. Weights are heuristics; tests assert outcomes on the development
//! ROM and the homebrew fixtures, not on the numbers.

use crate::error::RomError;
use crate::memory::map::MappingMode;
use crate::rom::header::RomHeader;

/// Points below which a slot is not believed.
pub const SCORE_FLOOR: u32 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotScore {
    pub mode: MappingMode,
    pub score: u32,
    pub header: RomHeader,
}

/// Score one slot; `None` when the payload does not reach it.
pub fn score_slot(payload: &[u8], mode: MappingMode) -> Option<SlotScore> {
    let header = RomHeader::parse(payload, mode.header_offset().as_usize())?;
    let mut score = 0;
    if header.complement_valid() {
        score += 4;
    }
    if header.mapping() == Some(mode) {
        score += 3;
    }
    if header.emulation.reset >= 0x8000 {
        score += 3;
    }
    if header.title_printable() {
        score += 2;
    }
    if (0x05..=0x0D).contains(&header.rom_size_code) {
        score += 1;
    }
    if header.ram_size_code <= 0x07 {
        score += 1;
    }
    if header.region <= 0x14 {
        score += 1;
    }
    Some(SlotScore {
        mode,
        score,
        header,
    })
}

/// Pick the mapping. Errors with [`RomError::NoValidHeader`] when nothing
/// reaches the floor and [`RomError::UnsupportedMapping`] when the winning
/// header names a mapping Phase 0 does not model.
pub fn detect_mapping(payload: &[u8]) -> Result<SlotScore, RomError> {
    let mut scores: Vec<SlotScore> = MappingMode::all()
        .into_iter()
        .filter_map(|mode| score_slot(payload, mode))
        .collect();
    // ExHiROM edges out HiROM only when both are valid and the image needs it.
    if payload.len() > 0x40_0000 {
        let both_valid = scores
            .iter()
            .filter(|s| matches!(s.mode, MappingMode::HiRom | MappingMode::ExHiRom))
            .filter(|s| s.header.complement_valid())
            .count()
            == 2;
        if both_valid && let Some(s) = scores.iter_mut().find(|s| s.mode == MappingMode::ExHiRom) {
            s.score += 1;
        }
    }
    // Stable order LoROM, HiROM, ExHiROM; max_by keeps the last maximum, so
    // reverse to prefer the earlier (LoROM) slot on ties.
    let best = scores
        .into_iter()
        .rev()
        .max_by_key(|s| (s.score, s.header.mapping() == Some(s.mode)))
        .ok_or(RomError::NoValidHeader)?;
    if best.score < SCORE_FLOOR {
        return Err(RomError::NoValidHeader);
    }
    if best.header.mapping().is_none() {
        return Err(RomError::UnsupportedMapping(best.header.map_mode));
    }
    Ok(best)
}
