//! Bitplane tile signature. **Annotates only; never classifies.**
//!
//! Tile data is the shakiest thing to recognise from bytes alone, and it is
//! also the commonest thing in a SNES ROM — so a wrong guess here would repaint
//! megabytes. Phase 2 therefore lets it contribute evidence and a tint in the
//! overview strip while leaving the region's kind alone, until the accuracy
//! harness can show it earns more (`16-phase2-plan.md` 2A.2). Promoting it is
//! one line: `classifies` returns `true`.

use crate::analysis::heuristics::{Heuristic, WindowHit};
use crate::graphics::tile::bitplane_score;
use crate::model::region::{DataKind, RegionKind};
use crate::rom::image::RomImage;

/// Below this a window says nothing; the score is a weak signal even above it.
const FLOOR: f32 = 0.55;

pub struct Graphics;

impl Heuristic for Graphics {
    fn name(&self) -> &'static str {
        "graphics"
    }

    fn classifies(&self) -> bool {
        false
    }

    fn detail(&self, value: f32) -> String {
        format!("4bpp tile signature {value:.2}")
    }

    fn window(
        &self,
        rom: &RomImage,
        _profile: &super::EntropyProfile,
        start: u32,
        len: u32,
    ) -> Option<WindowHit> {
        let bytes = &rom.bytes()[start as usize..(start + len) as usize];
        let score = bitplane_score(bytes, 4);
        if score < FLOOR {
            return None;
        }
        Some(WindowHit {
            kind: RegionKind::Data(DataKind::Graphics { bpp: 4 }),
            score,
            detail_value: score,
        })
    }
}
