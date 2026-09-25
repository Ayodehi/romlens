//! Reading `.romrec` files: [`RomrecSource`].

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

use crate::io::crc32::crc32;
use crate::recording::delta::{Run, apply, union};
use crate::recording::format::*;
use crate::recording::{
    Layers, MachineState, MachineStateSource, RecordingError, RecordingIdentity, StateRegion,
};

/// A layer chunk's magic and body.
pub type LayerChunk = ([u8; 4], Vec<u8>);

pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

/// An open recording. Reads chunks on demand, so a long session never has to
/// fit in memory; the last decoded frame is cached so stepping forward costs
/// one delta.
pub struct RomrecSource {
    header: Header,
    identity: RecordingIdentity,
    index: Vec<IndexEntry>,
    recovered: bool,
    file_len: u64,
    file: Mutex<Box<dyn ReadSeek>>,
    cache: Mutex<Option<MachineState>>,
}

impl std::fmt::Debug for RomrecSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RomrecSource")
            .field("frames", &self.index.len())
            .field("recovered", &self.recovered)
            .finish()
    }
}

impl RomrecSource {
    /// Open a finished recording; one without a footer is refused with a
    /// message pointing at [`open_recovering`](Self::open_recovering).
    pub fn open(path: &Path) -> Result<Self, RecordingError> {
        Self::from_reader(Box::new(std::fs::File::open(path)?), false)
    }

    /// Open a recording, rebuilding the index by scanning for frame chunks
    /// when the footer is missing — what a crashed recorder leaves.
    pub fn open_recovering(path: &Path) -> Result<Self, RecordingError> {
        Self::from_reader(Box::new(std::fs::File::open(path)?), true)
    }

    /// Open over a stream already in hand, as the validator does.
    pub fn from_boxed(file: Box<dyn ReadSeek>, recover: bool) -> Result<Self, RecordingError> {
        Self::from_reader(file, recover)
    }

    pub fn from_bytes(bytes: Vec<u8>, recover: bool) -> Result<Self, RecordingError> {
        Self::from_reader(Box::new(Cursor::new(bytes)), recover)
    }

