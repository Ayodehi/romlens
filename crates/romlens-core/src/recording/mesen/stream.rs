//! The raw stream `mesen_recorder.lua` writes (`13-recording-format.md`,
//! "The Mesen stream").
//!
//! It is an interchange format between our script and `rec pack`, not a
//! recording: nothing reads it but [`StreamReader`]. Everything a Lua script
//! cannot do well (compression, hashing, the register layouts) happens on
//! this side. A stream cut short — the emulator closed without stopping the
//! script — ends at its last whole record and says so.

use std::io::Read;

pub const STREAM_MAGIC: &[u8; 8] = b"RLSTREAM";
pub const STREAM_VERSION: u16 = 3;
/// Header flag (stream version 3): the stream carries the sound side,
/// `A` and `S` records (docs/23).
pub const FLAG_AUDIO: u8 = 1;
/// Audio RAM, in blocks of [`BLOCK`].
pub const ARAM_LEN: usize = 0x10000;
pub const DSP_LEN: usize = 128;
/// The oldest stream this reads: version 1 has no line writes and no DMA
/// context.
pub const OLDEST_STREAM_VERSION: u16 = 1;
/// Blocks are this long, except the last of a region that is not a multiple.
pub const BLOCK: usize = 256;
/// The largest execution log record read: far above a real session's.
pub const MAX_EXEC_LOG: usize = 256 << 20;
/// More register writes than any frame could make: 1,364 master cycles a
/// line, 262 lines, one write at most every 6 cycles.
pub const MAX_LINE_WRITES: usize = 1 << 16;
/// Samples of the ROM the header carries, each [`SAMPLE_LEN`] bytes.
pub const SAMPLES: usize = 64;
pub const SAMPLE_LEN: usize = 16;
/// `$2100`–`$2133`.
pub const PPU_PORTS: usize = 0x34;
/// `$4300`–`$437F`.
pub const DMA_LEN: usize = 128;

/// The memory regions a frame record carries, in stream order.
pub const MEMORY: [(&str, usize); 4] = [
    ("vram", 0x10000),
    ("cgram", 0x200),
    ("oam", 0x220),
    ("wram", 0x20000),
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    #[error("could not read the stream: {0}")]
    Io(String),
    #[error("not a Romlens recorder stream: {0}")]
    BadFormat(String),
    #[error("the stream is damaged: {0}")]
    Corrupt(String),
}

impl From<std::io::Error> for StreamError {
    fn from(e: std::io::Error) -> Self {
        StreamError::Io(e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamHeader {
    pub version: u16,
    pub producer: String,
    /// Mesen's SHA-1 of the ROM, as hex; informational.
    pub rom_sha1: String,
    /// Unix seconds when recording started.
    pub created: i64,
    pub rom_size: u32,
    /// `SAMPLES` runs of `SAMPLE_LEN` bytes at `rom_size / 64 * i`.
    pub samples: Vec<u8>,
    /// The `emu.getState()` keys each frame's field changes index into.
    pub fields: Vec<String>,
    /// [`FLAG_AUDIO`] (version 3; 0 before).
    pub flags: u8,
}

impl StreamHeader {
    /// Whether the stream carries the sound side.
    pub fn audio(&self) -> bool {
        self.flags & FLAG_AUDIO != 0
    }

    /// Where sample `i` was taken.
    pub fn sample_offset(&self, i: usize) -> usize {
        (self.rom_size as usize / SAMPLES) * i
    }

    /// Whether `rom` is the ROM the stream was recorded from, by size and
    /// samples. `None` when they agree, else why not.
    pub fn rom_mismatch(&self, rom: &[u8]) -> Option<String> {
        let size = self.rom_size as usize;
        // Mesen pads a ROM whose size is not a power of two up to the next
        // one (Super Metroid's 3 MB reads as 4 MB), so that size is the same
        // ROM; its samples in the padding say nothing about the file.
        let padded = size > rom.len() && size == rom.len().next_power_of_two();
        if rom.len() != size && !padded {
            return Some(format!(
                "the stream was recorded from a {}-byte ROM; this one is {} bytes",
                self.rom_size,
                rom.len()
            ));
        }
        for i in 0..SAMPLES {
            let at = self.sample_offset(i);
            if padded && at >= rom.len() {
                continue;
            }
            let want = &self.samples[i * SAMPLE_LEN..(i + 1) * SAMPLE_LEN];
            // A sample past the end reads as zeroes in Mesen.
            let mut have = [0u8; SAMPLE_LEN];
            let n = rom.len().saturating_sub(at).min(SAMPLE_LEN);
            have[..n].copy_from_slice(&rom[at..at + n]);
            if have != want {
                return Some(format!("the ROM differs at file offset {at:#x}"));
            }
        }
        None
    }
}

/// One frame's end, as the recorder saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRecord {
    pub frame: u32,
    /// Fields whose value changed since the previous frame: (index, value).
    pub fields: Vec<(u16, i64)>,
    /// The last byte written to each of `$2100`–`$2133`.
    pub ports: [u8; PPU_PORTS],
    /// Whether each port has been written since recording (or the last
    /// savestate load) began.
    pub seen: [bool; PPU_PORTS],
    pub dma: [u8; DMA_LEN],
    /// Per [`MEMORY`] region, the blocks that changed: (block, bytes).
    pub blocks: [Vec<(u16, Vec<u8>)>; 4],
}

/// A write to `MDMAEN` (`$420B`), with the channel registers at that moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmaEvent {
    /// The frame it happened in: the next frame record's number.
    pub frame: u32,
    pub value: u8,
    pub scanline: u16,
    pub registers: [u8; DMA_LEN],
    /// Where the transfer's bytes go and who started it (stream version 2).
    pub context: Option<DmaContext>,
}

pub use crate::recording::wlog::DmaContext;

/// Bytes of a [`DmaContext`] in a `D` record.
pub const DMA_CONTEXT_LEN: usize = 14;

/// The PPU register writes made while one frame was drawn (stream version
/// 2): the `R` record before its frame's `F`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineRecord {
    pub frame: u32,
    pub writes: Vec<crate::recording::lines::RegWrite>,
}

