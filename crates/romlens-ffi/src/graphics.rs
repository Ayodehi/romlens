//! Graphics and recordings across the FFI (track 2B).
//!
//! The decoders are free functions over bytes, because that is what they are
//! in the core: the same call decodes ROM bytes (`Rom::bytes`) and a
//! recording's VRAM (`RecordingSession::region`). Images cross as one RGBA
//! buffer each, which the shell wraps in a `CGImage` without copying pixel by
//! pixel.

use std::path::Path;
use std::sync::Arc;

use romlens_core::graphics::Bitmap;
use romlens_core::graphics::compress::sm_lz;
use romlens_core::graphics::oam::ObjSelect;
use romlens_core::graphics::palette::{PaletteRef, resolve};
use romlens_core::graphics::render::{BgConfig, render_bg_layer, render_sprite, render_tile_sheet};
use romlens_core::graphics::tile::{self, TileFormat as CoreFormat};
use romlens_core::graphics::tilemap::ScreenSize as CoreSize;
use romlens_core::recording::{
    CpuRegisters, MachineStateSource, RecordingError, RomrecSource, StateRegion as CoreRegion,
};
use romlens_core::viewmodel::graphics as vm;
use romlens_core::viewmodel::preview::{Preview, PreviewView as CoreView};

use crate::{Rom, RomlensError};

