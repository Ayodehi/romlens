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
pub const STREAM_VERSION: u16 = 1;
/// Blocks are this long, except the last of a region that is not a multiple.
pub const BLOCK: usize = 256;
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
}

impl StreamHeader {
    /// Where sample `i` was taken.
    pub fn sample_offset(&self, i: usize) -> usize {
        (self.rom_size as usize / SAMPLES) * i
    }

    /// Whether `rom` is the ROM the stream was recorded from, by size and
    /// samples. `None` when they agree, else why not.
    pub fn rom_mismatch(&self, rom: &[u8]) -> Option<String> {
        if rom.len() != self.rom_size as usize {
            return Some(format!(
                "the stream was recorded from a {}-byte ROM; this one is {} bytes",
                self.rom_size,
                rom.len()
            ));
        }
        for i in 0..SAMPLES {
            let at = self.sample_offset(i);
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Frame(Box<FrameRecord>),
    Dma(Box<DmaEvent>),
    /// A savestate was loaded before this frame.
    StateLoaded {
        frame: u32,
    },
    /// The clean end, with the number of frames written.
    End {
        frames: u32,
    },
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
        if version != STREAM_VERSION {
            return Err(StreamError::BadFormat(format!(
                "stream version {version}; this Romlens reads version {STREAM_VERSION}"
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
        Ok(Some(StreamHeader {
            version,
            producer,
            rom_sha1,
            created,
            rom_size,
            samples,
            fields,
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
                Ok(Some(Record::Dma(Box::new(DmaEvent {
                    frame,
                    value,
                    scanline,
                    registers,
                }))))
            }
            b'L' => Ok(Some(Record::StateLoaded {
                frame: need!(t.u32()),
            })),
            b'E' => Ok(Some(Record::End {
                frames: need!(t.u32()),
            })),
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
        };
        let mut b = header(&head);
        for n in 0..frames {
            if n == 1 {
                b.extend(record(&Record::Dma(Box::new(DmaEvent {
                    frame: 1,
                    value: 1,
                    scanline: 240,
                    registers: [0x55; DMA_LEN],
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
            }
            Record::StateLoaded { frame } => {
                b.push(b'L');
                b.extend_from_slice(&frame.to_le_bytes());
            }
            Record::End { frames } => {
                b.push(b'E');
                b.extend_from_slice(&frames.to_le_bytes());
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
}