/// The sound side's events in one frame (stream version 3): the `A`
/// record before its frame's `F`, in [`ApuEvents`]'s event layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApuRecord {
    pub frame: u32,
    pub events: Vec<crate::recording::apu::ApuEvent>,
}

/// The sound side at one frame's end (stream version 3): the `S` record
/// before its frame's `F`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioRecord {
    pub frame: u32,
    /// The SPC700's cycle count when it was read.
    pub spc_cycle: u64,
    /// Audio RAM's changed blocks: (block, 256 bytes).
    pub blocks: Vec<(u16, Vec<u8>)>,
    pub dsp: [u8; DSP_LEN],
}

pub use crate::recording::apu::ApuEvents;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Frame(Box<FrameRecord>),
    Dma(Box<DmaEvent>),
    Lines(Box<LineRecord>),
    Apu(Box<ApuRecord>),
    Audio(Box<AudioRecord>),
    /// A savestate was loaded before this frame.
    StateLoaded {
        frame: u32,
    },
    /// The clean end, with the number of frames written.
    End {
        frames: u32,
    },
    /// An execution log (`io::import::exec_log`), whole or a delta: what the
    /// CPU did since the previous one. Sent on a live connection only.
    ExecLog(Vec<u8>),
}

/// Reads a stream record by record.
pub struct StreamReader<R: Read> {
    input: R,
    pub header: StreamHeader,
    /// Set when the stream ended inside a record or without `E`.
    pub truncated: bool,
    done: bool,
}

