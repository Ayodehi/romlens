//! The `.romrec` byte layout (`13-recording-format.md`, "File layout").
//! Little-endian throughout.
//!
//! ```text
//! header        128 bytes fixed, then the region table (16 bytes a region)
//!               and a string area; header_len covers all three
//! FRM\0 chunks  one per frame, in frame order
//! other chunks  FBUF WLOG TRCE RLOG LINE: layers, skipped by readers that do not use them
//! IDX\0 chunk   24 bytes per frame
//! footer        32 bytes, ending "ROMR"
//! ```
//!
//! A frame chunk is a 28-byte header, a directory of 20 bytes per region
//! present, the **run area** uncompressed, then one payload per region. The
//! run area is the layout choice that matters: saying what changed in a frame
//! reads a few hundred bytes, never the 197 KB the frame describes.

use crate::recording::delta::Run;
use crate::recording::{Layers, RecordingError, StateRegion};

pub const MAGIC: &[u8; 8] = b"ROMREC\0\0";
pub const VERSION_MAJOR: u16 = 1;
pub const VERSION_MINOR: u16 = 1;
pub const HEADER_FIXED_LEN: usize = 128;
pub const REGION_ENTRY_LEN: usize = 16;
pub const FRAME_MAGIC: &[u8; 4] = b"FRM\0";
pub const INDEX_MAGIC: &[u8; 4] = b"IDX\0";
/// Reserved layer chunks. Nothing in Phase 2 writes them.
/// The layer chunks, by header layer bit: framebuffer, write log, trace,
/// read log, and (1.1) PPU register writes by scanline.
pub const LAYER_MAGICS: [&[u8; 4]; 5] = [b"FBUF", b"WLOG", b"TRCE", b"RLOG", b"LINE"];
pub const FRAME_HEADER_LEN: usize = 28;
pub const DIR_ENTRY_LEN: usize = 20;
pub const INDEX_ENTRY_LEN: usize = 24;
pub const FOOTER_LEN: usize = 32;
pub const FOOTER_MAGIC: &[u8; 4] = b"ROMR";
/// `frame_count` while the file is being written; a file still saying this
/// has no valid footer yet and is *in progress*.
pub const IN_PROGRESS: u64 = u64::MAX;
/// Header offset of `frame_count`, patched when the writer finishes.
pub const FRAME_COUNT_AT: u64 = 56;

/// Header flag: WRAM is stored in keyframes only.
pub const FLAG_WRAM_KEYFRAME_ONLY: u8 = 1;

pub const COMPRESSION_NONE: u32 = 0;
pub const COMPRESSION_ZSTD: u32 = 1;

pub const KIND_KEY: u8 = 0;
pub const KIND_DELTA: u8 = 1;

pub(crate) fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
pub(crate) fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
pub(crate) fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// Everything the header says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub version: (u16, u16),
    pub rom_sha256: [u8; 32],
    /// 0 LoROM, 1 HiROM, 2 ExHiROM, `0xFF` unknown.
    pub mapping: u8,
    pub flags: u8,
    pub keyframe_interval: u16,
    pub regions: Vec<StateRegion>,
    pub frame_count: u64,
    /// Unix seconds.
    pub created: u64,
    pub layers: Layers,
    pub compression: u32,
    pub producer: String,
    pub producer_version: String,
}

