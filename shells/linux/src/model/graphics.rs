//! The graphics views' shared state: what they read, how they decode it and
//! what is selected inside them. They read raw ROM bytes at an offset (the
//! default, and all a ROM nobody has recorded needs), or bytes that are not in
//! the ROM as they stand, such as a decompressed block. Every selection that
//! has a ROM byte range selects that range in the editor too, which is what
//! keeps one selection across code and graphics (docs/02): the byte range is
//! the join key. The macOS twin is `GraphicsModel`.
//!
//! A recording is a source too: VRAM, CGRAM and OAM at a frame, and the Frame
//! and Layers views that draw the screen from them.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;

use romlens_ffi::live::{LiveSession, LiveStatus};
use romlens_ffi::{
    BgLayerInfo, BitmapInfo, ChangeHistory, FrameImageInfo, FrameLayersInfo, OamEntryInfo, OamSort,
    PaletteEntryInfo, PaletteSource, PixelWinnerInfo, PpuSummary, RecordingInfo, RecordingSession,
    Rom, ScreenLinesInfo, ScreenSize, StateRegion, TileFormat, TileInfo, TilemapCellInfo,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Frame,
    Layers,
    Tiles,
    Palette,
    Oam,
    Tilemap,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Frame,
        Tab::Layers,
        Tab::Tiles,
        Tab::Palette,
        Tab::Oam,
        Tab::Tilemap,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Tab::Frame => "frame",
            Tab::Layers => "layers",
            Tab::Tiles => "tiles",
            Tab::Palette => "palette",
            Tab::Oam => "oam",
            Tab::Tilemap => "tilemap",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Tab::Frame => "Frame",
            Tab::Layers => "Layers",
            Tab::Tiles => "Tile Decoder",
            Tab::Palette => "Palette",
            Tab::Oam => "OAM",
            Tab::Tilemap => "Tilemap",
        }
    }

    pub fn from_id(id: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.id() == id)
    }

    /// Views that need a recording: the screen exists only in one.
    pub fn needs_recording(self) -> bool {
        matches!(self, Tab::Frame | Tab::Layers)
    }
}

/// What the views read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// ROM bytes from `rom_offset`.
    Rom,
    /// The attached recording at `frame`.
    Recording,
    /// Bytes that are not in the ROM as they stand: a decompressed block.
    Bytes { label: String, data: Vec<u8> },
}

/// Where a tile view's colours come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteChoice {
    Grayscale,
    /// BGR15 colours in the ROM at this file offset.
    Rom(u32),
    /// A CGRAM row of the attached recording: 0 to 7 BG, 8 to 15 OBJ.
    Cgram(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OamSortChoice {
    Table,
    Screen,
    Priority,
}

impl OamSortChoice {
    pub const ALL: [OamSortChoice; 3] = [
        OamSortChoice::Table,
        OamSortChoice::Screen,
        OamSortChoice::Priority,
    ];

    pub fn title(self) -> &'static str {
        match self {
            OamSortChoice::Table => "Table Order",
            OamSortChoice::Screen => "Screen Position",
            OamSortChoice::Priority => "Priority",
        }
    }

    pub fn core(self) -> OamSort {
        match self {
            OamSortChoice::Table => OamSort::Table,
            OamSortChoice::Screen => OamSort::Screen,
            OamSortChoice::Priority => OamSort::Priority,
        }
    }
}

pub fn format_title(f: TileFormat) -> &'static str {
    match f {
        TileFormat::Bpp2 => "2 bpp",
        TileFormat::Bpp4 => "4 bpp",
        TileFormat::Bpp8 => "8 bpp",
        TileFormat::Mode7 => "Mode 7",
    }
}

/// Bits per pixel, which is also the number of planes the teaching view
/// draws (Mode 7's "planes" are the eight bits of its one byte).
pub fn bits_per_pixel(f: TileFormat) -> usize {
    match f {
        TileFormat::Bpp2 => 2,
        TileFormat::Bpp4 => 4,
        TileFormat::Bpp8 | TileFormat::Mode7 => 8,
    }
}

pub fn colours(f: TileFormat) -> usize {
    1 << bits_per_pixel(f)
}

pub fn screen_title(s: ScreenSize) -> &'static str {
    match s {
        ScreenSize::S32x32 => "32×32",
        ScreenSize::S64x32 => "64×32",
        ScreenSize::S32x64 => "32×64",
        ScreenSize::S64x64 => "64×64",
    }
}

/// A tilemap's cells across and down.
pub fn screen_cells(s: ScreenSize) -> (usize, usize) {
    match s {
        ScreenSize::S32x32 => (32, 32),
        ScreenSize::S64x32 => (64, 32),
        ScreenSize::S32x64 => (32, 64),
        ScreenSize::S64x64 => (64, 64),
    }
}

pub const SCREEN_SIZES: [ScreenSize; 4] = [
    ScreenSize::S32x32,
    ScreenSize::S64x32,
    ScreenSize::S32x64,
    ScreenSize::S64x64,
];

pub struct GraphicsModel {
    rom: Arc<Rom>,
    pub source: Source,
    /// The ROM offset the views start at.
    pub rom_offset: u32,

    // Tile decoder
    pub format: TileFormat,
    pub columns: usize,
    pub sheet_tiles: usize,
    pub palette: PaletteChoice,
    /// Index into the sheet of the tile the decoder shows.
    pub selected_tile: usize,
    bit_table: RefCell<Option<(TileFormat, Vec<u16>)>>,

    // Palette
    pub selected_colour: Option<usize>,

    // OAM
    pub obsel: u8,
    pub oam_sort: OamSortChoice,
    pub visible_sprites_only: bool,
    pub selected_sprite: Option<u8>,

    // Tilemap
    pub screen_size: ScreenSize,
    pub selected_cell: Option<usize>,
    /// The Tilemap view's size: screen pixels a map pixel; 0 fits the window.
    pub tilemap_scale: i32,
    pub background_layer: u8,

