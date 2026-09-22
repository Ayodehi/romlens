//! The two rules that decide whether scored guesses help or hurt:
//! a heuristic only fills what nothing else claimed, and it emits spans.

use romlens_core::analysis::heuristics::{
    self, EntropyProfile, HeuristicHit, MAX_CONFIDENCE, WINDOW, quantize,
};
use romlens_core::analysis::{
    AnalysisControl, AnalysisOptions, AnalysisSnapshot, analyze, analyze_with,
};
use romlens_core::fixtures;
use romlens_core::model::{Command, DataKind, Evidence, OverrideKind, Project, RegionKind};
use romlens_core::{FileOffset, RomImage};

fn rom() -> RomImage {
    RomImage::from_bytes(fixtures::mixed_data_lorom(), "heuristics.sfc").unwrap()
}

fn run(rom: &RomImage) -> AnalysisSnapshot {
    analyze(rom, &Project::new(rom), &AnalysisControl::silent()).unwrap()
}

/// The hit that decided this byte: the strongest classifying one covering it.
/// Several heuristics may cover the same byte — `graphics` in particular
/// covers a great deal — so "the first hit" is not the same question.
fn hit_at(s: &AnalysisSnapshot, off: u32) -> Option<&HeuristicHit> {
    s.heuristic_hits
        .iter()
        .find(|h| h.classifies && h.start <= off && off < h.end())
}

#[test]
fn each_heuristic_finds_its_own_block() {
    let rom = rom();
    let s = run(&rom);
    let kind_at = |off: u32| {
        s.regions
            .iter()
            .find(|r| r.contains(off))
            .map(|r| r.kind)
            .unwrap()
    };
    assert_eq!(kind_at(0x1000), RegionKind::Data(DataKind::Palette));
    assert_eq!(kind_at(0x1200), RegionKind::Data(DataKind::String));
    assert_eq!(kind_at(0x1400), RegionKind::Data(DataKind::Pointer));
    assert_eq!(kind_at(0x1600), RegionKind::Data(DataKind::Compressed));

    assert_eq!(hit_at(&s, 0x1000).unwrap().name, "palette");
    assert_eq!(hit_at(&s, 0x1200).unwrap().name, "ascii");
    assert_eq!(hit_at(&s, 0x1400).unwrap().name, "pointers");
    assert_eq!(hit_at(&s, 0x1600).unwrap().name, "entropy");
}

/// Rule one: a heuristic fills only what nothing else claimed. Code the
/// disassembler found, a jump table, and a user's mark all outrank it.
#[test]
fn heuristics_never_argue_with_anything_else() {
    let rom = rom();
    let plain = run(&rom);
    // The boot code is code, although its bytes sit in a window the entropy
    // heuristic also has an opinion about.
    assert_eq!(
        plain.regions.iter().find(|r| r.contains(0)).unwrap().kind,
        RegionKind::Code
    );

    // A user mark over a block a heuristic claimed wins outright.
    let mut project = Project::new(&rom);
    project
        .apply(
            &rom,
            Command::MarkRegion {
                start: FileOffset(0x1000),
                len: 0x40,
                kind: OverrideKind::Data(DataKind::Tilemap),
            },
        )
        .unwrap();
    let marked = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let region = marked.regions.iter().find(|r| r.contains(0x1000)).unwrap();
    assert_eq!(region.kind, RegionKind::Data(DataKind::Tilemap));
    assert_eq!(region.evidence, vec![Evidence::User]);
}

/// Rule two: spans, not per-byte verdicts. Every hit is window-aligned and
/// adjacent agreeing windows are merged before anything is painted.
#[test]
fn hits_are_window_aligned_spans() {
    let rom = rom();
    let s = run(&rom);
    assert!(!s.heuristic_hits.is_empty());
    let n = rom.len() as u32;
    for h in &s.heuristic_hits {
        assert_eq!(h.start % WINDOW, 0, "{h:?} does not start on a window");
        assert!(
            h.len >= WINDOW.min(n - h.start),
            "{h:?} is shorter than a window"
        );
        assert!(h.end() <= n, "{h:?} runs past the image");
        assert_eq!(
            h.score,
            quantize(h.score),
            "{h:?} carries an unquantized score"
        );
        assert!(h.confidence() <= MAX_CONFIDENCE, "{h:?} exceeds the cap");
    }
    // Strongest first: `analyze` paints in this order and the first hit on a
    // byte keeps it, so the sort is the conflict rule.
    for pair in s.heuristic_hits.windows(2) {
        assert!(
            pair[0].score >= pair[1].score,
            "hits are not sorted by score"
        );
    }
}

/// The palette block is 256 colours of the same sixteen, so every window of it
/// scores identically and the whole block has to arrive as one span.
#[test]
fn agreeing_windows_merge() {
    let rom = rom();
    let s = run(&rom);
    let hit = hit_at(&s, 0x1000).unwrap();
    assert_eq!(hit.start, 0x1000);
    assert_eq!(hit.len, 0x200, "two windows of identical palette, one span");
}

/// The rule that keeps the map legible: scoring per byte would shatter the
/// region list, and the guard is a bound on how many regions exist at all.
#[test]
fn scoring_does_not_shatter_the_region_list() {
    let rom = rom();
    let without = analyze_with(
        &rom,
        &Project::new(&rom),
        &AnalysisControl::silent(),
        AnalysisOptions {
            jump_tables: true,
            heuristics: false,
        },
    )
    .unwrap();
    let with = run(&rom);
    assert!(
        with.stats.regions > without.stats.regions,
        "nothing was found"
    );
    assert!(
        with.stats.regions < without.stats.regions + 32,
        "heuristics added {} regions to {}; they are not coalescing",
        with.stats.regions - without.stats.regions,
        without.stats.regions
    );
}

/// `graphics` produces evidence and a tint but must not set a region's kind
/// until the accuracy harness clears it.
#[test]
fn the_graphics_heuristic_only_annotates() {
    let rom = rom();
    let s = run(&rom);
    for h in &s.heuristic_hits {
        assert_eq!(
            h.classifies,
            h.name != "graphics",
            "{} has the wrong classifying flag",
            h.name
        );
    }
    for h in s.heuristic_hits.iter().filter(|h| h.name == "graphics") {
        let region = s.regions.iter().find(|r| r.contains(h.start)).unwrap();
        assert_ne!(
            region.kind,
            RegionKind::Data(DataKind::Graphics { bpp: 4 }),
            "an annotating heuristic classified {:#06X}",
            h.start
        );
    }
}

/// The profile is ROM-derived, so a caller may build it once and reuse it.
#[test]
fn a_cached_profile_gives_the_same_answer() {
    let rom = rom();
    let profile = EntropyProfile::build(&rom);
    let cached = romlens_core::analysis::analyze_cached(
        &rom,
        &Project::new(&rom),
        &AnalysisControl::silent(),
        AnalysisOptions::default(),
        Some(&profile),
    )
    .unwrap();
    assert_eq!(cached.heuristic_hits, run(&rom).heuristic_hits);
    assert_eq!(profile.windows.len(), rom.len().div_ceil(WINDOW as usize));
    // The high-entropy block reads as compressed, the zero fill as sparse.
    assert!(profile.at(0x1600) >= heuristics::entropy::COMPRESSED_BITS);
    assert!(profile.at(0x2000) <= heuristics::entropy::SPARSE_BITS);
}