impl Header {
    pub fn encode(&self) -> Vec<u8> {
        let mut strings: Vec<u8> = Vec::new();
        let mut string = |s: &str| {
            let at = strings.len() as u32;
            strings.extend_from_slice(s.as_bytes());
            (at, s.len() as u32)
        };
        let producer = string(&self.producer);
        let version = string(&self.producer_version);
        let names: Vec<(u32, u32)> = self.regions.iter().map(|r| string(r.name())).collect();
        let header_len = HEADER_FIXED_LEN + self.regions.len() * REGION_ENTRY_LEN + strings.len();
        let mut b = vec![0u8; HEADER_FIXED_LEN];
        b[0..8].copy_from_slice(MAGIC);
        b[8..10].copy_from_slice(&self.version.0.to_le_bytes());
        b[10..12].copy_from_slice(&self.version.1.to_le_bytes());
        b[12..16].copy_from_slice(&(header_len as u32).to_le_bytes());
        b[16..48].copy_from_slice(&self.rom_sha256);
        b[48] = self.mapping;
        b[49] = self.flags;
        b[50..52].copy_from_slice(&self.keyframe_interval.to_le_bytes());
        b[52..56].copy_from_slice(&(self.regions.len() as u32).to_le_bytes());
        b[56..64].copy_from_slice(&self.frame_count.to_le_bytes());
        b[64..72].copy_from_slice(&self.created.to_le_bytes());
        b[72..76].copy_from_slice(&self.layers.bits().to_le_bytes());
        b[76..80].copy_from_slice(&self.compression.to_le_bytes());
        b[80..84].copy_from_slice(&producer.0.to_le_bytes());
        b[84..88].copy_from_slice(&producer.1.to_le_bytes());
        b[88..92].copy_from_slice(&version.0.to_le_bytes());
        b[92..96].copy_from_slice(&version.1.to_le_bytes());
        for (r, (at, len)) in self.regions.iter().zip(names) {
            b.extend_from_slice(&r.id().to_le_bytes());
            b.extend_from_slice(&(r.size() as u32).to_le_bytes());
            b.extend_from_slice(&at.to_le_bytes());
            b.extend_from_slice(&len.to_le_bytes());
        }
        b.extend_from_slice(&strings);
        b
    }

    /// The header's total length from its first 16 bytes, so a reader knows
    /// how much to read before [`decode`](Self::decode).
    pub fn declared_len(first: &[u8]) -> Result<usize, RecordingError> {
        if first.len() < 16 || &first[0..8] != MAGIC {
            return Err(RecordingError::BadFormat(
                "it does not start with ROMREC".to_owned(),
            ));
        }
        let len = u32_at(first, 12) as usize;
        if !(HEADER_FIXED_LEN..=1 << 20).contains(&len) {
            return Err(RecordingError::Corrupt(format!(
                "the header claims to be {len} bytes long"
            )));
        }
        Ok(len)
    }

    pub fn decode(b: &[u8]) -> Result<Header, RecordingError> {
        let len = Header::declared_len(b)?;
        if b.len() < len {
            return Err(RecordingError::Corrupt(
                "the header is truncated".to_owned(),
            ));
        }
        let version = (u16_at(b, 8), u16_at(b, 10));
        if version.0 > VERSION_MAJOR {
            return Err(RecordingError::NewerVersion(version.0, version.1));
        }
        let count = u32_at(b, 52) as usize;
        let strings_at = HEADER_FIXED_LEN + count * REGION_ENTRY_LEN;
        if strings_at > len {
            return Err(RecordingError::Corrupt(format!(
                "{count} regions do not fit a {len}-byte header"
            )));
        }
        let strings = &b[strings_at..len];
        let text = |at: usize| -> Result<String, RecordingError> {
            let (o, l) = (u32_at(b, at) as usize, u32_at(b, at + 4) as usize);
            strings
                .get(o..o + l)
                .and_then(|s| std::str::from_utf8(s).ok())
                .map(str::to_owned)
                .ok_or_else(|| {
                    RecordingError::Corrupt("a header string is out of range".to_owned())
                })
        };
        let mut regions = Vec::with_capacity(count);
        for i in 0..count {
            let at = HEADER_FIXED_LEN + i * REGION_ENTRY_LEN;
            let id = u32_at(b, at);
            let size = u32_at(b, at + 4) as usize;
            let r = StateRegion::from_id(id)
                .ok_or_else(|| RecordingError::Corrupt(format!("unknown region id {id}")))?;
            if size != r.size() {
                return Err(RecordingError::Corrupt(format!(
                    "region {} is {size} bytes; the format fixes it at {}",
                    r.name(),
                    r.size()
                )));
            }
            regions.push(r);
        }
        let mut rom_sha256 = [0u8; 32];
        rom_sha256.copy_from_slice(&b[16..48]);
        Ok(Header {
            version,
            rom_sha256,
            mapping: b[48],
            flags: b[49],
            keyframe_interval: u16_at(b, 50),
            regions,
            frame_count: u64_at(b, 56),
            created: u64_at(b, 64),
            layers: Layers::from_bits(u32_at(b, 72)),
            compression: u32_at(b, 76),
            producer: text(80)?,
            producer_version: text(88)?,
        })
    }
}

