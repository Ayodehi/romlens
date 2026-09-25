//! The screen, drawn from the PPU's memories and registers (docs/22, P1),
//! with what drew each pixel.
//!
//! A recording has no framebuffer, so the frame is composed the way the PPU
//! does it, one scanline at a time: each background layer and the sprites
//! give a pixel or nothing, and the mode's priority order picks the one in
//! front. `winners` keeps, for every pixel, the layer and the tilemap cell
//! or sprite that won, which is what lets a click on the frame walk back to
//! the bytes (`provenance`).
//!
//! What it draws: every BG mode's layers at their depths, scroll, 8×8 and
//! 16×16 cells, sprites in all eight sizes with their priorities, the first
//! sprite in OAM winning among sprites, Mode 7's transform with its wrap
//! settings, the main and sub screens (`TM`, `TS`), the two windows and the
//! colour window with their logic (`TMW`, `TSW`), colour math (add or
//! subtract the sub screen or the fixed colour, halved or not, clipping the
//! main screen to black), forced blank and brightness. Each line can have
//! its own registers, for games that change them part way down the screen
//! (an IRQ split, HDMA).
//!
//! Offset-per-tile (modes 2 and 4) and mosaic are drawn as well.
//!
//! What it leaves out, and says so in [`Composed::unsupported`]: hi-res
//! (modes 5 and 6, pseudo hi-res), interlace, direct colour and Mode 7's
//! EXTBG. Sprites follow the
//! hardware's per-line limits of 32 sprites and 34 tiles.

use crate::graphics::Bitmap;
use crate::graphics::mode7;
use crate::graphics::oam::{OAM_LEN, decode_oam};
use crate::graphics::palette::{Colour, OBJ_BASE};
use crate::graphics::ppu_state::{PpuState, bg_format};
use crate::graphics::render::VRAM_LEN;
use crate::graphics::tile::{TileFormat, pixel_index};
use crate::graphics::tilemap::{ScreenSize, TilemapEntry};

pub const WIDTH: u32 = 256;

/// What drew one pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Winner {
    /// Forced blank: the screen is black.
    Blank,
    /// No layer had a pixel here: CGRAM colour 0.
    Backdrop,
    Bg(BgPixel),
    Sprite(SpritePixel),
}

/// A background layer's pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BgPixel {
    /// 1–4.
    pub layer: u8,
    /// The VRAM word holding the tilemap entry (Mode 7: the map byte's
    /// word).
    pub map_word: u16,
    pub entry: TilemapEntry,
    /// The 8×8 tile the pixel is in, after a 16×16 cell picks its quarter.
    pub tile: u16,
    /// The VRAM word the tile's data starts at.
    pub tile_word: u16,
    /// The pixel inside that tile, after flipping.
    pub x: u8,
    pub y: u8,
    /// The colour index in the tile, and the CGRAM colour it names.
    pub index: u8,
    pub colour: u8,
}

/// A sprite's pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpritePixel {
    /// The OAM entry, 0–127.
    pub sprite: u8,
    pub tile: u16,
    pub tile_word: u16,
    pub x: u8,
    pub y: u8,
    pub index: u8,
    pub colour: u8,
    pub priority: u8,
}

impl Winner {
    /// The CGRAM colour shown, where one is.
    pub fn colour(&self) -> Option<u8> {
        match self {
            Winner::Blank => None,
            Winner::Backdrop => Some(0),
            Winner::Bg(p) => Some(p.colour),
            Winner::Sprite(p) => Some(p.colour),
        }
    }
}

/// A composed frame.
#[derive(Debug, Clone)]
pub struct Composed {
    /// 256 wide; 224 lines, or 239 with overscan.
    pub bitmap: Bitmap,
    /// Row by row, one per pixel.
    pub winners: Vec<Winner>,
    /// Features the registers turn on that this does not draw, once each.
    pub unsupported: Vec<&'static str>,
}

impl Composed {
    pub fn winner(&self, x: u32, y: u32) -> Option<Winner> {
        (x < self.bitmap.width && y < self.bitmap.height)
            .then(|| self.winners[(y * self.bitmap.width + x) as usize])
    }
}

/// One thing that can draw a pixel, in a mode's priority list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// A layer, and whether this is its high-priority half.
    Bg(u8, bool),
    /// Sprites of one priority.
    Obj(u8),
}

use Source::{Bg, Obj};

