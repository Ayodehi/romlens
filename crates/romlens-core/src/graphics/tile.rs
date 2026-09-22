//! Bitplane tiles: 8×8 pixels, one bit per plane per pixel.
//!
//! One rule covers 2, 4 and 8 bpp: `byte_index(plane b, row y) =
//! (b/2)*16 + y*2 + (b%2)`, `bit_index(x) = 7-x`. Mode 7 is the exception,
//! one byte per pixel in reading order.

/// Bytes in one 8×8 tile at this depth.
pub const fn tile_len(bpp: u8) -> usize {
    bpp as usize * 8
}

/// How a tile's bytes are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TileFormat {
    Bpp2,
    Bpp4,
    Bpp8,
    /// One byte per pixel, `bytes[y*8 + x]`: Mode 7's character data once it
    /// is separated from the tilemap bytes it is interleaved with in VRAM.
    Mode7,
}

impl TileFormat {
    /// `2`, `4`, `8` or `7` (for Mode 7), the spelling the CLI and the
    /// shells use.
    pub fn from_bpp(bpp: u8) -> Option<Self> {
        Some(match bpp {
            2 => TileFormat::Bpp2,
            4 => TileFormat::Bpp4,
            8 => TileFormat::Bpp8,
            7 => TileFormat::Mode7,
            _ => return None,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            TileFormat::Bpp2 => "2bpp",
            TileFormat::Bpp4 => "4bpp",
            TileFormat::Bpp8 => "8bpp",
            TileFormat::Mode7 => "mode7",
        }
    }

    /// Bits per pixel: the number of colour-index bits.
    pub const fn bpp(self) -> u8 {
        match self {
            TileFormat::Bpp2 => 2,
            TileFormat::Bpp4 => 4,
            TileFormat::Bpp8 | TileFormat::Mode7 => 8,
        }
    }

    /// Bitplanes, which Mode 7 does not have.
    pub const fn planes(self) -> u8 {
        match self {
            TileFormat::Mode7 => 0,
            other => other.bpp(),
        }
    }

    /// Bytes in one 8×8 tile.
    pub const fn tile_len(self) -> usize {
        match self {
            TileFormat::Mode7 => 64,
            other => tile_len(other.bpp()),
        }
    }

    /// Colours a tile can index.
    pub const fn colours(self) -> usize {
        1 << self.bpp()
    }
}

/// Where one bit of one pixel comes from: the byte (relative to the tile's
/// first byte) and the bit within it, 7 being the most significant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitSource {
    pub byte: usize,
    pub bit: u8,
}

/// The source of plane `plane` of pixel (`x`, `y`).
///
/// Pure arithmetic, so the teaching view can light a pixel's bits under the
/// cursor without a round trip through the FFI. For Mode 7 there are no
/// planes; "plane" is then the bit of the pixel's own byte, so the same view
/// can show eight bits of one byte instead of one bit of eight.
pub const fn tile_bit_source(format: TileFormat, x: u8, y: u8, plane: u8) -> Option<BitSource> {
    if x >= 8 || y >= 8 || plane >= format.bpp() {
        return None;
    }
    Some(match format {
        TileFormat::Mode7 => BitSource {
            byte: y as usize * 8 + x as usize,
            bit: plane,
        },
        _ => BitSource {
            byte: byte_index(plane, y),
            bit: 7 - x,
        },
    })
}

/// The 64 colour indices of one tile, row by row. Missing bytes read as zero,
/// so a tile cut short by the end of the ROM still decodes.
pub fn decode_tile(bytes: &[u8], format: TileFormat) -> [u8; 64] {
    let mut out = [0u8; 64];
    let len = format.tile_len();
    let mut tile = [0u8; 64];
    let n = bytes.len().min(len);
    tile[..n].copy_from_slice(&bytes[..n]);
    match format {
        TileFormat::Mode7 => out.copy_from_slice(&tile),
        _ => {
            for y in 0..8u8 {
                for x in 0..8u8 {
                    out[y as usize * 8 + x as usize] = pixel_index(&tile, format.bpp(), x, y);
                }
            }
        }
    }
    out
}

/// Encode 64 indices as a tile, the inverse of [`decode_tile`]. Used to build
/// fixtures from pictures we drew, and by the round-trip tests.
pub fn encode_tile(indices: &[u8; 64], format: TileFormat) -> Vec<u8> {
    let mut out = vec![0u8; format.tile_len()];
    match format {
        TileFormat::Mode7 => out.copy_from_slice(indices),
        _ => {
            for y in 0..8u8 {
                for x in 0..8u8 {
                    let index = indices[y as usize * 8 + x as usize];
                    for plane in 0..format.bpp() {
                        if index >> plane & 1 != 0 {
                            out[byte_index(plane, y)] |= 0x80 >> x;
                        }
                    }
                }
            }
        }
    }
    out
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
    fn decode_and_encode_are_inverse_at_every_depth() {
        for format in [
            TileFormat::Bpp2,
            TileFormat::Bpp4,
            TileFormat::Bpp8,
            TileFormat::Mode7,
        ] {
            let colours = format.colours();
            let indices: [u8; 64] = std::array::from_fn(|i| ((i * 37 + 11) % colours) as u8);
            let bytes = encode_tile(&indices, format);
            assert_eq!(bytes.len(), format.tile_len());
            assert_eq!(decode_tile(&bytes, format), indices, "{}", format.name());
        }
    }

    #[test]
    fn bit_sources_agree_with_the_decoder() {
        // Set exactly one bit per source and check the decoder sees exactly
        // that bit of exactly that pixel: the hover view and the tile view
        // then cannot disagree.
        for format in [
            TileFormat::Bpp2,
            TileFormat::Bpp4,
            TileFormat::Bpp8,
            TileFormat::Mode7,
        ] {
            for (x, y, plane) in [(0, 0, 0), (7, 0, 1), (3, 5, format.bpp() - 1), (6, 7, 0)] {
                let src = tile_bit_source(format, x, y, plane).unwrap();
                let mut bytes = vec![0u8; format.tile_len()];
                bytes[src.byte] |= 1 << src.bit;
                let decoded = decode_tile(&bytes, format);
                for (i, v) in decoded.iter().enumerate() {
                    let want = if i == y as usize * 8 + x as usize {
                        1 << plane
                    } else {
                        0
                    };
                    assert_eq!(*v, want, "{} ({x},{y}) plane {plane}", format.name());
                }
            }
        }
        assert_eq!(tile_bit_source(TileFormat::Bpp2, 0, 0, 2), None);
        assert_eq!(tile_bit_source(TileFormat::Bpp4, 8, 0, 0), None);
    }

    #[test]
    fn a_short_tile_reads_zeroes() {
        let decoded = decode_tile(&[0xFF, 0xFF], TileFormat::Bpp4);
        assert_eq!(&decoded[..8], &[3u8; 8]);
        assert!(decoded[8..].iter().all(|v| *v == 0));
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
