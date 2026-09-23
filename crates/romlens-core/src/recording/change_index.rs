//! `ChangeIndex`: which frames changed a byte, without reading every frame.
//!
//! Each indexed region is cut into 256-byte blocks, and each block keeps the
//! sorted list of frames whose change runs touch it: delta-encoded varints,
//! or a bitmap once more than a quarter of the frames are in it. **The index
//! narrows; it never answers.** A block can list a frame whose changes fell
//! elsewhere in its 256 bytes, so every candidate is confirmed against that
//! frame's own change runs before it is returned (`16-phase2-plan.md` 2C).
//!
//! Frame 0 counts as a change to everything: it is where every byte's
//! recorded history starts, which is what "changed at frame N" needs.
//!
//! WRAM is indexed only when every frame carries it. Stored in keyframes
//! only, its changes between keyframes are not in the file, so the honest
//! answer is "somewhere in these 60 frames", which a per-frame index cannot
//! give.
//!
//! The sidecar is keyed on the recording's length, frame count and the
//! CRC-32 of its index entries: content, not a modification time, so a copied
//! recording keeps its index and a rewritten one never matches a stale one.

use crate::io::crc32::crc32;
use crate::recording::delta::Run;
use crate::recording::format::{FLAG_WRAM_KEYFRAME_ONLY, encode_index};
use crate::recording::{MachineStateSource, RecordingError, RomrecSource, StateRegion};

pub const BLOCK: u32 = 256;
/// Above this share of frames a block's list is stored as a bitmap.
pub const DENSE: f64 = 0.25;
pub const MAGIC: &[u8; 8] = b"ROMRIDX\0";
pub const VERSION: u32 = 1;

/// What a sidecar must match to belong to a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexKey {
    pub file_len: u64,
    pub frames: u64,
    pub index_crc: u32,
}

