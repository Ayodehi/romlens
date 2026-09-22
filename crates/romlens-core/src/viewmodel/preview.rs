//! Previews for typed ranges (checklist 2.25): a range the analyzer or the
//! user calls graphics, a palette, a tilemap or compressed data says, in the
//! inspector, what it would look like, with an "Open in …" for the full view.

use crate::analysis::AnalysisSnapshot;
use crate::graphics::Bitmap;
use crate::graphics::compress::sm_lz;
use crate::graphics::palette::{Colour, PaletteRef, resolve};
use crate::graphics::render::draw_tile;
use crate::graphics::render::render_tile_sheet;
use crate::graphics::tile::TileFormat;
use crate::graphics::tile::decode_tile;
use crate::graphics::tilemap::{ScreenSize, TilemapEntry, cell_tiles};
use crate::memory::address::FileOffset;
use crate::model::project::Project;
use crate::model::region::{DataKind, RegionKind, RegionParams};
use crate::rom::image::RomImage;

/// Tiles a preview draws at most; the full view scrolls the rest.
pub const PREVIEW_TILES: u32 = 256;

/// Which full view the "Open in …" button opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewView {
    TileDecoder,
    Palette,
    Tilemap,
}

impl PreviewView {
    pub const fn name(self) -> &'static str {
        match self {
            PreviewView::TileDecoder => "Tile Decoder",
            PreviewView::Palette => "Palette",
            PreviewView::Tilemap => "Tilemap",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The typed range, whole.
    pub start: u32,
    pub len: u32,
    pub kind: DataKind,
    /// One line for the inspector and `romlens inspect`.
    pub summary: String,
    pub bitmap: Option<Bitmap>,
    pub view: PreviewView,
    /// For compressed data: the bytes the view opens on, which are the
    /// decompressed output rather than the range itself.
    pub decompressed: Option<Vec<u8>>,
}

/// The preview for the typed range containing `offset`, if it has one.
pub fn preview_at(
    rom: &RomImage,
    snap: &AnalysisSnapshot,
    project: &Project,
    offset: u32,
) -> Option<Preview> {
    let region = snap.region_at(FileOffset(offset))?;
    let RegionKind::Data(kind) = region.kind else {
        return None;
    };
    let params = project
        .region_override_at(FileOffset(offset))
        .map(|o| o.params)
        .unwrap_or_default();
    let (start, len) = (region.start.0, region.len);
    let bytes = &rom.bytes()[start as usize..(start + len) as usize];
    build(rom, kind, start, bytes, &params)
}

fn palette_from(rom: &RomImage, params: &RegionParams, colours: usize) -> (Vec<[u8; 4]>, String) {
    match params
        .palette
        .and_then(|a| rom.file_offset_for(a).map(|o| (a, o)))
    {
        Some((a, off)) => {
            let at = off.0 as usize;
            let end = (at + colours * 2).min(rom.len());
            (
                resolve(PaletteRef::Bytes(&rom.bytes()[at..end]), colours, 0, 0),
                format!("palette at {a}"),
            )
        }
        None => (
            resolve(PaletteRef::Grayscale, colours, 0, 0),
            "grayscale".to_owned(),
        ),
    }
}

/// Build a preview for `bytes` typed `kind`. Separate from [`preview_at`] so
/// a shell can preview a selection the user has not marked yet.
pub fn build(
    rom: &RomImage,
    kind: DataKind,
    start: u32,
    bytes: &[u8],
    params: &RegionParams,
) -> Option<Preview> {
    let len = bytes.len() as u32;
    let columns = params.columns.unwrap_or(16).clamp(1, 64) as u32;
    let base = |summary: String, bitmap, view| Preview {
        start,
        len,
        kind,
        summary,
        bitmap,
        view,
        decompressed: None,
    };
    match kind {
        DataKind::Graphics { bpp } => {
            let format = TileFormat::from_bpp(bpp)?;
            let tiles = len / format.tile_len() as u32;
            let shown = tiles.min(PREVIEW_TILES);
            let (colours, note) = palette_from(rom, params, format.colours());
            Some(base(
                format!(
                    "{tiles} tiles at {}, {columns} across, {note}{}",
                    format.name(),
                    if shown < tiles {
                        format!("; the first {shown} shown")
                    } else {
                        String::new()
                    }
                ),
                Some(render_tile_sheet(bytes, format, &colours, shown, columns)),
                PreviewView::TileDecoder,
            ))
        }
        DataKind::Palette => {
            let n = (len / 2).min(256) as usize;
            let colours: Vec<Colour> = (0..n).map(|i| Colour::at(bytes, i)).collect();
            let mut distinct: Vec<u16> = colours.iter().map(|c| c.0 & 0x7FFF).collect();
            distinct.sort_unstable();
            distinct.dedup();
            let high = colours.iter().filter(|c| c.unused_bit()).count();
            // Sixteen swatches a row, one pixel each: the shell scales it.
            let rows = n.div_ceil(16).max(1) as u32;
            let mut bm = Bitmap::new(16, rows);
            for (i, c) in colours.iter().enumerate() {
                bm.set(i as u32 % 16, i as u32 / 16, c.rgba());
            }
            Some(base(
                format!(
                    "{n} colours in {rows} row{}, {} distinct{}",
                    if rows == 1 { "" } else { "s" },
                    distinct.len(),
                    if high > 0 {
                        format!(", bit 15 set in {high}: probably not a palette")
                    } else {
                        String::new()
                    }
                ),
                Some(bm),
                PreviewView::Palette,
            ))
        }
        DataKind::Tilemap => {
            let size = params.screen_size.unwrap_or(ScreenSize::S32x32);
            let entries = (len / 2) as usize;
            let mut tiles: Vec<u16> = (0..entries)
                .map(|i| TilemapEntry::at(bytes, i).tile)
                .collect();
            tiles.sort_unstable();
            tiles.dedup();
            let range = match (tiles.first(), tiles.last()) {
                (Some(a), Some(b)) => format!("tiles ${a:03X}–${b:03X}, {} distinct", tiles.len()),
                _ => "no tiles".to_owned(),
            };
            let (bitmap, note) = match params
                .tiles
                .and_then(|a| rom.file_offset_for(a).map(|o| (a, o)))
            {
                Some((a, off)) => (
                    Some(render_rom_tilemap(rom, bytes, size, off.0 as usize, params)),
                    format!("drawn with 4 bpp tiles at {a}"),
                ),
                None => (None, "set a tile address to draw it".to_owned()),
            };
            Some(base(
                format!("{entries} cells as {}, {range}; {note}", size.name()),
                bitmap,
                PreviewView::Tilemap,
            ))
        }
        DataKind::Compressed => match sm_lz::decompress(bytes) {
            Ok(d) => {
                let format = TileFormat::Bpp4;
                let tiles = (d.output.len() / format.tile_len()) as u32;
                let (colours, note) = palette_from(rom, params, 16);
                let shown = tiles.min(PREVIEW_TILES);
                let mut p = base(
                    format!(
                        "Super Metroid LZ: {} bytes from {}, shown as {tiles} 4 bpp tiles, {note}",
                        d.output.len(),
                        d.consumed
                    ),
                    Some(render_tile_sheet(
                        &d.output,
                        format,
                        &colours,
                        shown.max(1),
                        columns,
                    )),
                    PreviewView::TileDecoder,
                );
                p.decompressed = Some(d.output);
                Some(p)
            }
            Err(e) => Some(base(
                format!("not a Super Metroid LZ stream: {e}"),
                None,
                PreviewView::TileDecoder,
            )),
        },
        _ => None,
    }
}

/// A tilemap from ROM drawn with tiles from ROM, 4 bpp, index 0 transparent.
fn render_rom_tilemap(
    rom: &RomImage,
    map: &[u8],
    size: ScreenSize,
    tiles_at: usize,
    params: &RegionParams,
) -> Bitmap {
    let (cols, rows) = size.cells();
    let mut bm = Bitmap::new(cols * 8, rows * 8);
    let format = TileFormat::Bpp4;
    let (gray, _) = palette_from(rom, &RegionParams::default(), 16);
    let palette = params
        .palette
        .and_then(|a| rom.file_offset_for(a))
        .map(|o| &rom.bytes()[o.0 as usize..(o.0 as usize + 256).min(rom.len())]);
    for row in 0..rows {
        for col in 0..cols {
            let e = TilemapEntry::at(map, size.entry_index(col, row));
            let colours = match palette {
                Some(p) => resolve(PaletteRef::Cgram(p), 16, e.palette, 0),
                None => gray.clone(),
            };
            for (tile, dx, dy) in cell_tiles(&e, false) {
                let at = tiles_at + tile as usize * format.tile_len();
                let end = (at + format.tile_len()).min(rom.len());
                let t = if at < end {
                    &rom.bytes()[at..end]
                } else {
                    &[][..]
                };
                draw_tile(
                    &mut bm,
                    (col * 8) as i32 + dx as i32,
                    (row * 8) as i32 + dy as i32,
                    &decode_tile(t, format),
                    &colours,
                    true,
                    e.hflip,
                    e.vflip,
                );
            }
        }
    }
    bm
}