impl From<RecordingError> for RomlensError {
    fn from(e: RecordingError) -> Self {
        match e {
            RecordingError::Io(_) => RomlensError::Io { msg: e.to_string() },
            RecordingError::RomMismatch { .. } => RomlensError::RomMismatch { msg: e.to_string() },
            _ => RomlensError::Recording { msg: e.to_string() },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TileFormat {
    Bpp2,
    Bpp4,
    Bpp8,
    Mode7,
}

impl From<TileFormat> for CoreFormat {
    fn from(f: TileFormat) -> Self {
        match f {
            TileFormat::Bpp2 => CoreFormat::Bpp2,
            TileFormat::Bpp4 => CoreFormat::Bpp4,
            TileFormat::Bpp8 => CoreFormat::Bpp8,
            TileFormat::Mode7 => CoreFormat::Mode7,
        }
    }
}

impl From<CoreFormat> for TileFormat {
    fn from(f: CoreFormat) -> Self {
        match f {
            CoreFormat::Bpp2 => TileFormat::Bpp2,
            CoreFormat::Bpp4 => TileFormat::Bpp4,
            CoreFormat::Bpp8 => TileFormat::Bpp8,
            CoreFormat::Mode7 => TileFormat::Mode7,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ScreenSize {
    S32x32,
    S64x32,
    S32x64,
    S64x64,
}

impl From<ScreenSize> for CoreSize {
    fn from(s: ScreenSize) -> Self {
        match s {
            ScreenSize::S32x32 => CoreSize::S32x32,
            ScreenSize::S64x32 => CoreSize::S64x32,
            ScreenSize::S32x64 => CoreSize::S32x64,
            ScreenSize::S64x64 => CoreSize::S64x64,
        }
    }
}

impl From<CoreSize> for ScreenSize {
    fn from(s: CoreSize) -> Self {
        match s {
            CoreSize::S32x32 => ScreenSize::S32x32,
            CoreSize::S64x32 => ScreenSize::S64x32,
            CoreSize::S32x64 => ScreenSize::S32x64,
            CoreSize::S64x64 => ScreenSize::S64x64,
        }
    }
}

/// Where a tile view's colours come from.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PaletteSource {
    Grayscale,
    /// BGR15 colours, as many as the format needs, from the first byte.
    Colours {
        bytes: Vec<u8>,
    },
    /// A 512-byte CGRAM image and the palette row, with OBJ palettes at
    /// base 128.
    Cgram {
        cgram: Vec<u8>,
        row: u8,
        obj: bool,
    },
}

fn colours(source: &PaletteSource, format: CoreFormat) -> Vec<[u8; 4]> {
    let n = format.colours();
    match source {
        PaletteSource::Grayscale => resolve(PaletteRef::Grayscale, n, 0, 0),
        PaletteSource::Colours { bytes } => resolve(PaletteRef::Bytes(bytes), n, 0, 0),
        PaletteSource::Cgram { cgram, row, obj } => resolve(
            PaletteRef::Cgram(cgram),
            n,
            *row,
            if *obj {
                romlens_core::graphics::palette::OBJ_BASE
            } else {
                0
            },
        ),
    }
}

/// An RGBA image, eight bits a channel, rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BitmapInfo {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl From<Bitmap> for BitmapInfo {
    fn from(b: Bitmap) -> Self {
        BitmapInfo {
            width: b.width,
            height: b.height,
            rgba: b.rgba,
        }
    }
}

/// One tile, decoded for the teaching view.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TileInfo {
    pub format: TileFormat,
    /// The tile's bytes, zero-padded if the range was short.
    pub bytes: Vec<u8>,
    /// 64 colour indices, row by row.
    pub indices: Vec<u8>,
    /// Each plane's eight row bytes, plane after plane; empty for Mode 7.
    pub planes: Vec<u8>,
}

/// Decode one tile.
#[uniffi::export]
pub fn decode_tile(bytes: Vec<u8>, format: TileFormat) -> TileInfo {
    let v = vm::tile_view(&bytes, format.into());
    TileInfo {
        format,
        bytes: v.bytes,
        indices: v.indices.to_vec(),
        planes: v.planes.iter().flatten().copied().collect(),
    }
}

/// Bytes in one tile of `format`.
#[uniffi::export]
pub fn tile_byte_len(format: TileFormat) -> u32 {
    CoreFormat::from(format).tile_len() as u32
}

/// Where every bit of every pixel comes from, fetched once per format so
/// hovering a pixel costs no FFI call: for pixel (x, y) and plane p, entry
/// `(y * 8 + x) * planes + p` holds `byte << 3 | bit`, with the byte relative
/// to the tile's start and bit 7 the most significant. `planes` is the
/// format's bits per pixel.
#[uniffi::export]
pub fn tile_bit_sources(format: TileFormat) -> Vec<u16> {
    let f: CoreFormat = format.into();
    let mut out = Vec::with_capacity(64 * f.bpp() as usize);
    for y in 0..8u8 {
        for x in 0..8u8 {
            for p in 0..f.bpp() {
                let s = tile::tile_bit_source(f, x, y, p).expect("in range");
                out.push((s.byte as u16) << 3 | s.bit as u16);
            }
        }
    }
    out
}

/// Consecutive tiles as a sheet, `columns` across.
#[uniffi::export]
pub fn tile_sheet(
    bytes: Vec<u8>,
    format: TileFormat,
    count: u32,
    columns: u32,
    palette: PaletteSource,
) -> BitmapInfo {
    let f: CoreFormat = format.into();
    render_tile_sheet(&bytes, f, &colours(&palette, f), count, columns).into()
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PaletteEntryInfo {
    pub index: u16,
    pub raw: u16,
    pub red5: u8,
    pub green5: u8,
    pub blue5: u8,
    /// `0xRRGGBB` after the `(c << 3) | (c >> 2)` expansion.
    pub rgb: u32,
    pub unused_bit: bool,
    /// Offset of the entry's low byte from the first byte given.
    pub byte_offset: u32,
}

#[uniffi::export]
pub fn palette_entries(bytes: Vec<u8>, count: u16) -> Vec<PaletteEntryInfo> {
    vm::palette_entries(&bytes, count.min(256))
        .into_iter()
        .map(|e| {
            let (r, g, b) = e.colour.rgb8();
            PaletteEntryInfo {
                index: e.index,
                raw: e.colour.0,
                red5: e.colour.red5(),
                green5: e.colour.green5(),
                blue5: e.colour.blue5(),
                rgb: (r as u32) << 16 | (g as u32) << 8 | b as u32,
                unused_bit: e.colour.unused_bit(),
                byte_offset: e.byte,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum OamSort {
    Table,
    Screen,
    Priority,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OamEntryInfo {
    pub index: u8,
    pub x: i16,
    pub y: u8,
    pub tile: u16,
    pub palette: u8,
    pub priority: u8,
    pub hflip: bool,
    pub vflip: bool,
    pub large: bool,
    pub width: u8,
    pub height: u8,
    pub name_table: u8,
    /// VRAM word address of the first tile.
    pub tile_word: u16,
    pub on_screen: bool,
    /// Offset of the four low-table bytes, and of the high-table byte whose
    /// bits `2 * (index % 4)` and up belong to this sprite.
    pub low_offset: u32,
    pub high_offset: u32,
}

#[uniffi::export]
pub fn oam_entries(bytes: Vec<u8>, obsel: u8, sort: OamSort) -> Vec<OamEntryInfo> {
    let o = ObjSelect::from_register(obsel);
    let sort = match sort {
        OamSort::Table => vm::OamSort::Table,
        OamSort::Screen => vm::OamSort::Screen,
        OamSort::Priority => vm::OamSort::Priority,
    };
    vm::oam_rows(&bytes, sort)
        .into_iter()
        .map(|e| {
            let (w, h) = o.size_of(e.large);
            let (low, high) = e.byte_offsets();
            OamEntryInfo {
                index: e.index,
                x: e.x,
                y: e.y,
                tile: e.tile,
                palette: e.palette,
                priority: e.priority,
                hflip: e.hflip,
                vflip: e.vflip,
                large: e.large,
                width: w,
                height: h,
                name_table: e.name_table(),
                tile_word: o.tile_word_address(e.tile),
                on_screen: vm::on_screen(&e, o),
                low_offset: low[0] as u32,
                high_offset: high as u32,
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TilemapCellInfo {
    pub col: u32,
    pub row: u32,
    pub raw: u16,
    pub tile: u16,
    pub palette: u8,
    pub priority: bool,
    pub hflip: bool,
    pub vflip: bool,
    /// Offset of the entry's low byte from the first byte given.
    pub byte_offset: u32,
}

#[uniffi::export]
pub fn tilemap_cells(bytes: Vec<u8>, size: ScreenSize) -> Vec<TilemapCellInfo> {
    vm::tilemap_cells(&bytes, size.into())
        .into_iter()
        .map(|(col, row, i, e)| TilemapCellInfo {
            col,
            row,
            raw: e.raw,
            tile: e.tile,
            palette: e.palette,
            priority: e.priority,
            hflip: e.hflip,
            vflip: e.vflip,
            byte_offset: i as u32 * 2,
        })
        .collect()
}

/// The Mode 7 map's 128×128 cells from a whole VRAM: each entry is the low
/// byte of a word, so `raw` and `tile` are the same 8-bit number, and there is
/// no palette, priority or flip. `byte_offset` is the entry's offset in VRAM.
#[uniffi::export]
pub fn mode7_cells(vram: Vec<u8>) -> Vec<TilemapCellInfo> {
    use romlens_core::graphics::mode7::{MAP_CELLS, map_entry, map_entry_offset};
    (0..MAP_CELLS)
        .flat_map(|row| (0..MAP_CELLS).map(move |col| (col, row)))
        .map(|(col, row)| {
            let tile = map_entry(&vram, col, row);
            TilemapCellInfo {
                col,
                row,
                raw: tile as u16,
                tile: tile as u16,
                palette: 0,
                priority: false,
                hflip: false,
                vflip: false,
                byte_offset: map_entry_offset(col, row) as u32,
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DecompressInfo {
    pub output: Vec<u8>,
    pub consumed: u32,
    pub summary: String,
}

/// Decompress Super Metroid's format from the first byte.
#[uniffi::export]
pub fn decompress_sm(bytes: Vec<u8>) -> Result<DecompressInfo, RomlensError> {
    let d =
        sm_lz::decompress(&bytes).map_err(|e| RomlensError::Recording { msg: e.to_string() })?;
    Ok(DecompressInfo {
        summary: format!(
            "{} bytes from {} ({:.2}:1)",
            d.output.len(),
            d.consumed,
            d.output.len() as f64 / d.consumed.max(1) as f64
        ),
        consumed: d.consumed as u32,
        output: d.output,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PreviewView {
    TileDecoder,
    Palette,
    Tilemap,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PreviewInfo {
    pub start: u32,
    pub len: u32,
    /// The data kind's name: `graphics`, `palette`, `tilemap`, `compressed`.
    pub kind: String,
    pub summary: String,
    pub bitmap: Option<BitmapInfo>,
    pub view: PreviewView,
    /// For compressed data, what the view opens on instead of the range.
    pub decompressed: Option<Vec<u8>>,
    /// For graphics, the format the range is typed as.
    pub format: Option<TileFormat>,
}

impl From<Preview> for PreviewInfo {
    fn from(p: Preview) -> Self {
        PreviewInfo {
            start: p.start,
            len: p.len,
            kind: p.kind.name().to_owned(),
            summary: p.summary,
            bitmap: p.bitmap.map(Into::into),
            view: match p.view {
                CoreView::TileDecoder => PreviewView::TileDecoder,
                CoreView::Palette => PreviewView::Palette,
                CoreView::Tilemap => PreviewView::Tilemap,
            },
            decompressed: p.decompressed,
            format: match p.kind {
                romlens_core::model::DataKind::Graphics { bpp } => {
                    CoreFormat::from_bpp(bpp).map(Into::into)
                }
                _ => None,
            },
        }
    }
}

// ---- recordings -----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum StateRegion {
    Cpu,
    Ppu,
    Io,
    Wram,
    Vram,
    Cgram,
    Oam,
    Timing,
}

impl From<StateRegion> for CoreRegion {
    fn from(r: StateRegion) -> Self {
        match r {
            StateRegion::Cpu => CoreRegion::CpuRegisters,
            StateRegion::Ppu => CoreRegion::PpuState,
            StateRegion::Io => CoreRegion::IoState,
            StateRegion::Wram => CoreRegion::Wram,
            StateRegion::Vram => CoreRegion::Vram,
            StateRegion::Cgram => CoreRegion::Cgram,
            StateRegion::Oam => CoreRegion::Oam,
            StateRegion::Timing => CoreRegion::Timing,
        }
    }
}

impl From<CoreRegion> for StateRegion {
    fn from(r: CoreRegion) -> Self {
        match r {
            CoreRegion::CpuRegisters => StateRegion::Cpu,
            CoreRegion::PpuState => StateRegion::Ppu,
            CoreRegion::IoState => StateRegion::Io,
            CoreRegion::Wram => StateRegion::Wram,
            CoreRegion::Vram => StateRegion::Vram,
            CoreRegion::Cgram => StateRegion::Cgram,
            CoreRegion::Oam => StateRegion::Oam,
            CoreRegion::Timing => StateRegion::Timing,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecordingInfo {
    pub frame_count: u64,
    pub keyframe_interval: u16,
    pub producer: String,
    pub producer_version: String,
    pub rom_sha256: String,
    pub regions: Vec<StateRegion>,
    pub file_len: u64,
    /// The index was rebuilt by scanning a file with no footer.
    pub recovered: bool,
    /// The oldest frame still readable: 0 for a file; a live session keeps
    /// only its latest frames.
    pub first_frame: u64,
    /// Frames are still arriving from a live session.
    pub live: bool,
}

/// The registers the graphics views need at one frame, decoded.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PpuSummary {
    pub bg_mode: u8,
    pub obsel: u8,
    pub main_screen: u8,
    /// `$2100`: bit 7 forced blank, bits 0-3 brightness.
    pub inidisp: u8,
    /// Per BG 1–4: the format in this mode (absent where the mode has no
    /// such layer), map and character word addresses, size, 16×16 cells.
    pub layers: Vec<BgLayerInfo>,
    pub pc: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BgLayerInfo {
    pub bg: u8,
    pub format: Option<TileFormat>,
    pub map_word: u16,
    pub char_word: u16,
    pub size: ScreenSize,
    pub tile16: bool,
    pub hscroll: u16,
    pub vscroll: u16,
}

/// An open `.romrec`. The shell keeps one per attached recording.
#[derive(uniffi::Object)]
pub struct RecordingSession {
    source: Source,
    /// Where it came from, to validate the file itself and keep its index.
    origin: Origin,
    index: std::sync::Mutex<Option<romlens_core::recording::change_index::ChangeIndex>>,
    /// The last frame composed, so the pointer's pixel is a lookup.
    composed: std::sync::Mutex<Option<(u64, Arc<romlens_core::graphics::compose::Composed>)>>,
}

/// A frame drawn from the PPU state (docs/22, P2).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FrameImageInfo {
    pub image: BitmapInfo,
    pub bg_mode: u8,
    /// Drawn line by line from the recording's register writes, rather than
    /// from the registers as the frame ended.
    pub per_line: bool,
    /// Features the registers turn on that the drawing leaves out.
    pub unsupported: Vec<String>,
}

/// The screen's state on the lines a frame drew, from the recording's
/// register writes: a game usually turns the screen off in vertical blank
/// for its uploads and on again before the first line, so the registers as
/// the frame ended say "off" of a frame drawn in full.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScreenLinesInfo {
    pub lines: u32,
    /// Lines drawn in forced blank.
    pub blank: u32,
    /// The lowest and highest brightness on the lines drawn.
    pub brightness_min: u8,
    pub brightness_max: u8,
}

/// A stretch of lines drawn in one BG mode, and the order its layers go in
/// front to back.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ModeSpanInfo {
    pub first_line: u32,
    pub last_line: u32,
    pub mode: u8,
    pub order: Vec<String>,
}

/// A layer some line of the frame draws: 1–4 a background, 5 the sprites,
/// and in words what it is and on which lines.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FrameLayerInfo {
    pub layer: u8,
    pub detail: String,
}

/// The layers and modes of a frame, line by line where the recording has
/// the writes: a game can change mode part way down the screen (Final
/// Fantasy III's intro is Mode 1 above a Mode 7 ground).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FrameLayersInfo {
    pub spans: Vec<ModeSpanInfo>,
    pub layers: Vec<FrameLayerInfo>,
}

fn format_words(f: romlens_core::graphics::tile::TileFormat) -> &'static str {
    use romlens_core::graphics::tile::TileFormat::*;
    match f {
        Bpp2 => "2 bpp",
        Bpp4 => "4 bpp",
        Bpp8 => "8 bpp",
        Mode7 => "Mode 7",
    }
}

/// What drew one pixel of a composed frame.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PixelWinnerInfo {
    /// The screen was off (forced blank).
    Blank,
    /// No layer drew here: CGRAM colour 0.
    Backdrop,
    Background {
        /// 1–4.
        layer: u8,
        /// The VRAM word holding the tilemap entry.
        map_word: u16,
        /// The entry: `vhopppcc cccccccc`.
        entry: u16,
        /// The 8×8 tile drawn, and the VRAM word its data starts at.
        tile: u16,
        tile_word: u16,
        /// The pixel inside the tile, after flipping.
        x: u8,
        y: u8,
        index: u8,
        colour: u8,
    },
    Sprite {
        /// The OAM entry, 0–127.
        sprite: u8,
        tile: u16,
        tile_word: u16,
        x: u8,
        y: u8,
        index: u8,
        colour: u8,
        priority: u8,
    },
}

impl From<romlens_core::graphics::compose::Winner> for PixelWinnerInfo {
    fn from(w: romlens_core::graphics::compose::Winner) -> Self {
        use romlens_core::graphics::compose::Winner;
        match w {
            Winner::Blank => PixelWinnerInfo::Blank,
            Winner::Backdrop => PixelWinnerInfo::Backdrop,
            Winner::Bg(p) => PixelWinnerInfo::Background {
                layer: p.layer,
                map_word: p.map_word,
                entry: p.entry.raw,
                tile: p.tile,
                tile_word: p.tile_word,
                x: p.x,
                y: p.y,
                index: p.index,
                colour: p.colour,
            },
            Winner::Sprite(p) => PixelWinnerInfo::Sprite {
                sprite: p.sprite,
                tile: p.tile,
                tile_word: p.tile_word,
                x: p.x,
                y: p.y,
                index: p.index,
                colour: p.colour,
                priority: p.priority,
            },
        }
    }
}

impl RecordingSession {
    /// `frame` composed with `options`: line by line where the recording
    /// has the writes, from the frame's end state otherwise.
    fn compose(
        &self,
        frame: u64,
        options: romlens_core::graphics::compose::ComposeOptions,
    ) -> Result<(romlens_core::graphics::compose::Composed, u8, bool), RomlensError> {
        use romlens_core::graphics::compose::{Lines, compose_lines_with, compose_with};
        let src = self.source.dynamic();
        let state = src.state_at(frame)?;
        let missing = |r: &'static str| RomlensError::Recording {
            msg: format!("the recording has no {r}"),
        };
        let ppu = state.ppu().ok_or_else(|| missing("PPU registers"))?;
        if let Some(mut replay) = romlens_core::recording::lines::frame_replay(src, frame)? {
            return Ok((
                compose_lines_with(&mut replay, options),
                ppu.bg_mode(),
                true,
            ));
        }
        let f = compose_with(
            state.vram().ok_or_else(|| missing("VRAM"))?,
            state.cgram().ok_or_else(|| missing("CGRAM"))?,
            state.oam().ok_or_else(|| missing("OAM"))?,
            Lines::Frame(&ppu),
            options,
        );
        Ok((f, ppu.bg_mode(), false))
    }
}

enum Origin {
    Path(String),
    Bytes(Vec<u8>),
    Live,
}

/// A recording file, or the frames of a live session as they arrive.
enum Source {
    File(Box<RomrecSource>),
    Live(Arc<romlens_core::recording::live::LiveSource>),
}

impl Source {
    fn dynamic(&self) -> &dyn MachineStateSource {
        match self {
            Source::File(f) => &**f,
            Source::Live(l) => &**l,
        }
    }

    fn file(&self) -> Option<&RomrecSource> {
        match self {
            Source::File(f) => Some(f),
            Source::Live(_) => None,
        }
    }
}

impl RecordingSession {
    /// The session a live listener feeds.
    pub(crate) fn live(source: Arc<romlens_core::recording::live::LiveSource>) -> Arc<Self> {
        Arc::new(RecordingSession {
            source: Source::Live(source),
            origin: Origin::Live,
            index: Default::default(),
            composed: Default::default(),
        })
    }
}

/// The validator's verdict, for the open path to show verbatim.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ValidationSummary {
    pub errors: u32,
    pub warnings: u32,
    /// One line per diagnostic: code, severity, frame, message.
    pub lines: Vec<String>,
}

/// When a byte range last changed at or before a frame, and when it next
/// changes after it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChangeHistory {
    /// False for WRAM kept in keyframes only, which cannot be answered per
    /// frame.
    pub indexed: bool,
    pub last: Option<u64>,
    pub next: Option<u64>,
}

/// What a project keeps to refer to a recording.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecordingRefInfo {
    pub path: String,
    pub frames: u64,
    pub producer: String,
    pub fingerprint: String,
}

#[uniffi::export]
impl RecordingSession {
    /// Open a finished recording, or with `recover` one that has no footer.
    #[uniffi::constructor]
    pub fn open(path: String, recover: bool) -> Result<Arc<Self>, RomlensError> {
        let p = Path::new(&path);
        let source = if recover {
            RomrecSource::open_recovering(p)?
        } else {
            RomrecSource::open(p)?
        };
        Ok(Arc::new(RecordingSession {
            source: Source::File(Box::new(source)),
            origin: Origin::Path(path),
            index: Default::default(),
            composed: Default::default(),
        }))
    }

    #[uniffi::constructor]
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Arc<Self>, RomlensError> {
        Ok(Arc::new(RecordingSession {
            source: Source::File(Box::new(RomrecSource::from_bytes(bytes.clone(), false)?)),
            origin: Origin::Bytes(bytes),
            index: Default::default(),
            composed: Default::default(),
        }))
    }

    /// Everything wrong with the file, sampling `sample` frames.
    pub fn validate(&self, sample: u32) -> Result<ValidationSummary, RomlensError> {
        use romlens_core::recording::validate::{Severity, ValidateOptions, validate};
        // A live session is decoded as it arrives; there is no file to check.
        let Some(source) = self.source.file() else {
            return Ok(ValidationSummary {
                errors: 0,
                warnings: 0,
                lines: Vec::new(),
            });
        };
        let file: Box<dyn romlens_core::recording::reader::ReadSeek> = match &self.origin {
            Origin::Path(p) => Box::new(std::io::BufReader::new(
                std::fs::File::open(p).map_err(|e| RomlensError::Io { msg: e.to_string() })?,
            )),
            Origin::Bytes(b) => Box::new(std::io::Cursor::new(b.clone())),
            Origin::Live => unreachable!("a live session has no file"),
        };
        let report = validate(
            file,
            ValidateOptions {
                rom_sha256: None,
                sample,
                recover: source.recovered(),
            },
        );
        Ok(ValidationSummary {
            errors: report.errors() as u32,
            warnings: report.warnings() as u32,
            lines: report
                .diagnostics
                .iter()
                .map(|d| {
                    let level = match d.severity {
                        Severity::Error => "error",
                        Severity::Warning => "warning",
                    };
                    let at = d.frame.map(|f| format!(" (frame {f})")).unwrap_or_default();
                    format!("{} {level}{at}: {}", d.code, d.message)
                })
                .collect(),
        })
    }

    /// When `[offset, offset + len)` of `region` last changed at or before
    /// `frame`, and next changes after it. Builds the index on first use,
    /// saved beside a recording opened from a file.
    pub fn history(
        &self,
        region: StateRegion,
        offset: u32,
        len: u32,
        frame: u64,
    ) -> Result<ChangeHistory, RomlensError> {
        use romlens_core::recording::change_index::{ChangeIndex, load_or_build};
        // Not indexed live: the frames it would index are still arriving.
        let Some(source) = self.source.file() else {
            return Ok(ChangeHistory {
                indexed: false,
                last: None,
                next: None,
            });
        };
        let mut slot = self.index.lock().unwrap();
        if slot.is_none() {
            *slot = Some(match &self.origin {
                Origin::Path(p) => load_or_build(Path::new(p), source, false)?.0,
                Origin::Bytes(_) | Origin::Live => ChangeIndex::build(source)?,
            });
        }
        let index = slot.as_ref().expect("just built");
        let region = region.into();
        if !index.covers(region) {
            return Ok(ChangeHistory {
                indexed: false,
                last: None,
                next: None,
            });
        }
        Ok(ChangeHistory {
            indexed: true,
            last: index.when(source, region, offset, len, frame, true)?,
            next: index.when(source, region, offset, len, frame, false)?,
        })
    }

    /// What a project stores to refer to this recording; `None` for one
    /// opened from bytes, which has no path to refer to.
    pub fn reference(&self) -> Option<RecordingRefInfo> {
        let (Origin::Path(path), Some(source)) = (&self.origin, self.source.file()) else {
            return None;
        };
        let r = romlens_core::recording::change_index::reference(path, source);
        Some(RecordingRefInfo {
            path: r.path,
            frames: r.frames,
            producer: r.producer,
            fingerprint: r.fingerprint,
        })
    }

    pub fn info(&self) -> RecordingInfo {
        let s = self.source.dynamic();
        let common = RecordingInfo {
            frame_count: s.frame_count().unwrap_or(0),
            keyframe_interval: 0,
            producer: s.identity().producer.clone(),
            producer_version: s.identity().producer_version.clone(),
            rom_sha256: s.identity().sha256_hex(),
            regions: s.regions().into_iter().map(Into::into).collect(),
            file_len: 0,
            recovered: false,
            first_frame: 0,
            live: false,
        };
        match &self.source {
            Source::File(f) => RecordingInfo {
                keyframe_interval: f.header().keyframe_interval,
                file_len: f.file_len(),
                recovered: f.recovered(),
                ..common
            },
            Source::Live(l) => RecordingInfo {
                first_frame: l.first_frame(),
                live: true,
                ..common
            },
        }
    }

    /// Refuse a recording of another ROM with the core's message.
    pub fn check_rom(&self, rom: Arc<Rom>) -> Result<(), RomlensError> {
        match &self.source {
            Source::File(f) => Ok(f.check_rom(rom.image.sha256())?),
            // The listener refused any stream of another ROM on its header.
            Source::Live(_) => Ok(()),
        }
    }

    pub fn region(&self, frame: u64, region: StateRegion) -> Result<Vec<u8>, RomlensError> {
        Ok(self.source.dynamic().region_at(frame, region.into())?)
    }

    /// Byte ranges of `region` that may have changed from `from` to `to`, as
    /// flat `(offset, len)` pairs.
    pub fn changes(
        &self,
        from: u64,
        to: u64,
        region: StateRegion,
    ) -> Result<Vec<u32>, RomlensError> {
        Ok(self
            .source
            .dynamic()
            .changes(from, to, region.into())?
            .into_iter()
            .flat_map(|r| [r.offset, r.len])
            .collect())
    }

    pub fn ppu_summary(&self, frame: u64) -> Result<PpuSummary, RomlensError> {
        let state = self.source.dynamic().state_at(frame)?;
        let ppu = state.ppu().ok_or(RecordingError::MissingRegion("ppu"))?;
        let cpu = state
            .region(CoreRegion::CpuRegisters)
            .map(CpuRegisters::decode)
            .unwrap_or_default();
        Ok(PpuSummary {
            bg_mode: ppu.bg_mode(),
            obsel: ppu.register(0x2101),
            inidisp: ppu.register(0x2100),
            main_screen: ppu.register(0x212C),
            layers: (1..=4u8)
                .map(|bg| BgLayerInfo {
                    bg,
                    format: romlens_core::graphics::ppu_state::bg_format(ppu.bg_mode(), bg)
                        .map(Into::into),
                    map_word: ppu.tilemap_word(bg),
                    char_word: ppu.char_word(bg),
                    size: ppu.screen_size(bg).into(),
                    tile16: ppu.tile16(bg),
                    hscroll: ppu.scroll(bg, false),
                    vscroll: ppu.scroll(bg, true),
                })
                .collect(),
            pc: (cpu.pb as u32) << 16 | cpu.pc as u32,
        })
    }

    /// One BG layer's whole map at `frame`.
    pub fn render_bg(&self, frame: u64, bg: u8) -> Result<BitmapInfo, RomlensError> {
        let state = self.source.dynamic().state_at(frame)?;
        let missing = |r: &'static str| RomlensError::Recording {
            msg: format!("the recording has no {r}"),
        };
        let ppu = state.ppu().ok_or_else(|| missing("PPU registers"))?;
        let vram = state.vram().ok_or_else(|| missing("VRAM"))?;
        let cgram = state.cgram().ok_or_else(|| missing("CGRAM"))?;
        if ppu.bg_mode() == 7 && bg == 1 {
            return Ok(romlens_core::graphics::mode7::render_plane(vram, cgram).into());
        }
        let cfg = BgConfig::from_ppu(&ppu, bg).ok_or_else(|| RomlensError::Recording {
            msg: format!("mode {} has no BG{bg}", ppu.bg_mode()),
        })?;
        Ok(render_bg_layer(vram, cgram, &cfg).into())
    }

    /// One sprite at its own size.
    pub fn render_sprite(&self, frame: u64, index: u8) -> Result<BitmapInfo, RomlensError> {
        let state = self.source.dynamic().state_at(frame)?;
        let missing = |r: &'static str| RomlensError::Recording {
            msg: format!("the recording has no {r}"),
        };
        let ppu = state.ppu().ok_or_else(|| missing("PPU registers"))?;
        let oam = state.oam().ok_or_else(|| missing("OAM"))?;
        let e = romlens_core::graphics::oam::decode_oam(oam)[index.min(127) as usize];
        Ok(render_sprite(
            state.vram().ok_or_else(|| missing("VRAM"))?,
            state.cgram().ok_or_else(|| missing("CGRAM"))?,
            ppu.obj_select(),
            &e,
        )
        .into())
    }
}

#[uniffi::export]
impl RecordingSession {
    /// The screen at `frame`, drawn from the PPU state (docs/22, P2). The
    /// result is kept, so asking what drew a pixel of it is a lookup.
    pub fn render_frame(&self, frame: u64) -> Result<FrameImageInfo, RomlensError> {
        let (f, bg_mode, per_line) = self.compose(frame, Default::default())?;
        let f = Arc::new(f);
        *self.composed.lock().unwrap() = Some((frame, f.clone()));
        Ok(FrameImageInfo {
            image: f.bitmap.clone().into(),
            bg_mode,
            per_line,
            unsupported: f.unsupported.iter().map(|s| s.to_string()).collect(),
        })
    }

    /// What drew pixel (`x`, `y`) of `frame`; `None` off the screen.
    pub fn frame_pixel(
        &self,
        frame: u64,
        x: u32,
        y: u32,
    ) -> Result<Option<PixelWinnerInfo>, RomlensError> {
        let cached = self
            .composed
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(f, _)| *f == frame)
            .map(|(_, c)| c.clone());
        let c = match cached {
            Some(c) => c,
            None => {
                let (f, ..) = self.compose(frame, Default::default())?;
                let f = Arc::new(f);
                *self.composed.lock().unwrap() = Some((frame, f.clone()));
                f
            }
        };
        Ok(c.winner(x, y).map(Into::into))
    }

    /// The layers some line of `frame` draws, and its modes line by line.
    pub fn frame_layers(&self, frame: u64) -> Result<FrameLayersInfo, RomlensError> {
        use romlens_core::graphics::ppu_state::bg_format;
        let src = self.source.dynamic();
        let states = match romlens_core::recording::lines::frame_line_states(src, frame, 224)? {
            Some(s) => s,
            None => {
                let ppu = src
                    .state_at(frame)?
                    .ppu()
                    .ok_or(RecordingError::MissingRegion("ppu"))?;
                vec![ppu; 224]
            }
        };
        let mut spans: Vec<ModeSpanInfo> = Vec::new();
        for (y, p) in states.iter().enumerate() {
            let (mode, bg3) = (p.bg_mode(), p.register(0x2105) & 0x08 != 0);
            let order = romlens_core::graphics::compose::priority_names(mode, bg3);
            match spans.last_mut() {
                Some(s) if s.mode == mode && s.order == order => s.last_line = y as u32,
                _ => spans.push(ModeSpanInfo {
                    first_line: y as u32,
                    last_line: y as u32,
                    mode,
                    order,
                }),
            }
        }
        let whole = spans.len() == 1;
        let mut layers = Vec::new();
        for bg in 1..=4u8 {
            // The format it has in each span, where it has one.
            let parts: Vec<(String, &ModeSpanInfo)> = spans
                .iter()
                .filter_map(|s| bg_format(s.mode, bg).map(|f| (format_words(f).to_owned(), s)))
                .collect();
            if parts.is_empty() {
                continue;
            }
            let detail = if whole {
                parts[0].0.clone()
            } else {
                parts
                    .iter()
                    .map(|(f, s)| format!("{f} on lines {}–{}", s.first_line, s.last_line))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let shown = states.iter().any(|p| {
                bg_format(p.bg_mode(), bg).is_some() && p.register(0x212C) & (1 << (bg - 1)) != 0
            });
            let detail = if shown {
                detail
            } else {
                format!("{detail}, off on the main screen")
            };
            layers.push(FrameLayerInfo { layer: bg, detail });
        }
        let sprites = states.iter().any(|p| p.register(0x212C) & 0x10 != 0);
        layers.push(FrameLayerInfo {
            layer: 5,
            detail: if sprites {
                "4 bpp"
            } else {
                "4 bpp, off on the main screen"
            }
            .to_owned(),
        });
        Ok(FrameLayersInfo { spans, layers })
    }

    /// The screen's state on the lines `frame` drew; `None` where the
    /// recording has no line writes for it.
    pub fn screen_lines(&self, frame: u64) -> Result<Option<ScreenLinesInfo>, RomlensError> {
        let Some(states) =
            romlens_core::recording::lines::frame_line_states(self.source.dynamic(), frame, 224)?
        else {
            return Ok(None);
        };
        let inidisp: Vec<u8> = states.iter().map(|p| p.register(0x2100)).collect();
        let drawn: Vec<u8> = inidisp
            .iter()
            .filter(|v| *v & 0x80 == 0)
            .map(|v| v & 0x0F)
            .collect();
        Ok(Some(ScreenLinesInfo {
            lines: inidisp.len() as u32,
            blank: (inidisp.len() - drawn.len()) as u32,
            brightness_min: drawn.iter().copied().min().unwrap_or(0),
            brightness_max: drawn.iter().copied().max().unwrap_or(0),
        }))
    }

    /// The frame's layers front to back, in words, as its mode orders them
    /// (as the frame ends).
    pub fn priority_order(&self, frame: u64) -> Result<Vec<String>, RomlensError> {
        let state = self.source.dynamic().state_at(frame)?;
        let ppu = state.ppu().ok_or(RecordingError::MissingRegion("ppu"))?;
        Ok(romlens_core::graphics::compose::priority_names(
            ppu.bg_mode(),
            ppu.register(0x2105) & 0x08 != 0,
        ))
    }

    /// One layer of `frame` on its own, where it shows on the main screen:
    /// 1–4 a background, 5 the sprites. No colour math; transparent where
    /// the layer draws nothing.
    pub fn render_frame_layer(&self, frame: u64, layer: u8) -> Result<BitmapInfo, RomlensError> {
        use romlens_core::graphics::compose::ComposeOptions;
        let (f, ..) = self.compose(frame, ComposeOptions::alone(layer))?;
        Ok(f.bitmap.into())
    }
}

/// Where a pixel's bytes came from (docs/22, P5), as the inspector shows it:
/// each line already in words, with the addresses its buttons go to.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProvenanceInfo {
    /// What drew the pixel.
    pub summary: String,
    pub parts: Vec<ProvenancePartInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProvenancePartInfo {
    /// "its tile's bytes for this row", "its OAM entry", "its colour".
    pub what: String,
    pub links: Vec<ProvenanceLinkInfo>,
    pub hop: Option<ProvenanceHopInfo>,
}

/// Bytes that got there the same way.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProvenanceLinkInfo {
    pub summary: String,
    /// Each byte, with where a DMA read it from.
    pub bytes: Vec<String>,
    /// The instruction that started the DMA (SNES address).
    pub started_at: Option<u32>,
    /// A DMA straight from the ROM: its source's file offset and length.
    pub rom_start: Option<u32>,
    pub rom_len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProvenanceHopInfo {
    /// Who writes the buffer or the port.
    pub code_summary: String,
    pub code: Vec<CodeWriterInfo>,
    /// Where the bytes are in the ROM, or what was not found.
    pub placed_summary: Option<String>,
    pub placed: Option<PlacedInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CodeWriterInfo {
    /// SNES address of the instruction.
    pub pc: u32,
    pub count: u32,
    /// It writes 4 KB or more in a run: clearing memory.
    pub clears: bool,
}

/// Where a run of the part's bytes is in the ROM.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PlacedInfo {
    /// File offset and length: the bytes as they are, or the whole stream.
    pub start: u32,
    pub len: u32,
    pub compressed: bool,
    /// The ROM byte behind the part's first byte.
    pub focus: u32,
}

impl ProvenanceInfo {
    pub(crate) fn from_chain(
        c: &romlens_core::provenance::chain::Chain,
        rom: &romlens_core::RomImage,
        have_log: bool,
    ) -> Self {
        use romlens_core::provenance::chain::{Link, target_name};
        use romlens_core::provenance::source::RomSource;
        ProvenanceInfo {
            summary: c.describe(),
            parts: c
                .parts
                .iter()
                .map(|p| ProvenancePartInfo {
                    what: p.what.to_owned(),
                    links: p
                        .groups
                        .iter()
                        .map(|g| {
                            let (started_at, rom_start, rom_len) = match &g.link {
                                Link::Dma {
                                    started_at,
                                    source,
                                    bytes,
                                    ..
                                } => (
                                    started_at.map(|a| a.as_u24()),
                                    p.from_rom
                                        .and_then(|_| rom.file_offset_for(*source))
                                        .map(|o| o.0),
                                    *bytes,
                                ),
                                _ => (None, None, 0),
                            };
                            ProvenanceLinkInfo {
                                summary: g.link.describe(),
                                bytes: g
                                    .bytes
                                    .iter()
                                    .map(|b| match b.source {
                                        Some(s) => format!("{} from {s}", target_name(b.target)),
                                        None => target_name(b.target),
                                    })
                                    .collect(),
                                started_at,
                                rom_start,
                                rom_len,
                            }
                        })
                        .collect(),
                    hop: p.hop.as_ref().map(|h| ProvenanceHopInfo {
                        code_summary: h.describe_code(),
                        code: h
                            .code
                            .iter()
                            .flatten()
                            .take(4)
                            .map(|c| CodeWriterInfo {
                                pc: c.pc,
                                count: c.count,
                                clears: c.clears(),
                            })
                            .collect(),
                        placed_summary: h.describe_placed(rom, have_log),
                        placed: h.placed.as_ref().map(|pl| {
                            let (start, len, compressed) = match pl.source {
                                RomSource::Verbatim { at, .. } => (at.0, 0, false),
                                RomSource::Compressed {
                                    stream, consumed, ..
                                } => (stream.0, consumed as u32, true),
                            };
                            PlacedInfo {
                                start,
                                len,
                                compressed,
                                focus: pl.focus_offset().0,
                            }
                        }),
                    }),
                })
                .collect(),
        }
    }
}

impl RecordingSession {
    /// How far back to look for a byte's last write: the frame it last
    /// changed, from the change index (built on first use, as `history`
    /// does), and at most a minute of frames.
    pub(crate) fn earliest(&self, frame: u64, target: romlens_core::provenance::Target) -> u64 {
        use romlens_core::recording::change_index::{ChangeIndex, load_or_build};
        let floor = frame.saturating_sub(3600);
        let Some(source) = self.source.file() else {
            return floor;
        };
        let mut slot = self.index.lock().unwrap();
        if slot.is_none() {
            *slot = match &self.origin {
                Origin::Path(p) => load_or_build(Path::new(p), source, false).ok().map(|x| x.0),
                Origin::Bytes(_) | Origin::Live => ChangeIndex::build(source).ok(),
            };
        }
        slot.as_ref()
            .and_then(|i| {
                i.when(
                    source,
                    romlens_core::provenance::chain::region_of(target.memory),
                    target.byte,
                    1,
                    frame,
                    true,
                )
                .ok()
                .flatten()
            })
            .unwrap_or(0)
            .max(floor)
    }

    pub(crate) fn machine(&self) -> &dyn MachineStateSource {
        self.source.dynamic()
    }
}

/// The synthetic recording `romlens testrec` writes, for shell tests.
#[uniffi::export]
pub fn make_test_recording(frames: u32) -> Vec<u8> {
    use romlens_core::recording::writer::WriterOptions;
    use romlens_core::recording::{RomrecWriter, fixtures};
    let mut w = RomrecWriter::new(
        std::io::Cursor::new(Vec::new()),
        &fixtures::identity(),
        &CoreRegion::ALL,
        WriterOptions::default(),
        0,
    )
    .expect("an in-memory writer cannot fail");
    for s in fixtures::frames(frames.max(1)) {
        w.write_frame(&s).expect("fixture frames are well formed");
    }
    w.finish().expect("in memory").into_inner()
}

/// The Mesen recorder script, as `romlens rec script` writes it.
#[uniffi::export]
pub fn recorder_script() -> String {
    romlens_core::recording::mesen::RECORDER_SCRIPT.to_owned()
}

/// Write a one-frame recording of `rom` from loose memory dumps, the way
/// `romlens rec import-raw` does. Each dump must be its region's size (OAM
/// may be the 512-byte low table alone). Written beside `path` and moved
/// into place, so a failure leaves any file already there alone.
#[uniffi::export]
pub fn write_snapshot_recording(
    rom: Arc<Rom>,
    vram: Option<Vec<u8>>,
    cgram: Option<Vec<u8>>,
    oam: Option<Vec<u8>>,
    ppu: Option<Vec<u8>>,
    path: String,
) -> Result<(), RomlensError> {
    use romlens_core::recording::import::state_from_dumps;
    use romlens_core::recording::writer::WriterOptions;
    use romlens_core::recording::{RecordingIdentity, RomrecWriter};
    let dumps: Vec<(CoreRegion, Vec<u8>)> = [
        (CoreRegion::Vram, vram),
        (CoreRegion::Cgram, cgram),
        (CoreRegion::Oam, oam),
        (CoreRegion::PpuState, ppu),
    ]
    .into_iter()
    .filter_map(|(r, b)| b.map(|b| (r, b)))
    .collect();
    let state = state_from_dumps(&dumps)?;
    let regions: Vec<CoreRegion> = state.regions.keys().copied().collect();
    let identity = RecordingIdentity {
        rom_sha256: *rom.image.sha256(),
        producer: "Romlens Import Snapshot".to_owned(),
        producer_version: romlens_core::API_VERSION.to_owned(),
    };
    let io = |e: std::io::Error| RomlensError::Io { msg: e.to_string() };
    let part = format!("{path}.part");
    let result = (|| {
        let file = std::io::BufWriter::new(std::fs::File::create(&part).map_err(io)?);
        let mut w = RomrecWriter::new(file, &identity, &regions, WriterOptions::default(), 0)?;
        w.write_frame(&state)?;
        w.finish()?;
        std::fs::rename(&part, &path).map_err(io)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// The graphics test ROM, for shell tests.
#[uniffi::export]
pub fn make_graphics_test_rom() -> Vec<u8> {
    romlens_core::fixtures::graphics_lorom()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_7_draws_its_plane_and_lists_its_cells() {
        use romlens_core::recording::writer::WriterOptions;
        use romlens_core::recording::{MachineState, RomrecWriter, fixtures};
        let mut state = fixtures::frames(1).remove(0);
        let mut ppu = state.ppu().unwrap();
        ppu.set_register(0x2105, 7);
        state
            .regions
            .insert(CoreRegion::PpuState, ppu.bytes.to_vec());
        let mut vram = vec![0u8; 0x10000];
        vram[2 * (128 + 3)] = 0x42; // map cell (3, 1) is tile $42
        state.regions.insert(CoreRegion::Vram, vram.clone());
        let mut w = RomrecWriter::new(
            std::io::Cursor::new(Vec::new()),
            &fixtures::identity(),
            &CoreRegion::ALL,
            WriterOptions::default(),
            0,
        )
        .unwrap();
        w.write_frame(&MachineState { frame: 0, ..state }).unwrap();
        let rec = RecordingSession::from_bytes(w.finish().unwrap().into_inner()).unwrap();
        let plane = rec.render_bg(0, 1).unwrap();
        assert_eq!((plane.width, plane.height), (1024, 1024));
        assert!(matches!(
            rec.render_bg(0, 2),
            Err(RomlensError::Recording { .. })
        ));
        let cells = mode7_cells(vram);
        assert_eq!(cells.len(), 128 * 128);
        let c = &cells[128 + 3];
        assert_eq!((c.col, c.row, c.tile, c.byte_offset), (3, 1, 0x42, 2 * 131));
    }

    #[test]
    fn a_recording_validates_answers_history_and_refers_to_itself() {
        let rec = RecordingSession::from_bytes(make_test_recording(40)).unwrap();
        let v = rec.validate(8).unwrap();
        assert_eq!((v.errors, v.warnings), (0, 0), "{:?}", v.lines);
        // fixtures::frames rewrites tile 5 ($A0-$BF) at frame 30.
        let h = rec.history(StateRegion::Vram, 0xA0, 32, 35).unwrap();
        assert_eq!((h.indexed, h.last, h.next), (true, Some(30), None));
        let h = rec.history(StateRegion::Vram, 0xA0, 32, 10).unwrap();
        assert_eq!((h.last, h.next), (Some(0), Some(30)));
        assert_eq!(rec.reference(), None);
        assert!(recorder_script().contains("RLSTREAM"));

        let dir = std::env::temp_dir().join(format!("romlens-ffi-snap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.romrec").to_string_lossy().into_owned();
        let rom = Rom::from_bytes(make_graphics_test_rom(), "g.sfc".to_owned()).unwrap();
        write_snapshot_recording(
            rom.clone(),
            Some(vec![0; 0x10000]),
            Some(vec![0; 512]),
            Some(vec![0; 512]),
            None,
            path.clone(),
        )
        .unwrap();
        let snap = RecordingSession::open(path.clone(), false).unwrap();
        snap.check_rom(rom.clone()).unwrap();
        assert_eq!(snap.info().frame_count, 1);
        assert_eq!(snap.reference().unwrap().frames, 1);
        // A bad dump fails and leaves the good file alone.
        assert!(
            write_snapshot_recording(rom, Some(vec![0; 3]), None, None, None, path.clone())
                .is_err()
        );
        assert!(RecordingSession::open(path, false).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pixel_names_the_bytes_it_was_drawn_from() {
        let rec = RecordingSession::from_bytes(make_test_recording(40)).unwrap();
        let rom = Rom::from_bytes(make_graphics_test_rom(), "g.sfc".to_owned()).unwrap();
        let wb = crate::Workbench::new(rom);
        let p = wb
            .pixel_provenance_blocking(rec.clone(), 8, 150, 55)
            .unwrap();
        assert!(
            p.summary.starts_with("(150, 55) at frame 8: sprite 3"),
            "{}",
            p.summary
        );
        let what: Vec<&str> = p.parts.iter().map(|x| x.what.as_str()).collect();
        assert_eq!(
            what,
            [
                "its tile's bytes for this row",
                "its OAM entry",
                "its colour"
            ]
        );
        // The synthetic recording logs no writes, so none is found.
        assert!(p.parts[0].links[0].summary.starts_with("no write logged"));
        assert_eq!(wb.pixel_provenance_blocking(rec, 8, 999, 0), None);
    }

    #[test]
    fn a_frame_draws_and_names_what_drew_each_pixel() {
        let rec = RecordingSession::from_bytes(make_test_recording(40)).unwrap();
        let f = rec.render_frame(8).unwrap();
        assert_eq!((f.image.width, f.image.height), (256, 224));
        assert_eq!(f.bg_mode, 1);
        assert!(!f.per_line, "a synthetic recording has no line writes");
        assert!(matches!(
            rec.frame_pixel(8, 150, 55).unwrap(),
            Some(PixelWinnerInfo::Sprite { sprite: 3, .. })
        ));
        assert!(matches!(
            rec.frame_pixel(8, 68, 52).unwrap(),
            Some(PixelWinnerInfo::Background { layer: 1, .. })
        ));
        assert_eq!(
            rec.frame_pixel(8, 4, 4).unwrap(),
            Some(PixelWinnerInfo::Backdrop)
        );
        assert_eq!(rec.frame_pixel(8, 300, 4).unwrap(), None);
        // The sprites alone: the sprite, and transparency where BG1 was.
        let objs = rec.render_frame_layer(8, 5).unwrap();
        let alpha = |b: &BitmapInfo, x: u32, y: u32| b.rgba[((y * b.width + x) * 4 + 3) as usize];
        assert_eq!(alpha(&objs, 150, 55), 255);
        assert_eq!(alpha(&objs, 68, 52), 0);
    }

    #[test]
    fn a_recording_opens_from_bytes_and_draws() {
        let rec = RecordingSession::from_bytes(make_test_recording(40)).unwrap();
        let info = rec.info();
        assert_eq!(info.frame_count, 40);
        assert_eq!(info.regions.len(), 8);
        let rom = Rom::from_bytes(make_graphics_test_rom(), "g.sfc".to_owned()).unwrap();
        rec.check_rom(rom.clone()).unwrap();
        let other = Rom::from_bytes(
            crate::make_test_rom(crate::Mapping::LoRom),
            "t.sfc".to_owned(),
        )
        .unwrap();
        assert!(matches!(
            rec.check_rom(other),
            Err(RomlensError::RomMismatch { .. })
        ));
        let bg = rec.render_bg(0, 1).unwrap();
        assert_eq!((bg.width, bg.height), (256, 256));
        assert_eq!(bg.rgba.len(), 256 * 256 * 4);
        assert!(matches!(
            rec.render_bg(0, 4),
            Err(RomlensError::Recording { .. })
        ));
        let summary = rec.ppu_summary(3).unwrap();
        assert_eq!(summary.bg_mode, 1);
        assert_eq!(summary.layers[0].hscroll, 3);
        assert_eq!(summary.layers[3].format, None);
        assert_eq!(summary.pc, 0x80_800A);
        let runs = rec.changes(29, 30, StateRegion::Vram).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(rec.region(0, StateRegion::Cgram).unwrap().len(), 512);
        assert!(rec.region(40, StateRegion::Oam).is_err());
    }

    #[test]
    fn the_bit_table_matches_the_decoder() {
        for format in [
            TileFormat::Bpp2,
            TileFormat::Bpp4,
            TileFormat::Bpp8,
            TileFormat::Mode7,
        ] {
            let bpp = CoreFormat::from(format).bpp() as usize;
            let table = tile_bit_sources(format);
            assert_eq!(table.len(), 64 * bpp);
            // Pixel (3, 5), top plane.
            let entry = table[(5 * 8 + 3) * bpp + bpp - 1];
            let mut bytes = vec![0u8; tile_byte_len(format) as usize];
            bytes[(entry >> 3) as usize] |= 1 << (entry & 7);
            let t = decode_tile(bytes, format);
            assert_eq!(t.indices[5 * 8 + 3], 1 << (bpp - 1));
            assert_eq!(t.indices.iter().filter(|v| **v != 0).count(), 1);
        }
    }

    #[test]
    fn rom_bytes_feed_the_decoders() {
        let rom = Rom::from_bytes(make_graphics_test_rom(), "g.sfc".to_owned()).unwrap();
        let tiles = rom.bytes(0x1000, 1024);
        assert_eq!(tiles.len(), 1024);
        assert_eq!(rom.bytes(0xFFFF, 16).len(), 1, "cut short at the end");
        let sheet = tile_sheet(
            tiles.clone(),
            TileFormat::Bpp4,
            32,
            16,
            PaletteSource::Grayscale,
        );
        assert_eq!((sheet.width, sheet.height), (128, 16));
        let lens = decode_tile(tiles[8 * 32..9 * 32].to_vec(), TileFormat::Bpp4);
        assert_eq!(lens.planes.len(), 32);
        let pal = palette_entries(rom.bytes(0x1800, 512), 256);
        assert_eq!(pal[15].rgb, 0xFFFFFF);
        let oam = oam_entries(rom.bytes(0x1A00, 544), 0, OamSort::Table);
        assert_eq!((oam[0].width, oam[0].x, oam[3].vflip), (16, 64, true));
        assert_eq!(oam.iter().filter(|e| e.on_screen).count(), 4);
        let cells = tilemap_cells(rom.bytes(0x2000, 2048), ScreenSize::S32x32);
        assert_eq!(cells.len(), 1024);
        assert_eq!(cells[14 * 32 + 14].tile, 0x08);
        assert_eq!(cells[14 * 32 + 14].byte_offset, (14 * 32 + 14) * 2);
        let d = decompress_sm(rom.bytes(0x3000, 0x1000)).unwrap();
        assert_eq!(d.output, tiles);
    }
}
