//! The Atlas's data (docs/22, A1): a zoomable map of the whole ROM.
//!
//! Zooming is a query, not a resample: the shell asks for the columns of
//! the window it shows ([`region_summary::summarize_window`]), the calls
//! between them ([`call_arcs`]), and, once a window is small enough to show
//! single instructions and data rows, those ([`items`], from the listing's
//! own line index, so the two views agree on where each item starts).
//!
//! [`region_summary::summarize_window`]: super::region_summary::summarize_window

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::model::region::RegionKind;
use crate::model::xref::XRefKind;
use crate::viewmodel::asm_lines::LineIndex;

/// Where one end of a call arc falls in a window of columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ArcEnd {
    /// Before the window's first byte.
    Before,
    Column(u32),
    /// At or after the window's end, or outside the ROM (a call into RAM).
    After,
}

/// The calls from one column to another, summed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallArc {
    pub from: ArcEnd,
    pub to: ArcEnd,
    /// Call instructions from `from`'s bytes to `to`'s.
    pub calls: u32,
    /// Some of them an emulator saw happen.
    pub observed: bool,
}

/// The column of `off` among `buckets` columns spread over `len` bytes from
/// `start`, as `summarize_window` spreads them.
fn end_of(off: Option<u32>, start: u32, len: u32, buckets: u32) -> ArcEnd {
    match off {
        Some(o) if o < start => ArcEnd::Before,
        Some(o) if o < start + len => {
            // The column `c` with c*len/buckets <= o-start < (c+1)*len/buckets.
            let rel = (o - start) as u64;
            let c = ((rel + 1) * buckets as u64 - 1) / len as u64;
            ArcEnd::Column(c.min(buckets as u64 - 1) as u32)
        }
        _ => ArcEnd::After,
    }
}

/// The calls (`JSR`, `JSL`) with at least one end in the window, summed per
/// pair of columns. A call within one column draws nothing, so it is left
/// out; so is one with neither end in the window. Most calls first.
pub fn call_arcs(
    snap: &AnalysisSnapshot,
    rom_len: u32,
    start: u32,
    len: u32,
    buckets: u32,
) -> Vec<CallArc> {
    let start = start.min(rom_len);
    let len = len.min(rom_len - start);
    if len == 0 {
        return Vec::new();
    }
    let buckets = buckets.clamp(1, len);
    let mut sums: std::collections::BTreeMap<(ArcEnd, ArcEnd), (u32, bool)> = Default::default();
    for x in snap
        .xrefs_by_source
        .iter()
        .filter(|x| x.kind == XRefKind::Call)
    {
        let from = end_of(Some(x.from.0), start, len, buckets);
        let to = end_of(x.to_offset.map(|o| o.0), start, len, buckets);
        let inside = |e: ArcEnd| matches!(e, ArcEnd::Column(_));
        if from == to || !(inside(from) || inside(to)) {
            continue;
        }
        let e = sums.entry((from, to)).or_default();
        e.0 += 1;
        e.1 |= x.observed;
    }
    let mut out: Vec<CallArc> = sums
        .into_iter()
        .map(|((from, to), (calls, observed))| CallArc {
            from,
            to,
            calls,
            observed,
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.calls));
    out
}

/// One instruction or data row, the Atlas's finest level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasItem {
    pub offset: u32,
    pub len: u32,
    pub instruction: bool,
    /// The region the item is in.
    pub kind: RegionKind,
    pub confidence: f32,
}