/// Front to back, as the PPU orders them (fullsnes, "BG priority").
fn priority_order(mode: u8, bg3_high: bool) -> &'static [Source] {
    match mode {
        0 => &[
            Obj(3),
            Bg(1, true),
            Bg(2, true),
            Obj(2),
            Bg(1, false),
            Bg(2, false),
            Obj(1),
            Bg(3, true),
            Bg(4, true),
            Obj(0),
            Bg(3, false),
            Bg(4, false),
        ],
        1 if bg3_high => &[
            Bg(3, true),
            Obj(3),
            Bg(1, true),
            Bg(2, true),
            Obj(2),
            Bg(1, false),
            Bg(2, false),
            Obj(1),
            Obj(0),
            Bg(3, false),
        ],
        1 => &[
            Obj(3),
            Bg(1, true),
            Bg(2, true),
            Obj(2),
            Bg(1, false),
            Bg(2, false),
            Obj(1),
            Bg(3, true),
            Obj(0),
            Bg(3, false),
        ],
        7 => &[Obj(3), Obj(2), Obj(1), Bg(1, false), Obj(0)],
        6 => &[Obj(3), Bg(1, true), Obj(2), Obj(1), Bg(1, false), Obj(0)],
        _ => &[
            Obj(3),
            Bg(1, true),
            Obj(2),
            Bg(2, true),
            Obj(1),
            Bg(1, false),
            Obj(0),
            Bg(2, false),
        ],
    }
}

/// The registers one line was drawn with: one state for the whole frame,
/// or one per line.
pub enum Lines<'a> {
    Frame(&'a PpuState),
    PerLine(&'a [PpuState]),
}

impl Lines<'_> {
    fn at(&self, y: u32) -> &PpuState {
        match self {
            Lines::Frame(p) => p,
            Lines::PerLine(v) => &v[(y as usize).min(v.len().saturating_sub(1))],
        }
    }
}

/// What one line was drawn from: the registers and the three PPU memories.
pub struct LineView<'a> {
    pub ppu: &'a PpuState,
    pub vram: &'a [u8],
    pub cgram: &'a [u8],
    pub oam: &'a [u8],
}

/// The machine line by line, in order: a recording's replay
/// (`recording::lines::Replay`), where registers and memory change while
/// the frame is drawn, or fixed memory under [`Lines`].
pub trait LineSource {
    /// Line `y`'s state; called for `y` = 0, 1, 2… in turn.
    fn line(&mut self, y: u32) -> LineView<'_>;
}

struct Fixed<'a> {
    vram: &'a [u8],
    cgram: &'a [u8],
    oam: &'a [u8],
    lines: Lines<'a>,
}

impl LineSource for Fixed<'_> {
    fn line(&mut self, y: u32) -> LineView<'_> {
        LineView {
            ppu: self.lines.at(y),
            vram: self.vram,
            cgram: self.cgram,
            oam: self.oam,
        }
    }
}

fn byte(vram: &[u8], at: usize) -> u8 {
    vram.get(at % VRAM_LEN).copied().unwrap_or(0)
}

fn word(vram: &[u8], word: usize) -> u16 {
    u16::from_le_bytes([byte(vram, word * 2), byte(vram, word * 2 + 1)])
}

/// Compose the frame.
pub fn compose(vram: &[u8], cgram: &[u8], oam: &[u8], lines: Lines<'_>) -> Composed {
    compose_lines(&mut Fixed {
        vram,
        cgram,
        oam,
        lines,
    })
}

/// Compose the frame from a line-by-line source.
pub fn compose_lines(src: &mut dyn LineSource) -> Composed {
    let height = if src.line(0).ppu.register(0x2133) & 0x04 != 0 {
        239
    } else {
        224
    };
    let mut bitmap = Bitmap::new(WIDTH, height);
    let mut winners = vec![Winner::Backdrop; (WIDTH * height) as usize];
    let mut unsupported = Vec::new();
    let mut mosaic = Mosaic::new(src.line(0).ppu.register(0x2106));
    for y in 0..height {
        let LineView {
            ppu,
            vram,
            cgram,
            oam,
        } = src.line(y);
        note_unsupported(ppu, &mut unsupported);
        let inidisp = ppu.register(0x2100);
        let row = (y * WIDTH) as usize;
        if inidisp & 0x80 != 0 {
            mosaic.line(y + 1, ppu.register(0x2106));
            for x in 0..WIDTH {
                winners[row + x as usize] = Winner::Blank;
                bitmap.set(x, y, [0, 0, 0, 255]);
            }
            continue;
        }
        let sprites = decode_oam(&oam[..oam.len().min(OAM_LEN)]);
        let colours: Vec<u16> = (0..256).map(|i| Colour::at(cgram, i).0 & 0x7FFF).collect();
        let windows = Windows::new(ppu);
        let line = mosaic.line(y + 1, ppu.register(0x2106));
        let main = screen_line(
            vram,
            &sprites,
            ppu,
            y,
            line,
            ppu.register(0x212C),
            ppu.register(0x212E),
            &windows,
        );
        let sub = screen_line(
            vram,
            &sprites,
            ppu,
            y,
            line,
            ppu.register(0x212D),
            ppu.register(0x212F),
            &windows,
        );
        let brightness = u32::from(inidisp & 0x0F);
        for x in 0..WIDTH as usize {
            let w = main[x];
            winners[row + x] = w;
            let c = blend(ppu, &colours, &windows, x as u32, w, sub[x]);
            bitmap.set(x as u32, y, rgb(c, brightness));
        }
    }
    Composed {
        bitmap,
        winners,
        unsupported,
    }
}

