//! Shannon entropy per window, and the two bands it can speak to.

use crate::analysis::heuristics::{Heuristic, WINDOW, WindowHit};
use crate::model::region::{DataKind, RegionKind};
use crate::rom::image::RomImage;

/// Entropy of every window of the image, computed once.
///
/// It is ROM-derived and project-independent, so `Workbench` caches it across
/// edits: the analyzer re-runs on every mark, and recomputing this each time
/// would put a linear pass over the image inside the edit loop for no reason.
#[derive(Debug, Clone, PartialEq)]
pub struct EntropyProfile {
    /// Bits per byte for each `WINDOW`-sized window, in order.
    pub windows: Vec<f32>,
}

impl EntropyProfile {
    pub fn build(rom: &RomImage) -> Self {
        let bytes = rom.bytes();
        let windows = bytes.chunks(WINDOW as usize).map(window_entropy).collect();
        Self { windows }
    }

    /// Bits per byte at a file offset.
    pub fn at(&self, offset: u32) -> f32 {
        self.windows
            .get((offset / WINDOW) as usize)
            .copied()
            .unwrap_or(0.0)
    }
}

/// Shannon entropy in bits per byte, 0.0 (one repeated byte) to 8.0.
pub fn window_entropy(bytes: &[u8]) -> f32 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    for b in bytes {
        counts[*b as usize] += 1;
    }
    let n = bytes.len() as f32;
    let sum: f32 = counts
        .iter()
        .filter(|c| **c > 0)
        .map(|c| {
            let p = *c as f32 / n;
            p * p.log2()
        })
        .sum();
    // `max` rather than plain negation: one repeated byte gives a sum of
    // exactly 0.0, and negating that prints "-0.0 bits per byte".
    (-sum).max(0.0)
}

/// Bank `$85` of Super Metroid is 1.5 bits per byte, `$80` (code with tables)
/// is 4.6 and `$96` (compressed graphics) is 7.0
/// (`04-snes-technical-primer.md`). The bands are set inside those
/// measurements, not at them: the middle of the range is where code and data
/// overlap, and a heuristic that guessed there would be guessing.
pub const COMPRESSED_BITS: f32 = 7.2;
pub const SPARSE_BITS: f32 = 2.0;

pub struct Entropy;

impl Heuristic for Entropy {
    fn name(&self) -> &'static str {
        "entropy"
    }

    fn detail(&self, value: f32) -> String {
        if value == 0.0 {
            // Worth its own wording: a reader seeing "0.0 bits per byte" over
            // half a megabyte wants to know it is one repeated byte, which is
            // unused space rather than a table the game reads.
            "constant fill".to_owned()
        } else {
            format!("{value:.1} bits per byte")
        }
    }

    fn window(
        &self,
        _rom: &RomImage,
        profile: &EntropyProfile,
        start: u32,
        _len: u32,
    ) -> Option<WindowHit> {
        let bits = profile.at(start);
        if bits >= COMPRESSED_BITS {
            // Nothing else in a ROM is this dense. Code is around 5 to 6.
            Some(WindowHit {
                kind: RegionKind::Data(DataKind::Compressed),
                score: 0.50 + 0.05 * (bits - COMPRESSED_BITS) / (8.0 - COMPRESSED_BITS),
                detail_value: bits,
            })
        } else if bits <= SPARSE_BITS {
            // A sparse table or filler. Either way it is data, and saying so
            // is worth more than leaving a megabyte of padding "unknown".
            Some(WindowHit {
                kind: RegionKind::Data(DataKind::Byte),
                score: 0.40 + 0.10 * (SPARSE_BITS - bits) / SPARSE_BITS,
                detail_value: bits,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entropy_spans_the_documented_range() {
        assert_eq!(window_entropy(&[0xFF; 256]), 0.0, "one repeated byte");
        let all: Vec<u8> = (0..=255u8).collect();
        assert!((window_entropy(&all) - 8.0).abs() < 1e-4, "every byte once");
        assert_eq!(window_entropy(&[]), 0.0);
    }
}
