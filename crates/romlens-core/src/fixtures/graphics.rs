//! The graphics fixture: tiles, a palette, a sprite table, a tilemap and a
//! compressed block, all drawn by hand for this repository, so every graphics
//! golden runs with no commercial ROM (`12-content-policy.md` rule 1).
//!
//! | File offset | Contents |
//! |---|---|
//! | `0x1000` | 32 tiles, 4 bpp: shapes, then a 16×16 lens at tiles `$08 $09 $18 $19` |
//! | `0x1400` | 8 tiles, 2 bpp: the letters `ROMLENS` and a blank |
//! | `0x1800` | 256 colours: a gray ramp, then seven gradient rows, then eight OBJ rows |
//! | `0x1A00` | 544 bytes of OAM: four sprites on screen, the rest parked at y = 240 |
//! | `0x2000` | a 32×32 tilemap, 2048 bytes, framing the lens |
//! | `0x3000` | the 4 bpp tiles again, compressed in Super Metroid's format |
//!
//! [`machine`] loads the same pieces into VRAM, CGRAM and OAM with the
//! registers set, which is what the synthetic recording and the render
//! tests draw.

use crate::analysis::accuracy::TruthRange;
use crate::graphics::ppu_state::PpuState;
use crate::graphics::tile::{TileFormat, encode_tile};
use crate::memory::map::MappingMode;
use crate::model::region::{DataKind, RegionKind};

pub const GRAPHICS_TITLE: &str = "ROMLENS GRAPHICS";

pub const TILES_4BPP: usize = 0x1000;
pub const TILES_4BPP_LEN: usize = 32 * 32;
pub const TILES_2BPP: usize = 0x1400;
pub const TILES_2BPP_LEN: usize = 8 * 16;
pub const PALETTE: usize = 0x1800;
pub const OAM_TABLE: usize = 0x1A00;
pub const TILEMAP: usize = 0x2000;
pub const COMPRESSED: usize = 0x3000;

/// Parse an 8×8 picture: `.` is 0, `1`–`9` and `a`–`f` their values.
fn picture(rows: [&str; 8]) -> [u8; 64] {
    let mut out = [0u8; 64];
    for (y, row) in rows.iter().enumerate() {
        for (x, c) in row.chars().enumerate().take(8) {
            out[y * 8 + x] = c.to_digit(16).unwrap_or(0) as u8;
        }
    }
    out
}

/// The 32 four-bpp tiles, by number.
pub fn tiles_4bpp() -> Vec<[u8; 64]> {
    let mut tiles = vec![[0u8; 64]; 32];
    tiles[1] = [1; 64];
    tiles[2] = std::array::from_fn(|i| if (i % 8 + i / 8) % 2 == 0 { 1 } else { 2 });
    tiles[3] = std::array::from_fn(|i| ((i % 8 + i / 8) % 16) as u8);
    tiles[4] = picture([
        "33333333", "3......3", "3......3", "3......3", "3......3", "3......3", "3......3",
        "33333333",
    ]);
    tiles[5] = picture([
        "...11...", "..1221..", ".123321.", "12344321", "12344321", ".123321.", "..1221..",
        "...11...",
    ]);
    tiles[6] = std::array::from_fn(|i| (i / 8) as u8 + 8);
    tiles[7] = std::array::from_fn(|i| (i % 8) as u8 + 8);
    // The lens, 16×16: a ring of 1, a body of 2, a highlight of 3, with the
    // four quarters at $08 $09 $18 $19 so a large sprite finds them.
    let lens = |x: i32, y: i32| -> u8 {
        let (dx, dy) = (2 * x - 15, 2 * y - 15);
        let d = dx * dx + dy * dy;
        if d > 15 * 15 {
            0
        } else if d > 11 * 11 {
            1
        } else if (dx + 6) * (dx + 6) + (dy + 6) * (dy + 6) < 5 * 5 {
            3
        } else {
            2
        }
    };
    for (slot, (qx, qy)) in [
        (8usize, (0, 0)),
        (9, (8, 0)),
        (0x18, (0, 8)),
        (0x19, (8, 8)),
    ] {
        tiles[slot] = std::array::from_fn(|i| lens(qx + (i % 8) as i32, qy + (i / 8) as i32));
    }
    for (k, tile) in tiles.iter_mut().enumerate().skip(0x10).take(8) {
        // Horizontal stripes in colour k − 15.
        *tile = std::array::from_fn(|i| if (i / 8) % 2 == 0 { (k - 15) as u8 } else { 0 });
    }
    tiles
}

/// The 2 bpp letters.
pub fn tiles_2bpp() -> Vec<[u8; 64]> {
    vec![
        picture([
            "1111....", "1...1...", "1...1...", "1111....", "1.1.....", "1..1....", "1...1...",
            "........",
        ]),
        picture([
            ".111....", "1...1...", "1...1...", "1...1...", "1...1...", "1...1...", ".111....",
            "........",
        ]),
        picture([
            "1...1...", "11.11...", "1.1.1...", "1...1...", "1...1...", "1...1...", "1...1...",
            "........",
        ]),
        picture([
            "1.......", "1.......", "1.......", "1.......", "1.......", "1.......", "11111...",
            "........",
        ]),
        picture([
            "11111...", "1.......", "1.......", "1111....", "1.......", "1.......", "11111...",
            "........",
        ]),
        picture([
            "1...1...", "11..1...", "1.1.1...", "1..11...", "1...1...", "1...1...", "1...1...",
            "........",
        ]),
        picture([
            ".1111...", "1.......", "1.......", ".111....", "....1...", "....1...", "1111....",
            "........",
        ]),
        [0; 64],
    ]
}

