//! `rec pack`: a recorder stream to a `.romrec` recording.
//!
//! The register blocks are built here, not in Lua. Mesen's `getState()`
//! holds the PPU registers decoded (`ppu.bgMode`, `ppu.layers[0].chrAddress`
//! …), so each one-write register is re-encoded from those fields, the
//! inverse of `SnesPpu::Write`. That works from the first frame, even for a
//! recording that starts from a savestate, where the game wrote its
//! registers long before the script could see them. Three registers have
//! fields Mesen does not export (`WBGLOG`, `WOBJLOG`, `CGWSEL`'s window
//! bits); for those the last byte the game wrote is used, and until it
//! writes one the register is reported as unknown and stored as zero.

use std::collections::HashMap;
use std::io::{Read, Seek, Write};

use crate::graphics::ppu_state::PpuState;
use crate::memory::map::MappingMode;
use crate::recording::apu::ApuEvents;
use crate::recording::format::LAYER_MAGICS;
use crate::recording::io_state::{self, IoState};
use crate::recording::mesen::stream::{
    ARAM_LEN, ApuRecord, AudioRecord, BLOCK, DSP_LEN, DmaEvent, FrameRecord, MEMORY, PPU_PORTS,
    Record, StreamError, StreamHeader, StreamReader,
};
use crate::recording::writer::{RomrecWriter, WriterOptions};
use crate::recording::{
    CpuRegisters, Layers, MachineState, RecordingError, RecordingIdentity, SpcState, StateRegion,
};
use crate::rom::image::RomImage;

/// How much WRAM the recording keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WramMode {
    /// Every frame's changes.
    Full,
    /// Keyframes only (the header flag), the default: Phase 2 needs WRAM for
    /// nothing, and it is most of a recording's size.
    #[default]
    Keyframe,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackOptions {
    pub wram: WramMode,
    pub keyframe_interval: u16,
    pub compress: bool,
}

impl Default for PackOptions {
    fn default() -> Self {
        PackOptions {
            wram: WramMode::default(),
            keyframe_interval: 60,
            compress: true,
        }
    }
}

