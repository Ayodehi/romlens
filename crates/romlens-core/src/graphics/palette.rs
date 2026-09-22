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

/// Bytes of CGRAM: 256 colours.
pub const CGRAM_LEN: usize = 512;
/// The first OBJ colour. BG palettes are rows 0–7, OBJ palettes rows 8–15.
pub const OBJ_BASE: usize = 128;

/// One CGRAM entry, `0bbbbbgggggrrrrr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Colour(pub u16);

impl Colour {
    /// Read the little-endian entry at `index`; missing bytes read as zero.
    pub fn at(bytes: &[u8], index: usize) -> Colour {
        let lo = bytes.get(index * 2).copied().unwrap_or(0);
        let hi = bytes.get(index * 2 + 1).copied().unwrap_or(0);
        Colour(u16::from_le_bytes([lo, hi]))
    }

    pub const fn red5(self) -> u8 {
        (self.0 & 0x1F) as u8
    }
    pub const fn green5(self) -> u8 {
        (self.0 >> 5 & 0x1F) as u8
    }
    pub const fn blue5(self) -> u8 {
        (self.0 >> 10 & 0x1F) as u8
    }
    /// Bit 15, which the PPU ignores. Set in a real palette only by accident,
    /// which is why the palette heuristic refuses it.
    pub const fn unused_bit(self) -> bool {
        self.0 & 0x8000 != 0
    }

    pub const fn rgb8(self) -> (u8, u8, u8) {
        bgr15_to_rgb(self.0)
    }

    pub const fn rgba(self) -> [u8; 4] {
        let (r, g, b) = self.rgb8();
        [r, g, b, 255]
    }
}

/// Where a tile view takes its colours from.
///
/// This is what makes the tile view usable on unknown ROM bytes: before the
/// student knows which palette a tile uses, grayscale shows its shape; once
/// they find a palette in the ROM, `Bytes` points at it; with a recording,
/// `Cgram` is the machine's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteRef<'a> {
    /// A whole CGRAM image, 512 bytes; the row chooses which sixteen colours.
    Cgram(&'a [u8]),
    /// BGR15 colours starting at the first byte, as many as the format needs.
    Bytes(&'a [u8]),
    /// Index `i` of `n` colours drawn as `i * 255 / (n - 1)` gray.
    Grayscale,
}

/// The colours a tile of `colours` indices uses, as RGBA.
///
/// `row` picks the palette the way the hardware does: a 4 bpp tile with
/// palette 3 reads CGRAM entries 48–63, a 2 bpp tile reads 12–15 (4 × 3),
/// and 8 bpp ignores it. `base` is added first, which is how BG palettes
/// (base 0) and OBJ palettes (base 128) share one function, and how Mode 0
/// gives each background its own 32 colours.
pub fn resolve(palette: PaletteRef<'_>, colours: usize, row: u8, base: usize) -> Vec<[u8; 4]> {
    match palette {
        PaletteRef::Grayscale => (0..colours)
            .map(|i| {
                let v = (i * 255 / (colours - 1).max(1)) as u8;
                [v, v, v, 255]
            })
            .collect(),
        PaletteRef::Bytes(bytes) => (0..colours).map(|i| Colour::at(bytes, i).rgba()).collect(),
        PaletteRef::Cgram(cgram) => {
            let first = if colours >= 256 {
                0
            } else {
                base + row as usize * colours
            };
            (0..colours)
                .map(|i| Colour::at(cgram, (first + i) % 256).rgba())
                .collect()
        }
    }
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
        .as_chunks::<COLOUR_LEN>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    if colours.iter().any(|c| c & 0x8000 != 0) {
        return 0.0;
    }
    let mut rows = 0usize;
    let mut total = 0.0f32;
    for row in colours.as_chunks::<ROW_LEN>().0 {
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
    fn fields_come_out_of_the_right_bits() {
        let c = Colour(0b0_10101_01010_11111);
        assert_eq!(
            (c.blue5(), c.green5(), c.red5()),
            (0b10101, 0b01010, 0b11111)
        );
        assert!(!c.unused_bit());
        assert!(Colour(0x8000).unused_bit());
        assert_eq!(Colour::at(&[0x1F, 0x00], 0).rgb8(), (255, 0, 0));
        assert_eq!(Colour::at(&[0x1F], 3), Colour(0), "past the end reads zero");
    }

    #[test]
    fn rows_select_like_the_hardware() {
        let cgram: Vec<u8> = (0..256u16).flat_map(|i| i.to_le_bytes()).collect();
        let idx = |c: [u8; 4]| {
            // Invert the expansion: every value here is below 32 per field.
            let r5 = c[0] >> 3;
            let g5 = c[1] >> 3;
            let b5 = c[2] >> 3;
            r5 as u16 | (g5 as u16) << 5 | (b5 as u16) << 10
        };
        let four = resolve(PaletteRef::Cgram(&cgram), 16, 3, 0);
        assert_eq!(idx(four[0]), 48);
        let two = resolve(PaletteRef::Cgram(&cgram), 4, 3, 0);
        assert_eq!(idx(two[0]), 12);
        let obj = resolve(PaletteRef::Cgram(&cgram), 16, 1, OBJ_BASE);
        assert_eq!(idx(obj[0]), 144);
        let full = resolve(PaletteRef::Cgram(&cgram), 256, 7, 0);
        assert_eq!(idx(full[200]), 200, "8 bpp ignores the row");
        let gray = resolve(PaletteRef::Grayscale, 4, 0, 0);
        assert_eq!(
            gray,
            vec![
                [0, 0, 0, 255],
                [85, 85, 85, 255],
                [170, 170, 170, 255],
                [255, 255, 255, 255]
            ]
        );
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
