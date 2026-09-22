//! The bounded reference renderer: one BG layer, or one sprite, from VRAM,
//! CGRAM and the registers. Nothing else.
//!
//! No priority, no `TM`/`TS`, no windows, no colour math, no OBJ-over-BG
//! compositing, no Mode 7, no hi-res or interlace. The only honest oracle for
//! those is a recorded framebuffer to diff against, and that is a Phase 3
//! consumer; a compositor written now would be several hundred lines of
//! unverified code (`16-phase2-plan.md` 2B.6). Phase 3 adds `compose(…)`
//! above exactly these inner loops.

use crate::graphics::Bitmap;
use crate::graphics::oam::{OamEntry, ObjSelect, sprite_tiles};
use crate::graphics::palette::{OBJ_BASE, PaletteRef, resolve};
use crate::graphics::ppu_state::{PpuState, bg_format};
use crate::graphics::tile::{TileFormat, decode_tile};
use crate::graphics::tilemap::{ScreenSize, TilemapEntry, cell_tiles};

pub const VRAM_LEN: usize = 0x10000;

/// Everything one BG layer needs, read out of the registers once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BgConfig {
    pub format: TileFormat,
    /// VRAM word addresses.
    pub char_word: u16,
    pub map_word: u16,
    pub size: ScreenSize,
    pub tile16: bool,
    /// Added to every CGRAM index: Mode 0 gives each background its own 32
    /// colours, every other mode shares the first 128.
    pub palette_base: usize,
}

impl BgConfig {
    /// The configuration of BG `bg` (1–4) under the current `BGMODE`.
    /// `None` where the mode has no such layer, and for Mode 7, which this
    /// renderer does not draw.
    pub fn from_ppu(ppu: &PpuState, bg: u8) -> Option<BgConfig> {
        let mode = ppu.bg_mode();
        let format = bg_format(mode, bg)?;
        if format == TileFormat::Mode7 {
            return None;
        }
        Some(BgConfig {
            format,
            char_word: ppu.char_word(bg),
            map_word: ppu.tilemap_word(bg),
            size: ppu.screen_size(bg),
            tile16: ppu.tile16(bg),
            palette_base: if mode == 0 { (bg as usize - 1) * 32 } else { 0 },
        })
    }

    /// The whole map's size in pixels.
    pub fn pixel_size(&self) -> (u32, u32) {
        let (c, r) = self.size.cells();
        let cell = if self.tile16 { 16 } else { 8 };
        (c * cell, r * cell)
    }
}

/// 8×8 pixels of a tile, drawn at (`x0`, `y0`), optionally flipped. With
/// `transparent_zero`, index 0 leaves the pixel untouched — how the hardware
/// draws every layer and sprite — and without it index 0 takes colour 0, which
/// is what a tile viewer shows.
#[allow(clippy::too_many_arguments)]
pub fn draw_tile(
    bm: &mut Bitmap,
    x0: i32,
    y0: i32,
    indices: &[u8; 64],
    colours: &[[u8; 4]],
    transparent_zero: bool,
    hflip: bool,
    vflip: bool,
) {
    for y in 0..8i32 {
        for x in 0..8i32 {
            let sx = if hflip { 7 - x } else { x };
            let sy = if vflip { 7 - y } else { y };
            let index = indices[(sy * 8 + sx) as usize] as usize;
            if transparent_zero && index == 0 {
                continue;
            }
            let (px, py) = (x0 + x, y0 + y);
            if px >= 0 && py >= 0 {
                bm.set(px as u32, py as u32, colours[index % colours.len()]);
            }
        }
    }
}

/// The bytes of tile `tile` at a character base, wrapping inside VRAM.
fn vram_tile(vram: &[u8], char_word: u16, tile: u16, format: TileFormat) -> Vec<u8> {
    let len = format.tile_len();
    let start = (char_word as usize * 2 + tile as usize * len) % VRAM_LEN;
    (0..len)
        .map(|i| vram.get((start + i) % VRAM_LEN).copied().unwrap_or(0))
        .collect()
}

