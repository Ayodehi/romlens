//! BG tilemaps: one 16-bit entry per cell, `vhopppcc cccccccc`.
//!
//! A sub-map is 32×32 entries, 2048 bytes. `BGnSC` (`$2107`–`$210A`) gives
//! the base in bits 2–7 and the screen size in bits 0–1; the larger sizes
//! place further sub-maps after the first, 1K words apart. `BGnNBA` gives the
//! character base the tile numbers count from.

/// Entries in one sub-map, 32 × 32.
pub const SUBMAP_ENTRIES: usize = 1024;
/// Bytes in one sub-map.
pub const SUBMAP_LEN: usize = SUBMAP_ENTRIES * 2;

/// One tilemap cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TilemapEntry {
    pub raw: u16,
    /// Ten bits.
    pub tile: u16,
    /// 0–7.
    pub palette: u8,
    pub priority: bool,
    pub hflip: bool,
    pub vflip: bool,
}

impl TilemapEntry {
    pub const fn from_raw(raw: u16) -> Self {
        TilemapEntry {
            raw,
            tile: raw & 0x03FF,
            palette: (raw >> 10 & 7) as u8,
            priority: raw & 0x2000 != 0,
            hflip: raw & 0x4000 != 0,
            vflip: raw & 0x8000 != 0,
        }
    }

    /// Read the little-endian entry at `index`; missing bytes read as zero.
    pub fn at(bytes: &[u8], index: usize) -> Self {
        let lo = bytes.get(index * 2).copied().unwrap_or(0);
        let hi = bytes.get(index * 2 + 1).copied().unwrap_or(0);
        Self::from_raw(u16::from_le_bytes([lo, hi]))
    }
}

/// `BGnSC` bits 0–1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenSize {
    S32x32,
    S64x32,
    S32x64,
    S64x64,
}

impl ScreenSize {
    pub const fn from_bits(bits: u8) -> Self {
        match bits & 3 {
            0 => ScreenSize::S32x32,
            1 => ScreenSize::S64x32,
            2 => ScreenSize::S32x64,
            _ => ScreenSize::S64x64,
        }
    }

    pub const fn bits(self) -> u8 {
        match self {
            ScreenSize::S32x32 => 0,
            ScreenSize::S64x32 => 1,
            ScreenSize::S32x64 => 2,
            ScreenSize::S64x64 => 3,
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "32x32" => ScreenSize::S32x32,
            "64x32" => ScreenSize::S64x32,
            "32x64" => ScreenSize::S32x64,
            "64x64" => ScreenSize::S64x64,
            _ => return None,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            ScreenSize::S32x32 => "32x32",
            ScreenSize::S64x32 => "64x32",
            ScreenSize::S32x64 => "32x64",
            ScreenSize::S64x64 => "64x64",
        }
    }

    /// `(columns, rows)` of entries.
    pub const fn cells(self) -> (u32, u32) {
        match self {
            ScreenSize::S32x32 => (32, 32),
            ScreenSize::S64x32 => (64, 32),
            ScreenSize::S32x64 => (32, 64),
            ScreenSize::S64x64 => (64, 64),
        }
    }

    /// Entries in the whole map.
    pub const fn entries(self) -> usize {
        let (c, r) = self.cells();
        (c * r) as usize
    }

    /// Where cell (`col`, `row`) is, as an entry index from the first
    /// sub-map's start (so ×2 for bytes, or + base for a VRAM word address).
    ///
    /// Sub-maps are laid out SC0, then right, then below, then diagonal; a
    /// 32×64 map has no "right", so its second sub-map is the one below.
    pub const fn entry_index(self, col: u32, row: u32) -> usize {
        let (cols, rows) = self.cells();
        let (col, row) = (col % cols, row % rows);
        let across = if cols == 64 { 2 } else { 1 };
        let submap = col / 32 + (row / 32) * across;
        (submap * 1024 + (row % 32) * 32 + col % 32) as usize
    }
}