    fn from_reader(mut file: Box<dyn ReadSeek>, recover: bool) -> Result<Self, RecordingError> {
        let file_len = file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::Start(0))?;
        let mut first = [0u8; 16];
        let n = (file_len as usize).min(16);
        read_exact(&mut file, &mut first[..n], "the header")?;
        if first[..n.min(8)] != MAGIC[..n.min(8)] || n < 8 {
            return Err(RecordingError::BadFormat(
                "it does not start with ROMREC".to_owned(),
            ));
        }
        if n < 16 {
            return Err(RecordingError::Corrupt(
                "the file ends inside the header".to_owned(),
            ));
        }
        let header_len = Header::declared_len(&first)?;
        let mut hb = vec![0u8; header_len];
        file.seek(SeekFrom::Start(0))?;
        read_exact(&mut file, &mut hb, "the header")?;
        let header = Header::decode(&hb)?;
        let footer = if file_len >= (header_len + FOOTER_LEN) as u64 {
            let mut fb = [0u8; FOOTER_LEN];
            file.seek(SeekFrom::End(-(FOOTER_LEN as i64)))?;
            read_exact(&mut file, &mut fb, "the footer")?;
            Footer::decode(&fb)
        } else {
            None
        };
        let (index, recovered) = match footer {
            Some(f) => {
                if f.header_crc != crc32(&hb) {
                    return Err(RecordingError::Corrupt(
                        "the header does not match the footer's CRC-32".to_owned(),
                    ));
                }
                let mut head = [0u8; 8];
                file.seek(SeekFrom::Start(f.index_offset))?;
                read_exact(&mut file, &mut head, "the index")?;
                if &head[0..4] != INDEX_MAGIC {
                    return Err(RecordingError::Corrupt(
                        "the footer does not point at the index".to_owned(),
                    ));
                }
                let mut body = vec![0u8; u32_at(&head, 4) as usize];
                read_exact(&mut file, &mut body, "the index")?;
                if f.index_crc != crc32(&body) {
                    return Err(RecordingError::Corrupt(
                        "the index does not match the footer's CRC-32".to_owned(),
                    ));
                }
                let index = decode_index(&body);
                if index.len() as u64 != f.frame_count || header.frame_count != f.frame_count {
                    return Err(RecordingError::Corrupt(format!(
                        "the header says {} frames, the footer {}, the index {}",
                        header.frame_count,
                        f.frame_count,
                        index.len()
                    )));
                }
                (index, false)
            }
            None if recover => (scan(&mut file, header_len as u64, file_len)?, true),
            None => {
                return Err(RecordingError::Corrupt(
                    "there is no footer, so it is still being written or was cut short; \
                     open it with recovery to read the frames that made it to disk"
                        .to_owned(),
                ));
            }
        };
        for (i, e) in index.iter().enumerate() {
            if e.frame != i as u64 || e.offset + e.len as u64 > file_len {
                return Err(RecordingError::Corrupt(format!(
                    "index entry {i} points outside the file or out of order"
                )));
            }
        }
        if index.first().is_some_and(|e| e.kind != KIND_KEY) {
            return Err(RecordingError::Corrupt(
                "the first frame is not a keyframe".to_owned(),
            ));
        }
        Ok(RomrecSource {
            identity: RecordingIdentity {
                rom_sha256: header.rom_sha256,
                producer: header.producer.clone(),
                producer_version: header.producer_version.clone(),
            },
            header,
            index,
            recovered,
            file_len,
            file: Mutex::new(file),
            cache: Mutex::new(None),
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn index(&self) -> &[IndexEntry] {
        &self.index
    }

    /// Whether the index was rebuilt by scanning.
    pub fn recovered(&self) -> bool {
        self.recovered
    }

    pub fn file_len(&self) -> u64 {
        self.file_len
    }

    /// Refuse a recording of another ROM, the way a project package is
    /// refused (`16-phase2-plan.md` 2C.7, deviation 3).
    pub fn check_rom(&self, rom_sha256: &[u8; 32]) -> Result<(), RecordingError> {
        if &self.identity.rom_sha256 == rom_sha256 {
            return Ok(());
        }
        Err(RecordingError::RomMismatch {
            expected: self.identity.sha256_hex(),
            found: rom_sha256.iter().map(|b| format!("{b:02x}")).collect(),
        })
    }

    fn entry(&self, frame: u64) -> Result<IndexEntry, RecordingError> {
        self.index
            .get(frame as usize)
            .copied()
            .ok_or(RecordingError::NoSuchFrame {
                frame,
                count: self.index.len() as u64,
            })
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, RecordingError> {
        let mut f = self.file.lock().unwrap();
        f.seek(SeekFrom::Start(offset))?;
        let mut b = vec![0u8; len];
        read_exact(&mut *f, &mut b, "a frame")?;
        Ok(b)
    }

    /// A frame's header, directory and run tables, without its payloads.
    pub fn frame_head(&self, frame: u64) -> Result<FrameHead, RecordingError> {
        let e = self.entry(frame)?;
        let fixed = self.read_at(e.offset, FRAME_HEADER_LEN)?;
        if &fixed[0..4] != FRAME_MAGIC {
            return Err(RecordingError::Corrupt(format!(
                "frame {frame} does not start with FRM"
            )));
        }
        let prefix = FRAME_HEADER_LEN
            + u32_at(&fixed, 20) as usize * DIR_ENTRY_LEN
            + u32_at(&fixed, 24) as usize;
        if prefix > e.len as usize {
            return Err(RecordingError::Corrupt(format!(
                "frame {frame}'s directory is longer than the frame"
            )));
        }
        let head = FrameHead::decode(&self.read_at(e.offset, prefix)?)?;
        if head.frame != frame {
            return Err(RecordingError::Corrupt(format!(
                "the index's frame {frame} holds frame {}",
                head.frame
            )));
        }
        Ok(head)
    }

    /// The layer chunks that follow a frame's chunk, magic and body each.
    pub fn layer_chunks(&self, frame: u64) -> Result<Vec<LayerChunk>, RecordingError> {
        let e = self.entry(frame)?;
        let mut at = e.offset + u64::from(e.len);
        let mut out = Vec::new();
        while at + 8 <= self.file_len {
            let head = self.read_at(at, 8)?;
            let magic: [u8; 4] = head[0..4].try_into().unwrap();
            if !LAYER_MAGICS.iter().any(|m| **m == magic) {
                break;
            }
            let len = u32_at(&head, 4) as u64;
            if at + 8 + len > self.file_len {
                break;
            }
            out.push((magic, self.read_at(at + 8, len as usize)?));
            at += 8 + len;
        }
        Ok(out)
    }

    /// Apply one frame's payloads on top of `state`.
    fn apply_frame(&self, frame: u64, state: &mut MachineState) -> Result<(), RecordingError> {
        let e = self.entry(frame)?;
        let chunk = self.read_at(e.offset, e.len as usize)?;
        let head = FrameHead::decode(&chunk)?;
        let mut at = head.payload_at;
        for d in &head.dir {
            let stored = chunk.get(at..at + d.stored_len as usize).ok_or_else(|| {
                RecordingError::Corrupt(format!("frame {frame}'s payloads are truncated"))
            })?;
            at += d.stored_len as usize;
            let raw = unpack(stored, d.raw_len as usize, self.header.compression)?;
            let buf = state
                .regions
                .entry(d.region)
                .or_insert_with(|| vec![0u8; d.region.size()]);
            apply(buf, &d.runs, &raw)
                .map_err(|m| RecordingError::Corrupt(format!("frame {frame}: {m}")))?;
        }
        state.frame = frame;
        Ok(())
    }
}

impl MachineStateSource for RomrecSource {
    fn identity(&self) -> &RecordingIdentity {
        &self.identity
    }

    fn frame_count(&self) -> Option<u64> {
        Some(self.index.len() as u64)
    }

    fn regions(&self) -> Vec<StateRegion> {
        self.header.regions.clone()
    }

    fn state_at(&self, frame: u64) -> Result<MachineState, RecordingError> {
        let e = self.entry(frame)?;
        let key = self.index[..=frame as usize]
            .iter()
            .rposition(|i| i.kind == KIND_KEY)
            .unwrap_or(0) as u64;
        let mut cache = self.cache.lock().unwrap();
        let mut state = match cache.take() {
            Some(c) if c.frame >= key && c.frame <= frame => c,
            _ => {
                let mut s = MachineState::default();
                self.apply_frame(key, &mut s)?;
                s
            }
        };
        for f in state.frame + 1..=frame {
            self.apply_frame(f, &mut state)?;
        }
        *cache = Some(state.clone());
        if self.header.flags & FLAG_WRAM_KEYFRAME_ONLY != 0 && e.kind != KIND_KEY {
            // Between keyframes the WRAM we hold is the keyframe's, which is
            // not this frame's; saying nothing is better than being wrong.
            state.regions.remove(&StateRegion::Wram);
        }
        Ok(state)
    }

    fn changes(&self, from: u64, to: u64, region: StateRegion) -> Result<Vec<Run>, RecordingError> {
        self.entry(to)?;
        if from >= to {
            return Ok(Vec::new());
        }
        let wram_sparse = region == StateRegion::Wram
            && self.header.flags & FLAG_WRAM_KEYFRAME_ONLY != 0
            && self.index[from as usize + 1..=to as usize]
                .iter()
                .any(|e| e.kind != KIND_KEY);
        if wram_sparse {
            // WRAM was not recorded in those frames, so nothing narrower than
            // the whole region is honest.
            return Ok(vec![Run {
                offset: 0,
                len: region.size() as u32,
            }]);
        }
        let mut out = Vec::new();
        for f in from + 1..=to {
            if let Some(d) = self
                .frame_head(f)?
                .dir
                .into_iter()
                .find(|d| d.region == region)
            {
                out = union(&out, &d.changes);
            }
        }
        Ok(out)
    }

    fn layers(&self) -> Layers {
        self.header.layers
    }

    fn line_writes(
        &self,
        frame: u64,
    ) -> Result<Option<Vec<crate::recording::lines::RegWrite>>, RecordingError> {
        if !self.header.layers.line_writes {
            return Ok(None);
        }
        for (magic, body) in self.layer_chunks(frame)? {
            if &magic == LAYER_MAGICS[4] {
                let (_, writes) =
                    crate::recording::lines::decode(&body).map_err(RecordingError::Corrupt)?;
                return Ok(Some(writes));
            }
        }
        Ok(None)
    }

    fn dma_records(
        &self,
        frame: u64,
    ) -> Result<Vec<crate::recording::wlog::DmaRecord>, RecordingError> {
        let mut out = Vec::new();
        for (magic, body) in self.layer_chunks(frame)? {
            if &magic == LAYER_MAGICS[1] {
                let (_, r) =
                    crate::recording::wlog::decode(&body).map_err(RecordingError::Corrupt)?;
                out.extend(r);
            }
        }
        Ok(out)
    }
}

fn read_exact(r: &mut dyn ReadSeek, buf: &mut [u8], what: &str) -> Result<(), RecordingError> {
    r.read_exact(buf).map_err(|e| match e.kind() {
        std::io::ErrorKind::UnexpectedEof => {
            RecordingError::Corrupt(format!("the file ends inside {what}"))
        }
        _ => RecordingError::Io(e.to_string()),
    })
}

/// Rebuild the index by walking chunks from the end of the header, keeping
/// every whole frame chunk in sequence and stopping at the first gap.
fn scan(
    file: &mut Box<dyn ReadSeek>,
    start: u64,
    len: u64,
) -> Result<Vec<IndexEntry>, RecordingError> {
    let mut index = Vec::new();
    let mut at = start;
    while at + 8 <= len {
        let mut head = [0u8; 20];
        let n = (len - at).min(20) as usize;
        file.seek(SeekFrom::Start(at))?;
        file.read_exact(&mut head[..n])?;
        let body = u32_at(&head, 4) as u64;
        let total = 8 + body;
        if at + total > len {
            break; // cut short mid-chunk
        }
        let magic: &[u8] = &head[0..4];
        if magic == FRAME_MAGIC && n >= 20 {
            let frame = u64_at(&head, 8);
            if frame != index.len() as u64 {
                break;
            }
            index.push(IndexEntry {
                frame,
                offset: at,
                len: total as u32,
                kind: head[16],
            });
        } else if !LAYER_MAGICS.iter().any(|m| magic == *m) {
            break; // the index, or garbage
        }
        at += total;
    }
    Ok(index)
}