/// Draw a whole BG layer's map, unscrolled, index 0 transparent.
pub fn render_bg_layer(vram: &[u8], cgram: &[u8], config: &BgConfig) -> Bitmap {
    let (w, h) = config.pixel_size();
    let mut bm = Bitmap::new(w, h);
    let (cols, rows) = config.size.cells();
    let cell = if config.tile16 { 16 } else { 8 };
    let colours = config.format.colours();
    for row in 0..rows {
        for col in 0..cols {
            let at = config.map_word as usize * 2 + config.size.entry_index(col, row) * 2;
            let raw = u16::from_le_bytes([
                vram.get(at % VRAM_LEN).copied().unwrap_or(0),
                vram.get((at + 1) % VRAM_LEN).copied().unwrap_or(0),
            ]);
            let entry = TilemapEntry::from_raw(raw);
            let pal = resolve(
                PaletteRef::Cgram(cgram),
                colours,
                entry.palette,
                config.palette_base,
            );
            for (tile, dx, dy) in cell_tiles(&entry, config.tile16) {
                let bytes = vram_tile(vram, config.char_word, tile, config.format);
                draw_tile(
                    &mut bm,
                    (col * cell) as i32 + dx as i32,
                    (row * cell) as i32 + dy as i32,
                    &decode_tile(&bytes, config.format),
                    &pal,
                    true,
                    entry.hflip,
                    entry.vflip,
                );
            }
        }
    }
    bm
}

/// Draw one sprite at its own size, index 0 transparent. The whole block
/// flips, not each tile, which is what the hardware does.
pub fn render_sprite(vram: &[u8], cgram: &[u8], obsel: ObjSelect, entry: &OamEntry) -> Bitmap {
    let (w, h) = obsel.size_of(entry.large);
    let mut bm = Bitmap::new(w as u32, h as u32);
    let pal = resolve(PaletteRef::Cgram(cgram), 16, entry.palette, OBJ_BASE);
    let cols = w as usize / 8;
    for (i, tile) in sprite_tiles(entry, obsel).into_iter().enumerate() {
        let (c, r) = ((i % cols) as i32, (i / cols) as i32);
        let x0 = if entry.hflip {
            w as i32 - 8 - c * 8
        } else {
            c * 8
        };
        let y0 = if entry.vflip {
            h as i32 - 8 - r * 8
        } else {
            r * 8
        };
        let start = obsel.tile_word_address(tile) as usize * 2;
        let bytes: Vec<u8> = (0..32)
            .map(|k| vram.get((start + k) % VRAM_LEN).copied().unwrap_or(0))
            .collect();
        draw_tile(
            &mut bm,
            x0,
            y0,
            &decode_tile(&bytes, TileFormat::Bpp4),
            &pal,
            true,
            entry.hflip,
            entry.vflip,
        );
    }
    bm
}