impl IndexKey {
    pub fn of(src: &RomrecSource) -> IndexKey {
        IndexKey {
            file_len: src.file_len(),
            frames: src.index().len() as u64,
            index_crc: crc32(&encode_index(src.index())[8..]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Frames {
    /// Delta-encoded LEB128 varints, first value absolute.
    Sparse { count: u32, bytes: Vec<u8> },
    /// One bit per frame.
    Dense { count: u32, bits: Vec<u64> },
}

impl Frames {
    fn from_sorted(list: &[u64], frames: u64) -> Frames {
        if list.len() as f64 > frames as f64 * DENSE {
            let mut bits = vec![0u64; (frames as usize).div_ceil(64)];
            for &f in list {
                bits[f as usize / 64] |= 1 << (f % 64);
            }
            return Frames::Dense {
                count: list.len() as u32,
                bits,
            };
        }
        let mut bytes = Vec::new();
        let mut prev = 0u64;
        for &f in list {
            let mut v = f - prev;
            prev = f;
            loop {
                let b = (v & 0x7F) as u8;
                v >>= 7;
                if v == 0 {
                    bytes.push(b);
                    break;
                }
                bytes.push(b | 0x80);
            }
        }
        Frames::Sparse {
            count: list.len() as u32,
            bytes,
        }
    }

    fn list(&self) -> Vec<u64> {
        match self {
            Frames::Sparse { count, bytes } => {
                let mut out = Vec::with_capacity(*count as usize);
                let (mut acc, mut shift, mut prev) = (0u64, 0u32, 0u64);
                for &b in bytes {
                    acc |= ((b & 0x7F) as u64) << shift;
                    if b & 0x80 == 0 {
                        prev += acc;
                        out.push(prev);
                        acc = 0;
                        shift = 0;
                    } else {
                        shift += 7;
                    }
                }
                out
            }
            Frames::Dense { bits, .. } => bits
                .iter()
                .enumerate()
                .flat_map(|(w, &word)| {
                    (0..64)
                        .filter(move |b| word >> b & 1 == 1)
                        .map(move |b| w as u64 * 64 + b)
                })
                .collect(),
        }
    }

    fn count(&self) -> u32 {
        match self {
            Frames::Sparse { count, .. } | Frames::Dense { count, .. } => *count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegionIndex {
    region: StateRegion,
    blocks: Vec<Frames>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeIndex {
    pub key: IndexKey,
    regions: Vec<RegionIndex>,
}

fn overlaps(runs: &[Run], offset: u32, len: u32) -> bool {
    let end = offset + len.max(1);
    runs.iter().any(|r| r.offset < end && offset < r.end())
}

impl ChangeIndex {
    /// Read every frame's directory once and build the index.
    pub fn build(src: &RomrecSource) -> Result<ChangeIndex, RecordingError> {
        let frames = src.index().len() as u64;
        let wram_whole = src.header().flags & FLAG_WRAM_KEYFRAME_ONLY == 0;
        let regions: Vec<StateRegion> = src
            .regions()
            .into_iter()
            .filter(|r| *r != StateRegion::Wram || wram_whole)
            .collect();
        let mut lists: Vec<Vec<Vec<u64>>> = regions
            .iter()
            .map(|r| vec![Vec::new(); (r.size() as u32).div_ceil(BLOCK) as usize])
            .collect();
        for f in 0..frames {
            let head = src.frame_head(f)?;
            for (i, r) in regions.iter().enumerate() {
                let Some(d) = head.dir.iter().find(|d| d.region == *r) else {
                    continue;
                };
                let runs: &[Run] = if f == 0 { &d.runs } else { &d.changes };
                let mut touched = Vec::new();
                for run in runs {
                    let first = run.offset / BLOCK;
                    let last = (run.end().max(run.offset + 1) - 1) / BLOCK;
                    touched.extend(first..=last);
                }
                touched.sort_unstable();
                touched.dedup();
                for b in touched {
                    lists[i][b as usize].push(f);
                }
            }
        }
        Ok(ChangeIndex {
            key: IndexKey::of(src),
            regions: regions
                .into_iter()
                .zip(lists)
                .map(|(region, blocks)| RegionIndex {
                    region,
                    blocks: blocks
                        .iter()
                        .map(|l| Frames::from_sorted(l, frames))
                        .collect(),
                })
                .collect(),
        })
    }

    /// Whether `region` is indexed (see the module note on WRAM).
    pub fn covers(&self, region: StateRegion) -> bool {
        self.regions.iter().any(|r| r.region == region)
    }

    /// Frames that may have changed `[offset, offset + len)`, sorted: a
    /// superset, before confirmation.
    pub fn candidates(&self, region: StateRegion, offset: u32, len: u32) -> Vec<u64> {
        let Some(ri) = self.regions.iter().find(|r| r.region == region) else {
            return Vec::new();
        };
        let first = (offset / BLOCK) as usize;
        let last = ((offset + len.max(1) - 1) / BLOCK) as usize;
        let mut out: Vec<u64> = ri
            .blocks
            .get(first..=last.min(ri.blocks.len().saturating_sub(1)))
            .unwrap_or(&[])
            .iter()
            .flat_map(Frames::list)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The first frame after `after` (or, `backward`, the last at or before
    /// it) whose change runs touch `[offset, offset + len)`: confirmed, never
    /// a guess. `None` when there is none, or the region is not indexed.
    pub fn when(
        &self,
        src: &RomrecSource,
        region: StateRegion,
        offset: u32,
        len: u32,
        after: u64,
        backward: bool,
    ) -> Result<Option<u64>, RecordingError> {
        let mut cands = self.candidates(region, offset, len);
        if backward {
            cands.retain(|&f| f <= after);
            cands.reverse();
        } else {
            cands.retain(|&f| f > after);
        }
        for f in cands {
            let head = src.frame_head(f)?;
            if let Some(d) = head.dir.iter().find(|d| d.region == region) {
                let runs = if f == 0 { &d.runs } else { &d.changes };
                if overlaps(runs, offset, len) {
                    return Ok(Some(f));
                }
            }
        }
        Ok(None)
    }

    /// Total list entries and the sidecar's size, for `rec index`.
    pub fn stats(&self) -> (u64, usize) {
        let entries = self
            .regions
            .iter()
            .flat_map(|r| &r.blocks)
            .map(|b| b.count() as u64)
            .sum();
        (entries, self.encode().len())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(&VERSION.to_le_bytes());
        b.extend_from_slice(&self.key.file_len.to_le_bytes());
        b.extend_from_slice(&self.key.frames.to_le_bytes());
        b.extend_from_slice(&self.key.index_crc.to_le_bytes());
        b.extend_from_slice(&(self.regions.len() as u32).to_le_bytes());
        for r in &self.regions {
            b.extend_from_slice(&r.region.id().to_le_bytes());
            b.extend_from_slice(&(r.blocks.len() as u32).to_le_bytes());
            for f in &r.blocks {
                let (kind, count, body): (u8, u32, Vec<u8>) = match f {
                    Frames::Sparse { count, bytes } => (0, *count, bytes.clone()),
                    Frames::Dense { count, bits } => (
                        1,
                        *count,
                        bits.iter().flat_map(|w| w.to_le_bytes()).collect(),
                    ),
                };
                b.push(kind);
                b.extend_from_slice(&count.to_le_bytes());
                b.extend_from_slice(&(body.len() as u32).to_le_bytes());
                b.extend_from_slice(&body);
            }
        }
        b
    }

    /// Read a sidecar; `None` if it is damaged or from another version.
    pub fn decode(b: &[u8]) -> Option<ChangeIndex> {
        let mut at = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = b.get(at..at + n)?;
            at += n;
            Some(s)
        };
        if take(8)? != MAGIC {
            return None;
        }
        let u32_ = |s: &[u8]| u32::from_le_bytes(s.try_into().unwrap());
        let u64_ = |s: &[u8]| u64::from_le_bytes(s.try_into().unwrap());
        if u32_(take(4)?) != VERSION {
            return None;
        }
        let key = IndexKey {
            file_len: u64_(take(8)?),
            frames: u64_(take(8)?),
            index_crc: u32_(take(4)?),
        };
        let mut regions = Vec::new();
        for _ in 0..u32_(take(4)?) {
            let region = StateRegion::from_id(u32_(take(4)?))?;
            let n = u32_(take(4)?) as usize;
            let mut blocks = Vec::with_capacity(n);
            for _ in 0..n {
                let kind = take(1)?[0];
                let count = u32_(take(4)?);
                let len = u32_(take(4)?) as usize;
                let body = take(len)?;
                blocks.push(match kind {
                    0 => Frames::Sparse {
                        count,
                        bytes: body.to_vec(),
                    },
                    1 => Frames::Dense {
                        count,
                        bits: body
                            .as_chunks::<8>()
                            .0
                            .iter()
                            .map(|c| u64::from_le_bytes(*c))
                            .collect(),
                    },
                    _ => return None,
                });
            }
            regions.push(RegionIndex { region, blocks });
        }
        (at == b.len()).then_some(ChangeIndex { key, regions })
    }
}

impl IndexKey {
    /// `len:frames:crc`, the form a project keeps.
    pub fn fingerprint(&self) -> String {
        format!("{}:{}:{:08x}", self.file_len, self.frames, self.index_crc)
    }
}

/// What a project stores to refer to the recording at `path`.
pub fn reference(path: &str, src: &RomrecSource) -> crate::model::project::RecordingRef {
    crate::model::project::RecordingRef {
        path: path.to_owned(),
        frames: src.index().len() as u64,
        producer: src.identity().producer.clone(),
        fingerprint: IndexKey::of(src).fingerprint(),
    }
}

/// Where a recording's index lives: beside it, `<name>.romrec.idx`.
pub fn sidecar_path(rec: &std::path::Path) -> std::path::PathBuf {
    let mut name = rec.as_os_str().to_owned();
    name.push(".idx");
    name.into()
}

/// The recording's index: its sidecar when that matches, otherwise built and
/// saved beside it. Saving is best effort (the folder may be read-only); the
/// second value says whether the index was built rather than loaded.
pub fn load_or_build(
    rec: &std::path::Path,
    src: &RomrecSource,
    rebuild: bool,
) -> Result<(ChangeIndex, bool), RecordingError> {
    let path = sidecar_path(rec);
    if !rebuild
        && let Some(index) = std::fs::read(&path)
            .ok()
            .and_then(|b| ChangeIndex::decode(&b))
        && index.key == IndexKey::of(src)
    {
        return Ok((index, false));
    }
    let index = ChangeIndex::build(src)?;
    if !src.recovered() {
        let _ = std::fs::write(&path, index.encode());
    }
    Ok((index, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_lists_round_trip_both_ways() {
        let sparse = [0u64, 3, 200, 70_000, 70_001];
        let f = Frames::from_sorted(&sparse, 100_000);
        assert!(matches!(f, Frames::Sparse { .. }));
        assert_eq!(f.list(), sparse);
        let dense: Vec<u64> = (0..100).filter(|n| n % 3 != 0).collect();
        let f = Frames::from_sorted(&dense, 100);
        assert!(matches!(f, Frames::Dense { .. }));
        assert_eq!(f.list(), dense);
    }
}
