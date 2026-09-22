//! BGR15 palette signature.
//!
//! The most reliable pure-data signal in the set, and the reason is bit 15:
//! it is unused in a SNES colour, so a real palette has it clear in every
//! entry while arbitrary data has it clear half the time. The scoring lives in
//! `graphics::palette` so the classifier and the Phase 2B palette view cannot
//! disagree about what a palette is.

use crate::analysis::heuristics::{Heuristic, WindowHit, ascii};
use crate::graphics::palette::palette_score;
use crate::model::region::{DataKind, RegionKind};
use crate::rom::image::RomImage;

pub struct Palette;

impl Heuristic for Palette {
    fn name(&self) -> &'static str {
        "palette"
    }

    fn detail(&self, value: f32) -> String {
        format!("BGR15 signature {value:.2}")
    }

    fn window(
        &self,
        rom: &RomImage,
        _profile: &super::EntropyProfile,
        start: u32,
        len: u32,
    ) -> Option<WindowHit> {
        let bytes = &rom.bytes()[start as usize..(start + len) as usize];
        // Printable text passes every palette test: two ASCII bytes make a
        // word with bit 15 clear, and a sentence is as varied as a palette
        // row. Text is the stronger signal of the two, so defer to it rather
        // than let the scores decide — they are close enough that the answer
        // would turn on a rounding step.
        if ascii::longest_run(bytes) as f32 / len as f32 >= 0.75 {
            return None;
        }
        let score = palette_score(bytes);
        if score <= 0.0 {
            return None;
        }
        Some(WindowHit {
            kind: RegionKind::Data(DataKind::Palette),
            score,
            detail_value: score,
        })
    }
}