/// A sheet of consecutive tiles from any bytes, `columns` across, index 0
/// drawn in colour 0. This is the tile decoder's view of raw ROM.
pub fn render_tile_sheet(
    bytes: &[u8],
    format: TileFormat,
    colours: &[[u8; 4]],
    count: u32,
    columns: u32,
) -> Bitmap {
    let columns = columns.max(1);
    let rows = count.div_ceil(columns);
    let mut bm = Bitmap::new(columns * 8, rows.max(1) * 8);
    let len = format.tile_len();
    for i in 0..count {
        let start = i as usize * len;
        if start >= bytes.len() {
            break;
        }
        let end = (start + len).min(bytes.len());
        draw_tile(
            &mut bm,
            (i % columns) as i32 * 8,
            (i / columns) as i32 * 8,
            &decode_tile(&bytes[start..end], format),
            colours,
            false,
            false,
            false,
        );
    }
    bm
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::oam::decode_oam;
    use crate::graphics::tile::encode_tile;

    fn solid(index: u8) -> [u8; 64] {
        [index; 64]
    }

    /// A 4 bpp tile whose left column is index 1 and the rest 0.
    fn left_bar() -> [u8; 64] {
        std::array::from_fn(|i| if i % 8 == 0 { 1 } else { 0 })
    }

    fn cgram_with(entries: &[(usize, u16)]) -> Vec<u8> {
        let mut c = vec![0u8; 512];
        for (i, v) in entries {
            c[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
        }
        c
    }

    #[test]
    fn a_bg_cell_draws_its_tile_with_its_palette_and_flip() {
        let mut vram = vec![0u8; VRAM_LEN];
        // Character base word $1000 (byte $2000); tile 1 is the bar.
        vram[0x2000 + 32..0x2000 + 64].copy_from_slice(&encode_tile(&left_bar(), TileFormat::Bpp4));
        // Map at word $0400 (byte $0800): cell (1, 0) = tile 1, palette 2, h-flip.
        let entry: u16 = 0x4000 | (2 << 10) | 1;
        vram[0x0800 + 2..0x0800 + 4].copy_from_slice(&entry.to_le_bytes());
        let cgram = cgram_with(&[(2 * 16 + 1, 0x001F)]);
        let cfg = BgConfig {
            format: TileFormat::Bpp4,
            char_word: 0x1000,
            map_word: 0x0400,
            size: ScreenSize::S32x32,
            tile16: false,
            palette_base: 0,
        };
        let bm = render_bg_layer(&vram, &cgram, &cfg);
        assert_eq!((bm.width, bm.height), (256, 256));
        // Flipped, the bar is the cell's rightmost column: x = 8 + 7.
        assert_eq!(bm.get(15, 3), [255, 0, 0, 255]);
        assert_eq!(bm.get(8, 3)[3], 0, "index 0 is transparent");
        assert_eq!(bm.get(0, 0)[3], 0, "tile 0 is empty");
    }

    #[test]
    fn config_comes_from_the_registers() {
        let mut ppu = PpuState::default();
        ppu.set_register(0x2105, 0x00); // mode 0
        ppu.set_register(0x2109, 0x22); // BG3SC: word $2000, 32×64
        ppu.set_register(0x210C, 0x05); // BG3 chars at $5000
        let cfg = BgConfig::from_ppu(&ppu, 3).unwrap();
        assert_eq!(cfg.format, TileFormat::Bpp2);
        assert_eq!(cfg.palette_base, 64, "Mode 0 gives BG3 colours 64–95");
        assert_eq!(cfg.map_word, 0x2000);
        assert_eq!(cfg.char_word, 0x5000);
        assert_eq!(cfg.pixel_size(), (256, 512));
        ppu.set_register(0x2105, 0x07);
        assert_eq!(BgConfig::from_ppu(&ppu, 1), None, "no Mode 7");
        ppu.set_register(0x2105, 0x01);
        assert_eq!(BgConfig::from_ppu(&ppu, 4), None, "Mode 1 has three layers");
    }

    #[test]
    fn a_large_sprite_flips_as_one_block() {
        let mut vram = vec![0u8; VRAM_LEN];
        let obsel = ObjSelect::from_register(0x00); // 8×8 / 16×16, base 0
        // Tile 0 solid index 1, tile 1 solid index 2; 16 and 17 empty.
        vram[0..32].copy_from_slice(&encode_tile(&solid(1), TileFormat::Bpp4));
        vram[32..64].copy_from_slice(&encode_tile(&solid(2), TileFormat::Bpp4));
        let cgram = cgram_with(&[(OBJ_BASE + 1, 0x001F), (OBJ_BASE + 2, 0x03E0)]);
        let mut oam = vec![0u8; 544];
        oam[3] = 0x40; // h-flip, palette 0
        oam[512] = 0b10; // sprite 0 large
        let e = decode_oam(&oam)[0];
        let bm = render_sprite(&vram, &cgram, obsel, &e);
        assert_eq!((bm.width, bm.height), (16, 16));
        // Unflipped tile 0 is top-left; flipped it is top-right.
        assert_eq!(bm.get(12, 2), [255, 0, 0, 255]);
        assert_eq!(bm.get(2, 2), [0, 255, 0, 255]);
        assert_eq!(bm.get(2, 12)[3], 0);
    }

    #[test]
    fn a_tile_sheet_wraps_at_the_column_count() {
        let bytes: Vec<u8> = [solid(1), solid(2), solid(3)]
            .iter()
            .flat_map(|t| encode_tile(t, TileFormat::Bpp2))
            .collect();
        let gray = resolve(PaletteRef::Grayscale, 4, 0, 0);
        let bm = render_tile_sheet(&bytes, TileFormat::Bpp2, &gray, 3, 2);
        assert_eq!((bm.width, bm.height), (16, 16));
        assert_eq!(bm.get(0, 0), gray[1]);
        assert_eq!(bm.get(8, 0), gray[2]);
        assert_eq!(bm.get(0, 8), gray[3]);
        assert_eq!(bm.get(8, 8)[3], 0, "past the last tile stays empty");
    }
}