/// The vertical mosaic's block start, as Mesen's `_mosaicScanlineCounter`
/// keeps it: restarted at the top of the frame, then every block's height.
struct Mosaic {
    counter: u32,
}

impl Mosaic {
    fn new(reg: u8) -> Self {
        let size = u32::from(reg >> 4) + 1;
        // Scanline 0 sets it to size + 1 and counts it down once.
        Mosaic {
            counter: if reg & 0x0F != 0 { size } else { 0 },
        }
    }

    /// The scanline `scanline`'s mosaic draws, then step past it.
    fn line(&mut self, scanline: u32, reg: u8) -> u32 {
        let size = u32::from(reg >> 4) + 1;
        let enabled = reg & 0x0F != 0;
        let start = if enabled && self.counter > 0 {
            scanline.saturating_sub(size.saturating_sub(self.counter))
        } else {
            scanline
        };
        if self.counter > 0 {
            self.counter -= 1;
            if enabled && self.counter == 0 {
                self.counter = size;
            }
        }
        start
    }
}

/// A BGR15 colour as RGBA at a brightness of 0–15: each 5-bit channel is
/// scaled by `brightness / 15` first, as Mesen does, then widened to 8 bits.
fn rgb(c: u16, brightness: u32) -> [u8; 4] {
    let ch = |shift: u16| {
        let v = u32::from(c >> shift & 0x1F) * brightness / 15;
        ((v << 3) | (v >> 2)) as u8
    };
    [ch(0), ch(5), ch(10), 255]
}

/// The window masks of one line: which of BG1–4, OBJ and the colour window
/// (0–5) each pixel is inside.
struct Windows {
    inside: [[bool; WIDTH as usize]; 6],
}

impl Windows {
    fn new(ppu: &PpuState) -> Self {
        let (l1, r1) = (ppu.register(0x2126), ppu.register(0x2127));
        let (l2, r2) = (ppu.register(0x2128), ppu.register(0x2129));
        // Four bits a layer: W1 invert, W1 on, W2 invert, W2 on.
        let sel = |k: usize| {
            let (reg, shift) = match k {
                0 | 1 => (0x2123, k * 4),
                2 | 3 => (0x2124, (k - 2) * 4),
                _ => (0x2125, (k - 4) * 4),
            };
            ppu.register(reg) >> shift & 0x0F
        };
        let logic = |k: usize| {
            if k < 4 {
                ppu.register(0x212A) >> (k * 2) & 3
            } else {
                ppu.register(0x212B) >> ((k - 4) * 2) & 3
            }
        };
        let mut inside = [[false; WIDTH as usize]; 6];
        for (k, row) in inside.iter_mut().enumerate() {
            let s = sel(k);
            let (inv1, on1, inv2, on2) = (s & 1 != 0, s & 2 != 0, s & 4 != 0, s & 8 != 0);
            if !on1 && !on2 {
                continue;
            }
            for (x, cell) in row.iter_mut().enumerate() {
                let x = x as u8;
                let w1 = (l1 <= x && x <= r1) != inv1;
                let w2 = (l2 <= x && x <= r2) != inv2;
                *cell = match (on1, on2) {
                    (true, false) => w1,
                    (false, true) => w2,
                    _ => match logic(k) {
                        0 => w1 || w2,
                        1 => w1 && w2,
                        2 => w1 != w2,
                        _ => w1 == w2,
                    },
                };
            }
        }
        Windows { inside }
    }

    /// Layer `k` (0–3 BG1–4, 4 OBJ) masked at `x` by a `TMW`/`TSW` value.
    fn masks(&self, k: usize, x: u32, mask: u8) -> bool {
        mask & (1 << k) != 0 && self.inside[k][x as usize]
    }

    fn colour_window(&self, x: u32) -> bool {
        self.inside[5][x as usize]
    }
}

