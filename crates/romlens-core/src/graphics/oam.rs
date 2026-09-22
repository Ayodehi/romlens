//! OAM: the sprite table. 128 entries of four bytes, then 32 bytes that pack
//! two more bits per sprite.
//!
//! ```text
//! low table, per sprite    x (low 8)   y   tile (low 8)   vhoopppn
//! high table, per byte     four sprites, two bits each: (size, x bit 8),
//!                          sprite 0 in bits 0–1
//! ```
//!
//! `n` is bit 8 of the tile number and selects the second name table; X is
//! nine bits and signed, so a sprite can hang off the left edge.

/// Sprites in OAM.
pub const SPRITES: usize = 128;
/// Bytes of the low table.
pub const LOW_LEN: usize = 512;
/// Bytes of the whole table, low and high.
pub const OAM_LEN: usize = 544;

/// One decoded sprite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OamEntry {
    pub index: u8,
    /// −256..=255.
    pub x: i16,
    pub y: u8,
    /// Nine bits: the low byte plus `n`.
    pub tile: u16,
    /// 0–7, an OBJ palette: CGRAM colours `128 + 16 * palette`.
    pub palette: u8,
    /// 0–3.
    pub priority: u8,
    pub hflip: bool,
    pub vflip: bool,
    /// The high table's size bit: the second of the OBSEL pair.
    pub large: bool,
}

impl OamEntry {
    /// The name table the tile is in, 0 or 1.
    pub const fn name_table(&self) -> u8 {
        (self.tile >> 8) as u8
    }

    /// The four low-table bytes and the high-table byte this entry was read
    /// from, as offsets into the 544-byte table. Selecting a sprite selects
    /// these, which is what keeps one selection across views.
    pub const fn byte_offsets(&self) -> ([usize; 4], usize) {
        let i = self.index as usize;
        ([i * 4, i * 4 + 1, i * 4 + 2, i * 4 + 3], LOW_LEN + i / 4)
    }
}

/// Decode the table. Missing bytes read as zero, so a short range still
/// decodes as many whole sprites as it holds and zeroes the rest.
pub fn decode_oam(bytes: &[u8]) -> Vec<OamEntry> {
    let at = |i: usize| bytes.get(i).copied().unwrap_or(0);
    (0..SPRITES)
        .map(|i| {
            let attr = at(i * 4 + 3);
            let high = at(LOW_LEN + i / 4) >> ((i % 4) * 2);
            let x9 = at(i * 4) as u16 | ((high as u16 & 1) << 8);
            OamEntry {
                index: i as u8,
                // Sign-extend nine bits.
                x: ((x9 << 7) as i16) >> 7,
                y: at(i * 4 + 1),
                tile: at(i * 4 + 2) as u16 | ((attr as u16 & 1) << 8),
                palette: attr >> 1 & 7,
                priority: attr >> 4 & 3,
                hflip: attr & 0x40 != 0,
                vflip: attr & 0x80 != 0,
                large: high & 2 != 0,
            }
        })
        .collect()
}

/// The eight `OBSEL` size pairs, `(small, large)` as `(width, height)`.
///
/// Rows 6 and 7 are the two rectangular pairs, which are undocumented in
/// Nintendo's manual but real hardware, and games use them.
pub const OBJ_SIZES: [((u8, u8), (u8, u8)); 8] = [
    ((8, 8), (16, 16)),
    ((8, 8), (32, 32)),
    ((8, 8), (64, 64)),
    ((16, 16), (32, 32)),
    ((16, 16), (64, 64)),
    ((32, 32), (64, 64)),
    ((16, 32), (32, 64)),
    ((16, 32), (32, 32)),
];

/// What `OBSEL` (`$2101`, `sssnnbbb`) says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjSelect {
    /// Bits 5–7: which row of [`OBJ_SIZES`].
    pub size: u8,
    /// Bits 3–4: the gap between the two name tables, in 4K-word steps
    /// beyond the first.
    pub name_select: u8,
    /// Bits 0–2: the first name table, in 8K-word steps.
    pub name_base: u8,
}

impl ObjSelect {
    pub const fn from_register(obsel: u8) -> Self {
        ObjSelect {
            size: obsel >> 5,
            name_select: obsel >> 3 & 3,
            name_base: obsel & 7,
        }
    }

    /// `(width, height)` in pixels for a sprite with this size bit.
    pub const fn size_of(self, large: bool) -> (u8, u8) {
        let pair = OBJ_SIZES[self.size as usize];
        if large { pair.1 } else { pair.0 }
    }

    /// The VRAM **word** address of a nine-bit tile number. Sprites are
    /// always 4 bpp, sixteen words a tile.
    pub const fn tile_word_address(self, tile: u16) -> u16 {
        let base = (self.name_base as u32) << 13;
        let table = if tile & 0x100 != 0 {
            ((self.name_select as u32) + 1) << 12
        } else {
            0
        };
        ((base + table + ((tile as u32 & 0xFF) << 4)) & 0x7FFF) as u16
    }
}

