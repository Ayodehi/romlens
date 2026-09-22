//! The whole-ROM overview, reduced in the core.
//!
//! The strip is a few hundred pixels wide and the image is megabytes, so one
//! column is thousands of bytes. The reduction happens here rather than in a
//! shell for two reasons: every shell would otherwise write the same loop and
//! get it subtly differently, and the alternative — handing a shell every
//! region and letting it bucket them — does not survive Phase 2's region
//! count. `Workbench::regions_summary` returns one record per region, which
//! was fine at a thousand and is not at forty thousand.
//!
//! A column that is not all one kind says so. A strip that painted the
//! majority kind and stopped would claim a bank was code because 51% of it
//! was, which is exactly the kind of confident wrongness the whole phase is
//! trying to avoid.

use crate::analysis::heuristics::EntropyProfile;
use crate::analysis::snapshot::AnalysisSnapshot;
use crate::model::coverage::Coverage;
use crate::model::region::RegionKind;

/// One column of the strip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SummaryBucket {
    pub start: u32,
    pub len: u32,
    /// The kind covering most of the column.
    pub kind: RegionKind,
    /// Mean confidence over the bytes of `kind`, 0.0..=1.0.
    pub confidence: f32,
    /// How much of the column is `kind`, 0.0..=1.0.
    pub share: f32,
    /// Mean entropy in bits per byte, for a second lane.
    pub entropy: f32,
    /// How much of the column an imported trace saw execute.
    pub executed: f32,
}

impl SummaryBucket {
    /// Whether the column blends kinds. The shell hatches these, so a reader
    /// can tell "this bank is code" from "this bank is mostly code".
    pub fn mixed(&self) -> bool {
        self.share < 0.9
    }

    pub fn end(&self) -> u32 {
        self.start + self.len
    }
}

/// Reduce a snapshot to `buckets` columns.
///
/// `buckets` is the strip's width in pixels. Asking for more columns than the
/// image has bytes gives one column per byte and no more.
pub fn summarize(
    snapshot: &AnalysisSnapshot,
    rom_len: u32,
    buckets: u32,
    entropy: Option<&EntropyProfile>,
    coverage: Option<&Coverage>,
) -> Vec<SummaryBucket> {
    let buckets = buckets.clamp(1, rom_len.max(1));
    let mut out = Vec::with_capacity(buckets as usize);
    // Regions are sorted and cover the image, so one pass over them fills every
    // column: the alternative, a region lookup per column, is a binary search
    // per pixel for no gain.
    let mut region_i = 0usize;
    for i in 0..buckets {
        // Spread the remainder rather than leaving a short last column, so the
        // strip's columns stay within a byte of each other.
        let start = (i as u64 * rom_len as u64 / buckets as u64) as u32;
        let end = ((i as u64 + 1) * rom_len as u64 / buckets as u64) as u32;
        let end = end.max(start + 1).min(rom_len);
        // Bytes per kind code, and the confidence they carry.
        let mut bytes = [0u32; 16];
        let mut weighted = [0f32; 16];
        let mut best: Option<RegionKind> = None;
        while region_i > 0 && snapshot.regions[region_i].start.0 > start {
            region_i -= 1;
        }
        let mut i = region_i;
        while i < snapshot.regions.len() {
            let r = &snapshot.regions[i];
            if r.start.0 >= end {
                break;
            }
            if r.end() > start {
                let overlap = r.end().min(end) - r.start.0.max(start);
                let code = r.kind.code().min(15) as usize;
                bytes[code] += overlap;
                weighted[code] += r.confidence * overlap as f32;
                if best.is_none_or(|b| bytes[code] > bytes[b.code().min(15) as usize]) {
                    best = Some(r.kind);
                }
            }
            if r.end() <= end {
                region_i = i;
            }
            i += 1;
        }
        let span = (end - start).max(1);
        let kind = best.unwrap_or(RegionKind::Unknown);
        let code = kind.code().min(15) as usize;
        out.push(SummaryBucket {
            start,
            len: end - start,
            kind,
            confidence: if bytes[code] == 0 {
                0.0
            } else {
                weighted[code] / bytes[code] as f32
            },
            share: bytes[code] as f32 / span as f32,
            entropy: entropy.map_or(0.0, |p| mean_entropy(p, start, end)),
            executed: coverage.map_or(0.0, |c| {
                (start..end).filter(|o| c.executed.get(*o)).count() as f32 / span as f32
            }),
        });
    }
    out
}

fn mean_entropy(profile: &EntropyProfile, start: u32, end: u32) -> f32 {
    let window = crate::analysis::heuristics::WINDOW;
    let first = (start / window) as usize;
    let last = ((end - 1) / window) as usize;
    let slice = &profile.windows[first.min(profile.windows.len())..];
    let n = (last + 1 - first).min(slice.len());
    if n == 0 {
        return 0.0;
    }
    slice[..n].iter().sum::<f32>() / n as f32
}

/// Batch header length, then one fixed-size record per column.
pub const SUMMARY_HEADER_LEN: usize = 8;
pub const SUMMARY_RECORD_LEN: usize = 16;
/// Bumped whenever the record layout changes, so a stale shell fails loudly.
pub const SUMMARY_VERSION: u8 = 1;