/// CGRAM: row 0 a gray ramp, rows 1–7 gradients, rows 8–15 the OBJ palettes.
pub fn palette() -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    for i in 0..256u16 {
        let (row, col) = (i / 16, i % 16);
        let v = col * 31 / 15;
        let c = match row % 8 {
            0 => v | v << 5 | v << 10,
            1 => v,
            2 => v << 5,
            3 => v << 10,
            4 => v | v << 5,
            5 => v << 5 | v << 10,
            6 => v | v << 10,
            _ => (31 - v) | v << 10,
        };
        out.extend_from_slice(&c.to_le_bytes());
    }
    out
}

/// OAM: the lens as a large sprite, a small ring beside it, two flipped
/// copies, and the other 124 parked below the screen.
pub fn oam() -> Vec<u8> {
    let mut t = vec![0u8; 544];
    for i in 0..128 {
        t[i * 4 + 1] = 240;
    }
    let sprites: [(u8, u8, u8, u8, bool); 4] = [
        // x, y, tile, vhoopppn, large
        (0x40, 0x30, 0x08, 0b0010_0000, true),
        (0x58, 0x34, 0x05, 0b0010_0010, false),
        (0x70, 0x30, 0x08, 0b0110_0100, true),
        (0x90, 0x30, 0x08, 0b1111_0110, true),
    ];
    for (i, (x, y, tile, attr, large)) in sprites.into_iter().enumerate() {
        t[i * 4..i * 4 + 4].copy_from_slice(&[x, y, tile, attr]);
        if large {
            t[512 + i / 4] |= 0b10 << ((i % 4) * 2);
        }
    }
    t
}

/// The 32×32 tilemap: a border of tile 4, a checker field, and the lens in
/// the middle in palette 2, with one mirrored copy.
pub fn tilemap() -> Vec<u8> {
    let mut out = Vec::with_capacity(2048);
    for row in 0..32u16 {
        for col in 0..32u16 {
            let entry: u16 = if row == 0 || col == 0 || row == 31 || col == 31 {
                4 | 1 << 10
            } else if (14..16).contains(&row) && (14..16).contains(&col) {
                let q = (row - 14) * 16 + (col - 14);
                (0x08 + q) | 2 << 10 | 1 << 13
            } else if (14..16).contains(&row) && (18..20).contains(&col) {
                // Mirrored: the right-hand cell draws the left quarter.
                let q = (row - 14) * 16 + (19 - col);
                (0x08 + q) | 3 << 10 | 0x4000
            } else if (row + col) % 7 == 0 {
                5 | 4 << 10
            } else {
                2 | ((row / 8) & 7) << 10
            };
            out.extend_from_slice(&entry.to_le_bytes());
        }
    }
    out
}

fn bytes_4bpp() -> Vec<u8> {
    tiles_4bpp()
        .iter()
        .flat_map(|t| encode_tile(t, TileFormat::Bpp4))
        .collect()
}

fn bytes_2bpp() -> Vec<u8> {
    tiles_2bpp()
        .iter()
        .flat_map(|t| encode_tile(t, TileFormat::Bpp2))
        .collect()
}

/// A Super Metroid stream that decompresses to `data`, using only byte fills
/// and direct copies. Deliberately minimal: the fixture needs a stream that is
/// right by construction, and the decoder's tests carry a compressor that uses
/// every command.
pub fn pack_runs(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut literal: Vec<u8> = Vec::new();
    let flush = |out: &mut Vec<u8>, literal: &mut Vec<u8>| {
        for chunk in literal.chunks(32) {
            out.push((chunk.len() - 1) as u8);
            out.extend_from_slice(chunk);
        }
        literal.clear();
    };
    let mut i = 0;
    while i < data.len() {
        let run = data[i..]
            .iter()
            .take(32)
            .take_while(|b| **b == data[i])
            .count();
        if run >= 3 {
            flush(&mut out, &mut literal);
            out.extend([0x20 | (run - 1) as u8, data[i]]);
            i += run;
        } else {
            literal.push(data[i]);
            i += 1;
        }
    }
    flush(&mut out, &mut literal);
    out.push(0xFF);
    out
}

