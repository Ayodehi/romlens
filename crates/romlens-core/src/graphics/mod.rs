//! SNES graphics formats (`07-memory-to-screen.md`).
//!
//! The decoders here work on bytes, not on a machine: a tile is 16, 32 or 64
//! bytes wherever they came from, so the same code reads raw ROM, a WRAM
//! buffer and a recording's VRAM. The scoring functions track 2A's heuristics
//! call live beside the decoders they score for, so the bitplane rule and the
//! BGR15 expansion are written once — a classifier that disagreed with the
//! decoder would be worse than no classifier.
//!
//! What stops here, deliberately: the reference renderer draws one BG layer or
//! one sprite. Priority, windows, colour math and compositing wait for a
//! recorded framebuffer to test them against (`16-phase2-plan.md` 2B.6).

pub mod compress;
pub mod mode7;
pub mod oam;
pub mod palette;
pub mod ppu_state;
pub mod render;
pub mod tile;
pub mod tilemap;

use sha2::{Digest, Sha256};

/// An RGBA image, eight bits per channel, rows top to bottom.
///
/// This is the only image type the core has, and it never leaves as a file:
/// shells draw it, tests pin its [`digest`](Self::digest), and nothing writes
/// it to disk (`12-content-policy.md` rule 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Bitmap {
    /// A fully transparent image.
    pub fn new(width: u32, height: u32) -> Self {
        Bitmap {
            width,
            height,
            rgba: vec![0; width as usize * height as usize * 4],
        }
    }

    pub fn set(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if x < self.width && y < self.height {
            let i = (y as usize * self.width as usize + x as usize) * 4;
            self.rgba[i..i + 4].copy_from_slice(&rgba);
        }
    }

    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    /// SHA-256 of the dimensions and the pixels, lower-case hex.
    ///
    /// The dimensions are hashed too, so a 16×8 image and an 8×16 one with
    /// the same bytes do not collide. This is what the goldens pin: small,
    /// diffable, and it ships no image.
    pub fn digest(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.width.to_le_bytes());
        h.update(self.height.to_le_bytes());
        h.update(&self.rgba);
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The image as characters, one per pixel, darkest to brightest, with a
    /// space for transparent. For goldens and a terminal, not for looking at.
    pub fn to_ascii(&self) -> String {
        const RAMP: &[u8] = b".:-=+*#%@";
        let mut out = String::with_capacity((self.width as usize + 1) * self.height as usize);
        for y in 0..self.height {
            for x in 0..self.width {
                let [r, g, b, a] = self.get(x, y);
                if a == 0 {
                    out.push(' ');
                    continue;
                }
                // Rec. 601 luma, integer.
                let luma = (299 * r as u32 + 587 * g as u32 + 114 * b as u32) / 1000;
                let i = (luma as usize * (RAMP.len() - 1) + 127) / 255;
                out.push(RAMP[i] as char);
            }
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_covers_the_shape() {
        let a = Bitmap::new(16, 8);
        let b = Bitmap::new(8, 16);
        assert_eq!(a.rgba, b.rgba);
        assert_ne!(a.digest(), b.digest());
    }

    #[test]
    fn ascii_runs_dark_to_bright() {
        let mut bm = Bitmap::new(3, 1);
        bm.set(0, 0, [0, 0, 0, 255]);
        bm.set(1, 0, [255, 255, 255, 255]);
        assert_eq!(bm.to_ascii(), ".@ \n");
    }
}