/// Encode the strip as a flat batch.
///
/// Records rather than a typed list, for the reason `10-ffi-spike.md` measured:
/// a few hundred generated records per redraw is affordable but pointless, and
/// the shell draws one rect per column from a buffer it walks once.
///
/// Header: `version u8`, `reserved u8`, `count u16`, `rom_len u32`.
/// Record: `start u32`, `len u32`, `kind u8`, `confidence u8` (0..=255),
/// `share u8`, `entropy u8` (bits per byte × 32), `executed u8`, three
/// reserved bytes.
pub fn encode_summary(buckets: &[SummaryBucket], rom_len: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(SUMMARY_HEADER_LEN + buckets.len() * SUMMARY_RECORD_LEN);
    out.push(SUMMARY_VERSION);
    out.push(0);
    out.extend_from_slice(&(buckets.len().min(u16::MAX as usize) as u16).to_le_bytes());
    out.extend_from_slice(&rom_len.to_le_bytes());
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    for b in buckets {
        out.extend_from_slice(&b.start.to_le_bytes());
        out.extend_from_slice(&b.len.to_le_bytes());
        out.push(b.kind.code());
        out.push(byte(b.confidence));
        out.push(byte(b.share));
        out.push(((b.entropy.clamp(0.0, 8.0)) * 32.0).round().min(255.0) as u8);
        out.push(byte(b.executed));
        out.extend_from_slice(&[0, 0, 0]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{AnalysisControl, analyze};
    use crate::fixtures;
    use crate::model::project::Project;
    use crate::rom::image::RomImage;

    fn summary(buckets: u32) -> (RomImage, Vec<SummaryBucket>) {
        let rom = RomImage::from_bytes(fixtures::mixed_data_lorom(), "m.sfc").unwrap();
        let snap = analyze(&rom, &Project::new(&rom), &AnalysisControl::silent()).unwrap();
        let profile = EntropyProfile::build(&rom);
        let n = rom.len() as u32;
        let s = summarize(&snap, n, buckets, Some(&profile), None);
        (rom, s)
    }

    #[test]
    fn columns_tile_the_image_exactly() {
        let (rom, s) = summary(64);
        assert_eq!(s.len(), 64);
        assert_eq!(s[0].start, 0);
        assert_eq!(s.last().unwrap().end(), rom.len() as u32);
        for pair in s.windows(2) {
            assert_eq!(pair[0].end(), pair[1].start, "a gap or an overlap");
        }
        // Columns stay within a byte of each other.
        let (min, max) = s
            .iter()
            .fold((u32::MAX, 0), |(a, b), c| (a.min(c.len), b.max(c.len)));
        assert!(max - min <= 1, "{min}..{max}");
    }

    #[test]
    fn a_column_reports_what_it_is_mostly() {
        // One column per 256 bytes puts the palette block in its own columns.
        let (rom, s) = summary(0x1_0000 / 256);
        assert_eq!(rom.len(), 0x1_0000);
        let at = |off: u32| s.iter().find(|b| b.start <= off && off < b.end()).unwrap();
        let palette = at(0x1000);
        assert_eq!(palette.kind.name(), "palette");
        assert!(!palette.mixed(), "share {}", palette.share);
        assert_eq!(at(0x1200).kind.name(), "string");
        assert_eq!(at(0x1600).kind.name(), "compressed");
        // The compressed block is the high-entropy one, and the strip carries
        // that as its own lane.
        assert!(at(0x1600).entropy > 7.0, "{}", at(0x1600).entropy);
        assert!(at(0x4000).entropy < 1.0);
    }

    /// Sixty-four columns of 1 KB each: the one covering `0x1000` holds the
    /// 512-byte palette, the 256-byte string and 256 bytes of filler, so no
    /// kind has more than half of it.
    #[test]
    fn a_blended_column_says_so() {
        let (_, s) = summary(64);
        let blended = s
            .iter()
            .find(|b| b.start <= 0x1000 && 0x1000 < b.end())
            .unwrap();
        assert_eq!(blended.len, 1024);
        assert!(blended.mixed(), "share {}", blended.share);
        assert!(blended.share > 0.0 && blended.share < 0.75);
        // A column of nothing but filler is not mixed, and must not be
        // hatched: hatching everything says nothing.
        let plain = s.iter().find(|b| b.start >= 0x4000).unwrap();
        assert!(!plain.mixed(), "share {}", plain.share);
    }

    #[test]
    fn asking_for_more_columns_than_bytes_gives_one_per_byte() {
        let rom = RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap();
        let snap = analyze(&rom, &Project::new(&rom), &AnalysisControl::silent()).unwrap();
        let s = summarize(&snap, rom.len() as u32, u32::MAX, None, None);
        assert_eq!(s.len(), rom.len());
        assert!(s.iter().all(|b| b.len == 1));
    }

    #[test]
    fn the_batch_round_trips_its_header() {
        let (rom, s) = summary(40);
        let bytes = encode_summary(&s, rom.len() as u32);
        assert_eq!(bytes.len(), SUMMARY_HEADER_LEN + 40 * SUMMARY_RECORD_LEN);
        assert_eq!(bytes[0], SUMMARY_VERSION);
        assert_eq!(u16::from_le_bytes([bytes[2], bytes[3]]), 40);
        assert_eq!(
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            rom.len() as u32
        );
        let first = &bytes[SUMMARY_HEADER_LEN..SUMMARY_HEADER_LEN + SUMMARY_RECORD_LEN];
        assert_eq!(
            u32::from_le_bytes(first[..4].try_into().unwrap()),
            s[0].start
        );
        assert_eq!(first[8], s[0].kind.code());
    }
}