/// One region's entry in a frame's directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub region: StateRegion,
    /// The runs the payload fills.
    pub runs: Vec<Run>,
    /// What changed since the previous frame. Always written out, because it
    /// differs from `runs` exactly when it matters: a keyframe stores every
    /// region whole though only some of it changed. A region missing from a
    /// delta frame's directory did not change.
    pub changes: Vec<Run>,
    pub stored_len: u32,
    pub raw_len: u32,
}

/// A frame chunk's header and directory, without the payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHead {
    pub frame: u64,
    pub kind: u8,
    pub dir: Vec<DirEntry>,
    /// Bytes from the chunk's start to its first payload.
    pub payload_at: usize,
}

impl FrameHead {
    /// Parse from the start of a chunk. `b` need only reach the end of the
    /// run area.
    pub fn decode(b: &[u8]) -> Result<FrameHead, RecordingError> {
        let bad = |m: &str| RecordingError::Corrupt(m.to_owned());
        if b.len() < FRAME_HEADER_LEN || &b[0..4] != FRAME_MAGIC {
            return Err(bad("a frame chunk does not start with FRM"));
        }
        let frame = u64_at(b, 8);
        let kind = b[16];
        let count = u32_at(b, 20) as usize;
        let runs_len = u32_at(b, 24) as usize;
        let dir_at = FRAME_HEADER_LEN;
        let runs_at = dir_at + count * DIR_ENTRY_LEN;
        if b.len() < runs_at + runs_len {
            return Err(bad("a frame's directory runs past the chunk"));
        }
        let mut cursor = runs_at;
        let mut read_runs = |n: usize| -> Result<Vec<Run>, RecordingError> {
            if cursor + n * 8 > runs_at + runs_len {
                return Err(bad("a frame's run area is shorter than its directory says"));
            }
            let v = (0..n)
                .map(|k| Run {
                    offset: u32_at(b, cursor + k * 8),
                    len: u32_at(b, cursor + k * 8 + 4),
                })
                .collect();
            cursor += n * 8;
            Ok(v)
        };
        let mut dir = Vec::with_capacity(count);
        for i in 0..count {
            let at = dir_at + i * DIR_ENTRY_LEN;
            let id = u32_at(b, at);
            let region =
                StateRegion::from_id(id).ok_or_else(|| bad("a frame names an unknown region"))?;
            let runs = read_runs(u32_at(b, at + 4) as usize)?;
            let changes = read_runs(u32_at(b, at + 8) as usize)?;
            dir.push(DirEntry {
                region,
                runs,
                changes,
                stored_len: u32_at(b, at + 12),
                raw_len: u32_at(b, at + 16),
            });
        }
        Ok(FrameHead {
            frame,
            kind,
            dir,
            payload_at: runs_at + runs_len,
        })
    }
}