/// What packing found, for the command to print.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackReport {
    pub frames: u64,
    pub dma_events: u64,
    /// PPU register writes placed on their scanlines (stream version 2).
    pub line_writes: u64,
    /// The sound side's events: port writes both ways and the SPC700's I/O
    /// writes (stream version 3).
    pub apu_events: u64,
    /// Of those, writes to the DSP's registers.
    pub dsp_writes: u64,
    pub state_loads: u64,
    /// The stream ended without its end record: the emulator closed
    /// mid-recording. Every whole frame was kept.
    pub truncated: bool,
    /// Fields the mapping reads that the stream lacks; their bits are zero.
    pub missing_fields: Vec<&'static str>,
    /// PPU registers that neither Mesen's state nor a write ever gave a
    /// value, stored as zero.
    pub unknown_registers: Vec<u16>,
    /// Frames where a register rebuilt from Mesen's state disagreed with the
    /// byte the game last wrote to it, in the bits that register keeps: a
    /// check on the rebuilding itself, which should never fire. The first
    /// few are kept as (register, frame, rebuilt, written).
    pub disagreements: u64,
    pub first_disagreements: Vec<(u16, u64, u8, u8)>,
    pub producer: String,
    pub created: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PackError {
    #[error(transparent)]
    Stream(#[from] StreamError),
    #[error(transparent)]
    Recording(#[from] RecordingError),
    #[error("the stream was not recorded from this ROM: {0}")]
    RomMismatch(String),
    #[error("the stream has no frames")]
    Empty,
}

/// Every `getState()` field the mapping reads.
const FIELDS: &[&str] = &[
    "cpu.a",
    "cpu.x",
    "cpu.y",
    "cpu.sp",
    "cpu.d",
    "cpu.dbr",
    "cpu.k",
    "cpu.pc",
    "cpu.ps",
    "cpu.emulationMode",
    "ppu.forcedBlank",
    "ppu.screenBrightness",
    "ppu.oamMode",
    "ppu.oamBaseAddress",
    "ppu.oamAddressOffset",
    "ppu.oamRamAddress",
    "ppu.enableOamPriority",
    "ppu.bgMode",
    "ppu.mode1Bg3Priority",
    "ppu.mosaicSize",
    "ppu.mosaicEnabled",
    "ppu.vramIncrementValue",
    "ppu.vramAddressRemapping",
    "ppu.vramAddrIncrementOnSecondReg",
    "ppu.vramAddress",
    "ppu.cgramAddress",
    "ppu.mainScreenLayers",
    "ppu.subScreenLayers",
    "ppu.colorMathAddSubscreen",
    "ppu.directColorMode",
    "ppu.colorMathEnabled",
    "ppu.colorMathSubtractMode",
    "ppu.colorMathHalveResult",
    "ppu.fixedColor",
    "ppu.extBgEnabled",
    "ppu.hiResMode",
    "ppu.overscanMode",
    "ppu.objInterlace",
    "ppu.screenInterlace",
    "ppu.mode7.largeMap",
    "ppu.mode7.fillWithTile0",
    "ppu.mode7.horizontalMirroring",
    "ppu.mode7.verticalMirroring",
    "ppu.mode7.hscroll",
    "ppu.mode7.vscroll",
    "ppu.mode7.centerX",
    "ppu.mode7.centerY",
    "ppu.mode7.matrix[0]",
    "ppu.mode7.matrix[1]",
    "ppu.mode7.matrix[2]",
    "ppu.mode7.matrix[3]",
    "internalRegisters.enableNmi",
    "internalRegisters.enableVerticalIrq",
    "internalRegisters.enableHorizontalIrq",
    "internalRegisters.enableAutoJoypadRead",
    "internalRegisters.enableFastRom",
    "internalRegisters.ioPortOutput",
    "internalRegisters.horizontalTimer",
    "internalRegisters.verticalTimer",
    "internalRegisters.hCounter",
    "internalRegisters.vCounter",
    "internalRegisters.aluMulDiv.multOperand1",
    "internalRegisters.aluMulDiv.multOperand2",
    "internalRegisters.aluMulDiv.dividend",
    "internalRegisters.aluMulDiv.divisor",
    "dmaController.hdmaChannels",
];

/// The per-layer and per-window fields, generated so the list above stays
/// readable.
fn indexed_fields() -> Vec<String> {
    let mut v = Vec::new();
    for i in 0..4 {
        for f in [
            "largeTiles",
            "tilemapAddress",
            "doubleWidth",
            "doubleHeight",
            "chrAddress",
            "hscroll",
            "vscroll",
        ] {
            v.push(format!("ppu.layers[{i}].{f}"));
        }
        v.push(format!("internalRegisters.controllerData[{i}]"));
    }
    for w in 0..2 {
        for l in 0..6 {
            v.push(format!("ppu.window[{w}].activeLayers[{l}]"));
            v.push(format!("ppu.window[{w}].invertedLayers[{l}]"));
        }
        v.push(format!("ppu.window[{w}].left"));
        v.push(format!("ppu.window[{w}].right"));
    }
    for i in 0..5 {
        v.push(format!("ppu.windowMaskMain[{i}]"));
        v.push(format!("ppu.windowMaskSub[{i}]"));
    }
    v
}

/// The current value of every field, by name.
struct Fields {
    index: HashMap<String, usize>,
    values: Vec<i64>,
}

impl Fields {
    fn new(header: &StreamHeader) -> Self {
        Fields {
            index: header
                .fields
                .iter()
                .enumerate()
                .map(|(i, k)| (k.clone(), i))
                .collect(),
            values: vec![0; header.fields.len()],
        }
    }

    fn apply(&mut self, changes: &[(u16, i64)]) {
        for &(i, v) in changes {
            self.values[i as usize] = v;
        }
    }

    fn has(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    fn get(&self, name: &str) -> Option<i64> {
        self.index.get(name).map(|&i| self.values[i])
    }

    /// A field as an integer, zero when absent (reported once, up front).
    fn n(&self, name: &str) -> i64 {
        self.get(name).unwrap_or(0)
    }

    fn b(&self, name: &str) -> u8 {
        (self.n(name) != 0) as u8
    }
}

/// The SPC700 from Mesen's `spc.*` fields (stream version 3); `cycle` is
/// the audio record's, read at the same moment.
fn spc_state(f: &Fields, cycle: u64) -> SpcState {
    let b = |n: &str| f.n(n) as u8;
    SpcState {
        a: b("spc.a"),
        x: b("spc.x"),
        y: b("spc.y"),
        sp: b("spc.sp"),
        psw: b("spc.ps"),
        pc: f.n("spc.pc") as u16,
        from_cpu: std::array::from_fn(|i| b(&format!("spc.cpuRegs[{i}]"))),
        to_cpu: std::array::from_fn(|i| b(&format!("spc.outputReg[{i}]"))),
        aux: [b("spc.ramReg[0]"), b("spc.ramReg[1]")],
        dspaddr: b("spc.dspReg"),
        rom_enabled: f.n("spc.romEnabled") != 0,
        timers_on: std::array::from_fn(|i| f.n(&format!("spc.timer{i}.enabled")) != 0),
        dividers: std::array::from_fn(|i| b(&format!("spc.timer{i}.target"))),
        counts: std::array::from_fn(|i| b(&format!("spc.timer{i}.output")) & 0xF),
        cycle,
    }
}

fn cpu_registers(f: &Fields) -> CpuRegisters {
    CpuRegisters {
        a: f.n("cpu.a") as u16,
        x: f.n("cpu.x") as u16,
        y: f.n("cpu.y") as u16,
        s: f.n("cpu.sp") as u16,
        d: f.n("cpu.d") as u16,
        db: f.n("cpu.dbr") as u8,
        pb: f.n("cpu.k") as u8,
        pc: f.n("cpu.pc") as u16,
        p: f.n("cpu.ps") as u8,
        e: f.n("cpu.emulationMode") != 0,
    }
}

/// The byte a game would have written to `port` to leave the PPU in the
/// state `f` describes; `None` for the data ports, the two-write registers
/// (their built values have their own slots) and the registers Mesen does
/// not export.
fn encode_port(port: u16, f: &Fields) -> Option<u8> {
    let layer = |i: u16, name: &str| f.n(&format!("ppu.layers[{i}].{name}"));
    let window_bits = |first: u16| -> u8 {
        // ProcessWindowMaskSettings: per layer pair, window 1 invert/enable
        // in bits 0/1 and 4/5, window 2 in bits 2/3 and 6/7.
        let bit = |w: u16, what: &str, l: u16| f.b(&format!("ppu.window[{w}].{what}[{l}]"));
        bit(0, "invertedLayers", first)
            | bit(0, "activeLayers", first) << 1
            | bit(1, "invertedLayers", first) << 2
            | bit(1, "activeLayers", first) << 3
            | bit(0, "invertedLayers", first + 1) << 4
            | bit(0, "activeLayers", first + 1) << 5
            | bit(1, "invertedLayers", first + 1) << 6
            | bit(1, "activeLayers", first + 1) << 7
    };
    let mask_bits = |which: &str| -> u8 {
        (0..5)
            .map(|i| f.b(&format!("ppu.{which}[{i}]")) << i)
            .fold(0, |a, b| a | b)
    };
    let v: i64 = match port {
        0x2100 => (f.b("ppu.forcedBlank") as i64) << 7 | (f.n("ppu.screenBrightness") & 0x0F),
        0x2101 => {
            let size_gap = ((f.n("ppu.oamAddressOffset") >> 12) - 1) & 3;
            (f.n("ppu.oamMode") & 7) << 5 | size_gap << 3 | (f.n("ppu.oamBaseAddress") >> 13) & 7
        }
        0x2102 => f.n("ppu.oamRamAddress") & 0xFF,
        0x2103 => (f.b("ppu.enableOamPriority") as i64) << 7 | (f.n("ppu.oamRamAddress") >> 8) & 1,
        0x2105 => {
            let mut v = f.n("ppu.bgMode") & 7 | (f.b("ppu.mode1Bg3Priority") as i64) << 3;
            for i in 0..4 {
                v |= (layer(i, "largeTiles") & 1) << (4 + i);
            }
            v
        }
        0x2106 => (f.n("ppu.mosaicSize") - 1).clamp(0, 15) << 4 | f.n("ppu.mosaicEnabled") & 0x0F,
        0x2107..=0x210A => {
            let i = port - 0x2107;
            (layer(i, "tilemapAddress") >> 8) & 0x7C
                | layer(i, "doubleWidth") & 1
                | (layer(i, "doubleHeight") & 1) << 1
        }
        0x210B | 0x210C => {
            let i = (port - 0x210B) * 2;
            (layer(i, "chrAddress") >> 12) & 0x07 | (layer(i + 1, "chrAddress") >> 8) & 0x70
        }
        0x2115 => {
            let step = match f.n("ppu.vramIncrementValue") {
                1 => 0,
                32 => 1,
                _ => 2,
            };
            step | (f.n("ppu.vramAddressRemapping") & 3) << 2
                | (f.b("ppu.vramAddrIncrementOnSecondReg") as i64) << 7
        }
        0x2116 => f.n("ppu.vramAddress") & 0xFF,
        0x2117 => (f.n("ppu.vramAddress") >> 8) & 0x7F,
        0x211A => {
            (f.b("ppu.mode7.largeMap") as i64) << 7
                | (f.b("ppu.mode7.fillWithTile0") as i64) << 6
                | (f.b("ppu.mode7.verticalMirroring") as i64) << 1
                | f.b("ppu.mode7.horizontalMirroring") as i64
        }
        0x2121 => f.n("ppu.cgramAddress") & 0xFF,
        0x2123 => window_bits(0) as i64,
        0x2124 => window_bits(2) as i64,
        0x2125 => window_bits(4) as i64,
        0x2126 => f.n("ppu.window[0].left"),
        0x2127 => f.n("ppu.window[0].right"),
        0x2128 => f.n("ppu.window[1].left"),
        0x2129 => f.n("ppu.window[1].right"),
        0x212A => {
            let logic = (0..4)
                .map(|i| f.get(&format!("ppu.maskLogic[{i}]")))
                .collect::<Option<Vec<i64>>>()?;
            logic
                .iter()
                .enumerate()
                .map(|(i, l)| (l & 3) << (2 * i))
                .sum()
        }
        0x212B => {
            let logic = (4..6)
                .map(|i| f.get(&format!("ppu.maskLogic[{i}]")))
                .collect::<Option<Vec<i64>>>()?;
            logic[0] & 3 | (logic[1] & 3) << 2
        }
        0x212C => f.n("ppu.mainScreenLayers") & 0x1F,
        0x212D => f.n("ppu.subScreenLayers") & 0x1F,
        0x212E => mask_bits("windowMaskMain") as i64,
        0x212F => mask_bits("windowMaskSub") as i64,
        0x2130 => {
            let clip = f.get("ppu.colorMathClipMode")?;
            let prevent = f.get("ppu.colorMathPreventMode")?;
            (clip & 3) << 6
                | (prevent & 3) << 4
                | (f.b("ppu.colorMathAddSubscreen") as i64) << 1
                | f.b("ppu.directColorMode") as i64
        }
        0x2131 => {
            f.n("ppu.colorMathEnabled") & 0x3F
                | (f.b("ppu.colorMathHalveResult") as i64) << 6
                | (f.b("ppu.colorMathSubtractMode") as i64) << 7
        }
        0x2133 => {
            (f.b("ppu.extBgEnabled") as i64) << 6
                | (f.b("ppu.hiResMode") as i64) << 3
                | (f.b("ppu.overscanMode") as i64) << 2
                | (f.b("ppu.objInterlace") as i64) << 1
                | f.b("ppu.screenInterlace") as i64
        }
        _ => return None,
    };
    Some(v as u8)
}

/// The bits of `port` the PPU keeps, for checking a rebuilt value against
/// the byte written; `None` where the two legitimately differ. The address
/// registers move on every data write, so their state is not the byte that
/// set them.
fn kept_bits(port: u16) -> Option<u8> {
    Some(match port {
        0x2100 => 0x8F,
        0x2103 => 0x81,
        0x2107..=0x210A => 0x7F,
        0x210B | 0x210C => 0x77,
        // VMAIN: increments 2 and 3 are the same step.
        0x2115 => 0x8E,
        0x211A => 0xC3,
        0x212C..=0x212F => 0x1F,
        0x2133 => 0x4F,
        0x2101 | 0x2102 | 0x2105 | 0x2106 | 0x2123..=0x212B | 0x2130 | 0x2131 => 0xFF,
        _ => return None,
    })
}

/// Ports with no single-write value to keep: the data ports and the
/// two-write registers, whose built values have their own slots.
fn port_has_no_value(port: u16) -> bool {
    matches!(port, 0x2104 | 0x210D..=0x2114 | 0x2118 | 0x2119 | 0x211B..=0x2120 | 0x2122 | 0x2132)
}

fn ppu_state(f: &Fields, record: &FrameRecord, report: &mut PackReport) -> PpuState {
    let mut s = PpuState::default();
    let unknown = &mut report.unknown_registers;
    for i in 0..PPU_PORTS {
        let port = 0x2100 + i as u16;
        let value = match encode_port(port, f) {
            Some(v) => {
                if let (true, Some(mask)) = (record.seen[i], kept_bits(port))
                    && (v ^ record.ports[i]) & mask != 0
                {
                    report.disagreements += 1;
                    if report.first_disagreements.len() < 8 {
                        report.first_disagreements.push((
                            port,
                            record.frame as u64,
                            v,
                            record.ports[i],
                        ));
                    }
                }
                v
            }
            None if record.seen[i] => record.ports[i],
            None if port_has_no_value(port) => 0,
            None => {
                if !unknown.contains(&port) {
                    unknown.push(port);
                }
                0
            }
        };
        s.set_register(port, value);
    }
    let mode7 = f.n("ppu.bgMode") == 7;
    for bg in 0..4u8 {
        let (h, v) = if bg == 0 && mode7 {
            // BG1's scroll ports are M7HOFS/M7VOFS in Mode 7, 13 bits wide.
            (f.n("ppu.mode7.hscroll"), f.n("ppu.mode7.vscroll"))
        } else {
            (
                f.n(&format!("ppu.layers[{bg}].hscroll")),
                f.n(&format!("ppu.layers[{bg}].vscroll")),
            )
        };
        s.set_scroll(bg + 1, false, h as u16);
        s.set_scroll(bg + 1, true, v as u16);
    }
    for i in 0..4 {
        s.set_mode7(i, f.n(&format!("ppu.mode7.matrix[{i}]")) as i16);
    }
    s.set_mode7(4, f.n("ppu.mode7.centerX") as i16);
    s.set_mode7(5, f.n("ppu.mode7.centerY") as i16);
    s.set_oam_address(
        (f.n("ppu.oamRamAddress") & 0x1FF) as u16 | (f.b("ppu.enableOamPriority") as u16) << 15,
    );
    s.set_vram_address(f.n("ppu.vramAddress") as u16);
    s.set_cgram_address(f.n("ppu.cgramAddress") as u8);
    s.set_fixed_colour(f.n("ppu.fixedColor") as u16);
    s
}

fn io_state(f: &Fields, record: &FrameRecord) -> IoState {
    let mut s = IoState::default();
    s.set_u8(
        io_state::NMITIMEN,
        f.b("internalRegisters.enableNmi") << 7
            | f.b("internalRegisters.enableVerticalIrq") << 5
            | f.b("internalRegisters.enableHorizontalIrq") << 4
            | f.b("internalRegisters.enableAutoJoypadRead"),
    );
    s.set_u8(io_state::HDMAEN, f.n("dmaController.hdmaChannels") as u8);
    s.set_u8(io_state::MEMSEL, f.b("internalRegisters.enableFastRom"));
    s.set_u8(io_state::WRIO, f.n("internalRegisters.ioPortOutput") as u8);
    s.set_u16(
        io_state::HTIME,
        f.n("internalRegisters.horizontalTimer") as u16,
    );
    s.set_u16(
        io_state::VTIME,
        f.n("internalRegisters.verticalTimer") as u16,
    );
    s.set_u8(
        io_state::WRMPYA,
        f.n("internalRegisters.aluMulDiv.multOperand1") as u8,
    );
    s.set_u8(
        io_state::WRMPYB,
        f.n("internalRegisters.aluMulDiv.multOperand2") as u8,
    );
    s.set_u16(
        io_state::WRDIV,
        f.n("internalRegisters.aluMulDiv.dividend") as u16,
    );
    s.set_u8(
        io_state::WRDIVB,
        f.n("internalRegisters.aluMulDiv.divisor") as u8,
    );
    s.set_u16(
        io_state::H_COUNTER,
        f.n("internalRegisters.hCounter") as u16,
    );
    s.set_u16(
        io_state::V_COUNTER,
        f.n("internalRegisters.vCounter") as u16,
    );
    for i in 0..4 {
        let pad = f.n(&format!("internalRegisters.controllerData[{i}]")) as u16;
        s.set_u16(io_state::JOY + i * 2, pad);
    }
    s.set_dma(&record.dma);
    s
}

fn wlog_body(frame: u64, events: &[DmaEvent]) -> Vec<u8> {
    use crate::recording::wlog::{DmaRecord, channels_from, encode};
    let records: Vec<DmaRecord> = events
        .iter()
        .map(|e| DmaRecord {
            value: e.value,
            scanline: e.scanline,
            channels: channels_from(&e.registers),
            context: e.context,
        })
        .collect();
    encode(frame, &records)
}

fn mapping_byte(m: MappingMode) -> u8 {
    match m {
        MappingMode::LoRom => 0,
        MappingMode::HiRom => 1,
        MappingMode::ExHiRom => 2,
    }
}

/// Pack a recorder stream made while running `rom`.
/// Turns a stream's records into machine states one frame at a time: what
/// [`pack`] writes to a file, and what a live session holds in memory.
pub struct StreamDecoder {
    fields: Fields,
    memory: Vec<Vec<u8>>,
    aram: Vec<u8>,
    dsp: [u8; DSP_LEN],
    spc_cycle: u64,
    /// `DSPADDR` after the last events, for naming the next frame's first
    /// DSP writes.
    dspaddr: u8,
    pub report: PackReport,
}

impl StreamDecoder {
    pub fn new(header: &StreamHeader) -> Self {
        let fields = Fields::new(header);
        let mut report = PackReport {
            producer: header.producer.clone(),
            created: header.created,
            missing_fields: FIELDS.iter().copied().filter(|n| !fields.has(n)).collect(),
            ..PackReport::default()
        };
        if indexed_fields().iter().any(|n| !fields.has(n)) {
            report
                .missing_fields
                .push("ppu.layers / window / controller fields");
        }
        StreamDecoder {
            fields,
            memory: MEMORY.iter().map(|(_, size)| vec![0u8; *size]).collect(),
            aram: vec![0u8; ARAM_LEN],
            dsp: [0; DSP_LEN],
            spc_cycle: 0,
            dspaddr: 0,
            report,
        }
    }

    /// Apply a frame's sound snapshot, which comes before its frame record.
    pub fn audio(&mut self, s: &AudioRecord) {
        for (block, bytes) in &s.blocks {
            let at = *block as usize * BLOCK;
            self.aram[at..at + bytes.len()].copy_from_slice(bytes);
        }
        self.dsp = s.dsp;
        self.spc_cycle = s.spc_cycle;
    }

    /// A frame's sound events, numbered `number`, with `DSPADDR` carried
    /// from the frame before.
    pub fn apu(&mut self, a: &ApuRecord, number: u64) -> ApuEvents {
        let events = ApuEvents {
            frame: number,
            dspaddr: self.dspaddr,
            events: a.events.clone(),
        };
        self.dspaddr = events.dspaddr_after();
        self.report.apu_events += events.events.len() as u64;
        self.report.dsp_writes += events.dsp_writes().len() as u64;
        events
    }

    /// Apply one frame record and return the machine after it, numbered
    /// `number`, holding `regions`.
    pub fn frame(&mut self, f: &FrameRecord, number: u64, regions: &[StateRegion]) -> MachineState {
        self.fields.apply(&f.fields);
        for (image, blocks) in self.memory.iter_mut().zip(&f.blocks) {
            for (block, bytes) in blocks {
                let at = *block as usize * BLOCK;
                image[at..at + bytes.len()].copy_from_slice(bytes);
            }
        }
        let mut state = MachineState {
            frame: number,
            ..MachineState::default()
        };
        state.regions.insert(
            StateRegion::CpuRegisters,
            cpu_registers(&self.fields).encode().to_vec(),
        );
        state.regions.insert(
            StateRegion::PpuState,
            ppu_state(&self.fields, f, &mut self.report).bytes.to_vec(),
        );
        state.regions.insert(
            StateRegion::IoState,
            io_state(&self.fields, f).bytes.to_vec(),
        );
        let mut timing = vec![0u8; StateRegion::Timing.size()];
        timing[..8].copy_from_slice(&number.to_le_bytes());
        state.regions.insert(StateRegion::Timing, timing);
        for (region, image) in [
            StateRegion::Vram,
            StateRegion::Cgram,
            StateRegion::Oam,
            StateRegion::Wram,
        ]
        .into_iter()
        .zip(&self.memory)
        {
            if regions.contains(&region) {
                state.regions.insert(region, image.clone());
            }
        }
        if regions.contains(&StateRegion::Aram) {
            state.regions.insert(StateRegion::Aram, self.aram.clone());
            state
                .regions
                .insert(StateRegion::DspRegisters, self.dsp.to_vec());
            state.regions.insert(
                StateRegion::SpcState,
                spc_state(&self.fields, self.spc_cycle).encode().to_vec(),
            );
        }
        state
    }
}

pub fn pack<R: Read, W: Write + Seek>(
    stream: R,
    rom: &RomImage,
    out: W,
    options: PackOptions,
) -> Result<PackReport, PackError> {
    let mut reader = StreamReader::new(stream)?;
    let header = reader.header.clone();
    if let Some(why) = header.rom_mismatch(rom.bytes()) {
        return Err(PackError::RomMismatch(why));
    }
    let mut decoder = StreamDecoder::new(&header);

    let mut regions = vec![
        StateRegion::CpuRegisters,
        StateRegion::PpuState,
        StateRegion::IoState,
        StateRegion::Vram,
        StateRegion::Cgram,
        StateRegion::Oam,
        StateRegion::Timing,
    ];
    if options.wram != WramMode::Off {
        regions.push(StateRegion::Wram);
    }
    if header.audio() {
        regions.extend(StateRegion::AUDIO);
    }
    let identity = RecordingIdentity {
        rom_sha256: *rom.sha256(),
        producer: header.producer.clone(),
        producer_version: format!("recorder stream {}", header.version),
    };
    let writer_options = WriterOptions {
        keyframe_interval: options.keyframe_interval,
        compress: options.compress,
        wram_keyframe_only: options.wram == WramMode::Keyframe,
        mapping: mapping_byte(rom.mapping()),
        layers: Layers {
            write_log: true,
            line_writes: header.version >= 2,
            apu_events: header.audio(),
            ..Layers::default()
        },
    };
    let mut writer = RomrecWriter::new(
        out,
        &identity,
        &regions,
        writer_options,
        header.created.max(0) as u64,
    )?;

    let mut pending: Vec<DmaEvent> = Vec::new();
    let mut lines: Option<Vec<crate::recording::lines::RegWrite>> = None;
    let mut apu: Option<ApuRecord> = None;
    while let Some(record) = reader.next_record()? {
        match record {
            Record::Frame(f) => {
                let frames = decoder.report.frames;
                if f.frame as u64 != frames {
                    return Err(StreamError::Corrupt(format!(
                        "frame {} follows frame {}",
                        f.frame,
                        frames.wrapping_sub(1)
                    ))
                    .into());
                }
                let state = decoder.frame(&f, frames, &regions);
                writer.write_frame(&state)?;
                let report = &mut decoder.report;
                if !pending.is_empty() {
                    writer.write_layer(LAYER_MAGICS[1], &wlog_body(report.frames, &pending))?;
                    report.dma_events += pending.len() as u64;
                    pending.clear();
                }
                // A version-2 stream has line writes for every frame after
                // the first, even when there were none.
                if let Some(w) = lines.take() {
                    report.line_writes += w.len() as u64;
                    let body = crate::recording::lines::encode(report.frames, &w, options.compress);
                    writer.write_layer(LAYER_MAGICS[4], &body)?;
                }
                if let Some(a) = apu.take() {
                    let events = decoder.apu(&a, frames);
                    writer.write_layer(LAYER_MAGICS[5], &events.encode(options.compress))?;
                }
                let report = &mut decoder.report;
                report.frames += 1;
            }
            Record::Dma(d) => pending.push(*d),
            Record::Lines(l) => lines = Some(l.writes),
            Record::Apu(a) => apu = Some(*a),
            Record::Audio(s) => decoder.audio(&s),
            Record::StateLoaded { .. } => decoder.report.state_loads += 1,
            // A live connection's; a file keeps its log beside it instead.
            Record::ExecLog(_) => {}
            Record::End { .. } => {}
        }
    }
    let mut report = decoder.report;
    report.truncated = reader.truncated;
    if report.frames == 0 {
        return Err(PackError::Empty);
    }
    report.unknown_registers.sort_unstable();
    writer.finish()?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::mesen::stream::{STREAM_VERSION, encode};
    use crate::recording::{MachineStateSource, RomrecSource};

    fn fields_from(pairs: &[(&str, i64)]) -> Fields {
        let header = StreamHeader {
            version: STREAM_VERSION,
            producer: String::new(),
            rom_sha1: String::new(),
            created: 0,
            rom_size: 0,
            samples: Vec::new(),
            fields: pairs.iter().map(|(k, _)| (*k).to_owned()).collect(),
            flags: 0,
        };
        let mut f = Fields::new(&header);
        let changes: Vec<(u16, i64)> = pairs
            .iter()
            .enumerate()
            .map(|(i, (_, v))| (i as u16, *v))
            .collect();
        f.apply(&changes);
        f
    }

    /// The values are Super Metroid's (a Mesen 2.2.1 `getState()` dump),
    /// and each expected byte is the one that makes `SnesPpu::Write`
    /// produce them.
    #[test]
    fn ports_re_encode_what_the_ppu_decoded() {
        let f = fields_from(&[
            ("ppu.forcedBlank", 1),
            ("ppu.screenBrightness", 0x0F),
            ("ppu.oamMode", 0),
            ("ppu.oamBaseAddress", 0x6000),
            ("ppu.oamAddressOffset", 0x1000),
            ("ppu.bgMode", 1),
            ("ppu.mode1Bg3Priority", 1),
            ("ppu.layers[0].largeTiles", 0),
            ("ppu.layers[1].largeTiles", 1),
            ("ppu.layers[0].tilemapAddress", 0x5000),
            ("ppu.layers[0].doubleWidth", 1),
            ("ppu.layers[0].doubleHeight", 0),
            ("ppu.layers[0].chrAddress", 0x0000),
            ("ppu.layers[1].chrAddress", 0x4000),
            ("ppu.vramIncrementValue", 128),
            ("ppu.vramAddrIncrementOnSecondReg", 1),
            ("ppu.mosaicSize", 1),
            ("ppu.mosaicEnabled", 0),
            ("ppu.colorMathEnabled", 0x21),
            ("ppu.colorMathSubtractMode", 1),
        ]);
        assert_eq!(encode_port(0x2100, &f), Some(0x8F));
        assert_eq!(encode_port(0x2101, &f), Some(0x03));
        assert_eq!(encode_port(0x2105, &f), Some(0x29));
        assert_eq!(encode_port(0x2106, &f), Some(0x00));
        assert_eq!(encode_port(0x2107, &f), Some(0x51));
        assert_eq!(encode_port(0x210B, &f), Some(0x40));
        assert_eq!(encode_port(0x2115, &f), Some(0x82));
        assert_eq!(encode_port(0x2131, &f), Some(0xA1));
        // Mesen does not export these, so they need a write.
        assert_eq!(encode_port(0x212A, &f), None);
        assert_eq!(encode_port(0x2130, &f), None);
        // Data and two-write ports have no one-byte value.
        assert_eq!(encode_port(0x2118, &f), None);
        assert!(port_has_no_value(0x2118) && !port_has_no_value(0x212A));
    }

    #[test]
    fn window_selects_pack_both_windows_for_a_layer_pair() {
        let f = fields_from(&[
            ("ppu.window[0].activeLayers[0]", 1),
            ("ppu.window[1].invertedLayers[1]", 1),
            ("ppu.window[1].activeLayers[5]", 1),
        ]);
        assert_eq!(encode_port(0x2123, &f), Some(0x02 | 0x40));
        assert_eq!(encode_port(0x2125, &f), Some(0x80));
    }

    fn rom() -> RomImage {
        RomImage::from_bytes(crate::fixtures::minimal_lorom(), "fixture.sfc").unwrap()
    }

    fn stream_for(rom: &RomImage, frames: u32) -> Vec<u8> {
        encode::fixture(rom.bytes(), frames, true)
    }

    #[test]
    fn a_stream_packs_into_a_readable_recording() {
        let rom = rom();
        let mut out = std::io::Cursor::new(Vec::new());
        let report = pack(
            stream_for(&rom, 5).as_slice(),
            &rom,
            &mut out,
            PackOptions::default(),
        )
        .unwrap();
        assert_eq!(
            (report.frames, report.dma_events, report.truncated),
            (5, 1, false)
        );
        assert!(report.unknown_registers.contains(&0x212A));
        let rec = RomrecSource::from_bytes(out.into_inner(), false).unwrap();
        assert_eq!(rec.frame_count(), Some(5));
        assert!(rec.layers().write_log);
        let s = rec.state_at(3).unwrap();
        assert_eq!(
            CpuRegisters::decode(s.region(StateRegion::CpuRegisters).unwrap()).pc,
            0x8003
        );
        assert_eq!(s.ppu().unwrap().bg_mode(), 1);
        let vram = s.vram().unwrap();
        // Each frame wrote one more block; earlier ones persist.
        assert_eq!((vram[0], vram[3 * 256], vram[4 * 256]), (1, 4, 0));
    }

    #[test]
    fn the_sound_side_packs_into_its_regions_and_events() {
        let rom = rom();
        let mut out = std::io::Cursor::new(Vec::new());
        let stream = encode::fixture_with_audio(rom.bytes(), 5);
        let report = pack(stream.as_slice(), &rom, &mut out, PackOptions::default()).unwrap();
        assert_eq!(
            (report.frames, report.apu_events, report.dsp_writes),
            (5, 22, 10)
        );
        let rec = RomrecSource::from_bytes(out.into_inner(), false).unwrap();
        for r in StateRegion::AUDIO {
            assert!(rec.regions().contains(&r), "{}", r.name());
        }
        assert!(rec.layers().apu_events);
        let s = rec.state_at(2).unwrap();
        let aram = s.region(StateRegion::Aram).unwrap();
        assert_eq!(&aram[0x3C00..0x3C04], &[0x00, 0x40, 0x12, 0x40]);
        assert_eq!(aram[0x4000], 0xB0, "the sample's first header");
        let dsp = s.region(StateRegion::DspRegisters).unwrap();
        assert_eq!((dsp[0x5D], dsp[0x03], dsp[0x08]), (0x3C, 0x10, 0x7F));
        let spc = SpcState::decode(s.region(StateRegion::SpcState).unwrap());
        assert_eq!((spc.pc, spc.dspaddr, spc.a), (0x0202, 0x4C, 0x12));
        assert_eq!((spc.timers_on[0], spc.dividers[0]), (true, 0x50));
        assert_eq!(spc.cycle, 3 * 17_066);
        // Frame 1's events: the S-CPU's command, the reply, nine DSP writes.
        let e = rec.apu_events(1).unwrap().unwrap();
        assert_eq!(e.events.len(), 20);
        let w = e.dsp_writes();
        assert_eq!(w.len(), 9);
        assert_eq!((w[0].register, w[0].value), (0x5D, 0x3C));
        assert_eq!((w[8].register, w[8].value), (0x4C, 0x01));
        // Frame 3 keys the voice off.
        let e = rec.apu_events(3).unwrap().unwrap();
        assert_eq!(e.dspaddr, 0x4C, "DSPADDR carried from frame 1");
        assert_eq!(e.dsp_writes()[0].register, 0x5C);
        assert!(rec.apu_events(2).unwrap().unwrap().events.is_empty());
        // A recording without the layer has none.
        let mut plain = std::io::Cursor::new(Vec::new());
        pack(
            stream_for(&rom, 2).as_slice(),
            &rom,
            &mut plain,
            PackOptions::default(),
        )
        .unwrap();
        let plain = RomrecSource::from_bytes(plain.into_inner(), false).unwrap();
        assert!(!plain.regions().contains(&StateRegion::Aram));
        assert_eq!(plain.apu_events(0).unwrap(), None);
    }

    #[test]
    fn a_stream_from_another_rom_is_refused() {
        let rom = rom();
        let mut stream = stream_for(&rom, 2);
        // The first sample follows the magic, version, two strings, the
        // creation time and the size.
        let first_sample = 8 + 2 + (2 + 5) + 2 + 8 + 4;
        stream[first_sample] ^= 0xFF;
        let err = pack(
            stream.as_slice(),
            &rom,
            std::io::Cursor::new(Vec::new()),
            PackOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(err, PackError::RomMismatch(_)), "{err}");
    }
}