/// The colour a main-screen pixel shows after colour math (`CGWSEL`,
/// `CGADSUB`, `COLDATA`), in BGR15.
fn blend(
    ppu: &PpuState,
    colours: &[u16],
    windows: &Windows,
    x: u32,
    main: Winner,
    sub: Winner,
) -> u16 {
    let cgwsel = ppu.register(0x2130);
    let cgadsub = ppu.register(0x2131);
    let win = windows.colour_window(x);
    let region = |mode: u8| match mode {
        0 => false,
        1 => !win,
        2 => win,
        _ => true,
    };
    let mut c = colours[main.colour().unwrap_or(0) as usize];
    // Bits 6–7: where the main screen is clipped to black.
    if region(cgwsel >> 6) {
        c = 0;
    }
    // Bits 4–5: where colour math is prevented.
    if region(cgwsel >> 4 & 3) {
        return c;
    }
    let layer_on = match main {
        Winner::Bg(p) => cgadsub & (1 << (p.layer - 1)) != 0,
        // Only sprites with palettes 4–7 take part.
        Winner::Sprite(p) => cgadsub & 0x10 != 0 && p.colour >= 128 + 64,
        _ => cgadsub & 0x20 != 0,
    };
    if !layer_on {
        return c;
    }
    let fixed = ppu.fixed_colour() & 0x7FFF;
    // Bit 1: the sub screen, where it has a pixel; the fixed colour
    // otherwise, and then halving does not apply.
    let (other, from_sub) = if cgwsel & 0x02 != 0 {
        match sub {
            Winner::Backdrop | Winner::Blank => (fixed, false),
            w => (colours[w.colour().unwrap_or(0) as usize], true),
        }
    } else {
        (fixed, true)
    };
    let subtract = cgadsub & 0x80 != 0;
    let half = cgadsub & 0x40 != 0 && from_sub;
    let mut out = 0u16;
    for shift in [0u16, 5, 10] {
        let a = (c >> shift & 0x1F) as i32;
        let b = (other >> shift & 0x1F) as i32;
        let mut v = if subtract { (a - b).max(0) } else { a + b };
        if half {
            v >>= 1;
        }
        out |= (v.min(31) as u16) << shift;
    }
    out
}

fn note_unsupported(ppu: &PpuState, out: &mut Vec<&'static str>) {
    let mut add = |on: bool, what: &'static str| {
        if on && !out.contains(&what) {
            out.push(what);
        }
    };
    let mode = ppu.bg_mode();
    add(
        mode == 5 || mode == 6 || ppu.register(0x2133) & 0x08 != 0,
        "hi-res",
    );
    add(ppu.register(0x2133) & 0x01 != 0, "interlace");
    add(
        ppu.register(0x2130) & 0x01 != 0 && matches!(mode, 3 | 4 | 7),
        "direct colour",
    );
    add(mode == 7 && ppu.register(0x2133) & 0x40 != 0, "EXTBG");
}

/// One layer's pixel at `x` on line `y`: its winner and whether its entry
/// has the priority bit.
/// Offset-per-tile (modes 2, 4 and 6): the scroll BG1 or BG2 takes for the
/// 8-pixel column `k`, from BG3's tilemap entries fetched for the column
/// before it (Mesen's `GetHvOffsetByteAddress`).
fn column_scroll(
    vram: &[u8],
    ppu: &PpuState,
    layer: u8,
    k: u32,
    hofs: u32,
    vofs: u32,
) -> (u32, u32) {
    let mode = ppu.bg_mode();
    if k == 0 || !matches!(mode, 2 | 4 | 6) || layer > 2 {
        return (hofs, vofs);
    }
    let size = ppu.screen_size(3);
    let (cols, rows) = size.cells();
    let (wide, tall) = (cols == 64, rows == 64);
    let v_shift = if ppu.tile16(3) { 4 } else { 3 };
    let h_shift = if mode == 6 { 3 } else { v_shift };
    let entry = |vertical: bool| {
        let mut addr = u32::from(ppu.tilemap_word(3));
        let base = (((k - 1) << 3) + (u32::from(ppu.scroll(3, false)) & !7)) >> h_shift;
        let mut col = base & if wide { 0x3F } else { 0x1F };
        if col >= 0x20 {
            addr += 0x400;
            col &= 0x1F;
        }
        let mut row = (u32::from(ppu.scroll(3, true)) + if vertical { 8 } else { 0 }) >> v_shift;
        row &= if tall { 0x3F } else { 0x1F };
        if row >= 0x20 {
            addr += 0x400 << u32::from(wide);
            row &= 0x1F;
        }
        word(
            vram,
            ((addr + ((col + (row << 5)) & 0x3FF)) & 0x7FFF) as usize,
        )
    };
    let enable = if layer == 1 { 0x2000 } else { 0x4000 };
    let (mut h, mut v) = (hofs, vofs);
    let horizontal = entry(false);
    if mode == 4 {
        if horizontal & enable != 0 {
            if horizontal & 0x8000 == 0 {
                h = (hofs & 7) | u32::from(horizontal & 0x3F8);
            } else {
                v = u32::from(horizontal & 0x3FF);
            }
        }
    } else {
        if horizontal & enable != 0 {
            h = (hofs & 7) | u32::from(horizontal & 0x3F8);
        }
        let vertical = entry(true);
        if vertical & enable != 0 {
            v = u32::from(vertical & 0x3FF);
        }
    }
    (h, v)
}