/// Reads exact byte counts, telling "ran out" apart from I/O errors.
fn fill<R: Read>(input: &mut R, buf: &mut [u8]) -> Result<bool, StreamError> {
    let mut at = 0;
    while at < buf.len() {
        match input.read(&mut buf[at..]) {
            Ok(0) => return Ok(false),
            Ok(n) => at += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(true)
}

/// Pulls fixed-size values out of the input; `None` means it ran out.
struct Take<'a, R: Read>(&'a mut R);

impl<R: Read> Take<'_, R> {
    fn bytes(&mut self, n: usize) -> Result<Option<Vec<u8>>, StreamError> {
        let mut b = vec![0u8; n];
        Ok(fill(self.0, &mut b)?.then_some(b))
    }
    fn array<const N: usize>(&mut self) -> Result<Option<[u8; N]>, StreamError> {
        let mut b = [0u8; N];
        Ok(fill(self.0, &mut b)?.then_some(b))
    }
    fn u8(&mut self) -> Result<Option<u8>, StreamError> {
        Ok(self.array::<1>()?.map(|b| b[0]))
    }
    fn u16(&mut self) -> Result<Option<u16>, StreamError> {
        Ok(self.array::<2>()?.map(u16::from_le_bytes))
    }
    fn u32(&mut self) -> Result<Option<u32>, StreamError> {
        Ok(self.array::<4>()?.map(u32::from_le_bytes))
    }
    fn i64(&mut self) -> Result<Option<i64>, StreamError> {
        Ok(self.array::<8>()?.map(i64::from_le_bytes))
    }
    fn string(&mut self, wide: bool) -> Result<Option<String>, StreamError> {
        let len = if wide {
            self.u16()?.map(usize::from)
        } else {
            self.u8()?.map(usize::from)
        };
        let Some(len) = len else { return Ok(None) };
        let Some(b) = self.bytes(len)? else {
            return Ok(None);
        };
        String::from_utf8(b)
            .map(Some)
            .map_err(|_| StreamError::Corrupt("a string is not UTF-8".to_owned()))
    }
}

macro_rules! need {
    ($e:expr) => {
        match $e? {
            Some(v) => v,
            None => return Ok(None),
        }
    };
}

impl<R: Read> StreamReader<R> {
    pub fn new(mut input: R) -> Result<Self, StreamError> {
        let header = Self::read_header(&mut input)?
            .ok_or_else(|| StreamError::BadFormat("the header is cut short".to_owned()))?;
        Ok(StreamReader {
            input,
            header,
            truncated: false,
            done: false,
        })
    }

    fn read_header(input: &mut R) -> Result<Option<StreamHeader>, StreamError> {
        let mut t = Take(input);
        let magic: [u8; 8] = need!(t.array());
        if &magic != STREAM_MAGIC {
            return Err(StreamError::BadFormat(
                "it does not start with RLSTREAM".to_owned(),
            ));
        }
        let version = need!(t.u16());
        if !(OLDEST_STREAM_VERSION..=STREAM_VERSION).contains(&version) {
            return Err(StreamError::BadFormat(format!(
                "stream version {version}; this Romlens reads versions {OLDEST_STREAM_VERSION} to {STREAM_VERSION}"
            )));
        }
        let producer = need!(t.string(true));
        let rom_sha1 = need!(t.string(true));
        let created = need!(t.i64());
        let rom_size = need!(t.u32());
        let samples = need!(t.bytes(SAMPLES * SAMPLE_LEN));
        let count = need!(t.u16());
        let mut fields = Vec::with_capacity(count as usize);
        for _ in 0..count {
            fields.push(need!(t.string(false)));
        }
        let flags = if version >= 3 { need!(t.u8()) } else { 0 };
        Ok(Some(StreamHeader {
            version,
            producer,
            rom_sha1,
            created,
            rom_size,
            samples,
            fields,
            flags,
        }))
    }

    /// The next record, or `None` at the end (clean or cut short; see
    /// [`truncated`](Self::truncated)).
    pub fn next_record(&mut self) -> Result<Option<Record>, StreamError> {
        if self.done {
            return Ok(None);
        }
        let record = self.read_record()?;
        match &record {
            None => {
                self.truncated = true;
                self.done = true;
            }
            Some(Record::End { .. }) => self.done = true,
            Some(_) => {}
        }
        Ok(record)
    }