/// Encode a whole frame chunk from its directory and stored payloads.
pub fn encode_frame(frame: u64, kind: u8, dir: &[DirEntry], payloads: &[Vec<u8>]) -> Vec<u8> {
    let runs_len: usize = dir
        .iter()
        .map(|d| (d.runs.len() + d.changes.len()) * 8)
        .sum();
    let body: usize = payloads.iter().map(Vec::len).sum();
    let total = FRAME_HEADER_LEN + dir.len() * DIR_ENTRY_LEN + runs_len + body;
    let mut b = Vec::with_capacity(total);
    b.extend_from_slice(FRAME_MAGIC);
    b.extend_from_slice(&((total - 8) as u32).to_le_bytes());
    b.extend_from_slice(&frame.to_le_bytes());
    b.extend_from_slice(&[kind, 0, 0, 0]);
    b.extend_from_slice(&(dir.len() as u32).to_le_bytes());
    b.extend_from_slice(&(runs_len as u32).to_le_bytes());
    for d in dir {
        b.extend_from_slice(&d.region.id().to_le_bytes());
        b.extend_from_slice(&(d.runs.len() as u32).to_le_bytes());
        b.extend_from_slice(&(d.changes.len() as u32).to_le_bytes());
        b.extend_from_slice(&d.stored_len.to_le_bytes());
        b.extend_from_slice(&d.raw_len.to_le_bytes());
    }
    for d in dir {
        for r in d.runs.iter().chain(&d.changes) {
            b.extend_from_slice(&r.offset.to_le_bytes());
            b.extend_from_slice(&r.len.to_le_bytes());
        }
    }
    for p in payloads {
        b.extend_from_slice(p);
    }
    b
}

/// One frame's index entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    pub frame: u64,
    pub offset: u64,
    pub len: u32,
    pub kind: u8,
}

pub fn encode_index(entries: &[IndexEntry]) -> Vec<u8> {
    let mut b = Vec::with_capacity(8 + entries.len() * INDEX_ENTRY_LEN);
    b.extend_from_slice(INDEX_MAGIC);
    b.extend_from_slice(&((entries.len() * INDEX_ENTRY_LEN) as u32).to_le_bytes());
    for e in entries {
        b.extend_from_slice(&e.frame.to_le_bytes());
        b.extend_from_slice(&e.offset.to_le_bytes());
        b.extend_from_slice(&e.len.to_le_bytes());
        b.extend_from_slice(&[e.kind, 0, 0, 0]);
    }
    b
}