/// A layer's pixel at `x`, drawn for `scanline` (1 is screen row 0; mosaic
/// moves both back to the start of their block).
fn bg_pixel(
    vram: &[u8],
    ppu: &PpuState,
    layer: u8,
    format: TileFormat,
    x: u32,
    scanline: u32,
) -> Option<(BgPixel, bool)> {
    if format == TileFormat::Mode7 {
        return mode7_pixel(vram, ppu, x, scanline).map(|p| (p, false));
    }
    let mode = ppu.bg_mode();
    let size: ScreenSize = ppu.screen_size(layer);
    let tile16 = ppu.tile16(layer);
    let cell = if tile16 { 16 } else { 8 };
    let (cols, rows) = size.cells();
    let (w, h) = (cols * cell, rows * cell);
    let hofs = u32::from(ppu.scroll(layer, false) & 0x3FF);
    let vofs = u32::from(ppu.scroll(layer, true) & 0x3FF);
    let k = (x + (hofs & 7)) >> 3;
    let (hofs, vofs) = column_scroll(vram, ppu, layer, k, hofs, vofs);
    let bx = (x + hofs) % w;
    let by = (scanline + vofs) % h;
    let map = ppu.tilemap_word(layer) as usize + size.entry_index(bx / cell, by / cell);
    let entry = TilemapEntry::from_raw(word(vram, map));
    let (mut cx, mut cy) = (bx % cell, by % cell);
    if entry.hflip {
        cx = cell - 1 - cx;
    }
    if entry.vflip {
        cy = cell - 1 - cy;
    }
    let tile = (entry.tile + (cy / 8) as u16 * 16 + (cx / 8) as u16) & 0x03FF;
    let (tx, ty) = ((cx % 8) as u8, (cy % 8) as u8);
    let len = format.tile_len();
    let words = len / 2;
    let tile_word = (ppu.char_word(layer) as usize + tile as usize * words) & 0x7FFF;
    let bytes: Vec<u8> = (0..len).map(|i| byte(vram, tile_word * 2 + i)).collect();
    let index = pixel_index(&bytes, format.bpp(), tx, ty);
    if index == 0 {
        return None;
    }
    let colour = match format {
        TileFormat::Bpp2 => {
            let base = if mode == 0 { (layer - 1) * 32 } else { 0 };
            base + entry.palette * 4 + index
        }
        TileFormat::Bpp4 => entry.palette * 16 + index,
        _ => index,
    };
    Some((
        BgPixel {
            layer,
            map_word: map as u16,
            entry,
            tile,
            tile_word: tile_word as u16,
            x: tx,
            y: ty,
            index,
            colour,
        },
        entry.priority,
    ))
}

/// A 13-bit signed register value.
fn s13(v: u16) -> i32 {
    ((v as i32) << 19) >> 19
}

/// Mode 7's pixel: the screen position through the matrix into the plane
/// (anomie's formula; the matrix and centre are signed, 8.8 fixed point).
fn mode7_pixel(vram: &[u8], ppu: &PpuState, x: u32, scanline: u32) -> Option<BgPixel> {
    let m7sel = ppu.register(0x211A);
    let [a, b, c, d] = [0, 1, 2, 3].map(|i| i32::from(ppu.mode7(i)));
    let cx = s13(ppu.mode7(4) as u16);
    let cy = s13(ppu.mode7(5) as u16);
    let hofs = s13(ppu.scroll(1, false));
    let vofs = s13(ppu.scroll(1, true));
    let sx = if m7sel & 1 != 0 {
        255 - x as i32
    } else {
        x as i32
    };
    let line = scanline as i32;
    let sy = if m7sel & 2 != 0 { 255 - line } else { line };
    // The offset from the centre wraps at 10 bits, signed.
    let clip = |n: i32| if n & 0x2000 != 0 { n | !1023 } else { n & 1023 };
    let (hc, vc) = (clip(hofs - cx), clip(vofs - cy));
    // Each product is rounded down to a multiple of 64 before the sum, as
    // the hardware's multiplier does (anomie, bsnes).
    let r = |n: i32| n & !63;
    let px = (r(a * hc) + r(b * vc) + r(b * sy) + (cx << 8) + a * sx) >> 8;
    let py = (r(c * hc) + r(d * vc) + r(d * sy) + (cy << 8) + c * sx) >> 8;
    let outside = !(0..1024).contains(&px) || !(0..1024).contains(&py);
    let tile_of = |col: u32, row: u32| mode7::map_entry(vram, col, row);
    let (tile, map_word) = if outside {
        match m7sel >> 6 {
            2 => return None,
            3 => (0u8, 0u16),
            _ => {
                let (col, row) = ((px as u32 & 1023) / 8, (py as u32 & 1023) / 8);
                (
                    tile_of(col, row),
                    (mode7::map_entry_offset(col, row) / 2) as u16,
                )
            }
        }
    } else {
        let (col, row) = (px as u32 / 8, py as u32 / 8);
        (
            tile_of(col, row),
            (mode7::map_entry_offset(col, row) / 2) as u16,
        )
    };
    let (tx, ty) = ((px & 7) as u8, (py & 7) as u8);
    let index = byte(
        vram,
        (tile as usize * 64 + ty as usize * 8 + tx as usize) * 2 + 1,
    );
    if index == 0 {
        return None;
    }
    Some(BgPixel {
        layer: 1,
        map_word,
        entry: TilemapEntry::from_raw(tile as u16),
        tile: tile as u16,
        tile_word: (tile as u16) * 64,
        x: tx,
        y: ty,
        index,
        colour: index,
    })
}