/// 64 KB LoROM holding the pieces in the table above.
pub fn graphics_lorom() -> Vec<u8> {
    let mut rom = super::build_with_code(
        MappingMode::LoRom,
        0x1_0000,
        false,
        &super::BOOT_CODE,
        GRAPHICS_TITLE,
    );
    let four = bytes_4bpp();
    rom[TILES_4BPP..TILES_4BPP + four.len()].copy_from_slice(&four);
    let two = bytes_2bpp();
    rom[TILES_2BPP..TILES_2BPP + two.len()].copy_from_slice(&two);
    rom[PALETTE..PALETTE + 512].copy_from_slice(&palette());
    rom[OAM_TABLE..OAM_TABLE + 544].copy_from_slice(&oam());
    rom[TILEMAP..TILEMAP + 2048].copy_from_slice(&tilemap());
    let packed = pack_runs(&four);
    rom[COMPRESSED..COMPRESSED + packed.len()].copy_from_slice(&packed);
    super::fix_checksum(&mut rom, MappingMode::LoRom);
    rom
}

/// Ground truth for [`graphics_lorom`].
pub fn truth_for_graphics() -> Vec<TruthRange> {
    let packed = pack_runs(&bytes_4bpp()).len() as u32;
    let mut ranges = super::truth_for(MappingMode::LoRom);
    let data = |start: usize, len: u32, kind: DataKind| TruthRange {
        start: start as u32,
        end: start as u32 + len,
        kind: RegionKind::Data(kind),
    };
    ranges.extend([
        data(
            TILES_4BPP,
            TILES_4BPP_LEN as u32,
            DataKind::Graphics { bpp: 4 },
        ),
        data(
            TILES_2BPP,
            TILES_2BPP_LEN as u32,
            DataKind::Graphics { bpp: 2 },
        ),
        data(PALETTE, 512, DataKind::Palette),
        data(OAM_TABLE, 544, DataKind::Struct),
        data(TILEMAP, 2048, DataKind::Tilemap),
        data(COMPRESSED, packed, DataKind::Compressed),
    ]);
    ranges.sort_by_key(|r| r.start);
    ranges
}

/// VRAM word addresses [`machine`] uses.
pub const VRAM_BG1_CHARS: u16 = 0x0000;
pub const VRAM_BG1_MAP: u16 = 0x1000;
pub const VRAM_BG3_MAP: u16 = 0x1400;
pub const VRAM_BG3_CHARS: u16 = 0x2000;

/// The fixture's pieces as a machine would hold them: 64 KB of VRAM, 512
/// bytes of CGRAM, 544 of OAM, and the registers that make sense of them —
/// Mode 1, BG1 the tilemap over the 4 bpp tiles, BG3 the word `ROMLENS` in
/// the 2 bpp letters, sprites from the 4 bpp tiles at 8×8 / 16×16.
pub fn machine() -> (Vec<u8>, Vec<u8>, Vec<u8>, PpuState) {
    let mut vram = vec![0u8; 0x10000];
    let four = bytes_4bpp();
    let at = VRAM_BG1_CHARS as usize * 2;
    vram[at..at + four.len()].copy_from_slice(&four);
    let map = tilemap();
    let at = VRAM_BG1_MAP as usize * 2;
    vram[at..at + map.len()].copy_from_slice(&map);
    let two = bytes_2bpp();
    let at = VRAM_BG3_CHARS as usize * 2;
    vram[at..at + two.len()].copy_from_slice(&two);
    // BG3: every cell the blank letter (7) except `ROMLENS` on row 2, in
    // palette 1 with priority, the way a status bar is drawn.
    let at = VRAM_BG3_MAP as usize * 2;
    for cell in 0..1024usize {
        let (row, col) = (cell / 32, cell % 32);
        let entry: u16 = if row == 2 && (2..9).contains(&col) {
            (col - 2) as u16 | 1 << 10 | 1 << 13
        } else {
            7
        };
        vram[at + cell * 2..at + cell * 2 + 2].copy_from_slice(&entry.to_le_bytes());
    }
    let mut ppu = PpuState::default();
    ppu.set_register(0x2100, 0x0F); // full brightness
    ppu.set_register(0x2101, 0x00); // OBSEL: 8×8 / 16×16, tiles at word 0
    ppu.set_register(0x2105, 0x09); // Mode 1, BG3 priority
    ppu.set_register(0x2107, (VRAM_BG1_MAP >> 8) as u8); // BG1SC, 32×32
    ppu.set_register(0x2109, (VRAM_BG3_MAP >> 8) as u8); // BG3SC, 32×32
    ppu.set_register(0x210B, (VRAM_BG1_CHARS >> 12) as u8);
    ppu.set_register(0x210C, (VRAM_BG3_CHARS >> 12) as u8);
    ppu.set_register(0x212C, 0x15); // TM: BG1, BG3, OBJ
    (vram, palette(), oam(), ppu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::compress::sm_lz::try_decompress;

    #[test]
    fn the_compressed_block_is_the_tiles() {
        let rom = graphics_lorom();
        let (out, used) = try_decompress(&rom[COMPRESSED..]).unwrap();
        assert_eq!(out, &rom[TILES_4BPP..TILES_4BPP + TILES_4BPP_LEN]);
        assert!(used < TILES_4BPP_LEN, "it compresses: {used}");
    }

    #[test]
    fn nothing_overlaps() {
        let t = truth_for_graphics();
        for w in t.windows(2) {
            assert!(w[0].end <= w[1].start, "{:x?} overlaps {:x?}", w[0], w[1]);
        }
    }
}