    fn read_record(&mut self) -> Result<Option<Record>, StreamError> {
        let field_count = self.header.fields.len();
        let version = self.header.version;
        let mut t = Take(&mut self.input);
        let tag = need!(t.u8());
        match tag {
            b'F' => {
                let frame = need!(t.u32());
                let n = need!(t.u16());
                let mut fields = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    let index = need!(t.u16());
                    let value = need!(t.i64());
                    if index as usize >= field_count {
                        return Err(StreamError::Corrupt(format!(
                            "frame {frame} changes field {index} of {field_count}"
                        )));
                    }
                    fields.push((index, value));
                }
                let ports: [u8; PPU_PORTS] = need!(t.array());
                let seen: [u8; PPU_PORTS] = need!(t.array());
                let dma: [u8; DMA_LEN] = need!(t.array());
                let mut blocks: [Vec<(u16, Vec<u8>)>; 4] = Default::default();
                for (slot, (name, size)) in blocks.iter_mut().zip(MEMORY) {
                    let count = need!(t.u16());
                    for _ in 0..count {
                        let block = need!(t.u16()) as usize;
                        let at = block * BLOCK;
                        if at >= size {
                            return Err(StreamError::Corrupt(format!(
                                "frame {frame} has {name} block {block}, past the region's end"
                            )));
                        }
                        let bytes = need!(t.bytes(BLOCK.min(size - at)));
                        slot.push((block as u16, bytes));
                    }
                }
                Ok(Some(Record::Frame(Box::new(FrameRecord {
                    frame,
                    fields,
                    ports,
                    seen: seen.map(|b| b != 0),
                    dma,
                    blocks,
                }))))
            }
            b'D' => {
                let frame = need!(t.u32());
                let value = need!(t.u8());
                let scanline = need!(t.u16());
                let registers: [u8; DMA_LEN] = need!(t.array());
                let context = if version >= 2 {
                    let c: [u8; DMA_CONTEXT_LEN] = need!(t.array());
                    Some(DmaContext {
                        h_clock: u16::from_le_bytes([c[0], c[1]]),
                        vram_address: u16::from_le_bytes([c[2], c[3]]),
                        cgram_address: c[4],
                        oam_address: u16::from_le_bytes([c[5], c[6]]),
                        wram_address: u32::from_le_bytes([c[7], c[8], c[9], c[10]]),
                        k: c[11],
                        pc: u16::from_le_bytes([c[12], c[13]]),
                    })
                } else {
                    None
                };
                Ok(Some(Record::Dma(Box::new(DmaEvent {
                    frame,
                    value,
                    scanline,
                    registers,
                    context,
                }))))
            }
            b'R' => {
                let frame = need!(t.u32());
                let count = need!(t.u32()) as usize;
                if count > MAX_LINE_WRITES {
                    return Err(StreamError::Corrupt(format!(
                        "frame {frame} claims {count} register writes"
                    )));
                }
                let raw = need!(t.bytes(count * crate::recording::lines::WRITE_LEN));
                let writes = raw
                    .chunks(crate::recording::lines::WRITE_LEN)
                    .map(|w| crate::recording::lines::RegWrite {
                        line: i16::from_le_bytes([w[0], w[1]]),
                        dot: u16::from_le_bytes([w[2], w[3]]),
                        reg: w[4],
                        value: w[5],
                    })
                    .collect();
                Ok(Some(Record::Lines(Box::new(LineRecord { frame, writes }))))
            }
            b'A' => {
                let frame = need!(t.u32());
                let count = need!(t.u32()) as usize;
                if count > crate::recording::apu::MAX_EVENTS {
                    return Err(StreamError::Corrupt(format!(
                        "frame {frame} claims {count} sound events"
                    )));
                }
                let raw = need!(t.bytes(count * crate::recording::apu::EVENT_LEN));
                let events = ApuEvents::parse_events(&raw).map_err(StreamError::Corrupt)?;
                Ok(Some(Record::Apu(Box::new(ApuRecord { frame, events }))))
            }
            b'S' => {
                let frame = need!(t.u32());
                let spc_cycle = need!(t.i64()) as u64;
                let count = need!(t.u16());
                let mut blocks = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let block = need!(t.u16());
                    if block as usize * BLOCK >= ARAM_LEN {
                        return Err(StreamError::Corrupt(format!(
                            "frame {frame} has audio RAM block {block}, past its end"
                        )));
                    }
                    blocks.push((block, need!(t.bytes(BLOCK))));
                }
                let dsp: [u8; DSP_LEN] = need!(t.array());
                Ok(Some(Record::Audio(Box::new(AudioRecord {
                    frame,
                    spc_cycle,
                    blocks,
                    dsp,
                }))))
            }
            b'L' => Ok(Some(Record::StateLoaded {
                frame: need!(t.u32()),
            })),
            b'E' => Ok(Some(Record::End {
                frames: need!(t.u32()),
            })),
            b'X' => {
                let len = need!(t.u32()) as usize;
                if len > MAX_EXEC_LOG {
                    return Err(StreamError::Corrupt(format!(
                        "an execution log record claims {len} bytes"
                    )));
                }
                Ok(Some(Record::ExecLog(need!(t.bytes(len)))))
            }
            other => Err(StreamError::Corrupt(format!(
                "unknown record tag {other:#04x}"
            ))),
        }
    }
}

