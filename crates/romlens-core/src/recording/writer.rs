//! Writing `.romrec` files.
//!
//! The header goes out first with `frame_count` = [`IN_PROGRESS`], frames
//! stream after it, and [`finish`](RomrecWriter::finish) appends the index
//! and footer and seeks back to patch the count. A crashed producer — the
//! common failure — leaves a file whose frames are all still readable by
//! scanning for chunk magics (`RomrecSource::open_recovering`).

use std::collections::BTreeMap;
use std::io::{Seek, SeekFrom, Write};

use crate::io::crc32::crc32;
use crate::recording::delta::{Run, diff};
use crate::recording::format::*;
use crate::recording::{Layers, MachineState, RecordingError, RecordingIdentity, StateRegion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriterOptions {
    pub keyframe_interval: u16,
    pub compress: bool,
    /// Store WRAM in keyframes only (header flag bit 0).
    pub wram_keyframe_only: bool,
    /// 0 LoROM, 1 HiROM, 2 ExHiROM, `0xFF` unknown.
    pub mapping: u8,
}

impl Default for WriterOptions {
    fn default() -> Self {
        WriterOptions {
            keyframe_interval: 60,
            compress: true,
            wram_keyframe_only: false,
            mapping: 0xFF,
        }
    }
}

pub struct RomrecWriter<W: Write + Seek> {
    out: W,
    header: Vec<u8>,
    header_len: u64,
    regions: Vec<StateRegion>,
    options: WriterOptions,
    compression: u32,
    at: u64,
    previous: Option<BTreeMap<StateRegion, Vec<u8>>>,
    index: Vec<IndexEntry>,
}

impl<W: Write + Seek> RomrecWriter<W> {
    pub fn new(
        mut out: W,
        identity: &RecordingIdentity,
        regions: &[StateRegion],
        options: WriterOptions,
        created: u64,
    ) -> Result<Self, RecordingError> {
        let mut regions = regions.to_vec();
        regions.sort();
        regions.dedup();
        let compression = if options.compress && cfg!(feature = "recording") {
            COMPRESSION_ZSTD
        } else {
            COMPRESSION_NONE
        };
        let header = Header {
            version: (VERSION_MAJOR, VERSION_MINOR),
            rom_sha256: identity.rom_sha256,
            mapping: options.mapping,
            flags: if options.wram_keyframe_only {
                FLAG_WRAM_KEYFRAME_ONLY
            } else {
                0
            },
            keyframe_interval: options.keyframe_interval.max(1),
            regions: regions.clone(),
            frame_count: IN_PROGRESS,
            created,
            layers: Layers::default(),
            compression,
            producer: identity.producer.clone(),
            producer_version: identity.producer_version.clone(),
        }
        .encode();
        out.write_all(&header)?;
        Ok(RomrecWriter {
            out,
            header_len: header.len() as u64,
            header,
            regions,
            options,
            compression,
            at: 0,
            previous: None,
            index: Vec::new(),
        })
    }

    /// Frames written so far.
    pub fn frames(&self) -> u64 {
        self.index.len() as u64
    }

    /// Append the next frame. Its `frame` field is ignored: frames are
    /// numbered in the order they are written, from 0.
    pub fn write_frame(&mut self, state: &MachineState) -> Result<(), RecordingError> {
        let frame = self.index.len() as u64;
        for r in &self.regions {
            match state.regions.get(r) {
                Some(bytes) if bytes.len() == r.size() => {}
                Some(bytes) => {
                    return Err(RecordingError::BadFormat(format!(
                        "frame {frame}: {} is {} bytes, not {}",
                        r.name(),
                        bytes.len(),
                        r.size()
                    )));
                }
                None => return Err(RecordingError::MissingRegion(r.name())),
            }
        }
        let key = self.previous.is_none()
            || frame.is_multiple_of(self.options.keyframe_interval.max(1) as u64);
        let mut dir = Vec::new();
        let mut payloads = Vec::new();
        for r in &self.regions {
            let now = &state.regions[r];
            let before = self.previous.as_ref().map(|p| &p[r]);
            let changed = match before {
                Some(b) => diff(b, now),
                None => vec![Run {
                    offset: 0,
                    len: r.size() as u32,
                }],
            };
            let whole = vec![Run {
                offset: 0,
                len: r.size() as u32,
            }];
            let (runs, changes) = if key || r.always_whole() {
                // Stored whole, with what actually changed said separately.
                (whole, changed)
            } else if changed.is_empty()
                || (*r == StateRegion::Wram && self.options.wram_keyframe_only)
            {
                continue;
            } else {
                (changed.clone(), changed)
            };
            let raw: Vec<u8> = runs
                .iter()
                .flat_map(|run| now[run.offset as usize..run.end() as usize].iter().copied())
                .collect();
            let stored = pack(&raw, self.compression);
            dir.push(DirEntry {
                region: *r,
                runs,
                changes,
                stored_len: stored.len() as u32,
                raw_len: raw.len() as u32,
            });
            payloads.push(stored);
        }
        let kind = if key { KIND_KEY } else { KIND_DELTA };
        let chunk = encode_frame(frame, kind, &dir, &payloads);
        self.out.write_all(&chunk)?;
        self.index.push(IndexEntry {
            frame,
            offset: self.header_len + self.at,
            len: chunk.len() as u32,
            kind,
        });
        self.at += chunk.len() as u64;
        self.previous = Some(state.regions.clone());
        Ok(())
    }

    /// Write the index and footer and patch the header's frame count.
    pub fn finish(mut self) -> Result<W, RecordingError> {
        let index_offset = self.header_len + self.at;
        let index = encode_index(&self.index);
        self.out.write_all(&index)?;
        let count = self.index.len() as u64;
        self.header[FRAME_COUNT_AT as usize..FRAME_COUNT_AT as usize + 8]
            .copy_from_slice(&count.to_le_bytes());
        let footer = Footer {
            index_offset,
            frame_count: count,
            header_crc: crc32(&self.header),
            index_crc: crc32(&index[8..]),
        };
        self.out.write_all(&footer.encode())?;
        self.out.seek(SeekFrom::Start(FRAME_COUNT_AT))?;
        self.out.write_all(&count.to_le_bytes())?;
        self.out.seek(SeekFrom::End(0))?;
        self.out.flush()?;
        Ok(self.out)
    }
}
