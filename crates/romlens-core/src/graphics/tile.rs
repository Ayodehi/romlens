//! Bitplane tiles: 8×8 pixels, one bit per plane per pixel.

/// Bytes in one 8×8 tile at this depth.
pub const fn tile_len(bpp: u8) -> usize {
    bpp as usize * 8
}

/// Where plane `plane`'s byte for row `y` sits inside a tile.
///
/// Planes are interleaved in pairs: rows 0–7 for planes 0 and 1, then rows 0–7
/// again for planes 2 and 3, and so on. This one rule covers 2, 4 and 8 bpp
/// (`07-memory-to-screen.md`).
pub const fn byte_index(plane: u8, y: u8) -> usize {
    (plane as usize / 2) * 16 + y as usize * 2 + (plane as usize % 2)
}

/// The colour index of pixel (`x`, `y`) in a tile of `bpp` bitplanes.
pub fn pixel_index(bytes: &[u8], bpp: u8, x: u8, y: u8) -> u8 {
    let mut index = 0u8;
    for plane in 0..bpp {
        let byte = bytes[byte_index(plane, y)];
        index |= ((byte >> (7 - x)) & 1) << plane;
    }
    index
}

/// How much `bytes` look like bitplane tile data, 0.0 to 1.0.
///
/// This is the shakiest signal in the set, which is why Phase 2 uses it only
/// to annotate and tint the overview strip, never to classify a region
/// (`16-phase2-plan.md` 2A.2). Turning it into a classifier is a one-line
/// change once the accuracy harness can show it earns its place.
///
/// What it measures is that drawn tiles are *sparse and repetitive*: most
/// pixels are index 0 because sprites and backgrounds are mostly transparent
/// or flat, and a tile uses a handful of its available indices rather than all
/// of them. Random bytes do neither. It deliberately refuses a tile that is
/// entirely one index, because a run of identical bytes is filler and would
/// otherwise score perfectly on both counts.
pub fn bitplane_score(bytes: &[u8], bpp: u8) -> f32 {
    let len = tile_len(bpp);
    if bpp == 0 || bpp > 8 || bytes.len() < len {
        return 0.0;
    }
    let mut scored = 0usize;
    let mut total = 0.0f32;
    for tile in bytes.chunks_exact(len) {
        let mut counts = [0u32; 256];
        for y in 0..8u8 {
            for x in 0..8u8 {
                counts[pixel_index(tile, bpp, x, y) as usize] += 1;
            }
        }
        let used = counts.iter().filter(|c| **c > 0).count();
        if used < 2 {
            continue; // flat fill: no evidence either way
        }
        let transparent = counts[0] as f32 / 64.0;
        let palette_size = 1usize << bpp;
        // Using few of the available indices is the signal; using nearly all
        // of them is what random bytes do.
        let sparse = 1.0 - (used as f32 / palette_size as f32);
        total += (0.5 * transparent + 0.5 * sparse).clamp(0.0, 1.0);
        scored += 1;
    }
    if scored == 0 {
        return 0.0;
    }
    total / scored as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interleave_matches_the_documented_layout() {
        // 4bpp: planes 0 and 1 for rows 0..8, then planes 2 and 3.
        assert_eq!(byte_index(0, 0), 0);
        assert_eq!(byte_index(1, 0), 1);
        assert_eq!(byte_index(0, 7), 14);
        assert_eq!(byte_index(1, 7), 15);
        assert_eq!(byte_index(2, 0), 16);
        assert_eq!(byte_index(3, 7), 31);
        assert_eq!(tile_len(2), 16);
        assert_eq!(tile_len(4), 32);
        assert_eq!(tile_len(8), 64);
    }

    #[test]
    fn a_pixel_takes_one_bit_from_each_plane() {
        let mut tile = [0u8; 32];
        // Pixel (0, 0) with index 0b1011: planes 0, 1 and 3 set at bit 7.
        tile[byte_index(0, 0)] = 0x80;
        tile[byte_index(1, 0)] = 0x80;
        tile[byte_index(3, 0)] = 0x80;
        assert_eq!(pixel_index(&tile, 4, 0, 0), 0b1011);
        assert_eq!(pixel_index(&tile, 4, 1, 0), 0);
        assert_eq!(pixel_index(&tile, 2, 0, 0), 0b11, "2bpp sees two planes");
    }

    #[test]
    fn a_drawn_tile_outscores_noise() {
        // A tile whose left half is index 1 and right half transparent.
        let mut drawn = [0u8; 32];
        for y in 0..8 {
            drawn[byte_index(0, y)] = 0xF0;
        }
        let noise: Vec<u8> = (0..32u8)
            .map(|i| i.wrapping_mul(97).wrapping_add(31))
            .collect();
        assert!(bitplane_score(&drawn, 4) > bitplane_score(&noise, 4));
        assert_eq!(bitplane_score(&[0u8; 32], 4), 0.0, "flat fill says nothing");
        assert_eq!(bitplane_score(&[0u8; 8], 4), 0.0, "shorter than a tile");
    }
}