/// The listing's instructions and data rows that start in the window, at
/// most `limit` of them; the flag says there were more.
pub fn items(
    snap: &AnalysisSnapshot,
    lines: &LineIndex,
    start: u32,
    len: u32,
    limit: usize,
) -> (Vec<AtlasItem>, bool) {
    let end = start.saturating_add(len);
    let first = lines.lines.partition_point(|l| l.offset < start);
    let mut out = Vec::new();
    for l in lines.lines[first..].iter().take_while(|l| l.offset < end) {
        if !l.kind.is_content() {
            continue;
        }
        if out.len() == limit {
            return (out, true);
        }
        let region = snap.region_at(crate::FileOffset(l.offset));
        out.push(AtlasItem {
            offset: l.offset,
            len: l.sub as u32,
            instruction: l.kind == crate::viewmodel::asm_lines::LineKind::Instruction,
            kind: region.map_or(RegionKind::Unknown, |r| r.kind),
            confidence: region.map_or(0.0, |r| r.confidence),
        });
    }
    (out, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{AnalysisControl, analyze};
    use crate::fixtures;
    use crate::model::project::Project;
    use crate::rom::image::RomImage;
    use crate::viewmodel::region_summary::{summarize, summarize_window};

    fn analysed(bytes: Vec<u8>) -> (RomImage, AnalysisSnapshot, Project) {
        let rom = RomImage::from_bytes(bytes, "a.sfc").unwrap();
        let project = Project::new(&rom);
        let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
        (rom, snap, project)
    }

    #[test]
    fn a_window_is_the_same_columns_the_whole_map_has_there() {
        let (rom, snap, _) = analysed(fixtures::mixed_data_lorom());
        let n = rom.len() as u32;
        let whole = summarize(&snap, n, 256, None, None);
        // Columns 16..32 of 256 are bytes 0x1000..0x2000.
        let window = summarize_window(&snap, n, 0x1000, 0x1000, 16, None, None);
        assert_eq!(window, whole[16..32]);
        // A deeper zoom: 64 columns of 16 bytes in the palette.
        let deep = summarize_window(&snap, n, 0x1000, 0x400, 64, None, None);
        assert_eq!(
            (deep[0].start, deep[0].len, deep.last().unwrap().end()),
            (0x1000, 16, 0x1400)
        );
        assert_eq!(deep[0].kind.name(), "palette");
        // Clipped to the image, and empty past it.
        assert_eq!(
            summarize_window(&snap, n, n - 8, 100, 4, None, None)
                .last()
                .unwrap()
                .end(),
            n
        );
        assert!(summarize_window(&snap, n, n, 100, 4, None, None).is_empty());
    }

    #[test]
    fn calls_become_arcs_between_columns() {
        let (rom, snap, _) = analysed(fixtures::routines_lorom());
        let n = rom.len() as u32;
        let calls = snap
            .xrefs_by_source
            .iter()
            .filter(|x| x.kind == XRefKind::Call)
            .count() as u32;
        assert!(calls > 0);
        // One column per byte of the code: every call is its own arc, and
        // the sums account for every call.
        let code = snap
            .regions
            .iter()
            .find(|r| r.kind == RegionKind::Code)
            .unwrap();
        let arcs = call_arcs(&snap, n, 0, n, n);
        assert_eq!(arcs.iter().map(|a| a.calls).sum::<u32>(), calls);
        // One column for everything: no arc leaves it.
        assert!(call_arcs(&snap, n, 0, n, 1).is_empty());
        // A window past the code sees its calls from before it at most.
        let past = call_arcs(&snap, n, code.end(), n - code.end(), 8);
        assert!(
            past.iter()
                .all(|a| a.from == ArcEnd::Before || a.to == ArcEnd::Before)
        );
    }

    #[test]
    fn items_are_the_listings_lines() {
        let (rom, snap, project) = analysed(fixtures::routines_lorom());
        let lines = LineIndex::build(&rom, &snap, &project);
        let code = snap
            .regions
            .iter()
            .find(|r| r.kind == RegionKind::Code)
            .unwrap();
        let (all, more) = items(&snap, &lines, code.start.0, code.len, usize::MAX);
        assert!(!more && !all.is_empty());
        assert!(
            all.iter()
                .all(|i| i.instruction && i.kind == RegionKind::Code)
        );
        // Back to back, as the listing has them.
        for w in all.windows(2) {
            assert_eq!(w[0].offset + w[0].len, w[1].offset);
        }
        let (some, more) = items(&snap, &lines, code.start.0, code.len, 3);
        assert_eq!((some.len(), more), (3, true));
        assert_eq!(some[..], all[..3]);
    }
}
