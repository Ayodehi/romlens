//! Runs of printable ASCII.
//!
//! SNES games mostly encode text in custom tile indices, not ASCII, so this
//! fires rarely. Where it does — copyright strings, debug messages, the
//! occasional file name left in a build — it is nearly always right, which is
//! why it is allowed a higher score than entropy.

use crate::analysis::heuristics::{Heuristic, WindowHit};
use crate::model::region::{DataKind, RegionKind};
use crate::rom::image::RomImage;

/// Shorter than this and any window of text-like bytes would qualify.
pub const MIN_RUN: usize = 8;

/// The fraction of a window that must be inside one printable run.
const MIN_COVERAGE: f32 = 0.75;

fn printable(b: u8) -> bool {
    matches!(b, 0x20..=0x7E | b'\n' | b'\r' | b'\t')
}

/// The longest run of printable bytes in `bytes`.
pub fn longest_run(bytes: &[u8]) -> usize {
    let mut best = 0;
    let mut run = 0;
    for b in bytes {
        if printable(*b) {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

pub struct Ascii;

impl Heuristic for Ascii {
    fn name(&self) -> &'static str {
        "ascii"
    }

    fn detail(&self, value: f32) -> String {
        format!("{:.0}% printable", value * 100.0)
    }

    fn window(
        &self,
        rom: &RomImage,
        _profile: &super::EntropyProfile,
        start: u32,
        len: u32,
    ) -> Option<WindowHit> {
        let bytes = &rom.bytes()[start as usize..(start + len) as usize];
        let run = longest_run(bytes);
        if run < MIN_RUN {
            return None;
        }
        let coverage = run as f32 / len as f32;
        if coverage < MIN_COVERAGE {
            return None;
        }
        Some(WindowHit {
            kind: RegionKind::Data(DataKind::String),
            score: 0.55 + 0.05 * coverage,
            detail_value: coverage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_longest_printable_run() {
        assert_eq!(longest_run(b"hello\x00world!"), 6);
        assert_eq!(longest_run(&[0u8; 16]), 0);
        assert_eq!(longest_run(b"abc"), 3);
    }
}
