//! Static analysis: recursive descent from the vectors, flag propagation,
//! a gated linear sweep, auto labels, cross-references and the snapshot.
//!
//! The whole pipeline runs again on every analysis-affecting command; it is
//! far inside the 2 s budget, so incremental invalidation is not worth its
//! bugs yet (docs/03 delta).

pub mod control;
pub mod descent;
pub mod flow;
pub mod inline;
pub mod labels;
pub mod snapshot;
pub mod sweep;
pub mod xrefs;

use std::collections::BTreeMap;
use std::time::Instant;

pub use control::{AnalysisControl, AnalysisPhase, Cancelled, Progress};
pub use snapshot::{AnalysisSnapshot, AnalysisStats, InsnRecord, Warning, WarningKind};

use crate::analysis::descent::{KIND_NONE, SeedSource, Walk};
use crate::cpu65816::ASSUMED_WIDTHS;
use crate::memory::address::FileOffset;
use crate::model::project::Project;
use crate::model::region::{DataKind, Evidence, OverrideKind, Region, RegionKind};
use crate::rom::header::{EXTENDED_HEADER_LEN, HEADER_LEN};
use crate::rom::image::RomImage;

const TAG_NONE: u8 = 0;
const TAG_VECTOR: u8 = 1;
const TAG_SWEEP: u8 = 2;
const TAG_OPERAND: u8 = 3;
const TAG_POINTER: u8 = 4;
const TAG_HEADER: u8 = 5;
const TAG_USER: u8 = 6;
const TAG_INLINE: u8 = 7;

/// Passes of the descent: a callee found adjusting its return address in
/// one pass makes its callers skip the inline arguments in the next, which
/// can uncover the next such callee. Each pass is a few milliseconds.
const MAX_PASSES: u32 = 6;