/// Line `y`'s sprite pixels, as the PPU fetches them (Mesen's
/// `EvaluateNextLineSprites` and `FetchSpriteData`): the first 32 sprites
/// in range, starting at the rotated first sprite when OAM priority
/// rotation is on, then their tiles from the last back to the first, at
/// most 34 8-pixel tiles on screen. A later-fetched sprite, which is earlier
/// in OAM, is drawn over an earlier-fetched one, so the first sprite wins;
/// past 34 tiles the first sprites lose theirs.
fn sprite_line(
    vram: &[u8],
    sprites: &[crate::graphics::oam::OamEntry],
    ppu: &PpuState,
    y: u32,
) -> Vec<Option<SpritePixel>> {
    let obsel = ppu.obj_select();
    let mut out = vec![None; WIDTH as usize];
    let (lo, hi) = (ppu.register(0x2102), ppu.register(0x2103));
    let first = if hi & 0x80 != 0 {
        let internal = (u16::from(lo) | u16::from(hi & 1) << 8) << 1;
        ((internal & 0x1FC) >> 2) as usize
    } else {
        0
    };
    let mut in_range: Vec<&crate::graphics::oam::OamEntry> = Vec::with_capacity(32);
    for k in 0..128 {
        let s = &sprites[(first + k) & 0x7F];
        let (_, h) = obsel.size_of(s.large);
        if (y.wrapping_sub(u32::from(s.y)) & 0xFF) < u32::from(h) {
            if in_range.len() == 32 {
                break;
            }
            in_range.push(s);
        }
    }
    let mut tiles_left = 34;
    'sprites: for s in in_range.iter().rev() {
        let (w, _) = obsel.size_of(s.large);
        let cols = i32::from(w) / 8;
        let gap = (y.wrapping_sub(u32::from(s.y)) & 0xFF) as i32;
        // Rectangular sprites flip each half apart (undocumented, as the
        // hardware does).
        let pos = if s.vflip {
            if gap < i32::from(w) {
                i32::from(w) - 1 - gap
            } else {
                i32::from(w) * 3 - 1 - gap
            }
        } else {
            gap
        };
        let (row, ty) = (pos >> 3, (pos & 7) as u8);
        // Tiles hidden off the left edge are not fetched.
        let skip = if s.x <= -8 && s.x != -256 {
            -(i32::from(s.x) / 8)
        } else {
            0
        };
        let x0 = if s.x == -256 { 0 } else { i32::from(s.x) };
        for c in skip..cols {
            if tiles_left == 0 {
                break 'sprites;
            }
            tiles_left -= 1;
            let column = if s.hflip { cols - 1 - c } else { c };
            let n = s.tile;
            let tile = (n & 0x100)
                | ((((n >> 4) as i32 + row) & 0x0F) as u16) << 4
                | (((n as i32 & 0x0F) + column) & 0x0F) as u16;
            let tile_word = obsel.tile_word_address(tile);
            let bytes: Vec<u8> = (0..32)
                .map(|i| byte(vram, tile_word as usize * 2 + i))
                .collect();
            let draw_x = i32::from(s.x) + c * 8;
            for px in 0..8 {
                let sx = draw_x + px;
                if !(0..WIDTH as i32).contains(&sx) {
                    continue;
                }
                let tx = if s.hflip { 7 - px } else { px } as u8;
                let index = pixel_index(&bytes, 4, tx, ty);
                if index == 0 {
                    continue;
                }
                out[sx as usize] = Some(SpritePixel {
                    sprite: s.index,
                    tile,
                    tile_word,
                    x: tx,
                    y: ty,
                    index,
                    colour: (OBJ_BASE as u8).wrapping_add(s.palette * 16 + index),
                    priority: s.priority,
                });
            }
            // A tile reaching the right edge is the sprite's last.
            if x0 + (c + 1) * 8 >= WIDTH as i32 {
                break;
            }
        }
    }
    out
}

