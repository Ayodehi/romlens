//! Recordings: frame-by-frame machine state from outside the app
//! (`13-recording-format.md`).
//!
//! [`MachineStateSource`] is frozen here so Phase 3's views and Phase 5's
//! embedded core need not redesign it. It deviates from the docs/13 sketch
//! four ways, on purpose (`16-phase2-plan.md` 2C.7): every call returns a
//! `Result`, because a recording can be truncated or mid-write; `regions()`
//! says what is present, so a savestate import without WRAM is "absent"
//! rather than "all zeroes"; `identity()` lets a caller refuse a recording
//! of a different ROM; and live control (stepping, breakpoints, poking)
//! belongs to a separate `LiveSource` later, so this trait stays small,
//! read-only and object-safe.

pub mod apu;
pub mod change_index;
pub mod conformance;
pub mod delta;
pub mod fixtures;
pub mod format;
pub mod import;
pub mod io_state;
pub mod lines;
pub mod live;
pub mod memory;
pub mod mesen;
pub mod reader;
pub mod validate;
pub mod wlog;
pub mod writer;

use std::collections::BTreeMap;

pub use delta::Run;
pub use memory::MemorySource;
pub use reader::RomrecSource;
pub use writer::RomrecWriter;

use crate::graphics::ppu_state::PpuState;

/// One region of machine state. Named `StateRegion` because
/// `model::region::Region` is taken, and renaming after the format is
/// published would be expensive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StateRegion {
    /// A, X, Y, S, D, DB, PB, PC, P, E at the frame boundary: 16 bytes.
    CpuRegisters,
    /// The write-only PPU registers ([`PpuState`]): 256 bytes.
    PpuState,
    /// NMITIMEN, HDMAEN, the DMA channels and the rest of `$42xx`/`$43xx`:
    /// 128 bytes.
    IoState,
    Wram,
    Vram,
    Cgram,
    Oam,
    /// Frame index, master clock, interlace: 16 bytes.
    Timing,
    /// The sound CPU's 64 KB of RAM (docs/23): driver, songs, samples, echo.
    Aram,
    /// The S-DSP's 128 registers, as the SPC700 reads them.
    DspRegisters,
    /// The SPC700's registers, ports, timers and clock ([`SpcState`]): 32 bytes.
    SpcState,
}

impl StateRegion {
    pub const ALL: [StateRegion; 11] = [
        StateRegion::CpuRegisters,
        StateRegion::PpuState,
        StateRegion::IoState,
        StateRegion::Wram,
        StateRegion::Vram,
        StateRegion::Cgram,
        StateRegion::Oam,
        StateRegion::Timing,
        StateRegion::Aram,
        StateRegion::DspRegisters,
        StateRegion::SpcState,
    ];

    /// The main machine's regions, every recording's before the audio layer.
    pub const MAIN: [StateRegion; 8] = [
        StateRegion::CpuRegisters,
        StateRegion::PpuState,
        StateRegion::IoState,
        StateRegion::Wram,
        StateRegion::Vram,
        StateRegion::Cgram,
        StateRegion::Oam,
        StateRegion::Timing,
    ];

    /// The audio layer's regions (docs/23, A4).
    pub const AUDIO: [StateRegion; 3] = [
        StateRegion::Aram,
        StateRegion::DspRegisters,
        StateRegion::SpcState,
    ];

    /// The id written in the file's region table.
    pub const fn id(self) -> u32 {
        match self {
            StateRegion::CpuRegisters => 0,
            StateRegion::PpuState => 1,
            StateRegion::IoState => 2,
            StateRegion::Wram => 3,
            StateRegion::Vram => 4,
            StateRegion::Cgram => 5,
            StateRegion::Oam => 6,
            StateRegion::Timing => 7,
            StateRegion::Aram => 8,
            StateRegion::DspRegisters => 9,
            StateRegion::SpcState => 10,
        }
    }

    pub fn from_id(id: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.id() == id)
    }

    pub const fn name(self) -> &'static str {
        match self {
            StateRegion::CpuRegisters => "cpu",
            StateRegion::PpuState => "ppu",
            StateRegion::IoState => "io",
            StateRegion::Wram => "wram",
            StateRegion::Vram => "vram",
            StateRegion::Cgram => "cgram",
            StateRegion::Oam => "oam",
            StateRegion::Timing => "timing",
            StateRegion::Aram => "aram",
            StateRegion::DspRegisters => "dsp",
            StateRegion::SpcState => "spc",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.name() == name)
    }

    pub const fn size(self) -> usize {
        match self {
            StateRegion::CpuRegisters => 16,
            StateRegion::PpuState => 256,
            StateRegion::IoState => 128,
            StateRegion::Wram => 0x20000,
            StateRegion::Vram => 0x10000,
            StateRegion::Cgram => 512,
            StateRegion::Oam => 544,
            StateRegion::Timing => 16,
            StateRegion::Aram => 0x10000,
            StateRegion::DspRegisters => 128,
            StateRegion::SpcState => 32,
        }
    }

    /// Small records are stored whole in every frame: they are tiny, and
    /// random access to them must be instant (docs/13).
    pub const fn always_whole(self) -> bool {
        self.size() <= 256
    }
}