/// Encoders for tests and fixtures: the inverse of [`StreamReader`], in the
/// byte layout `mesen_recorder.lua` writes.
pub mod encode {
    use super::*;

    pub fn header(h: &StreamHeader) -> Vec<u8> {
        let mut b = STREAM_MAGIC.to_vec();
        b.extend_from_slice(&h.version.to_le_bytes());
        for s in [&h.producer, &h.rom_sha1] {
            b.extend_from_slice(&(s.len() as u16).to_le_bytes());
            b.extend_from_slice(s.as_bytes());
        }
        b.extend_from_slice(&h.created.to_le_bytes());
        b.extend_from_slice(&h.rom_size.to_le_bytes());
        b.extend_from_slice(&h.samples);
        b.extend_from_slice(&(h.fields.len() as u16).to_le_bytes());
        for f in &h.fields {
            b.push(f.len() as u8);
            b.extend_from_slice(f.as_bytes());
        }
        if h.version >= 3 {
            b.push(h.flags);
        }
        b
    }

    /// A small stream recorded from `rom`, for tests and the CLI goldens:
    /// `frames` frames, frame *n* writing VRAM block *n* with `n + 1` and
    /// setting `PC` to `$8000 + n` in Mode 1, and one DMA transfer before
    /// frame 1. With `end`, it closes cleanly; without, it is cut short the
    /// way a closed emulator leaves it.
    pub fn fixture(rom: &[u8], frames: u32, end: bool) -> Vec<u8> {
        let size = rom.len() as u32;
        let samples = (0..SAMPLES)
            .flat_map(|i| {
                let at = (size as usize / SAMPLES) * i;
                rom[at..at + SAMPLE_LEN].to_vec()
            })
            .collect();
        let head = StreamHeader {
            version: STREAM_VERSION,
            producer: "Mesen".to_owned(),
            rom_sha1: String::new(),
            created: 1_790_000_000,
            rom_size: size,
            samples,
            fields: vec![
                "cpu.pc".to_owned(),
                "ppu.bgMode".to_owned(),
                "ppu.layers[0].tilemapAddress".to_owned(),
            ],
            flags: 0,
        };
        let mut b = header(&head);
        for n in 0..frames {
            // From the second frame on, the writes made while it was drawn:
            // BG1's horizontal scroll set in the vertical blank.
            if n >= 1 {
                b.extend(record(&Record::Lines(Box::new(LineRecord {
                    frame: n,
                    writes: vec![
                        crate::recording::lines::RegWrite {
                            line: -10,
                            dot: 100,
                            reg: 0x0D,
                            value: n as u8,
                        },
                        crate::recording::lines::RegWrite {
                            line: -10,
                            dot: 110,
                            reg: 0x0D,
                            value: 0,
                        },
                    ],
                }))));
            }
            if n == 1 {
                b.extend(record(&Record::Dma(Box::new(DmaEvent {
                    frame: 1,
                    value: 1,
                    scanline: 240,
                    registers: [0x55; DMA_LEN],
                    context: Some(DmaContext {
                        vram_address: 0x6000,
                        k: 0x80,
                        pc: 0x8123,
                        ..DmaContext::default()
                    }),
                }))));
            }
            let mut blocks: [Vec<(u16, Vec<u8>)>; 4] = Default::default();
            blocks[0] = vec![(n as u16, vec![n as u8 + 1; BLOCK])];
            let fields = if n == 0 {
                vec![(0, 0x8000), (1, 1), (2, 0x5000)]
            } else {
                vec![(0, 0x8000 + n as i64)]
            };
            b.extend(record(&Record::Frame(Box::new(FrameRecord {
                frame: n,
                fields,
                ports: [0; PPU_PORTS],
                seen: [false; PPU_PORTS],
                dma: [0; DMA_LEN],
                blocks,
            }))));
        }
        if end {
            b.extend(record(&Record::End { frames }));
        }
        b
    }