pub fn decode_index(body: &[u8]) -> Vec<IndexEntry> {
    body.as_chunks::<INDEX_ENTRY_LEN>()
        .0
        .iter()
        .map(|e| IndexEntry {
            frame: u64_at(e, 0),
            offset: u64_at(e, 8),
            len: u32_at(e, 16),
            kind: e[20],
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Footer {
    pub index_offset: u64,
    pub frame_count: u64,
    pub header_crc: u32,
    pub index_crc: u32,
}

impl Footer {
    pub fn encode(&self) -> [u8; FOOTER_LEN] {
        let mut b = [0u8; FOOTER_LEN];
        b[0..8].copy_from_slice(&self.index_offset.to_le_bytes());
        b[8..16].copy_from_slice(&self.frame_count.to_le_bytes());
        b[16..20].copy_from_slice(&self.header_crc.to_le_bytes());
        b[20..24].copy_from_slice(&self.index_crc.to_le_bytes());
        b[28..32].copy_from_slice(FOOTER_MAGIC);
        b
    }

    /// `None` when the last four bytes are not `ROMR`: the file is in
    /// progress or truncated.
    pub fn decode(b: &[u8]) -> Option<Footer> {
        if b.len() != FOOTER_LEN || &b[28..32] != FOOTER_MAGIC {
            return None;
        }
        Some(Footer {
            index_offset: u64_at(b, 0),
            frame_count: u64_at(b, 8),
            header_crc: u32_at(b, 16),
            index_crc: u32_at(b, 20),
        })
    }
}

/// Compress one payload.
pub fn pack(raw: &[u8], compression: u32) -> Vec<u8> {
    match compression {
        #[cfg(feature = "recording")]
        COMPRESSION_ZSTD => {
            ruzstd::encoding::compress_to_vec(raw, ruzstd::encoding::CompressionLevel::Fastest)
        }
        _ => raw.to_vec(),
    }
}

/// Decompress one payload, checking it comes to `raw_len`.
pub fn unpack(stored: &[u8], raw_len: usize, compression: u32) -> Result<Vec<u8>, RecordingError> {
    let out = match compression {
        COMPRESSION_NONE => stored.to_vec(),
        #[cfg(feature = "recording")]
        COMPRESSION_ZSTD => {
            use std::io::Read;
            let mut src = stored;
            let mut dec = ruzstd::decoding::StreamingDecoder::new(&mut src)
                .map_err(|e| RecordingError::Corrupt(format!("a payload is not zstd: {e}")))?;
            let mut out = Vec::with_capacity(raw_len);
            dec.read_to_end(&mut out).map_err(|e| {
                RecordingError::Corrupt(format!("a payload does not decompress: {e}"))
            })?;
            out
        }
        other => {
            return Err(RecordingError::BadFormat(format!(
                "compression {other} is not supported by this build"
            )));
        }
    };
    if out.len() != raw_len {
        return Err(RecordingError::Corrupt(format!(
            "a payload decompressed to {} bytes; its directory says {raw_len}",
            out.len()
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trips() {
        let h = Header {
            version: (VERSION_MAJOR, VERSION_MINOR),
            rom_sha256: [7; 32],
            mapping: 0,
            flags: FLAG_WRAM_KEYFRAME_ONLY,
            keyframe_interval: 60,
            regions: StateRegion::ALL.to_vec(),
            frame_count: IN_PROGRESS,
            created: 1_790_000_000,
            layers: Layers::default(),
            compression: COMPRESSION_ZSTD,
            producer: "romlens testrec".to_owned(),
            producer_version: "0.1.0".to_owned(),
        };
        let b = h.encode();
        assert_eq!(Header::declared_len(&b).unwrap(), b.len());
        assert_eq!(Header::decode(&b).unwrap(), h);
        assert_eq!(u64_at(&b, FRAME_COUNT_AT as usize), IN_PROGRESS);
        let mut newer = b.clone();
        newer[8] = 9;
        assert_eq!(
            Header::decode(&newer),
            Err(RecordingError::NewerVersion(9, VERSION_MINOR))
        );
        assert!(matches!(
            Header::decode(b"NOTAREC!"),
            Err(RecordingError::BadFormat(_))
        ));
    }

    #[test]
    fn frame_head_and_footer_round_trip() {
        let dir = vec![DirEntry {
            region: StateRegion::Oam,
            runs: vec![Run { offset: 0, len: 4 }],
            changes: vec![Run { offset: 1, len: 1 }],
            stored_len: 4,
            raw_len: 4,
        }];
        let chunk = encode_frame(9, KIND_DELTA, &dir, &[vec![1, 2, 3, 4]]);
        let head = FrameHead::decode(&chunk).unwrap();
        assert_eq!((head.frame, head.kind), (9, KIND_DELTA));
        assert_eq!(head.dir, dir);
        assert_eq!(&chunk[head.payload_at..], &[1, 2, 3, 4]);
        assert_eq!(u32_at(&chunk, 4) as usize, chunk.len() - 8);
        let f = Footer {
            index_offset: 100,
            frame_count: 3,
            header_crc: 1,
            index_crc: 2,
        };
        assert_eq!(Footer::decode(&f.encode()), Some(f));
        assert_eq!(Footer::decode(&[0; FOOTER_LEN]), None);
    }

    #[test]
    fn payloads_compress_and_come_back() {
        let raw: Vec<u8> = (0..4096u32).map(|i| (i % 7) as u8).collect();
        let stored = pack(&raw, COMPRESSION_ZSTD);
        assert!(stored.len() < raw.len() / 4, "{}", stored.len());
        assert_eq!(unpack(&stored, raw.len(), COMPRESSION_ZSTD).unwrap(), raw);
        assert!(unpack(&stored, raw.len() + 1, COMPRESSION_ZSTD).is_err());
        assert!(unpack(&[1, 2, 3], 3, COMPRESSION_ZSTD).is_err());
    }
}