/// The tilemap's VRAM word address from `BGnSC`.
pub const fn sc_base_word(bgsc: u8) -> u16 {
    ((bgsc as u16) & 0xFC) << 8
}

/// The character base's VRAM word address from one nibble of `BGnNBA`.
pub const fn char_base_word(nibble: u8) -> u16 {
    ((nibble as u16) & 0x0F) << 12
}

/// The 8×8 tiles a 16×16 cell is drawn from, as (tile, x, y) in pixels
/// inside the cell, after flipping.
///
/// `n, n+1` across and `n+16, n+17` down, then the whole block is mirrored by
/// the entry's flips. For an 8×8 cell it is just the one tile at (0, 0).
pub fn cell_tiles(entry: &TilemapEntry, tile16: bool) -> Vec<(u16, u8, u8)> {
    if !tile16 {
        return vec![(entry.tile, 0, 0)];
    }
    let mut out = Vec::with_capacity(4);
    for (dy, dx) in [(0u16, 0u16), (0, 1), (1, 0), (1, 1)] {
        let tile = (entry.tile + dy * 16 + dx) & 0x03FF;
        let x = if entry.hflip { 1 - dx } else { dx } as u8 * 8;
        let y = if entry.vflip { 1 - dy } else { dy } as u8 * 8;
        out.push((tile, x, y));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Grouped by field, `v h o ppp cc cccccccc`.
    #[allow(clippy::unusual_byte_groupings)]
    fn every_field_comes_from_its_bits() {
        let e = TilemapEntry::from_raw(0b1_0_1_110_1100110011);
        assert_eq!(e.tile, 0b1100110011);
        assert_eq!(e.palette, 0b110);
        assert!(e.priority);
        assert!(!e.hflip);
        assert!(e.vflip);
        assert_eq!(TilemapEntry::at(&[0x34, 0x12], 0).raw, 0x1234);
    }

    #[test]
    fn sub_maps_are_laid_out_right_then_below() {
        use ScreenSize::*;
        assert_eq!(S32x32.entry_index(31, 31), 1023);
        assert_eq!(S64x32.entry_index(32, 0), 0x400, "the right-hand sub-map");
        assert_eq!(
            S32x64.entry_index(0, 32),
            0x400,
            "the one below comes second"
        );
        assert_eq!(S64x64.entry_index(32, 0), 0x400);
        assert_eq!(S64x64.entry_index(0, 32), 0x800);
        assert_eq!(S64x64.entry_index(32, 32), 0xC00);
        assert_eq!(S64x64.entry_index(63, 63), 0xFFF);
        assert_eq!(
            S32x32.entry_index(33, 0),
            1,
            "cells wrap like scrolling does"
        );
        assert_eq!(ScreenSize::parse("64x32"), Some(S64x32));
        assert_eq!(ScreenSize::from_bits(S32x64.bits()), S32x64);
    }

    #[test]
    fn registers_give_word_addresses() {
        // BG1SC = $51: base bits $50 → word $5000, size 1 (64×32).
        assert_eq!(sc_base_word(0x51), 0x5000);
        assert_eq!(ScreenSize::from_bits(0x51), ScreenSize::S64x32);
        assert_eq!(char_base_word(0x4), 0x4000);
    }

    #[test]
    fn a_16x16_cell_uses_four_tiles_and_flips_as_a_block() {
        let e = TilemapEntry::from_raw(0x0020);
        assert_eq!(
            cell_tiles(&e, true),
            vec![(0x20, 0, 0), (0x21, 8, 0), (0x30, 0, 8), (0x31, 8, 8)]
        );
        let flipped = TilemapEntry::from_raw(0x4020);
        assert_eq!(cell_tiles(&flipped, true)[0], (0x20, 8, 0));
        assert_eq!(cell_tiles(&e, false), vec![(0x20, 0, 0)]);
    }
}
