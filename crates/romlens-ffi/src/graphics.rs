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
}

/// The registers the graphics views need at one frame, decoded.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PpuSummary {
    pub bg_mode: u8,
    pub obsel: u8,
    pub main_screen: u8,
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
    source: RomrecSource,
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
        Ok(Arc::new(RecordingSession { source }))
    }

    #[uniffi::constructor]
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Arc<Self>, RomlensError> {
        Ok(Arc::new(RecordingSession {
            source: RomrecSource::from_bytes(bytes, false)?,
        }))
    }

    pub fn info(&self) -> RecordingInfo {
        let h = self.source.header();
        RecordingInfo {
            frame_count: self.source.frame_count().unwrap_or(0),
            keyframe_interval: h.keyframe_interval,
            producer: h.producer.clone(),
            producer_version: h.producer_version.clone(),
            rom_sha256: self.source.identity().sha256_hex(),
            regions: self.source.regions().into_iter().map(Into::into).collect(),
            file_len: self.source.file_len(),
            recovered: self.source.recovered(),
        }
    }

    /// Refuse a recording of another ROM with the core's message.
    pub fn check_rom(&self, rom: Arc<Rom>) -> Result<(), RomlensError> {
        Ok(self.source.check_rom(rom.image.sha256())?)
    }

    pub fn region(&self, frame: u64, region: StateRegion) -> Result<Vec<u8>, RomlensError> {
        Ok(self.source.region_at(frame, region.into())?)
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
            .changes(from, to, region.into())?
            .into_iter()
            .flat_map(|r| [r.offset, r.len])
            .collect())
    }

    pub fn ppu_summary(&self, frame: u64) -> Result<PpuSummary, RomlensError> {
        let state = self.source.state_at(frame)?;
        let ppu = state.ppu().ok_or(RecordingError::MissingRegion("ppu"))?;
        let cpu = state
            .region(CoreRegion::CpuRegisters)
            .map(CpuRegisters::decode)
            .unwrap_or_default();
        Ok(PpuSummary {
            bg_mode: ppu.bg_mode(),
            obsel: ppu.register(0x2101),
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
        let state = self.source.state_at(frame)?;
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
        let state = self.source.state_at(frame)?;
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