/// The CPU register block, 16 bytes:
///
/// | Offset | Size | Register |
/// |---|---|---|
/// | 0 | 2 | A (C) |
/// | 2 | 2 | X |
/// | 4 | 2 | Y |
/// | 6 | 2 | S |
/// | 8 | 2 | D |
/// | 10 | 1 | DB |
/// | 11 | 1 | PB |
/// | 12 | 2 | PC |
/// | 14 | 1 | P |
/// | 15 | 1 | E (0 native, 1 emulation) |
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuRegisters {
    pub a: u16,
    pub x: u16,
    pub y: u16,
    pub s: u16,
    pub d: u16,
    pub db: u8,
    pub pb: u8,
    pub pc: u16,
    pub p: u8,
    pub e: bool,
}

impl CpuRegisters {
    pub fn encode(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        for (i, v) in [self.a, self.x, self.y, self.s, self.d].iter().enumerate() {
            b[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
        }
        b[10] = self.db;
        b[11] = self.pb;
        b[12..14].copy_from_slice(&self.pc.to_le_bytes());
        b[14] = self.p;
        b[15] = self.e as u8;
        b
    }

    pub fn decode(b: &[u8]) -> Self {
        let mut p = [0u8; 16];
        let n = b.len().min(16);
        p[..n].copy_from_slice(&b[..n]);
        let w = |i: usize| u16::from_le_bytes([p[i], p[i + 1]]);
        CpuRegisters {
            a: w(0),
            x: w(2),
            y: w(4),
            s: w(6),
            d: w(8),
            db: p[10],
            pb: p[11],
            pc: w(12),
            p: p[14],
            e: p[15] & 1 != 0,
        }
    }
}

/// The SPC700 at a frame's end, 32 bytes:
///
/// | Offset | Size | What |
/// |---|---|---|
/// | 0 | 5 | A, X, Y, SP, PSW |
/// | 5 | 2 | PC |
/// | 7 | 4 | the ports as the SPC700 reads them: the S-CPU's last bytes |
/// | 11 | 4 | the ports as the S-CPU reads them: the SPC700's last bytes |
/// | 15 | 2 | AUXIO4, AUXIO5 |
/// | 17 | 1 | DSPADDR |
/// | 18 | 1 | bit 0 the boot ROM mapped, bits 2-4 timers 0-2 on |
/// | 19 | 3 | the timers' dividers (0 is 256) |
/// | 22 | 2 | the timers' 4-bit counts: 0 and 1, then 2 |
/// | 24 | 8 | the SPC700's cycle count, 1.024 MHz |
///
/// Mesen runs the SPC700 behind the main CPU and catches it up at port
/// accesses and the frame's end, after the recorder reads it; `cycle` says
/// how far it had run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpcState {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub psw: u8,
    pub pc: u16,
    pub from_cpu: [u8; 4],
    pub to_cpu: [u8; 4],
    pub aux: [u8; 2],
    pub dspaddr: u8,
    pub rom_enabled: bool,
    pub timers_on: [bool; 3],
    pub dividers: [u8; 3],
    pub counts: [u8; 3],
    pub cycle: u64,
}

impl SpcState {
    pub fn encode(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[..5].copy_from_slice(&[self.a, self.x, self.y, self.sp, self.psw]);
        b[5..7].copy_from_slice(&self.pc.to_le_bytes());
        b[7..11].copy_from_slice(&self.from_cpu);
        b[11..15].copy_from_slice(&self.to_cpu);
        b[15..17].copy_from_slice(&self.aux);
        b[17] = self.dspaddr;
        b[18] = self.rom_enabled as u8
            | (self.timers_on[0] as u8) << 2
            | (self.timers_on[1] as u8) << 3
            | (self.timers_on[2] as u8) << 4;
        b[19..22].copy_from_slice(&self.dividers);
        b[22] = self.counts[0] & 0xF | (self.counts[1] & 0xF) << 4;
        b[23] = self.counts[2] & 0xF;
        b[24..32].copy_from_slice(&self.cycle.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Self {
        let mut p = [0u8; 32];
        let n = b.len().min(32);
        p[..n].copy_from_slice(&b[..n]);
        SpcState {
            a: p[0],
            x: p[1],
            y: p[2],
            sp: p[3],
            psw: p[4],
            pc: u16::from_le_bytes([p[5], p[6]]),
            from_cpu: p[7..11].try_into().unwrap(),
            to_cpu: p[11..15].try_into().unwrap(),
            aux: [p[15], p[16]],
            dspaddr: p[17],
            rom_enabled: p[18] & 1 != 0,
            timers_on: [p[18] & 4 != 0, p[18] & 8 != 0, p[18] & 16 != 0],
            dividers: [p[19], p[20], p[21]],
            counts: [p[22] & 0xF, p[22] >> 4, p[23] & 0xF],
            cycle: u64::from_le_bytes(p[24..32].try_into().unwrap()),
        }
    }
}

/// Which optional layers a source carries. Phase 2 produces none of them; the
/// format reserves them and the reader skips them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Layers {
    pub framebuffer: bool,
    pub write_log: bool,
    pub trace: bool,
    pub read_log: bool,
    /// PPU register writes by scanline (`LINE`).
    pub line_writes: bool,
    /// The two CPUs' port writes and the SPC700's I/O writes, the DSP's
    /// among them, each on the SPC700's clock (`APUL`, docs/23).
    pub apu_events: bool,
}

impl Layers {
    pub const fn bits(self) -> u32 {
        self.framebuffer as u32
            | (self.write_log as u32) << 1
            | (self.trace as u32) << 2
            | (self.read_log as u32) << 3
            | (self.line_writes as u32) << 4
            | (self.apu_events as u32) << 5
    }

