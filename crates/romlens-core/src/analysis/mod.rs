//! Static analysis: recursive descent from the vectors, flag propagation,
//! a gated linear sweep, auto labels, cross-references and the snapshot.
//!
//! The whole pipeline runs again on every analysis-affecting command; it is
//! far inside the 2 s budget, so incremental invalidation is not worth its
//! bugs yet (docs/03 delta).

pub mod control;
pub mod descent;
pub mod flow;
pub mod heuristics;
pub mod inline;
pub mod jumptable;
pub mod labels;
pub mod snapshot;
pub mod sweep;
pub mod xrefs;

use std::collections::BTreeMap;
use std::time::Instant;

pub use control::{AnalysisControl, AnalysisPhase, Cancelled, Progress};
pub use jumptable::{JumpTable, StopReason};
pub use snapshot::{AnalysisSnapshot, AnalysisStats, InsnRecord, Severity, Warning, WarningKind};

use crate::analysis::descent::{KIND_NONE, SeedSource, Walk};
use crate::analysis::heuristics::{EntropyProfile, HeuristicHit};
use crate::analysis::jumptable::Resolution;
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
const TAG_TABLE: u8 = 8;
const TAG_HEURISTIC: u8 = 9;

/// Passes of the descent. A callee found adjusting its return address in one
/// pass makes its callers skip the inline arguments in the next, which can
/// uncover the next such callee; a jump table resolved after one pass is walked
/// in the next, and its targets can dispatch through tables of their own. Eight
/// covers the nested dispatchers a real ROM has, and the loop leaves early the
/// moment a pass adds nothing. Each pass is a few milliseconds.
const MAX_PASSES: u32 = 8;

/// The per-byte lanes the classifier paints, then collapses into regions.
///
/// They are one struct rather than four vectors because every stage has to
/// write all of them: paint `cls` and forget `hid` and the byte keeps whatever
/// evidence the stage before it left there.
///
/// `hid` indexes `evidence`, with 0 meaning "the tag says it all". It exists
/// because the evidence that matters in Phase 2 varies *per span*, not per tag:
/// "jump table, 14 entries, dispatched from $80:9C15" names a specific
/// dispatcher and cannot be derived from `TAG_TABLE` alone. Regions then break
/// on a change of id, so two adjacent tables do not merge into one region that
/// claims both dispatchers.
struct Paint {
    cls: Vec<u8>,
    conf: Vec<u8>,
    tag: Vec<u8>,
    hid: Vec<u32>,
    /// Indexed from 1; slot 0 is the empty placeholder for `hid == 0`.
    evidence: Vec<Vec<Evidence>>,
}

impl Paint {
    fn new(n: usize) -> Self {
        Self {
            cls: vec![0; n],
            conf: vec![0; n],
            tag: vec![TAG_NONE; n],
            // u32, not u16: window-aligned heuristics over a 3 MB ROM can
            // produce more than 64K spans, and a lane that silently wrapped
            // would attach one region's evidence to another's bytes.
            hid: vec![0; n],
            evidence: vec![Vec::new()],
        }
    }

    /// Record evidence belonging to one span and return the id to paint with.
    fn intern(&mut self, evidence: Vec<Evidence>) -> u32 {
        self.evidence.push(evidence);
        (self.evidence.len() - 1) as u32
    }

    fn set(&mut self, b: usize, cls: u8, conf: u8, tag: u8, hid: u32) {
        self.cls[b] = cls;
        self.conf[b] = conf;
        self.tag[b] = tag;
        self.hid[b] = hid;
    }

    /// Whether two bytes belong to the same region.
    fn same(&self, a: usize, b: usize) -> bool {
        self.cls[a] == self.cls[b]
            && self.conf[a] == self.conf[b]
            && self.tag[a] == self.tag[b]
            && self.hid[a] == self.hid[b]
    }

    fn span_evidence(&self, b: usize) -> &[Evidence] {
        &self.evidence[self.hid[b] as usize]
    }
}

/// Knobs that exist to attribute a result, not to configure the product: a
/// shell always runs with the defaults, and only `romlens analyze` changes
/// them, to answer "how much of this did jump tables buy us?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisOptions {
    /// Resolve `JMP`/`JSR (abs,X)` dispatch tables (`jumptable`).
    pub jump_tables: bool,
    /// Score unclassified bytes (`heuristics`).
    pub heuristics: bool,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            jump_tables: true,
            heuristics: true,
        }
    }
}