    /// The sound fixture's audio RAM: the sample directory at `$3C00`
    /// (entry 0 is [`crate::fixtures::sound::brr_sample`] at `$4000`, its
    /// loop 18 bytes in) and the sample.
    pub fn fixture_aram() -> Vec<u8> {
        let mut aram = vec![0u8; ARAM_LEN];
        aram[0x3C00..0x3C04].copy_from_slice(&[0x00, 0x40, 0x12, 0x40]);
        let s = crate::fixtures::sound::brr_sample();
        aram[0x4000..0x4000 + s.len()].copy_from_slice(&s);
        aram
    }

    /// [`fixture`] with a sound side (stream version 3): frame 0 holds the
    /// directory and sample in audio RAM; in frame 1 the S-CPU writes `$01`
    /// to port 0 and the driver answers, sets up voice 0 (sample 0, pitch
    /// `$1000`, ADSR, full volume) and keys it on; in frame 3 it keys it off.
    pub fn fixture_with_audio(rom: &[u8], frames: u32) -> Vec<u8> {
        use crate::recording::apu::{ApuEvent, ApuEventKind};
        let plain = fixture(rom, frames, true);
        let mut reader = StreamReader::new(plain.as_slice()).unwrap();
        let mut head = reader.header.clone();
        head.flags = FLAG_AUDIO;
        head.fields.extend(
            [
                "spc.a",
                "spc.pc",
                "spc.dspReg",
                "spc.cpuRegs[0]",
                "spc.outputReg[0]",
                "spc.timer0.target",
                "spc.timer0.enabled",
            ]
            .map(str::to_owned),
        );
        let spc_fields = head.fields.len() - 7;
        let mut b = header(&head);
        let aram = fixture_aram();
        let mut dsp = [0u8; DSP_LEN];
        let mut cycle = 0u64;
        let io = |address: u8, value: u8, spc_cycle: u64| ApuEvent {
            kind: ApuEventKind::SpcIo,
            address,
            value,
            spc_cycle,
            master_clock: 0,
        };
        let mut n = 0u32;
        while let Some(r) = reader.next_record().unwrap() {
            let Record::Frame(mut f) = r else {
                b.extend(record(&r));
                continue;
            };
            // Each frame is 17,066 SPC700 cycles (1.024 MHz at 60 Hz).
            let start = cycle;
            cycle += 17_066;
            let mut events = Vec::new();
            let mut writes: Vec<(u8, u8)> = Vec::new();
            if n == 1 {
                events.push(ApuEvent {
                    kind: ApuEventKind::CpuPort,
                    address: 0,
                    value: 0x01,
                    spc_cycle: start + 100,
                    master_clock: 357_366 + 2000,
                });
                events.push(io(4, 0x01, start + 150));
                writes = vec![
                    (0x5D, 0x3C),
                    (0x04, 0x00),
                    (0x02, 0x00),
                    (0x03, 0x10),
                    (0x05, 0x8F),
                    (0x06, 0xE0),
                    (0x00, 0x7F),
                    (0x01, 0x7F),
                    (0x4C, 0x01),
                ];
            }
            if n == 3 {
                writes = vec![(0x5C, 0x01)];
            }
            for (i, (reg, value)) in writes.iter().enumerate() {
                let at = start + 200 + i as u64 * 10;
                events.push(io(2, *reg, at));
                events.push(io(3, *value, at + 5));
                dsp[*reg as usize] = *value;
            }
            if n >= 1 {
                // The envelope rises after key on: ENVX as the DSP rewrites it.
                dsp[0x08] = if n >= 3 { 0x20 } else { 0x7F };
                dsp[0x4C] = 0;
            }
            b.extend(record(&Record::Apu(Box::new(ApuRecord {
                frame: n,
                events,
            }))));
            let blocks = if n == 0 {
                (0..ARAM_LEN / BLOCK)
                    .filter(|blk| aram[blk * BLOCK..(blk + 1) * BLOCK].iter().any(|&x| x != 0))
                    .map(|blk| (blk as u16, aram[blk * BLOCK..(blk + 1) * BLOCK].to_vec()))
                    .collect()
            } else {
                Vec::new()
            };
            b.extend(record(&Record::Audio(Box::new(AudioRecord {
                frame: n,
                spc_cycle: cycle,
                blocks,
                dsp,
            }))));
            let base = spc_fields as u16;
            f.fields.extend([
                (base, 0x12),
                (base + 1, 0x0200 + n as i64),
                (base + 2, if n >= 1 { 0x4C } else { 0 }),
            ]);
            if n == 1 {
                f.fields.extend([(base + 3, 0x01), (base + 4, 0x01)]);
            }
            if n == 0 {
                f.fields.extend([(base + 5, 0x50), (base + 6, 1)]);
            }
            b.extend(record(&Record::Frame(f)));
            n += 1;
        }
        b
    }

