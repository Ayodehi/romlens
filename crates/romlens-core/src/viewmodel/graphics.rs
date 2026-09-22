//! The graphics views as rows and text: what the shells draw and the CLI
//! prints, computed once (`07-memory-to-screen.md`, the Phase 2 views).
//!
//! Every row carries the byte range it was decoded from, relative to the
//! first byte handed in. That range is the join key that keeps one selection
//! across the hex view, the disassembly and the graphics tabs
//! (`16-phase2-plan.md` 2B.8).

use std::fmt::Write as _;

use crate::graphics::oam::{OamEntry, ObjSelect, decode_oam};
use crate::graphics::palette::Colour;
use crate::graphics::tile::{TileFormat, byte_index, decode_tile};
use crate::graphics::tilemap::{ScreenSize, TilemapEntry};

/// One tile, decoded for the teaching view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileView {
    pub format: TileFormat,
    /// The tile's bytes, zero-padded if the range was short.
    pub bytes: Vec<u8>,
    /// Colour index per pixel, row by row.
    pub indices: [u8; 64],
    /// Per plane, the eight row bytes that plane contributes. Empty for
    /// Mode 7, which has no planes.
    pub planes: Vec<[u8; 8]>,
}

pub fn tile_view(bytes: &[u8], format: TileFormat) -> TileView {
    let mut padded = vec![0u8; format.tile_len()];
    let n = bytes.len().min(padded.len());
    padded[..n].copy_from_slice(&bytes[..n]);
    let planes = (0..format.planes())
        .map(|p| std::array::from_fn(|y| padded[byte_index(p, y as u8)]))
        .collect();
    TileView {
        format,
        indices: decode_tile(&padded, format),
        bytes: padded,
        planes,
    }
}

fn digit(v: u8) -> char {
    char::from_digit(v as u32 % 16, 16)
        .unwrap()
        .to_ascii_uppercase()
}

/// Consecutive tiles as index grids, `columns` across, one hex digit per
/// pixel (two for 8 bpp), a space between tiles and a blank line between rows
/// of tiles. The CLI's `tiles --text`, and what a golden pins.
pub fn format_tile_grid(bytes: &[u8], format: TileFormat, count: u32, columns: u32) -> String {
    let columns = columns.max(1) as usize;
    let len = format.tile_len();
    let tiles: Vec<[u8; 64]> = (0..count as usize)
        .map(|i| {
            let start = (i * len).min(bytes.len());
            let end = (start + len).min(bytes.len());
            decode_tile(&bytes[start..end], format)
        })
        .collect();
    let wide = format.bpp() == 8;
    let mut out = String::new();
    for (band, chunk) in tiles.chunks(columns).enumerate() {
        if band > 0 {
            out.push('\n');
        }
        for y in 0..8 {
            let row: Vec<String> = chunk
                .iter()
                .map(|t| {
                    t[y * 8..y * 8 + 8]
                        .iter()
                        .map(|v| {
                            if wide {
                                format!("{v:02X}")
                            } else {
                                digit(*v).to_string()
                            }
                        })
                        .collect::<String>()
                })
                .collect();
            let _ = writeln!(out, "{}", row.join(" "));
        }
    }
    out
}

/// The plane bytes of one tile, as the teaching view shows them: each
/// plane's eight rows in binary, side by side.
pub fn format_planes(view: &TileView) -> String {
    let mut out = String::new();
    let heads: Vec<String> = (0..view.planes.len())
        .map(|p| format!("plane {p} "))
        .collect();
    let _ = writeln!(out, "{}", heads.join("  ").trim_end());
    for y in 0..8 {
        let row: Vec<String> = view
            .planes
            .iter()
            .map(|p| format!("{:08b}", p[y]))
            .collect();
        let _ = writeln!(out, "{}", row.join("  "));
    }
    out
}

/// One palette entry for the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEntryView {
    pub index: u16,
    pub colour: Colour,
    /// Byte offset of the entry's low byte, relative to the range start.
    pub byte: u32,
}

pub fn palette_entries(bytes: &[u8], count: u16) -> Vec<PaletteEntryView> {
    (0..count)
        .map(|i| PaletteEntryView {
            index: i,
            colour: Colour::at(bytes, i as usize),
            byte: i as u32 * 2,
        })
        .collect()
}

pub fn format_palette_text(entries: &[PaletteEntryView]) -> String {
    let mut out = String::from("index  raw    b  g  r   rgb\n");
    for e in entries {
        let c = e.colour;
        let (r, g, b) = c.rgb8();
        let _ = writeln!(
            out,
            "{:3}  ${:04X}  {:2} {:2} {:2}  #{r:02X}{g:02X}{b:02X}{}",
            e.index,
            c.0,
            c.blue5(),
            c.green5(),
            c.red5(),
            if c.unused_bit() { "  bit 15 set" } else { "" }
        );
    }
    out
}

/// Row orders for the OAM table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OamSort {
    /// Index 0 first, which is also the hardware's drawing priority among
    /// sprites of equal priority.
    Table,
    /// Top to bottom, then left to right.
    Screen,
    /// Priority 3 first, then table order.
    Priority,
}

impl OamSort {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "table" => OamSort::Table,
            "screen" => OamSort::Screen,
            "priority" => OamSort::Priority,
            _ => return None,
        })
    }
}

/// Decode and order the table.
pub fn oam_rows(bytes: &[u8], sort: OamSort) -> Vec<OamEntry> {
    let mut rows = decode_oam(bytes);
    match sort {
        OamSort::Table => {}
        OamSort::Screen => rows.sort_by_key(|e| (e.y, e.x, e.index)),
        OamSort::Priority => rows.sort_by_key(|e| (std::cmp::Reverse(e.priority), e.index)),
    }
    rows
}