/// Run the analyzer over a ROM under a project's overrides.
pub fn analyze(
    rom: &RomImage,
    project: &Project,
    control: &AnalysisControl,
) -> Result<AnalysisSnapshot, Cancelled> {
    analyze_with(rom, project, control, AnalysisOptions::default())
}

/// [`analyze`] with the attribution knobs.
pub fn analyze_with(
    rom: &RomImage,
    project: &Project,
    control: &AnalysisControl,
    options: AnalysisOptions,
) -> Result<AnalysisSnapshot, Cancelled> {
    analyze_cached(rom, project, control, options, None)
}

/// [`analyze_with`], reusing an entropy profile.
///
/// The profile is derived from the ROM alone, so it survives every edit; a
/// caller that re-analyzes on each command (every shell does) should build it
/// once and pass it here rather than putting a linear pass over the image
/// inside the edit loop.
pub fn analyze_cached(
    rom: &RomImage,
    project: &Project,
    control: &AnalysisControl,
    options: AnalysisOptions,
    entropy: Option<&EntropyProfile>,
) -> Result<AnalysisSnapshot, Cancelled> {
    let started = Instant::now();
    let mut inline_args: BTreeMap<u32, u16> = BTreeMap::new();
    let mut jump_tables: BTreeMap<u32, Resolution> = BTreeMap::new();
    let mut pass = 0;
    let mut walk = loop {
        pass += 1;
        let mut walk = Walk::new(rom, project);
        walk.inline_args = inline_args.clone();
        walk.jump_tables = jump_tables.clone();
        walk.seed();
        walk.run(control)?;
        let mut grew = false;
        for (callee, n) in &walk.found_inline_args {
            grew |= inline_args.insert(*callee, *n) != Some(*n);
        }
        // Resolving here, against a walk that has finished, is what keeps the
        // answer independent of worklist order (`jumptable`).
        if options.jump_tables {
            control.report(AnalysisPhase::Tables, 0, 1);
            for (site, resolution) in jumptable::resolve(&walk) {
                grew |= jump_tables.insert(site, resolution.clone()) != Some(resolution);
            }
            control.report(AnalysisPhase::Tables, 1, 1);
        }
        if !grew || pass == MAX_PASSES {
            break walk;
        }
    };
    let swept = sweep::sweep(&mut walk, control)?;
    control.check()?;
    control.report(AnalysisPhase::Labels, 0, 1);

    let n = rom.len();
    let mut paint = Paint::new(n);

    // Internal header (plus the extended header when present).
    let h = rom.header_offset().0 as usize;
    let hstart = if rom.header().extended.is_some() {
        h - EXTENDED_HEADER_LEN
    } else {
        h
    };
    let header_code = RegionKind::Data(DataKind::Struct).code();
    for b in hstart..(h + HEADER_LEN).min(n) {
        paint.set(b, header_code, 100, TAG_HEADER, 0);
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
            paint.set(b as usize, 1, c, TAG_VECTOR, 0);
        }
    }
    for &off in &walk.conflicts {
        let off = off as usize;
        if paint.cls[off] == 1 {
            paint.conf[off] = paint.conf[off].saturating_sub(30).max(10);
        }
    }
    for r in &swept {
        for b in r.offset..r.end() {
            paint.set(b as usize, 1, 30, TAG_SWEEP, 0);
        }
        records.push((*r, u16::MAX));
    }
    records.sort_by_key(|(r, _)| r.offset);
    // Data the code pointed at, on bytes nothing else claimed.
    for s in &walk.seeds {
        let end = (s.offset + s.len).min(n as u32);
        let jump_pointer = s.source == SeedSource::JumpPointer;
        let code = RegionKind::Data(s.kind).code();
        let conf = (s.confidence * 100.0) as u8;
        let tag = match s.source {
            SeedSource::Operand => TAG_OPERAND,
            SeedSource::JumpPointer => TAG_POINTER,
            SeedSource::InlineArgument => TAG_INLINE,
        };
        for b in s.offset..end {
            let b = b as usize;
            if paint.cls[b] != 0 && !(paint.tag[b] == TAG_OPERAND && jump_pointer) {
                continue;
            }
            paint.set(b, code, conf, tag, 0);
        }
    }
    // Jump-table extents. Unlike a data seed, a table's evidence names the
    // dispatcher that reads it, which is the answer to "why is this data?" and
    // differs per table — so it goes in the side table rather than the tag.
    //
    // It paints over an unclaimed byte, and over one the linear sweep claimed:
    // the sweep is a 0.3 guess made without a caller, while a table entry was
    // read by a real dispatch instruction. It never paints over the descent.
    for resolution in jump_tables.values() {
        let Resolution::Table(table) = resolution else {
            continue;
        };
        let conf = (table.confidence() * 100.0) as u8;
        let hid = paint.intern(vec![Evidence::Heuristic {
            name: table.description(),
            score: table.confidence(),
        }]);
        let code = RegionKind::Data(DataKind::Table { stride: 2 }).code();
        for b in table.base..table.end().min(n as u32) {
            let b = b as usize;
            if paint.cls[b] != 0 && paint.tag[b] != TAG_SWEEP {
                continue;
            }
            paint.set(b, code, conf, TAG_TABLE, hid);
        }
    }
    // Heuristics, strongest first. Two rules, both asserted in
    // `tests/heuristics.rs`: a heuristic only fills a byte nothing else
    // claimed, and the first hit to reach a byte keeps it. Together they mean
    // a guess can never argue with the disassembler, a table, or the user.
    let mut heuristic_hits: Vec<HeuristicHit> = Vec::new();
    if options.heuristics {
        control.report(AnalysisPhase::Heuristics, 0, 1);
        let owned;
        let profile = match entropy {
            Some(p) => p,
            None => {
                owned = EntropyProfile::build(rom);
                &owned
            }
        };
        control.check()?;
        heuristic_hits = heuristics::run(rom, profile);
        for hit in &heuristic_hits {
            if !hit.classifies {
                continue;
            }
            let code = hit.kind.code();
            let conf = hit.confidence();
            let mut id = None;
            for b in hit.start..hit.end().min(n as u32) {
                let b = b as usize;
                if paint.cls[b] != 0 {
                    continue;
                }
                let id = *id.get_or_insert_with(|| {
                    paint.intern(vec![Evidence::Heuristic {
                        name: format!("{}: {}", hit.name, hit.detail),
                        score: hit.score,
                    }])
                });
                paint.set(b, code, conf, TAG_HEURISTIC, id);
            }
        }
        control.report(AnalysisPhase::Heuristics, 1, 1);
    }
    // The user's word is final.
    for r in &project.region_overrides {
        let end = (r.end() as usize).min(n);
        let code = r.kind.region_kind().code();
        for b in r.start.0 as usize..end {
            paint.set(b, code, 100, TAG_USER, 0);
        }
    }

    // Runs → regions.
    let mut regions: Vec<Region> = Vec::new();
    let mut start = 0usize;
    while start < n {
        let mut end = start + 1;
        while end < n && paint.same(end, start) {
            end += 1;
        }
        let kind = if paint.tag[start] == TAG_USER {
            project
                .override_kind_at(start as u32)
                .map(OverrideKind::region_kind)
                .unwrap_or_else(|| kind_from_code(paint.cls[start]))
        } else {
            kind_from_code(paint.cls[start])
        };
        // Span evidence comes first: it names the specific thing that decided
        // these bytes, which is what the inspector's popover leads with.
        let mut evidence = paint.span_evidence(start).to_vec();
        evidence.extend(match paint.tag[start] {
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
            // TAG_TABLE's evidence is per-table and already in the side table.
            _ => Vec::new(),
        });
        regions.push(Region {
            start: FileOffset(start as u32),
            len: (end - start) as u32,
            kind,
            confidence: paint.conf[start] as f32 / 100.0,
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
        regions: regions.len() as u64,
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
    let mut tables: Vec<JumpTable> = jump_tables
        .into_values()
        .filter_map(|r| match r {
            Resolution::Table(t) => Some(t),
            Resolution::Unresolved(_) => None,
        })
        .collect();
    tables.sort_by_key(|t| (t.base, t.site));
    Ok(AnalysisSnapshot {
        instructions,
        heuristic_hits,
        jump_tables: tables,
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