/// The tile numbers a sprite is drawn from, row by row, before flipping.
///
/// A large sprite is a block of the 16-wide tile page: `n, n+1, …` across and
/// `n+16, n+32, …` down. Both halves of the number wrap inside the page —
/// the column in the low nibble and the row in the high nibble — so a sprite
/// that starts at tile `$0F` continues at `$00`, not `$10`. The name-table
/// bit is carried unchanged.
pub fn sprite_tiles(entry: &OamEntry, obsel: ObjSelect) -> Vec<u16> {
    let (w, h) = obsel.size_of(entry.large);
    let (cols, rows) = (w as u16 / 8, h as u16 / 8);
    let n = entry.tile;
    let mut out = Vec::with_capacity((cols * rows) as usize);
    for r in 0..rows {
        for c in 0..cols {
            let col = (n + c) & 0x0F;
            let row = ((n >> 4) + r) & 0x0F;
            out.push((n & 0x100) | row << 4 | col);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_with(index: usize, low: [u8; 4], high_bits: u8) -> Vec<u8> {
        let mut t = vec![0u8; OAM_LEN];
        t[index * 4..index * 4 + 4].copy_from_slice(&low);
        t[LOW_LEN + index / 4] |= (high_bits & 3) << ((index % 4) * 2);
        t
    }

    #[test]
    fn every_field_comes_from_its_bits() {
        // vhoopppn = 1 1 10 101 1
        let t = table_with(5, [0x20, 0x40, 0x7F, 0b1110_1011], 0b10);
        let e = decode_oam(&t)[5];
        assert_eq!(e.x, 0x20);
        assert_eq!(e.y, 0x40);
        assert_eq!(e.tile, 0x17F);
        assert_eq!(e.name_table(), 1);
        assert_eq!(e.palette, 0b101);
        assert_eq!(e.priority, 0b10);
        assert!(e.hflip && e.vflip && e.large);
        assert_eq!(e.byte_offsets(), ([20, 21, 22, 23], 513));
    }

    #[test]
    fn x_is_nine_bits_signed() {
        let t = table_with(2, [0xF0, 0, 0, 0], 0b01);
        assert_eq!(decode_oam(&t)[2].x, -16);
        let t = table_with(2, [0xFF, 0, 0, 0], 0b00);
        assert_eq!(decode_oam(&t)[2].x, 255);
        let t = table_with(127, [0x00, 0, 0, 0], 0b01);
        assert_eq!(decode_oam(&t)[127].x, -256);
    }

    #[test]
    fn the_high_table_packs_four_sprites_a_byte() {
        let mut t = vec![0u8; OAM_LEN];
        t[LOW_LEN] = 0b10_00_11_01;
        let e = decode_oam(&t);
        assert_eq!((e[0].large, e[0].x), (false, -256));
        assert_eq!((e[1].large, e[1].x), (true, -256));
        assert_eq!((e[2].large, e[2].x), (false, 0));
        assert_eq!((e[3].large, e[3].x), (true, 0));
    }

    #[test]
    // Grouped by field, `sss nn bbb`, which is the point of writing it in binary.
    #[allow(clippy::unusual_byte_groupings)]
    fn size_table_and_name_tables() {
        let s = ObjSelect::from_register(0b011_01_010);
        assert_eq!((s.size, s.name_select, s.name_base), (3, 1, 2));
        assert_eq!(s.size_of(false), (16, 16));
        assert_eq!(s.size_of(true), (32, 32));
        assert_eq!(ObjSelect::from_register(0xC0).size_of(true), (32, 64));
        assert_eq!(ObjSelect::from_register(0xE0).size_of(false), (16, 32));
        // Base 2 → word $4000; tile $10 is sixteen tiles in.
        assert_eq!(s.tile_word_address(0x010), 0x4100);
        // The second table is (1 + 1) × $1000 words further on.
        assert_eq!(s.tile_word_address(0x100), 0x6000);
    }

    #[test]
    fn large_sprites_wrap_inside_the_page() {
        let obsel = ObjSelect::from_register(0x00); // 8×8 / 16×16
        let mut e = decode_oam(&[0u8; OAM_LEN])[0];
        e.large = true;
        e.tile = 0x00;
        assert_eq!(sprite_tiles(&e, obsel), vec![0x00, 0x01, 0x10, 0x11]);
        e.tile = 0x1FF;
        assert_eq!(sprite_tiles(&e, obsel), vec![0x1FF, 0x1F0, 0x10F, 0x100]);
        e.large = false;
        assert_eq!(sprite_tiles(&e, obsel), vec![0x1FF]);
    }
}
