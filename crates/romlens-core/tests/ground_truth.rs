//! The classifier scored against ground truth.
//!
//! CI runs this on the fixtures, whose truth the builder states because it
//! wrote them. Truth for a commercial ROM is derived from that ROM and never
//! ships (`12-content-policy.md`), so the development-ROM part is opt-in: put
//! a `.tsv` beside the ROM in `ROMLENS_ROM_DIR` and it runs, otherwise it says
//! it skipped and passes.

use romlens_core::analysis::accuracy::{Class, GroundTruth, TruthRange, score};
use romlens_core::analysis::{AnalysisControl, AnalysisSnapshot, analyze};
use romlens_core::fixtures;
use romlens_core::model::Project;
use romlens_core::{MappingMode, RomImage};

/// Precision is what protects a reader: a map that confidently calls data
/// "code" is worse than one that says "unknown". Every threshold in
/// `analysis::heuristics` is tuned against this number first.
const MIN_CODE_PRECISION: f64 = 0.98;

/// Recall on the development ROM, against what a truth file labels as code.
/// Phase 1 reached 0.2% of the *image*; this is the phase's pass mark.
const MIN_CODE_RECALL: f64 = 0.50;

/// How many disagreements to keep for the failure message.
const WORST: usize = 10;

fn analyzed(bytes: Vec<u8>, name: &str) -> (RomImage, AnalysisSnapshot) {
    let rom = RomImage::from_bytes(bytes, name).unwrap();
    let snap = analyze(&rom, &Project::new(&rom), &AnalysisControl::silent()).unwrap();
    (rom, snap)
}

fn check(name: &str, snap: &AnalysisSnapshot, ranges: Vec<TruthRange>, min_recall: f64) {
    let truth = GroundTruth {
        sha256: None,
        ranges,
    };
    let a = score(snap, &truth, WORST);
    assert!(a.labelled_bytes > 0, "{name}: the truth names nothing");
    let code = a.class(Class::Code).expect("code is always scored");
    let report = || {
        let lines: Vec<String> = a.disagreements.iter().map(|d| d.to_string()).collect();
        format!(
            "{name}: precision {:.3}, recall {:.3} over {} labelled bytes\n{}",
            code.precision(),
            code.recall(),
            a.labelled_bytes,
            lines.join("\n")
        )
    };
    assert!(
        code.precision() >= MIN_CODE_PRECISION,
        "code precision below {MIN_CODE_PRECISION}\n{}",
        report()
    );
    assert!(
        code.recall() >= min_recall,
        "code recall below {min_recall}\n{}",
        report()
    );
}

/// Every mapping's minimal fixture: the boot routine is code and the internal
/// header is a struct, and nothing may claim otherwise.
#[test]
fn the_minimal_fixtures_score_perfectly() {
    for mode in MappingMode::all() {
        let (_, snap) = analyzed(fixtures::for_mapping(mode), "fixture.sfc");
        check(mode.name(), &snap, fixtures::truth_for(mode), 1.0);
    }
}

/// The fixture the data heuristics are actually measured on: a palette, a
/// string, a pointer table and a compressed-looking block, each written to be
/// what it is.
#[test]
fn the_mixed_data_fixture_scores_every_heuristic() {
    let (_, snap) = analyzed(fixtures::mixed_data_lorom(), "mixed.sfc");
    let truth = GroundTruth {
        sha256: None,
        ranges: fixtures::truth_for_mixed_data(),
    };
    let a = score(&snap, &truth, WORST);
    check("mixed-data", &snap, fixtures::truth_for_mixed_data(), 1.0);
    // Exact kinds, not just classes: the point of these blocks is that the
    // right *heuristic* recognised each one.
    assert_eq!(
        a.exact_bytes,
        a.labelled_bytes,
        "a block was filed under the wrong data kind\n{}",
        a.disagreements
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The dispatch fixture, where the tables themselves are the data under test.
#[test]
fn the_dispatch_fixture_keeps_its_tables() {
    let (_, snap) = analyzed(fixtures::dispatch_lorom(), "dispatch.sfc");
    check("dispatch", &snap, fixtures::truth_for_dispatch(), 1.0);
}

/// Built-in truth is chosen by title and never guessed: scoring against
/// another fixture's answers would be worse than not scoring.
#[test]
fn built_in_truth_is_only_offered_where_it_is_known() {
    assert!(fixtures::truth_for_title("ROMLENS OPCODES", MappingMode::LoRom).is_none());
    assert!(fixtures::truth_for_title("SUPER METROID", MappingMode::LoRom).is_none());
    assert_eq!(
        fixtures::truth_for_title("ROMLENS DISPATCH", MappingMode::LoRom),
        Some(fixtures::truth_for_dispatch())
    );
}

/// The development ROM, if the developer has recorded truth for it. Never
/// committed, so this is a no-op in CI and a real measurement locally.
#[test]
fn the_development_rom_meets_the_phase_target() {
    let Some(dir) = std::env::var_os("ROMLENS_ROM_DIR") else {
        eprintln!("skipped: ROMLENS_ROM_DIR unset");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let rom_path = dir.join("SuperMetroid.F8DF.sfc");
    let truth_path = dir.join("SuperMetroid.F8DF.tsv");
    if !rom_path.exists() || !truth_path.exists() {
        eprintln!(
            "skipped: needs {} and {}",
            rom_path.display(),
            truth_path.display()
        );
        return;
    }
    let rom = RomImage::load(&rom_path).unwrap();
    let truth = GroundTruth::parse(&std::fs::read_to_string(&truth_path).unwrap()).unwrap();
    if let Some(sha) = &truth.sha256 {
        assert_eq!(*sha, rom.sha256_hex(), "the truth file is for another ROM");
    }
    let snap = analyze(&rom, &Project::new(&rom), &AnalysisControl::silent()).unwrap();
    let a = score(&snap, &truth, WORST);
    let code = a.class(Class::Code).unwrap();
    eprintln!(
        "development ROM: code precision {:.3}, recall {:.3}, overall {:.3} over {} labelled bytes",
        code.precision(),
        code.recall(),
        a.overall(),
        a.labelled_bytes
    );
    check("development ROM", &snap, truth.ranges, MIN_CODE_RECALL);
}