/// One screen's line: the layers `enable` turns on (`TM` or `TS`), each
/// masked where `mask` (`TMW` or `TSW`) and its window say.
#[allow(clippy::too_many_arguments)]
fn screen_line(
    vram: &[u8],
    sprites: &[crate::graphics::oam::OamEntry],
    ppu: &PpuState,
    y: u32,
    mosaic_line: u32,
    enable: u8,
    mask: u8,
    windows: &Windows,
) -> Vec<Winner> {
    let mode = ppu.bg_mode();
    let order = priority_order(mode, ppu.register(0x2105) & 0x08 != 0);
    let layers: Vec<(u8, TileFormat)> = (1..=4u8)
        .filter(|&l| enable & (1 << (l - 1)) != 0)
        .filter_map(|l| bg_format(mode, l).map(|f| (l, f)))
        .collect();
    let objs = if enable & 0x10 != 0 {
        sprite_line(vram, sprites, ppu, y)
    } else {
        vec![None; WIDTH as usize]
    };
    // Mosaic: a block's pixels are its top-left one's.
    let mosaic = ppu.register(0x2106);
    let block = u32::from(mosaic >> 4) + 1;
    (0..WIDTH)
        .map(|x| {
            let bgs: Vec<(BgPixel, bool)> = layers
                .iter()
                .filter(|&&(l, _)| !windows.masks(l as usize - 1, x, mask))
                .filter_map(|&(l, f)| {
                    let on = block > 1 && mosaic & (1 << (l - 1)) != 0;
                    let (bx, line) = if on {
                        (x - x % block, mosaic_line)
                    } else {
                        (x, y + 1)
                    };
                    bg_pixel(vram, ppu, l, f, bx, line)
                })
                .collect();
            let obj = objs[x as usize].filter(|_| !windows.masks(4, x, mask));
            for s in order {
                match *s {
                    Obj(p) => {
                        if let Some(o) = obj
                            && o.priority == p
                        {
                            return Winner::Sprite(o);
                        }
                    }
                    Bg(l, high) => {
                        if let Some((px, _)) = bgs
                            .iter()
                            .find(|(px, h)| px.layer == l && (*h == high || mode == 7))
                        {
                            return Winner::Bg(*px);
                        }
                    }
                }
            }
            Winner::Backdrop
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::tile::encode_tile;

    fn solid(i: u8) -> [u8; 64] {
        [i; 64]
    }

    fn cgram(entries: &[(usize, u16)]) -> Vec<u8> {
        let mut c = vec![0u8; 512];
        for (i, v) in entries {
            c[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
        }
        c
    }

    /// Mode 1, BG1 map at word $0000 and tiles at $1000; tile 1 is solid
    /// index 1. Sprites 8×8 at word $4000, tile 0 solid index 2.
    fn machine() -> (Vec<u8>, Vec<u8>, Vec<u8>, PpuState) {
        let mut vram = vec![0u8; VRAM_LEN];
        vram[0x2000 + 32..0x2000 + 64].copy_from_slice(&encode_tile(&solid(1), TileFormat::Bpp4));
        vram[0x8000..0x8000 + 32].copy_from_slice(&encode_tile(&solid(2), TileFormat::Bpp4));
        let mut ppu = PpuState::default();
        ppu.set_register(0x2100, 0x0F);
        ppu.set_register(0x2105, 0x01);
        ppu.set_register(0x2107, 0x00);
        ppu.set_register(0x210B, 0x01);
        ppu.set_register(0x2101, 0x02); // sprite names at word $4000
        ppu.set_register(0x212C, 0x11);
        // Line 0 shows map row (0 + 1 + vofs): scroll up one line so row 0
        // is on screen, as games do.
        ppu.set_scroll(1, true, 0x3FF);
        let mut oam = vec![0u8; OAM_LEN];
        // Every sprite off screen at y = 240, then sprite 3 at (16, 8).
        for i in 0..128 {
            oam[i * 4 + 1] = 240;
        }
        oam[12] = 16;
        oam[13] = 8;
        oam[15] = 0x20; // priority 2, palette 0
        let cg = cgram(&[(0, 0x7C00), (1, 0x001F), (OBJ_BASE + 2, 0x03E0)]);
        (vram, cg, oam, ppu)
    }

    #[test]
    fn a_cell_draws_where_its_map_entry_says() {
        let (mut vram, cg, oam, ppu) = machine();
        // Cell (4, 1) is tile 1.
        let at = (32 + 4) * 2;
        vram[at..at + 2].copy_from_slice(&1u16.to_le_bytes());
        let f = compose(&vram, &cg, &oam, Lines::Frame(&ppu));
        assert_eq!((f.bitmap.width, f.bitmap.height), (256, 224));
        let Some(Winner::Bg(p)) = f.winner(33, 9) else {
            panic!("{:?}", f.winner(33, 9))
        };
        assert_eq!((p.layer, p.tile, p.map_word, p.colour), (1, 1, 36, 1));
        assert_eq!(f.bitmap.get(33, 9), [255, 0, 0, 255]);
        // Elsewhere the backdrop, colour 0.
        assert_eq!(f.winner(100, 100), Some(Winner::Backdrop));
        assert_eq!(f.bitmap.get(100, 100), [0, 0, 255, 255]);
    }

    #[test]
    fn a_sprite_in_front_of_a_low_priority_cell() {
        let (mut vram, cg, oam, ppu) = machine();
        let at = (32 + 2) * 2;
        vram[at..at + 2].copy_from_slice(&1u16.to_le_bytes());
        let f = compose(&vram, &cg, &oam, Lines::Frame(&ppu));
        // The sprite covers (16..24, 8..16), over the cell at (16..24, 8..16).
        let Some(Winner::Sprite(s)) = f.winner(18, 10) else {
            panic!("{:?}", f.winner(18, 10))
        };
        assert_eq!((s.sprite, s.colour), (3, OBJ_BASE as u8 + 2));
        // With the cell's priority bit, BG1 high is in front of sprites of
        // priority 2.
        let hi = 0x2000u16 | 1;
        vram[at..at + 2].copy_from_slice(&hi.to_le_bytes());
        let f = compose(&vram, &cg, &oam, Lines::Frame(&ppu));
        assert!(matches!(f.winner(18, 10), Some(Winner::Bg(_))));
    }

    #[test]
    fn scroll_moves_the_map_and_forced_blank_is_black() {
        let (mut vram, cg, oam, mut ppu) = machine();
        vram[0..2].copy_from_slice(&1u16.to_le_bytes()); // cell (0, 0)
        ppu.set_scroll(1, false, 4);
        let f = compose(&vram, &cg, &oam, Lines::Frame(&ppu));
        assert!(matches!(f.winner(0, 0), Some(Winner::Bg(_))));
        assert!(matches!(f.winner(3, 0), Some(Winner::Bg(_))));
        assert_eq!(f.winner(4, 0), Some(Winner::Backdrop));
        // The map wraps: cell 0 shows again 256 pixels on.
        assert!(matches!(f.winner(255, 0), Some(Winner::Bg(_))));
        ppu.set_register(0x2100, 0x80);
        let f = compose(&vram, &cg, &oam, Lines::Frame(&ppu));
        assert_eq!(f.winner(0, 0), Some(Winner::Blank));
        assert_eq!(f.bitmap.get(0, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn each_line_can_have_its_own_registers() {
        let (mut vram, cg, oam, ppu) = machine();
        vram[0..2].copy_from_slice(&1u16.to_le_bytes());
        let mut off = ppu.clone();
        off.set_register(0x212C, 0x00);
        let mut lines = vec![ppu.clone(); 224];
        for l in lines.iter_mut().skip(4) {
            *l = off.clone();
        }
        let f = compose(&vram, &cg, &oam, Lines::PerLine(&lines));
        assert!(matches!(f.winner(0, 3), Some(Winner::Bg(_))));
        assert_eq!(f.winner(0, 5), Some(Winner::Backdrop));
    }

    #[test]
    fn what_it_does_not_draw_is_named() {
        let (vram, cg, oam, mut ppu) = machine();
        ppu.set_register(0x2130, 0x01);
        ppu.set_register(0x2105, 0x03);
        ppu.set_register(0x2133, 0x01);
        let f = compose(&vram, &cg, &oam, Lines::Frame(&ppu));
        assert_eq!(f.unsupported, ["interlace", "direct colour"]);
    }

    #[test]
    fn colour_math_adds_the_fixed_colour_to_the_backdrop() {
        let (vram, cg, oam, mut ppu) = machine();
        // Backdrop blue (0x7C00) plus fixed red 0x001F: magenta.
        ppu.set_register(0x2131, 0x20);
        ppu.set_fixed_colour(0x001F);
        let f = compose(&vram, &cg, &oam, Lines::Frame(&ppu));
        assert_eq!(f.bitmap.get(100, 100), [255, 0, 255, 255]);
        // Halved, and subtracting takes it away.
        ppu.set_register(0x2131, 0xA0);
        ppu.set_fixed_colour(0x7C00);
        let f = compose(&vram, &cg, &oam, Lines::Frame(&ppu));
        assert_eq!(f.bitmap.get(100, 100), [0, 0, 0, 255]);
    }

    #[test]
    fn a_window_hides_a_layer_on_the_main_screen() {
        let (mut vram, cg, oam, mut ppu) = machine();
        vram[0..2].copy_from_slice(&1u16.to_le_bytes()); // cell (0, 0)
        ppu.set_register(0x2126, 0);
        ppu.set_register(0x2127, 3); // window 1: x 0–3
        ppu.set_register(0x2123, 0x02); // BG1 in window 1
        ppu.set_register(0x212E, 0x01); // masked on the main screen
        let f = compose(&vram, &cg, &oam, Lines::Frame(&ppu));
        assert_eq!(f.winner(2, 0), Some(Winner::Backdrop));
        assert!(matches!(f.winner(5, 0), Some(Winner::Bg(_))));
    }
}
