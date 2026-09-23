//! The Mode 7 plane, drawn untransformed.
//!
//! Mode 7 has one fixed layout and none of the registers other modes use: a
//! 128×128 map of 8-bit tile numbers in the low byte of VRAM words
//! `$0000`–`$3FFF`, and 256 tiles of 8×8 one-byte pixels in the high byte of
//! the same words, tile *t* at words `64t`–`64t + 63`. `BG1SC` and `BG12NBA`
//! mean nothing here.
//!
//! This draws the plane as it sits in VRAM, 1024×1024, before `M7A`–`M7D`
//! rotate and scale it onto the screen: the same picture an emulator's
//! tilemap viewer shows, so it can be checked against one. The transform
//! itself is a screen compositor's job, Phase 3 with the rest of them.

use crate::graphics::Bitmap;
use crate::graphics::palette::{PaletteRef, resolve};
use crate::graphics::render::{VRAM_LEN, draw_tile};

/// Cells on each side of the map.
pub const MAP_CELLS: u32 = 128;
/// Pixels on each side of the plane.
pub const PLANE_PIXELS: u32 = MAP_CELLS * 8;

fn byte(vram: &[u8], at: usize) -> u8 {
    vram.get(at % VRAM_LEN).copied().unwrap_or(0)
}

/// The tile number at (`col`, `row`): word `row × 128 + col`, low byte.
pub fn map_entry(vram: &[u8], col: u32, row: u32) -> u8 {
    byte(vram, map_entry_offset(col, row))
}

/// The VRAM byte offset of a map entry.
pub const fn map_entry_offset(col: u32, row: u32) -> usize {
    ((row % MAP_CELLS) * MAP_CELLS + col % MAP_CELLS) as usize * 2
}

/// Tile `tile`'s 64 pixels, one colour index each, row by row: the high
/// bytes of words `64 × tile` onward.
pub fn tile_pixels(vram: &[u8], tile: u8) -> [u8; 64] {
    let mut out = [0u8; 64];
    for (i, p) in out.iter_mut().enumerate() {
        *p = byte(vram, (tile as usize * 64 + i) * 2 + 1);
    }
    out
}

/// The whole plane, 1024×1024, with colour index 0 transparent as every
/// other layer is drawn. Direct colour (`CGWSEL` bit 0) is not applied: the
/// indices go through CGRAM's 256 colours.
pub fn render_plane(vram: &[u8], cgram: &[u8]) -> Bitmap {
    let mut bm = Bitmap::new(PLANE_PIXELS, PLANE_PIXELS);
    let colours = resolve(PaletteRef::Cgram(cgram), 256, 0, 0);
    let tiles: Vec<[u8; 64]> = (0..=255u8).map(|t| tile_pixels(vram, t)).collect();
    for row in 0..MAP_CELLS {
        for col in 0..MAP_CELLS {
            let tile = map_entry(vram, col, row);
            draw_tile(
                &mut bm,
                (col * 8) as i32,
                (row * 8) as i32,
                &tiles[tile as usize],
                &colours,
                true,
                false,
                false,
            );
        }
    }
    bm
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VRAM with map entry (c, r) = (c + r) & 0xFF and tile t's pixels all
    /// equal to t, so every pixel of the plane names the cell it is in.
    fn vram() -> Vec<u8> {
        let mut v = vec![0u8; VRAM_LEN];
        for w in 0..0x4000usize {
            let (col, row) = ((w % 128) as u32, (w / 128) as u32);
            v[w * 2] = ((col + row) & 0xFF) as u8;
            v[w * 2 + 1] = (w / 64) as u8;
        }
        v
    }

    #[test]
    fn entries_are_low_bytes_and_pixels_are_high_bytes() {
        let v = vram();
        assert_eq!(map_entry(&v, 5, 0), 5);
        assert_eq!(map_entry(&v, 127, 127), 254);
        assert_eq!(map_entry_offset(1, 1), 129 * 2);
        assert_eq!(tile_pixels(&v, 0), [0; 64]);
        assert_eq!(tile_pixels(&v, 9), [9; 64]);
        assert_eq!(tile_pixels(&v, 255), [255; 64]);
    }

    #[test]
    fn the_plane_draws_each_cell_with_its_tile() {
        let v = vram();
        // Colour i is (i, 0, 0), so a pixel's red channel is its index.
        let mut cgram = vec![0u8; 512];
        for i in 0..256usize {
            let c = (i as u16 >> 3) & 0x1F;
            cgram[i * 2..i * 2 + 2].copy_from_slice(&c.to_le_bytes());
        }
        let bm = render_plane(&v, &cgram);
        assert_eq!((bm.width, bm.height), (PLANE_PIXELS, PLANE_PIXELS));
        let red_at = |x: u32, y: u32| bm.rgba[((y * bm.width + x) * 4) as usize];
        let alpha_at = |x: u32, y: u32| bm.rgba[((y * bm.width + x) * 4 + 3) as usize];
        // Cell (0, 0) is tile 0: index 0, transparent.
        assert_eq!(alpha_at(3, 3), 0);
        // Cell (40, 2) is tile 42, whose pixels are all index 42.
        let expected = resolve(PaletteRef::Cgram(&cgram), 256, 0, 0)[42];
        assert_eq!(red_at(40 * 8 + 4, 2 * 8 + 4), expected[0]);
        assert_eq!(alpha_at(40 * 8 + 4, 2 * 8 + 4), 255);
    }
}