    pub fn record(r: &Record) -> Vec<u8> {
        let mut b = Vec::new();
        match r {
            Record::Frame(f) => {
                b.push(b'F');
                b.extend_from_slice(&f.frame.to_le_bytes());
                b.extend_from_slice(&(f.fields.len() as u16).to_le_bytes());
                for (i, v) in &f.fields {
                    b.extend_from_slice(&i.to_le_bytes());
                    b.extend_from_slice(&v.to_le_bytes());
                }
                b.extend_from_slice(&f.ports);
                b.extend(f.seen.iter().map(|&s| s as u8));
                b.extend_from_slice(&f.dma);
                for region in &f.blocks {
                    b.extend_from_slice(&(region.len() as u16).to_le_bytes());
                    for (block, bytes) in region {
                        b.extend_from_slice(&block.to_le_bytes());
                        b.extend_from_slice(bytes);
                    }
                }
            }
            Record::Dma(d) => {
                b.push(b'D');
                b.extend_from_slice(&d.frame.to_le_bytes());
                b.push(d.value);
                b.extend_from_slice(&d.scanline.to_le_bytes());
                b.extend_from_slice(&d.registers);
                if let Some(c) = d.context {
                    b.extend_from_slice(&c.h_clock.to_le_bytes());
                    b.extend_from_slice(&c.vram_address.to_le_bytes());
                    b.push(c.cgram_address);
                    b.extend_from_slice(&c.oam_address.to_le_bytes());
                    b.extend_from_slice(&c.wram_address.to_le_bytes());
                    b.push(c.k);
                    b.extend_from_slice(&c.pc.to_le_bytes());
                }
            }
            Record::Lines(l) => {
                b.push(b'R');
                b.extend_from_slice(&l.frame.to_le_bytes());
                b.extend_from_slice(&(l.writes.len() as u32).to_le_bytes());
                for w in &l.writes {
                    b.extend_from_slice(&w.line.to_le_bytes());
                    b.extend_from_slice(&w.dot.to_le_bytes());
                    b.extend_from_slice(&[w.reg, w.value]);
                }
            }
            Record::Apu(a) => {
                b.push(b'A');
                b.extend_from_slice(&a.frame.to_le_bytes());
                b.extend_from_slice(&(a.events.len() as u32).to_le_bytes());
                b.extend(ApuEvents::raw_events(&a.events));
            }
            Record::Audio(s) => {
                b.push(b'S');
                b.extend_from_slice(&s.frame.to_le_bytes());
                b.extend_from_slice(&s.spc_cycle.to_le_bytes());
                b.extend_from_slice(&(s.blocks.len() as u16).to_le_bytes());
                for (block, bytes) in &s.blocks {
                    b.extend_from_slice(&block.to_le_bytes());
                    b.extend_from_slice(bytes);
                }
                b.extend_from_slice(&s.dsp);
            }
            Record::StateLoaded { frame } => {
                b.push(b'L');
                b.extend_from_slice(&frame.to_le_bytes());
            }
            Record::End { frames } => {
                b.push(b'E');
                b.extend_from_slice(&frames.to_le_bytes());
            }
            Record::ExecLog(log) => {
                b.push(b'X');
                b.extend_from_slice(&(log.len() as u32).to_le_bytes());
                b.extend_from_slice(log);
            }
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> StreamHeader {
        StreamHeader {
            version: STREAM_VERSION,
            producer: "Mesen".to_owned(),
            rom_sha1: "ab".to_owned(),
            created: 1_790_000_000,
            rom_size: 0x1000,
            samples: vec![0; SAMPLES * SAMPLE_LEN],
            fields: vec!["cpu.a".to_owned(), "ppu.bgMode".to_owned()],
            flags: FLAG_AUDIO,
        }
    }

    fn frame(n: u32) -> Record {
        let mut blocks: [Vec<(u16, Vec<u8>)>; 4] = Default::default();
        blocks[2] = vec![(2, vec![7; 32])]; // OAM's short last block
        Record::Frame(Box::new(FrameRecord {
            frame: n,
            fields: vec![(1, 3)],
            ports: [1; PPU_PORTS],
            seen: [true; PPU_PORTS],
            dma: [2; DMA_LEN],
            blocks,
        }))
    }

    #[test]
    fn records_round_trip_and_the_end_is_clean() {
        let records = vec![
            frame(0),
            Record::Dma(Box::new(DmaEvent {
                frame: 1,
                value: 0x02,
                scanline: 225,
                registers: [9; DMA_LEN],
                context: Some(DmaContext {
                    h_clock: 40,
                    vram_address: 0x1234,
                    cgram_address: 7,
                    oam_address: 0x100,
                    wram_address: 0x1_0000,
                    k: 0x80,
                    pc: 0x8123,
                }),
            })),
            Record::Lines(Box::new(LineRecord {
                frame: 1,
                writes: vec![crate::recording::lines::RegWrite {
                    line: -3,
                    dot: 10,
                    reg: 0x05,
                    value: 1,
                }],
            })),
            Record::StateLoaded { frame: 1 },
            frame(1),
            Record::End { frames: 2 },
        ];
        let mut b = encode::header(&header());
        for r in &records {
            b.extend(encode::record(r));
        }
        let mut s = StreamReader::new(b.as_slice()).unwrap();
        assert_eq!(s.header, header());
        let mut got = Vec::new();
        while let Some(r) = s.next_record().unwrap() {
            got.push(r);
        }
        assert_eq!(got, records);
        assert!(!s.truncated);
    }

    #[test]
    fn a_stream_cut_short_keeps_its_whole_records() {
        let mut b = encode::header(&header());
        b.extend(encode::record(&frame(0)));
        let whole = b.len();
        b.extend(encode::record(&frame(1)));
        b.truncate(whole + 40);
        let mut s = StreamReader::new(b.as_slice()).unwrap();
        assert!(matches!(s.next_record().unwrap(), Some(Record::Frame(_))));
        assert_eq!(s.next_record().unwrap(), None);
        assert!(s.truncated);
    }

    #[test]
    fn bad_streams_are_refused() {
        assert!(matches!(
            StreamReader::new(&b"NOTASTRM"[..]),
            Err(StreamError::BadFormat(_))
        ));
        let mut b = encode::header(&header());
        b.push(b'Z');
        let mut s = StreamReader::new(b.as_slice()).unwrap();
        assert!(matches!(s.next_record(), Err(StreamError::Corrupt(_))));
    }

    #[test]
    fn the_rom_check_compares_size_and_samples() {
        let rom: Vec<u8> = (0..0x1000u32).map(|i| i as u8).collect();
        let mut h = header();
        h.samples = (0..SAMPLES)
            .flat_map(|i| rom[h.sample_offset(i)..h.sample_offset(i) + SAMPLE_LEN].to_vec())
            .collect();
        assert_eq!(h.rom_mismatch(&rom), None);
        let mut other = rom.clone();
        other[h.sample_offset(5) + 3] ^= 1;
        assert!(h.rom_mismatch(&other).unwrap().contains("0x140"));
        assert!(h.rom_mismatch(&rom[..0x800]).unwrap().contains("bytes"));
    }

    /// Mesen reads a ROM whose size is not a power of two as the next one,
    /// padded with $FF: Super Metroid's 3 MB as 4 MB.
    #[test]
    fn a_rom_padded_to_a_power_of_two_is_the_same_rom() {
        let rom: Vec<u8> = (0..0x3000u32).map(|i| (i * 7) as u8).collect();
        let mut padded = rom.clone();
        padded.resize(0x4000, 0xFF);
        let mut h = header();
        h.rom_size = 0x4000;
        h.samples = (0..SAMPLES)
            .flat_map(|i| padded[h.sample_offset(i)..h.sample_offset(i) + SAMPLE_LEN].to_vec())
            .collect();
        assert_eq!(h.rom_mismatch(&rom), None);
        // Still the same bytes where the ROM has them.
        let mut other = rom.clone();
        other[h.sample_offset(10)] ^= 1;
        assert!(h.rom_mismatch(&other).is_some());
        // A size that is not the next power of two is another ROM.
        assert!(h.rom_mismatch(&rom[..0x1800]).unwrap().contains("bytes"));
    }
}
