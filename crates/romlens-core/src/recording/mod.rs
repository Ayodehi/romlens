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

pub mod change_index;
pub mod conformance;
pub mod delta;
pub mod fixtures;
pub mod format;
pub mod import;
pub mod io_state;
pub mod memory;
pub mod mesen;
pub mod reader;
pub mod validate;
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
}

impl StateRegion {
    pub const ALL: [StateRegion; 8] = [
        StateRegion::CpuRegisters,
        StateRegion::PpuState,
        StateRegion::IoState,
        StateRegion::Wram,
        StateRegion::Vram,
        StateRegion::Cgram,
        StateRegion::Oam,
        StateRegion::Timing,
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

/// Which optional layers a source carries. Phase 2 produces none of them; the
/// format reserves them and the reader skips them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Layers {
    pub framebuffer: bool,
    pub write_log: bool,
    pub trace: bool,
    pub read_log: bool,
}

impl Layers {
    pub const fn bits(self) -> u32 {
        self.framebuffer as u32
            | (self.write_log as u32) << 1
            | (self.trace as u32) << 2
            | (self.read_log as u32) << 3
    }

    pub const fn from_bits(bits: u32) -> Self {
        Layers {
            framebuffer: bits & 1 != 0,
            write_log: bits & 2 != 0,
            trace: bits & 4 != 0,
            read_log: bits & 8 != 0,
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
}