    // Frame and layers
    /// The pixel clicked in the Frame view.
    pub selected_pixel: Option<(usize, usize)>,
    /// How many screen pixels a frame pixel takes.
    pub frame_scale: i32,
    /// The Layers view scales its grid to the window; off, 1:1.
    pub layers_fit: bool,
    /// The Layers view shows each layer with its colour math done, as it
    /// shows on screen; off, in its own colours.
    pub layers_colour_math: bool,

    // Recording
    /// The VRAM byte offset the tile views start at, with a recording.
    pub vram_offset: u32,
    /// A recorder stream being packed into a recording, by name.
    pub packing: Option<String>,
    /// Show each frame as it arrives from a live session.
    pub follow_live: bool,
    live: Option<Arc<LiveSession>>,
    live_status: Option<String>,
    /// Instructions the live session's execution log has added to the
    /// project so far.
    live_discovered: u64,
    applying_live: bool,
    recording: Option<Arc<RecordingSession>>,
    recording_info: Option<RecordingInfo>,
    recording_name: Option<String>,
    frame: u64,
    caches: RefCell<Caches>,
}

/// What is derived from a frame of the recording, kept per frame: a frame
/// never changes once recorded, and the views ask once per cell or pixel
/// while drawing.
#[derive(Default)]
struct Caches {
    ppu: Option<(u64, Option<PpuSummary>)>,
    changes: Vec<(StateRegion, FrameChange)>,
    frame_image: Option<(u64, Option<FrameImageInfo>)>,
    screen_lines: Option<(u64, Option<ScreenLinesInfo>)>,
}

/// One region's comparison with the previous frame: which bytes differ, and
/// both frames' bytes for the checks finer than a byte.
#[derive(Debug, Clone)]
pub struct FrameChange {
    pub frame: u64,
    pub bytes: BTreeSet<usize>,
    pub before: Vec<u8>,
    pub now: Vec<u8>,
}

/// Where the views should go for a pixel's winner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reveal {
    Sprite,
    Tile,
    Cell,
    Colour,
}

impl GraphicsModel {
    pub fn new(rom: Arc<Rom>) -> Self {
        Self {
            rom,
            source: Source::Rom,
            rom_offset: 0,
            format: TileFormat::Bpp4,
            columns: 16,
            sheet_tiles: 256,
            palette: PaletteChoice::Grayscale,
            selected_tile: 0,
            bit_table: RefCell::new(None),
            selected_colour: None,
            obsel: 0,
            oam_sort: OamSortChoice::Table,
            visible_sprites_only: true,
            selected_sprite: None,
            screen_size: ScreenSize::S32x32,
            selected_cell: None,
            tilemap_scale: 2,
            background_layer: 1,
            selected_pixel: None,
            frame_scale: 2,
            layers_fit: true,
            layers_colour_math: true,
            vram_offset: 0,
            packing: None,
            follow_live: true,
            live: None,
            live_status: None,
            live_discovered: 0,
            applying_live: false,
            recording: None,
            recording_info: None,
            recording_name: None,
            frame: 0,
            caches: RefCell::new(Caches::default()),
        }
    }

    // MARK: Recording

    /// Attach a recording, refusing one of another ROM with the core's words.
    pub fn attach(
        &mut self,
        session: Arc<RecordingSession>,
        name: &str,
    ) -> Result<(), romlens_ffi::RomlensError> {
        session.check_rom(Arc::clone(&self.rom))?;
        *self.caches.borrow_mut() = Caches::default();
        self.recording_info = Some(session.info());
        self.recording = Some(session);
        self.recording_name = Some(name.to_owned());
        self.frame = 0;
        self.source = Source::Recording;
        self.selected_pixel = None;
        if let Some(ppu) = self.ppu() {
            self.obsel = ppu.obsel;
        }
        Ok(())
    }

    pub fn detach(&mut self) {
        if let Some(live) = self.live.take() {
            live.stop();
        }
        self.live_status = None;
        *self.caches.borrow_mut() = Caches::default();
        self.recording = None;
        self.recording_info = None;
        self.recording_name = None;
        self.selected_pixel = None;
        if self.source == Source::Recording {
            self.source = Source::Rom;
        }
    }

    pub fn has_recording(&self) -> bool {
        self.recording.is_some()
    }

    pub fn recording(&self) -> Option<&Arc<RecordingSession>> {
        self.recording.as_ref()
    }

    pub fn recording_info(&self) -> Option<&RecordingInfo> {
        self.recording_info.as_ref()
    }

    pub fn recording_name(&self) -> Option<&str> {
        self.recording_name.as_deref()
    }

    /// The frames the recording has: 0 with none.
    pub fn frame_count(&self) -> u64 {
        self.recording_info.as_ref().map_or(0, |i| i.frame_count)
    }