/// Whether a sprite can be seen on a 224-line screen at all: games park the
/// ones they are not using at y = 224 or below, off the bottom.
pub fn on_screen(e: &OamEntry, obsel: ObjSelect) -> bool {
    let (w, h) = obsel.size_of(e.large);
    let x_visible = e.x < 256 && e.x + w as i16 > 0;
    let y = e.y as u16;
    let y_visible = y < 224 || y + h as u16 > 256;
    x_visible && y_visible
}

pub fn format_oam_text(rows: &[OamEntry], obsel: ObjSelect, only_visible: bool) -> String {
    let mut out = String::from("  #     x    y  tile  pal  pri  flip  size   table  bytes\n");
    let mut hidden = 0;
    for e in rows {
        if only_visible && !on_screen(e, obsel) {
            hidden += 1;
            continue;
        }
        let (w, h) = obsel.size_of(e.large);
        let flip = match (e.hflip, e.vflip) {
            (false, false) => "-",
            (true, false) => "h",
            (false, true) => "v",
            (true, true) => "hv",
        };
        let (low, high) = e.byte_offsets();
        let _ = writeln!(
            out,
            "{:3}  {:4}  {:3}  ${:03X}  {:3}  {:3}  {:>4}  {:>5}  {:5}  {:03X}+4 {:03X}",
            e.index,
            e.x,
            e.y,
            e.tile,
            e.palette,
            e.priority,
            flip,
            format!("{w}x{h}"),
            e.name_table(),
            low[0],
            high
        );
    }
    if hidden > 0 {
        let _ = writeln!(out, "({hidden} sprites off screen not shown)");
    }
    out
}

/// Every cell of a map in reading order, with its entry index into the
/// bytes (so ×2 is its byte offset).
pub fn tilemap_cells(bytes: &[u8], size: ScreenSize) -> Vec<(u32, u32, usize, TilemapEntry)> {
    let (cols, rows) = size.cells();
    let mut out = Vec::with_capacity(size.entries());
    for row in 0..rows {
        for col in 0..cols {
            let i = size.entry_index(col, row);
            out.push((col, row, i, TilemapEntry::at(bytes, i)));
        }
    }
    out
}

/// The map as a grid of tile numbers, one row of cells per line, with the
/// palette, priority and flips summarised underneath.
pub fn format_tilemap_text(bytes: &[u8], size: ScreenSize) -> String {
    let (cols, _) = size.cells();
    let cells = tilemap_cells(bytes, size);
    let mut out = String::new();
    for row in cells.chunks(cols as usize) {
        let line: Vec<String> = row
            .iter()
            .map(|(_, _, _, e)| format!("{:03X}", e.tile))
            .collect();
        let _ = writeln!(out, "{:2}  {}", row[0].1, line.join(" "));
    }
    let mut palettes = [0u32; 8];
    let (mut prio, mut h, mut v) = (0, 0, 0);
    for (_, _, _, e) in &cells {
        palettes[e.palette as usize] += 1;
        prio += e.priority as u32;
        h += e.hflip as u32;
        v += e.vflip as u32;
    }
    let used: Vec<String> = palettes
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(p, n)| format!("{p}×{n}"))
        .collect();
    let _ = writeln!(
        out,
        "{} cells; palettes {}; priority {prio}; h-flip {h}; v-flip {v}",
        cells.len(),
        used.join(" ")
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::tile::encode_tile;

    #[test]
    fn the_grid_prints_one_digit_per_pixel() {
        let mut tile = [0u8; 64];
        tile[0] = 0xF;
        tile[63] = 0x3;
        let bytes = [
            encode_tile(&tile, TileFormat::Bpp4),
            encode_tile(&[1; 64], TileFormat::Bpp4),
        ]
        .concat();
        let text = format_tile_grid(&bytes, TileFormat::Bpp4, 2, 2);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 8);
        assert_eq!(lines[0], "F0000000 11111111");
        assert_eq!(lines[7], "00000003 11111111");
        let stacked = format_tile_grid(&bytes, TileFormat::Bpp4, 2, 1);
        assert_eq!(stacked.lines().count(), 17, "two bands and a blank line");
    }

    #[test]
    fn plane_rows_come_from_the_interleave() {
        let mut bytes = vec![0u8; 32];
        bytes[byte_index(2, 3)] = 0xA5;
        let v = tile_view(&bytes, TileFormat::Bpp4);
        assert_eq!(v.planes.len(), 4);
        assert_eq!(v.planes[2][3], 0xA5);
        assert!(format_planes(&v).contains("10100101"));
        assert!(tile_view(&bytes, TileFormat::Mode7).planes.is_empty());
    }

    #[test]
    fn parked_sprites_are_off_screen() {
        let mut t = vec![0u8; 544];
        for i in 0..128 {
            t[i * 4 + 1] = 240;
        }
        t[1] = 100;
        let rows = oam_rows(&t, OamSort::Table);
        let obsel = ObjSelect::from_register(0);
        assert!(on_screen(&rows[0], obsel));
        assert!(!on_screen(&rows[1], obsel));
        let text = format_oam_text(&rows, obsel, true);
        assert!(text.contains("(127 sprites off screen not shown)"));
        let by_screen = oam_rows(&t, OamSort::Screen);
        assert_eq!(by_screen[0].index, 0);
    }
}
