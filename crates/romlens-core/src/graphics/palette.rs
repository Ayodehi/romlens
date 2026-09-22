//! SNES palettes: 15-bit BGR, two bytes per colour.

/// Bytes per colour.
pub const COLOUR_LEN: usize = 2;
/// Colours in one palette row.
pub const ROW_LEN: usize = 16;

/// `0bbbbbgggggrrrrr` → 8-bit R, G, B.
///
/// The expansion is `(c5 << 3) | (c5 >> 2)`, which maps 31 to 255 rather than
/// 248. It is pinned explicitly because the Phase 2B comparison against
/// Mesen2's tile viewer only means anything if both sides expand the same way
/// (`03-architecture.md`).
pub const fn bgr15_to_rgb(value: u16) -> (u8, u8, u8) {
    (expand5(value), expand5(value >> 5), expand5(value >> 10))
}

const fn expand5(value: u16) -> u8 {
    let v = (value & 0x1F) as u8;
    (v << 3) | (v >> 2)
}

/// How much `bytes` look like a run of BGR15 colours, 0.0 to 1.0.
///
/// The load-bearing signal is bit 15: it is unused in a SNES colour and so is
/// clear in every entry of a real palette, while in arbitrary data it is clear
/// about half the time. Over sixteen colours that alone is a 1-in-65,536
/// coincidence, so a single set bit anywhere in the window disqualifies it.
///
/// Two further tests stop it claiming every run of small numbers, and both are
/// applied per sixteen-colour row rather than across the window, because that
/// is how the hardware uses CGRAM: a block of palette data is rows, and a
/// block of eight copies of one row is still palette data.
///
/// - A row's colours differ from one another, so a row of one repeated value
///   scores nothing.
/// - A row reaches out of the bottom of the range. Colours that are all tiny
///   are small numbers — the shape zero-padding and short tables have.
pub fn palette_score(bytes: &[u8]) -> f32 {
    let row_bytes = ROW_LEN * COLOUR_LEN;
    if bytes.len() < row_bytes {
        return 0.0;
    }
    let colours: Vec<u16> = bytes
        .chunks_exact(COLOUR_LEN)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    if colours.iter().any(|c| c & 0x8000 != 0) {
        return 0.0;
    }
    let mut rows = 0usize;
    let mut total = 0.0f32;
    for row in colours.chunks_exact(ROW_LEN) {
        let mut distinct: Vec<u16> = row.to_vec();
        distinct.sort_unstable();
        distinct.dedup();
        let variety = distinct.len() as f32 / ROW_LEN as f32;
        let spread = row.iter().filter(|c| **c >= 0x0400).count() as f32 / ROW_LEN as f32;
        if variety < 0.25 || spread < 0.25 {
            return 0.0;
        }
        total += 0.5 + 0.3 * variety.min(spread);
        rows += 1;
    }
    if rows == 0 {
        return 0.0;
    }
    (total / rows as f32).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_reaches_both_ends() {
        assert_eq!(bgr15_to_rgb(0x0000), (0, 0, 0));
        assert_eq!(bgr15_to_rgb(0x7FFF), (255, 255, 255));
        // $001F is full red, and red is the low five bits.
        assert_eq!(bgr15_to_rgb(0x001F), (255, 0, 0));
        assert_eq!(bgr15_to_rgb(0x7C00), (0, 0, 255));
    }

    #[test]
    fn scores_a_palette_above_filler() {
        let palette: Vec<u8> = (0..16u16)
            .flat_map(|i| (i * 0x0421 + 0x0400).to_le_bytes())
            .collect();
        assert!(palette_score(&palette) > 0.5);
        // One high bit set anywhere is disqualifying.
        let mut spoiled = palette.clone();
        spoiled[9] |= 0x80;
        assert_eq!(palette_score(&spoiled), 0.0);
        assert_eq!(palette_score(&[0u8; 32]), 0.0, "zero fill is not a palette");
        assert_eq!(palette_score(&[0x01, 0x00].repeat(16)), 0.0, "one colour");
        assert_eq!(palette_score(&[1u8; 8]), 0.0, "too short to judge");
    }
}