    /// The oldest readable frame: a live session keeps only its latest.
    pub fn first_frame(&self) -> u64 {
        self.recording_info.as_ref().map_or(0, |i| i.first_frame)
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Move to `frame`, kept inside what the recording has.
    pub fn set_frame(&mut self, frame: u64) {
        let count = self.frame_count();
        let frame = if count > 0 {
            frame.min(count - 1)
        } else {
            frame
        };
        self.frame = frame.max(self.first_frame());
        // Moving off the newest frame pauses following; coming back to it
        // resumes. Only a person moves the frame while live, since a new
        // frame arriving sets it under `applying_live`.
        if self.live.is_some() && !self.applying_live {
            self.follow_live = self.frame + 1 >= self.frame_count();
        }
    }

    // MARK: Live

    /// Read a live session's frames as the recording, following the newest.
    pub fn attach_live(
        &mut self,
        session: Arc<LiveSession>,
        discovered_before: u64,
    ) -> Result<(), romlens_ffi::RomlensError> {
        self.attach(session.recording(), "Live")?;
        self.follow_live = true;
        self.live_discovered = discovered_before;
        self.live_status = Some(format!("waiting for Mesen on port {}", session.port()));
        let (status, latest) = (session.status(), session.latest_frame());
        self.live = Some(session);
        // The session may have heard from Mesen before this attached.
        if let Some(s) = status {
            self.live_status_changed(&s);
        }
        if let Some(l) = latest {
            self.live_arrived(l);
        }
        Ok(())
    }

    pub fn is_live(&self) -> bool {
        self.live.is_some()
    }

    pub fn live_status(&self) -> Option<&str> {
        self.live_status.as_deref()
    }

    /// New frames have arrived; `latest` is the newest.
    pub fn live_arrived(&mut self, latest: u64) {
        let (Some(recording), true) = (self.recording.as_ref(), self.live.is_some()) else {
            return;
        };
        self.recording_info = Some(recording.info());
        if !self.follow_live || latest == self.frame {
            return;
        }
        self.applying_live = true;
        self.set_frame(latest);
        self.applying_live = false;
    }

    pub fn live_status_changed(&mut self, status: &LiveStatus) {
        if self.live.is_none() {
            return;
        }
        if let Some(text) = super::live::status_text(self.live_status.as_deref(), status) {
            self.live_status = Some(text);
        }
    }

    /// The project gained `added` instructions from the live execution log.
    pub fn live_log_merged(&mut self, added: u64) {
        self.live_discovered += added;
    }

    /// Stop listening, keeping the frames already received.
    pub fn stop_live(&mut self) {
        if let Some(live) = self.live.take() {
            live.stop();
        }
        self.live_status = None;
        if self.recording.is_some() {
            self.recording_name = Some("Live (stopped)".into());
        }
        self.recording_info = self.recording.as_ref().map(|r| r.info());
    }

    pub fn step(&mut self, delta: i64) {
        if self.frame_count() > 0 {
            self.set_frame((self.frame as i64 + delta).max(0) as u64);
        }
    }

    /// The registers at the current frame, when reading a recording.
    pub fn ppu(&self) -> Option<PpuSummary> {
        if self.source != Source::Recording {
            return None;
        }
        let recording = self.recording.as_ref()?;
        let mut c = self.caches.borrow_mut();
        if let Some((f, v)) = &c.ppu
            && *f == self.frame
        {
            return v.clone();
        }
        let value = recording.ppu_summary(self.frame).ok();
        c.ppu = Some((self.frame, value.clone()));
        value
    }

    fn region(&self, r: StateRegion) -> Option<Vec<u8>> {
        self.recording.as_ref()?.region(self.frame, r).ok()
    }

    /// A region at the current frame, for File > Export Frame Region.
    pub fn region_bytes(&self, r: StateRegion) -> Option<Vec<u8>> {
        self.region(r)
    }

    // MARK: Changes

    /// Exactly what changed in `r` since the previous frame. The recording's
    /// change runs are a superset (nearby changes share a run), so they only
    /// narrow the search; the two frames' bytes decide. Cached per frame,
    /// since every swatch and cell asks.
    pub fn change(&self, r: StateRegion) -> Option<FrameChange> {
        if self.source != Source::Recording || self.frame == 0 {
            return None;
        }
        let recording = self.recording.as_ref()?;
        if let Some((_, c)) = self.caches.borrow().changes.iter().find(|(k, _)| *k == r)
            && c.frame == self.frame
        {
            return Some(c.clone());
        }
        let flat = recording.changes(self.frame - 1, self.frame, r).ok()?;
        let mut bytes = BTreeSet::new();
        let (mut before, mut now) = (Vec::new(), Vec::new());
        if !flat.is_empty() {
            before = recording.region(self.frame - 1, r).ok()?;
            now = recording.region(self.frame, r).ok()?;
            for pair in flat.as_chunks::<2>().0 {
                let start = pair[0] as usize;
                let end = (start + pair[1] as usize).min(before.len()).min(now.len());
                for at in start..end.max(start) {
                    if before[at] != now[at] {
                        bytes.insert(at);
                    }
                }
            }
        }
        let c = FrameChange {
            frame: self.frame,
            bytes,
            before,
            now,
        };
        {
            let mut caches = self.caches.borrow_mut();
            caches.changes.retain(|(k, _)| *k != r);
            caches.changes.push((r, c.clone()));
        }
        Some(c)
    }

    /// Whether any of `len` bytes from `offset` changed since the previous
    /// frame.
    pub fn changed(&self, r: StateRegion, offset: usize, len: usize) -> bool {
        self.change(r)
            .is_some_and(|c| c.bytes.range(offset..offset + len.max(1)).next().is_some())
    }

    /// Whether sprite `index` changed: its four low-table bytes, or its own
    /// two bits of the high-table byte it shares with three others.
    pub fn sprite_changed(&self, index: u8) -> bool {
        let low = usize::from(index) * 4;
        if self.changed(StateRegion::Oam, low, 4) {
            return true;
        }
        let Some(c) = self.change(StateRegion::Oam) else {
            return false;
        };
        let (high, shift) = (0x200 + usize::from(index) / 4, usize::from(index % 4) * 2);
        c.bytes.contains(&high)
            && high < c.before.len()
            && high < c.now.len()
            && (c.before[high] >> shift) & 3 != (c.now[high] >> shift) & 3
    }

    /// When these bytes last changed at or before this frame and next change
    /// after it; `None` unless reading a recording.
    pub fn history(&self, r: StateRegion, offset: u32, len: u32) -> Option<ChangeHistory> {
        if self.source != Source::Recording {
            return None;
        }
        self.recording
            .as_ref()?
            .history(r, offset, len, self.frame)
            .ok()
    }

    /// Where a tilemap cell's entry sits in VRAM, reading a recording.
    pub fn cell_vram(&self, cell: &TilemapCellInfo) -> Option<(u32, u32)> {
        if self.source != Source::Recording {
            return None;
        }
        if self.is_mode7() {
            return Some((cell.byte_offset, 1));
        }
        let layer = self.current_layer()?;
        Some((
            (u32::from(layer.map_word) * 2 + cell.byte_offset) % 0x10000,
            2,
        ))
    }

    /// Whether the screen was off or dimmed at this frame (INIDISP), which is
    /// why a frame can show nothing that makes sense. From the lines the
    /// frame drew where the recording has its register writes: a game turns
    /// the screen off in vertical blank for its uploads and on again before
    /// drawing, so INIDISP as the frame ended says "off" of a frame drawn in
    /// full.
    pub fn screen_note(&self) -> Option<String> {
        if let Some(l) = self.screen_lines() {
            if l.blank == l.lines {
                return Some("screen off (forced blank)".into());
            }
            if l.blank > 0 {
                return Some(format!("screen off on {} of {} lines", l.blank, l.lines));
            }
            if l.brightness_max < 15 {
                return Some(if l.brightness_min == l.brightness_max {
                    format!("brightness {}/15", l.brightness_max)
                } else {
                    format!("brightness {}-{}/15", l.brightness_min, l.brightness_max)
                });
            }
            return None;
        }
        let inidisp = self.ppu()?.inidisp;
        if inidisp & 0x80 != 0 {
            return Some("screen off (forced blank)".into());
        }
        let brightness = inidisp & 0x0F;
        (brightness < 15).then(|| format!("brightness {brightness}/15"))
    }

    fn screen_lines(&self) -> Option<ScreenLinesInfo> {
        if self.source != Source::Recording {
            return None;
        }
        let recording = self.recording.as_ref()?;
        let mut c = self.caches.borrow_mut();
        if let Some((f, v)) = &c.screen_lines
            && *f == self.frame
        {
            return v.clone();
        }
        let info = recording.screen_lines(self.frame).ok().flatten();
        c.screen_lines = Some((self.frame, info.clone()));
        info
    }

    // MARK: Bytes

    fn cut(data: &[u8], len: usize) -> Vec<u8> {
        data.iter().take(len).copied().collect()
    }

    /// `len` bytes the tile views decode, from wherever the source says.
    pub fn tile_source_bytes(&self, len: usize) -> Vec<u8> {
        match &self.source {
            Source::Rom => self.rom.bytes(self.rom_offset, len as u32),
            Source::Recording => {
                let Some(vram) = self.region(StateRegion::Vram) else {
                    return Vec::new();
                };
                let start = (self.vram_offset as usize).min(vram.len());
                vram[start..(start + len).min(vram.len())].to_vec()
            }
            Source::Bytes { data, .. } => Self::cut(data, len),
        }
    }

    /// 512 bytes of colours: CGRAM, or ROM from the offset.
    pub fn palette_bytes(&self) -> Vec<u8> {
        match &self.source {
            Source::Recording => self.region(StateRegion::Cgram).unwrap_or_default(),
            Source::Rom => self.rom.bytes(self.rom_offset, 512),
            Source::Bytes { data, .. } => Self::cut(data, 512),
        }
    }

    pub fn oam_bytes(&self) -> Vec<u8> {
        match &self.source {
            Source::Recording => self.region(StateRegion::Oam).unwrap_or_default(),
            Source::Rom => self.rom.bytes(self.rom_offset, 544),
            Source::Bytes { data, .. } => Self::cut(data, 544),
        }
    }

    /// The map bytes: VRAM at the layer's map address, or ROM.
    pub fn tilemap_bytes(&self) -> Vec<u8> {
        let (c, r) = self.map_cells();
        let len = c * r * 2;
        match &self.source {
            Source::Recording => {
                let (Some(vram), Some(layer)) =
                    (self.region(StateRegion::Vram), self.current_layer())
                else {
                    return Vec::new();
                };
                let start = usize::from(layer.map_word) * 2;
                // A map runs off the end of VRAM and wraps, like the hardware.
                vram.iter()
                    .chain(vram.iter())
                    .skip(start)
                    .take(len)
                    .copied()
                    .collect()
            }
            Source::Rom => self.rom.bytes(self.rom_offset, len as u32),
            Source::Bytes { data, .. } => Self::cut(data, len),
        }
    }

    pub fn current_layer(&self) -> Option<BgLayerInfo> {
        self.ppu()?
            .layers
            .into_iter()
            .find(|l| l.bg == self.background_layer)
    }

    /// What the header says the views are reading.
    pub fn source_description(&self) -> String {
        match &self.source {
            Source::Rom => {
                let snes = self
                    .rom
                    .snes_address_for(self.rom_offset)
                    .map_or_else(String::new, |a| {
                        format!("{} · ", romlens_ffi::format_snes_address(a))
                    });
                format!(
                    "ROM {snes}{}",
                    romlens_ffi::format_file_offset(self.rom_offset)
                )
            }
            Source::Recording => {
                if let Some(status) = &self.live_status {
                    let found = if self.live_discovered > 0 {
                        format!(", {} instructions found", self.live_discovered)
                    } else {
                        String::new()
                    };
                    return format!("Live, frame {}, {status}{found}", self.frame);
                }
                format!(
                    "{}, frame {} of {}",
                    self.recording_name.as_deref().unwrap_or("Recording"),
                    self.frame,
                    self.frame_count()
                )
            }
            Source::Bytes { label, data } => format!("{label}, {} bytes", data.len()),
        }
    }

    // MARK: Tile decoder

    pub fn tile_len(&self) -> usize {
        romlens_ffi::tile_byte_len(self.format) as usize
    }

    /// For pixel (x, y) and plane p: the byte (relative to the tile) and bit
    /// the pixel's bit comes from. The table is fetched once per format, so
    /// hovering costs no call into the core.
    pub fn bit_source(&self, x: usize, y: usize, plane: usize) -> (usize, usize) {
        let mut table = self.bit_table.borrow_mut();
        if table.as_ref().map(|t| t.0) != Some(self.format) {
            *table = Some((self.format, romlens_ffi::tile_bit_sources(self.format)));
        }
        let entry = table.as_ref().expect("just filled").1
            [(y * 8 + x) * bits_per_pixel(self.format) + plane];
        (usize::from(entry >> 3), usize::from(entry & 7))
    }

    pub fn palette_source(&self) -> PaletteSource {
        match self.palette {
            PaletteChoice::Grayscale => PaletteSource::Grayscale,
            PaletteChoice::Rom(offset) => PaletteSource::Colours {
                bytes: self.rom.bytes(offset, (colours(self.format) * 2) as u32),
            },
            PaletteChoice::Cgram(row) => match self.region(StateRegion::Cgram) {
                Some(cgram) => PaletteSource::Cgram {
                    cgram,
                    row,
                    obj: false,
                },
                None => PaletteSource::Grayscale,
            },
        }
    }

    pub fn sheet(&self) -> BitmapInfo {
        romlens_ffi::tile_sheet(
            self.tile_source_bytes(self.tile_len() * self.sheet_tiles),
            self.format,
            self.sheet_tiles as u32,
            self.columns as u32,
            self.palette_source(),
        )
    }

    pub fn selected_tile_info(&self) -> TileInfo {
        let len = self.tile_len();
        let all = self.tile_source_bytes(len * (self.selected_tile + 1));
        let start = (self.selected_tile * len).min(all.len());
        romlens_ffi::decode_tile(all[start..].to_vec(), self.format)
    }

    /// `0xRRGGBB` per index under the current palette choice, for drawing the
    /// zoomed tile.
    pub fn palette_rgb(&self, count: usize) -> Vec<u32> {
        match self.palette_source() {
            PaletteSource::Grayscale => (0..count)
                .map(|i| {
                    let v = (i * 255 / count.saturating_sub(1).max(1)) as u32;
                    v << 16 | v << 8 | v
                })
                .collect(),
            PaletteSource::Colours { bytes } => romlens_ffi::palette_entries(bytes, count as u16)
                .iter()
                .map(|e| e.rgb)
                .collect(),
            PaletteSource::Cgram { cgram, row, .. } => {
                let entries = romlens_ffi::palette_entries(cgram, 256);
                let first = if count >= 256 {
                    0
                } else {
                    usize::from(row) * count
                };
                (0..count)
                    .map(|i| entries.get((first + i) % 256).map_or(0, |e| e.rgb))
                    .collect()
            }
        }
    }

    /// Select tile `index` of the sheet. Returns the ROM bytes it is, to
    /// select in the editor too.
    pub fn select_tile(&mut self, index: usize) -> Option<Range<u32>> {
        self.selected_tile = index;
        if self.source == Source::Rom {
            let start = self.rom_offset + (index * self.tile_len()) as u32;
            return Some(start..start + self.tile_len() as u32);
        }
        None
    }

    /// The sheet's next or previous page, reading ROM.
    pub fn page(&mut self, delta: i64) -> Option<Range<u32>> {
        let step = (self.sheet_tiles * self.tile_len()) as i64 * delta;
        self.rom_offset = (i64::from(self.rom_offset) + step).max(0) as u32;
        self.select_tile(0)
    }

    // MARK: Palette, OAM, tilemap

    pub fn colours(&self) -> Vec<PaletteEntryInfo> {
        romlens_ffi::palette_entries(self.palette_bytes(), 256)
    }

    pub fn select_colour(&mut self, index: usize) -> Option<Range<u32>> {
        self.selected_colour = Some(index);
        (self.source == Source::Rom).then(|| {
            let start = self.rom_offset + (index * 2) as u32;
            start..start + 2
        })
    }

    pub fn sprites(&self) -> Vec<OamEntryInfo> {
        let rows = romlens_ffi::oam_entries(self.oam_bytes(), self.obsel, self.oam_sort.core());
        if self.visible_sprites_only {
            rows.into_iter().filter(|r| r.on_screen).collect()
        } else {
            rows
        }
    }

    /// Selecting a sprite selects its four low-table bytes; the high-table
    /// bits sit in a byte four sprites share, so they are named in the row
    /// rather than selected.
    pub fn select_sprite(&mut self, index: Option<u8>) -> Option<Range<u32>> {
        self.selected_sprite = index;
        let index = index?;
        (self.source == Source::Rom).then(|| {
            let start = self.rom_offset + u32::from(index) * 4;
            start..start + 4
        })
    }

    pub fn cells(&self) -> Vec<TilemapCellInfo> {
        if self.is_mode7() {
            return romlens_ffi::mode7_cells(self.region(StateRegion::Vram).unwrap_or_default());
        }
        let size = self.current_layer().map_or(self.screen_size, |l| l.size);
        romlens_ffi::tilemap_cells(self.tilemap_bytes(), size)
    }

    /// Whether the Tilemap view is showing the Mode 7 plane: one fixed
    /// 128x128 map of byte entries, not a `BGnSC` map.
    pub fn is_mode7(&self) -> bool {
        self.source == Source::Recording
            && self
                .current_layer()
                .is_some_and(|l| l.format == Some(TileFormat::Mode7))
    }

    /// The map's cells across and down, as the view lays them out.
    pub fn map_cells(&self) -> (usize, usize) {
        if self.is_mode7() {
            return (128, 128);
        }
        screen_cells(self.current_layer().map_or(self.screen_size, |l| l.size))
    }

    /// Select cell `index` in reading order. Its bytes come from the cell,
    /// not the index: a 64-wide map stores its right half in a second
    /// sub-map, so reading order and storage order differ.
    pub fn select_cell(&mut self, index: usize) -> Option<Range<u32>> {
        self.selected_cell = Some(index);
        if self.source != Source::Rom {
            return None;
        }
        let all = self.cells();
        let cell = all.get(index)?;
        let start = self.rom_offset + cell.byte_offset;
        Some(start..start + 2)
    }

    /// The rendered layer, from the recording's registers; `None` for ROM
    /// bytes, where the inspector's preview is the drawing and the view shows
    /// the entries.
    pub fn layer_image(&self) -> Option<BitmapInfo> {
        if self.source != Source::Recording {
            return None;
        }
        self.recording
            .as_ref()?
            .render_bg(self.frame, self.background_layer)
            .ok()
    }

    pub fn sprite_image(&self, index: u8) -> Option<BitmapInfo> {
        if self.source != Source::Recording {
            return None;
        }
        self.recording
            .as_ref()?
            .render_sprite(self.frame, index)
            .ok()
    }

    // MARK: Frame and layers

    /// The screen at the current frame, drawn from the PPU state. Kept per
    /// frame: the view redraws on every pointer move.
    pub fn frame_image(&self) -> Option<FrameImageInfo> {
        let recording = self.recording.as_ref()?;
        let mut c = self.caches.borrow_mut();
        if let Some((f, v)) = &c.frame_image
            && *f == self.frame
        {
            return v.clone();
        }
        let value = recording.render_frame(self.frame).ok();
        c.frame_image = Some((self.frame, value.clone()));
        value
    }

    /// What drew a pixel of the current frame.
    pub fn pixel(&self, x: usize, y: usize) -> Option<PixelWinnerInfo> {
        self.recording
            .as_ref()?
            .frame_pixel(self.frame, x as u32, y as u32)
            .ok()
            .flatten()
    }

    /// One layer alone: 1 to 4 a background, 5 the sprites.
    pub fn frame_layer(&self, layer: u8) -> Option<BitmapInfo> {
        self.recording
            .as_ref()?
            .render_frame_layer(self.frame, layer, self.layers_colour_math)
            .ok()
    }

    /// The layers some line of this frame draws, and its modes line by line
    /// (a game can change mode part way down the screen).
    pub fn frame_layers(&self) -> Option<FrameLayersInfo> {
        self.recording.as_ref()?.frame_layers(self.frame).ok()
    }

    /// Point the other views at what drew a pixel: its OAM entry, its tile in
    /// the Tile Decoder, its tilemap cell, or its colour. Returns the view to
    /// show.
    pub fn reveal(&mut self, what: Reveal, winner: &PixelWinnerInfo) -> Option<Tab> {
        self.source = Source::Recording;
        match (what, winner) {
            (Reveal::Sprite, PixelWinnerInfo::Sprite { sprite, .. }) => {
                self.visible_sprites_only = false;
                self.selected_sprite = Some(*sprite);
                Some(Tab::Oam)
            }
            (
                Reveal::Tile,
                PixelWinnerInfo::Sprite {
                    tile_word, colour, ..
                },
            ) => {
                self.show_tile(*tile_word, TileFormat::Bpp4, *colour);
                Some(Tab::Tiles)
            }
            (
                Reveal::Tile,
                PixelWinnerInfo::Background {
                    layer,
                    tile_word,
                    colour,
                    ..
                },
            ) => {
                // Mode 7's tiles sit in VRAM's high bytes, interleaved with
                // the map: the Tilemap view shows them, the Tile Decoder
                // cannot.
                let format = self
                    .ppu()?
                    .layers
                    .iter()
                    .find(|l| l.bg == *layer)?
                    .format
                    .filter(|f| *f != TileFormat::Mode7)?;
                self.show_tile(*tile_word, format, *colour);
                Some(Tab::Tiles)
            }
            (
                Reveal::Cell,
                PixelWinnerInfo::Background {
                    layer, map_word, ..
                },
            ) => {
                self.background_layer = *layer;
                let all = self.cells();
                let offset = if self.is_mode7() {
                    u32::from(*map_word) * 2
                } else {
                    u32::from(map_word.wrapping_sub(self.current_layer()?.map_word)) * 2
                };
                self.selected_cell = all.iter().position(|c| c.byte_offset == offset);
                Some(Tab::Tilemap)
            }
            (Reveal::Colour, w) => {
                self.selected_colour = Some(usize::from(winner_colour(w)?));
                Some(Tab::Palette)
            }
            _ => None,
        }
    }

    fn show_tile(&mut self, word: u16, format: TileFormat, colour: u8) {
        self.format = format;
        self.vram_offset = u32::from(word) * 2;
        self.selected_tile = 0;
        let n = colours(format).max(1);
        self.palette = PaletteChoice::Cgram((usize::from(colour) / n) as u8);
    }
}

/// The CGRAM colour a pixel's winner shows, where it has one.
pub fn winner_colour(w: &PixelWinnerInfo) -> Option<u8> {
    match w {
        PixelWinnerInfo::Blank => None,
        PixelWinnerInfo::Backdrop => Some(0),
        PixelWinnerInfo::Background { colour, .. } | PixelWinnerInfo::Sprite { colour, .. } => {
            Some(*colour)
        }
    }
}

/// One line for the status bar.
pub fn winner_summary(w: &PixelWinnerInfo) -> String {
    match w {
        PixelWinnerInfo::Blank => "the screen is off here (forced blank)".to_owned(),
        PixelWinnerInfo::Backdrop => {
            "the backdrop: no layer drew here, so it shows CGRAM colour 0".to_owned()
        }
        PixelWinnerInfo::Background {
            layer,
            map_word,
            entry,
            tile,
            x,
            y,
            index,
            colour,
            ..
        } => {
            let pal = (entry >> 10) & 7;
            let flips = format!(
                "{}{}",
                if entry & 0x4000 != 0 { ", h-flip" } else { "" },
                if entry & 0x8000 != 0 { ", v-flip" } else { "" }
            );
            format!(
                "BG{layer}: tilemap entry at VRAM ${map_word:04X} = ${entry:04X} (tile ${tile:03X}, palette {pal}{}{flips}); pixel ({x}, {y}) of the tile, index {index}, colour {colour}",
                if entry & 0x2000 != 0 {
                    ", high priority"
                } else {
                    ""
                }
            )
        }
        PixelWinnerInfo::Sprite {
            sprite,
            tile,
            tile_word,
            x,
            y,
            index,
            colour,
            priority,
        } => format!(
            "sprite {sprite} (priority {priority}): tile ${tile:03X} at VRAM ${tile_word:04X}; pixel ({x}, {y}) of the tile, index {index}, colour {colour}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> GraphicsModel {
        let rom = Rom::from_bytes(romlens_ffi::make_graphics_test_rom(), "g.sfc".into()).unwrap();
        GraphicsModel::new(rom)
    }

    #[test]
    fn the_formats_agree_with_the_hardware() {
        for (f, bpp, colours_) in [
            (TileFormat::Bpp2, 2, 4),
            (TileFormat::Bpp4, 4, 16),
            (TileFormat::Bpp8, 8, 256),
            (TileFormat::Mode7, 8, 256),
        ] {
            assert_eq!(bits_per_pixel(f), bpp);
            assert_eq!(colours(f), colours_);
        }
        assert_eq!(screen_cells(ScreenSize::S64x32), (64, 32));
        assert_eq!(screen_title(ScreenSize::S32x64), "32×64");
        assert_eq!(Tab::from_id("oam"), Some(Tab::Oam));
        assert!(Tab::Frame.needs_recording() && !Tab::Tiles.needs_recording());
    }

    #[test]
    fn a_tile_is_the_bytes_at_the_offset_and_selecting_it_selects_them() {
        let mut m = model();
        m.format = TileFormat::Bpp4;
        assert_eq!(m.tile_len(), 32);
        m.rom_offset = 0x100;
        let r = m.select_tile(2).expect("reading ROM");
        assert_eq!(r, 0x140..0x160);
        let tile = m.selected_tile_info();
        assert_eq!(tile.bytes.len(), 32);
        assert_eq!(tile.bytes, m.rom_bytes_for_test(0x140, 32));
        assert_eq!(tile.indices.len(), 64);
    }

    #[test]
    fn decompressed_bytes_have_no_rom_range_to_select() {
        let mut m = model();
        m.source = Source::Bytes {
            label: "Decompressed".into(),
            data: (0..=255u8).cycle().take(2048).collect(),
        };
        assert_eq!(m.select_tile(1), None);
        assert_eq!(m.selected_tile_info().bytes[0], 32);
        assert_eq!(m.source_description(), "Decompressed, 2048 bytes");
        assert_eq!(m.select_colour(3), None);
        assert_eq!(m.select_sprite(Some(1)), None);
    }

    #[test]
    fn paging_moves_by_a_sheet_and_stops_at_the_start() {
        let mut m = model();
        m.sheet_tiles = 4;
        m.page(1);
        assert_eq!(m.rom_offset, 4 * 32);
        m.page(-1);
        m.page(-1);
        assert_eq!(m.rom_offset, 0);
    }

    #[test]
    fn the_bit_table_names_the_byte_and_bit_of_every_pixel_plane() {
        let mut m = model();
        m.format = TileFormat::Bpp2;
        // 2 bpp: planes 0 and 1 of row y are bytes 2y and 2y+1; pixel x is
        // bit 7-x.
        assert_eq!(m.bit_source(0, 0, 0), (0, 7));
        assert_eq!(m.bit_source(3, 2, 1), (5, 4));
        m.format = TileFormat::Bpp4;
        assert_eq!(m.bit_source(0, 0, 2), (16, 7), "planes 2 and 3 follow");
    }

    #[test]
    fn colours_are_decoded_and_palette_choice_reads_rom() {
        let mut m = model();
        assert_eq!(m.colours().len(), 256);
        assert_eq!(m.palette_rgb(16).len(), 16);
        assert_eq!(m.palette_rgb(16)[0], 0, "grayscale starts at black");
        assert_eq!(m.palette_rgb(16)[15], 0xFFFFFF);
        m.palette = PaletteChoice::Rom(0);
        assert_eq!(m.palette_rgb(16).len(), 16);
        assert_eq!(m.select_colour(5), Some(10..12));
    }

    #[test]
    fn sprites_filter_to_the_visible_and_select_their_low_table_bytes() {
        let mut m = model();
        let all = {
            m.visible_sprites_only = false;
            m.sprites().len()
        };
        assert_eq!(all, 128);
        m.visible_sprites_only = true;
        assert!(m.sprites().len() <= all);
        m.rom_offset = 0x40;
        assert_eq!(m.select_sprite(Some(3)), Some(0x4c..0x50));
        assert_eq!(m.select_sprite(None), None);
        assert_eq!(m.selected_sprite, None);
    }

    #[test]
    fn a_tilemap_cell_selects_its_own_two_bytes_not_its_reading_order() {
        let mut m = model();
        m.screen_size = ScreenSize::S64x32;
        assert_eq!(m.map_cells(), (64, 32));
        let cells = m.cells();
        assert_eq!(cells.len(), 64 * 32);
        // Column 32 of row 0 lives in the second sub-map.
        let idx = cells
            .iter()
            .position(|c| c.col == 32 && c.row == 0)
            .unwrap();
        let r = m.select_cell(idx).unwrap();
        assert_eq!(r.start, cells[idx].byte_offset);
        assert_eq!(r.end - r.start, 2);
        assert_ne!(cells[idx].byte_offset, 64, "not reading order");
    }

    fn recorded() -> GraphicsModel {
        let mut m = model();
        let rec = RecordingSession::from_bytes(romlens_ffi::make_test_recording(40)).unwrap();
        m.attach(rec, "test.romrec").unwrap();
        m
    }

    #[test]
    fn attaching_reads_the_recording_and_frames_stay_inside_it() {
        let mut m = recorded();
        assert!(m.has_recording() && m.source == Source::Recording);
        assert_eq!(m.frame_count(), 40);
        assert_eq!(m.source_description(), "test.romrec, frame 0 of 40");
        m.set_frame(1000);
        assert_eq!(m.frame(), 39);
        m.step(-100);
        assert_eq!(m.frame(), 0);
        m.step(3);
        assert_eq!(m.frame(), 3);
        // Reading the recording: VRAM and CGRAM, not the ROM.
        assert_eq!(m.palette_bytes().len(), 512);
        assert_eq!(m.oam_bytes().len(), 544);
        assert_eq!(m.tile_source_bytes(64).len(), 64);
        m.detach();
        assert!(!m.has_recording() && m.source == Source::Rom);
    }

    #[test]
    fn a_recording_of_another_rom_is_refused_with_the_cores_words() {
        let mut m = GraphicsModel::new(
            Rom::from_bytes(romlens_ffi::make_routines_test_rom(), "r.sfc".into()).unwrap(),
        );
        let rec = RecordingSession::from_bytes(romlens_ffi::make_test_recording(3)).unwrap();
        let e = m.attach(rec, "x").unwrap_err().to_string();
        assert!(!e.is_empty());
        assert!(!m.has_recording(), "nothing is attached on refusal");
    }

    #[test]
    fn what_changed_is_exactly_the_bytes_that_differ_from_the_previous_frame() {
        let mut m = recorded();
        assert!(
            m.change(StateRegion::Oam).is_none(),
            "frame 0 has no previous"
        );
        m.set_frame(8);
        let c = m.change(StateRegion::Oam).expect("a recording has changes");
        // Every byte reported differs, and the two frames agree elsewhere.
        for &at in &c.bytes {
            assert_ne!(c.before[at], c.now[at]);
        }
        for at in 0..c.before.len().min(c.now.len()) {
            if !c.bytes.contains(&at) {
                assert_eq!(c.before[at], c.now[at]);
            }
        }
        let first = c.bytes.iter().next().copied();
        if let Some(at) = first {
            assert!(m.changed(StateRegion::Oam, at, 1));
            assert!(!m.changed(StateRegion::Oam, 100_000, 4));
        }
        // Per frame: moving on recomputes.
        m.set_frame(9);
        assert_eq!(m.change(StateRegion::Oam).unwrap().frame, 9);
    }

    #[test]
    fn the_frame_names_what_drew_a_pixel_and_reveal_points_the_other_views_at_it() {
        let mut m = recorded();
        m.set_frame(8);
        let image = m.frame_image().expect("a frame draws");
        assert_eq!((image.image.width, image.image.height), (256, 224));
        let winner = m.pixel(150, 55).expect("a pixel has a winner");
        assert!(matches!(winner, PixelWinnerInfo::Sprite { sprite: 3, .. }));
        assert!(winner_summary(&winner).starts_with("sprite 3 (priority"));
        assert_eq!(m.reveal(Reveal::Sprite, &winner), Some(Tab::Oam));
        assert_eq!(m.selected_sprite, Some(3));
        assert!(!m.visible_sprites_only, "so the sprite is in the list");
        assert_eq!(m.reveal(Reveal::Tile, &winner), Some(Tab::Tiles));
        assert_eq!(m.format, TileFormat::Bpp4);
        assert!(
            matches!(m.palette, PaletteChoice::Cgram(r) if r >= 8),
            "an OBJ palette"
        );
        assert_eq!(m.reveal(Reveal::Colour, &winner), Some(Tab::Palette));
        assert_eq!(m.selected_colour, winner_colour(&winner).map(usize::from));
        // A sprite has no tilemap cell.
        assert_eq!(m.reveal(Reveal::Cell, &winner), None);
    }

    #[test]
    fn a_background_pixel_reveals_its_cell_in_reading_order() {
        let mut m = recorded();
        m.set_frame(8);
        let winner = (0..224)
            .flat_map(|y| (0..256).map(move |x| (x, y)))
            .find_map(|(x, y)| match m.pixel(x, y)? {
                w @ PixelWinnerInfo::Background { layer: 1, .. } => Some(w),
                _ => None,
            })
            .expect("BG1 draws somewhere");
        assert_eq!(m.reveal(Reveal::Cell, &winner), Some(Tab::Tilemap));
        assert_eq!(m.background_layer, 1);
        let cell = m.selected_cell.expect("the cell is found");
        assert!(m.cells().get(cell).is_some());
    }

    #[test]
    fn the_tilemap_follows_the_layer_the_registers_name() {
        let mut m = recorded();
        m.set_frame(8);
        assert_eq!(m.map_cells(), (32, 32));
        let layer = m.current_layer().expect("BG1 exists in mode 1");
        assert_eq!(layer.bg, 1);
        assert_eq!(m.cells().len(), 32 * 32);
        assert!(m.layer_image().is_some());
        assert!(m.sprite_image(3).is_some());
        assert!(m.frame_layers().is_some_and(|l| !l.layers.is_empty()));
        assert!(m.frame_layer(1).is_some());
        // CGRAM rows colour the tile decoder.
        m.palette = PaletteChoice::Cgram(8);
        assert_eq!(m.palette_rgb(16).len(), 16);
    }

    impl GraphicsModel {
        fn rom_bytes_for_test(&self, at: u32, len: u32) -> Vec<u8> {
            self.rom.bytes(at, len)
        }
    }
}