    pub const fn from_bits(bits: u32) -> Self {
        Layers {
            framebuffer: bits & 1 != 0,
            write_log: bits & 2 != 0,
            trace: bits & 4 != 0,
            read_log: bits & 8 != 0,
            line_writes: bits & 16 != 0,
            apu_events: bits & 32 != 0,
        }
    }
}

/// Which ROM a recording was made from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingIdentity {
    pub rom_sha256: [u8; 32],
    pub producer: String,
    pub producer_version: String,
}

impl RecordingIdentity {
    pub fn sha256_hex(&self) -> String {
        self.rom_sha256.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// The machine at one frame boundary: every region the source has.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MachineState {
    pub frame: u64,
    pub regions: BTreeMap<StateRegion, Vec<u8>>,
}

impl MachineState {
    pub fn region(&self, r: StateRegion) -> Option<&[u8]> {
        self.regions.get(&r).map(Vec::as_slice)
    }
    pub fn vram(&self) -> Option<&[u8]> {
        self.region(StateRegion::Vram)
    }
    pub fn cgram(&self) -> Option<&[u8]> {
        self.region(StateRegion::Cgram)
    }
    pub fn oam(&self) -> Option<&[u8]> {
        self.region(StateRegion::Oam)
    }
    pub fn ppu(&self) -> Option<PpuState> {
        self.region(StateRegion::PpuState).map(PpuState::from_bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecordingError {
    #[error("could not read the recording: {0}")]
    Io(String),
    #[error("not a Romlens recording: {0}")]
    BadFormat(String),
    #[error("the recording is damaged: {0}")]
    Corrupt(String),
    #[error("the recording was written by a newer Romlens (format {0}.{1})")]
    NewerVersion(u16, u16),
    #[error("frame {frame} is past the end of the recording ({count} frames)")]
    NoSuchFrame { frame: u64, count: u64 },
    #[error("frame {frame} is no longer held by the live session, which keeps frames from {first}")]
    Dropped { frame: u64, first: u64 },
    #[error("the recording has no {0} region")]
    MissingRegion(&'static str),
    #[error(
        "the recording was made from a different ROM (expected SHA-256 {expected}, found {found})"
    )]
    RomMismatch { expected: String, found: String },
}

impl From<std::io::Error> for RecordingError {
    fn from(e: std::io::Error) -> Self {
        RecordingError::Io(e.to_string())
    }
}

/// A frame-by-frame view of a machine. `.romrec` files are one
/// implementation; [`MemorySource`] is the test double; the Phase 5 core
/// will be another.
pub trait MachineStateSource {
    fn identity(&self) -> &RecordingIdentity;
    /// `None` for a live source that has no end yet.
    fn frame_count(&self) -> Option<u64>;
    /// The regions every frame carries, in id order.
    fn regions(&self) -> Vec<StateRegion>;
    fn state_at(&self, frame: u64) -> Result<MachineState, RecordingError>;
    /// A single region at a frame; defaults to [`state_at`](Self::state_at),
    /// which an implementation may beat.
    fn region_at(&self, frame: u64, region: StateRegion) -> Result<Vec<u8>, RecordingError> {
        self.state_at(frame)?
            .regions
            .remove(&region)
            .ok_or(RecordingError::MissingRegion(region.name()))
    }
    /// Byte ranges of `region` that may differ between frames `from` and
    /// `to` (`from < to`): a superset of the bytes that actually differ, in
    /// offset order, never overlapping. Bytes outside them are equal.
    fn changes(&self, from: u64, to: u64, region: StateRegion) -> Result<Vec<Run>, RecordingError>;
    fn layers(&self) -> Layers;
    /// The PPU register writes made while `frame` was drawn, on their
    /// scanlines (`lines`); `None` where the source has none for it.
    fn line_writes(&self, _frame: u64) -> Result<Option<Vec<lines::RegWrite>>, RecordingError> {
        Ok(None)
    }
    /// The general DMA transfers started in `frame` (`wlog`).
    fn dma_records(&self, _frame: u64) -> Result<Vec<wlog::DmaRecord>, RecordingError> {
        Ok(Vec::new())
    }
    /// The sound side's events during `frame` (`apu`); `None` where the
    /// source has none for it.
    fn apu_events(&self, _frame: u64) -> Result<Option<apu::ApuEvents>, RecordingError> {
        Ok(None)
    }
}