/// Run the analyzer over a ROM under a project's overrides.
pub fn analyze(
    rom: &RomImage,
    project: &Project,
    control: &AnalysisControl,
) -> Result<AnalysisSnapshot, Cancelled> {
    let started = Instant::now();
    let mut inline_args: BTreeMap<u32, u16> = BTreeMap::new();
    let mut pass = 0;
    let mut walk = loop {
        pass += 1;
        let mut walk = Walk::new(rom, project);
        walk.inline_args = inline_args.clone();
        walk.seed();
        walk.run(control)?;
        let mut grew = false;
        for (callee, n) in &walk.found_inline_args {
            grew |= inline_args.insert(*callee, *n) != Some(*n);
        }
        if !grew || pass == MAX_PASSES {
            break walk;
        }
    };
    let swept = sweep::sweep(&mut walk, control)?;
    control.check()?;
    control.report(AnalysisPhase::Labels, 0, 1);

    let n = rom.len();
    let mut cls = vec![0u8; n];
    let mut conf = vec![0u8; n];
    let mut tag = vec![TAG_NONE; n];

    // Internal header (plus the extended header when present).
    let h = rom.header_offset().0 as usize;
    let hstart = if rom.header().extended.is_some() {
        h - EXTENDED_HEADER_LEN
    } else {
        h
    };
    for b in hstart..(h + HEADER_LEN).min(n) {
        cls[b] = RegionKind::Data(DataKind::Struct).code();
        conf[b] = 100;
        tag[b] = TAG_HEADER;
    }
    // Code from the descent.
    let mut records: Vec<(InsnRecord, u16)> = std::mem::take(&mut walk.records);
    for (r, _) in &records {
        let c = if r.assumptions & ASSUMED_WIDTHS != 0 {
            70
        } else {
            90
        };
        for b in r.offset..r.end() {
            cls[b as usize] = 1;
            conf[b as usize] = c;
            tag[b as usize] = TAG_VECTOR;
        }
    }
    for &off in &walk.conflicts {
        if cls[off as usize] == 1 {
            conf[off as usize] = conf[off as usize].saturating_sub(30).max(10);
        }
    }
    for r in &swept {
        for b in r.offset..r.end() {
            cls[b as usize] = 1;
            conf[b as usize] = 30;
            tag[b as usize] = TAG_SWEEP;
        }
        records.push((*r, u16::MAX));
    }
    records.sort_by_key(|(r, _)| r.offset);
    // Data the code pointed at, on bytes nothing else claimed.
    for s in &walk.seeds {
        let end = (s.offset + s.len).min(n as u32);
        let jump_pointer = s.source == SeedSource::JumpPointer;
        for b in s.offset..end {
            let b = b as usize;
            if cls[b] != 0 && !(tag[b] == TAG_OPERAND && jump_pointer) {
                continue;
            }
            cls[b] = RegionKind::Data(s.kind).code();
            conf[b] = (s.confidence * 100.0) as u8;
            tag[b] = match s.source {
                SeedSource::Operand => TAG_OPERAND,
                SeedSource::JumpPointer => TAG_POINTER,
                SeedSource::InlineArgument => TAG_INLINE,
            };
        }
    }
    // The user's word is final.
    for r in &project.region_overrides {
        let end = (r.end() as usize).min(n);
        let code = r.kind.region_kind().code();
        for b in r.start.0 as usize..end {
            cls[b] = code;
            conf[b] = 100;
            tag[b] = TAG_USER;
        }
    }

    // Runs → regions.
    let mut regions: Vec<Region> = Vec::new();
    let mut start = 0usize;
    while start < n {
        let mut end = start + 1;
        while end < n
            && cls[end] == cls[start]
            && conf[end] == conf[start]
            && tag[end] == tag[start]
        {
            end += 1;
        }
        let kind = if tag[start] == TAG_USER {
            project
                .override_kind_at(start as u32)
                .map(OverrideKind::region_kind)
                .unwrap_or_else(|| kind_from_code(cls[start]))
        } else {
            kind_from_code(cls[start])
        };
        let evidence = match tag[start] {
            TAG_VECTOR => {
                let first = records.partition_point(|(r, _)| (r.offset as usize) < start);
                let depth = records[first..]
                    .iter()
                    .take_while(|(r, _)| (r.offset as usize) < end)
                    .map(|(_, d)| *d)
                    .filter(|d| *d != u16::MAX)
                    .min()
                    .unwrap_or(0);
                vec![Evidence::VectorReach {
                    depth: depth as u32,
                }]
            }
            TAG_SWEEP => vec![Evidence::Heuristic {
                name: "linear sweep".into(),
                score: sweep::SWEEP_CONFIDENCE,
            }],
            TAG_OPERAND => vec![Evidence::Heuristic {
                name: "operand target".into(),
                score: 0.6,
            }],
            TAG_POINTER => vec![Evidence::Heuristic {
                name: "jump pointer".into(),
                score: 0.9,
            }],
            TAG_INLINE => vec![Evidence::Heuristic {
                name: "inline argument".into(),
                score: 0.8,
            }],
            TAG_HEADER => vec![Evidence::Heuristic {
                name: "internal header".into(),
                score: 1.0,
            }],
            TAG_USER => vec![Evidence::User],
            _ => Vec::new(),
        };
        regions.push(Region {
            start: FileOffset(start as u32),
            len: (end - start) as u32,
            kind,
            confidence: conf[start] as f32 / 100.0,
            evidence,
        });
        start = end;
    }

    // Labels and xrefs.
    let vectors: Vec<(&'static str, crate::memory::address::SnesAddress)> =
        xrefs::vector_slots(rom)
            .into_iter()
            .filter(|v| v.used && rom.file_offset_for(v.target).is_some())
            .map(|v| (v.name, Project::canonical(rom, v.target)))
            .collect();
    let labelable: std::collections::HashSet<(u32, u32)> = walk
        .xrefs
        .iter()
        .filter(|(_, l)| *l)
        .map(|(x, _)| (x.from.0, x.to.as_u24()))
        .collect();
    let all_xrefs: Vec<_> = walk.xrefs.iter().map(|(x, _)| *x).collect();
    let auto_labels = labels::build(&vectors, &all_xrefs, |x| {
        labelable.contains(&(x.from.0, x.to.as_u24()))
    });
    let (xrefs_by_target, xrefs_by_source) = xrefs::index(all_xrefs);
    let mut warnings = walk.warnings;
    warnings.sort_by_key(|w| (w.offset, w.kind as u8));
    warnings.dedup();
    control.report(AnalysisPhase::Labels, 1, 1);

    let instructions: Vec<InsnRecord> = records.iter().map(|(r, _)| *r).collect();
    let mut stats = AnalysisStats {
        instructions: instructions.len() as u64,
        blocks: regions
            .iter()
            .filter(|r| r.kind == RegionKind::Code)
            .count() as u64,
        labels: auto_labels.len() as u64,
        xrefs: xrefs_by_target.len() as u64,
        conflicts: warnings
            .iter()
            .filter(|w| w.kind == WarningKind::FlagConflict)
            .count() as u64,
        warnings: warnings.len() as u64,
        ..Default::default()
    };
    for r in &regions {
        match r.kind {
            RegionKind::Code => stats.code_bytes += r.len as u64,
            RegionKind::Data(_) => stats.data_bytes += r.len as u64,
            RegionKind::Unknown => stats.unknown_bytes += r.len as u64,
        }
    }
    stats.elapsed_ms = started.elapsed().as_millis() as u64;
    let _ = KIND_NONE;
    Ok(AnalysisSnapshot {
        instructions,
        regions,
        auto_labels,
        xrefs_by_target,
        xrefs_by_source,
        warnings,
        stats,
    })
}

fn kind_from_code(code: u8) -> RegionKind {
    match code {
        0 => RegionKind::Unknown,
        1 => RegionKind::Code,
        2 => RegionKind::Data(DataKind::Byte),
        3 => RegionKind::Data(DataKind::Word),
        4 => RegionKind::Data(DataKind::Long),
        5 => RegionKind::Data(DataKind::Pointer),
        6 => RegionKind::Data(DataKind::Table { stride: 2 }),
        7 => RegionKind::Data(DataKind::String),
        8 => RegionKind::Data(DataKind::Graphics { bpp: 4 }),
        9 => RegionKind::Data(DataKind::Tilemap),
        10 => RegionKind::Data(DataKind::Palette),
        11 => RegionKind::Data(DataKind::Compressed),
        _ => RegionKind::Data(DataKind::Struct),
    }
}
