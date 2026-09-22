//! Imported coverage: what it outranks, what it must not touch, and the M/X
//! widths that are worth more to the disassembler than the coverage is.

use romlens_core::analysis::{AnalysisControl, AnalysisSnapshot, analyze};
use romlens_core::fixtures;
use romlens_core::io::import::{cdl, read_trace, usage_map};
use romlens_core::io::{COVERAGE_FILE, from_files, to_files};
use romlens_core::model::{
    Command, Coverage, DataKind, Evidence, OverrideKind, Project, RegionKind, TraceRecord,
};
use romlens_core::{FileOffset, RomImage, SnesAddress};

fn rom() -> RomImage {
    RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap()
}

/// A CDL marking `[start, end)` as executed, with an optional entry.
fn executed(rom: &RomImage, ranges: &[(u32, u32, bool)], read: &[(u32, u32)]) -> Coverage {
    let mut c = Coverage::new(rom.len() as u32);
    c.flags.recorded = true;
    for (start, end, entry) in ranges {
        for off in *start..*end {
            c.executed.insert(off);
            // 8-bit A and X, which is what the fixture's boot sets up.
            c.flags.m8.insert(off);
            c.flags.x8.insert(off);
        }
        c.mark_opcode(*start, *entry);
    }
    for (start, end) in read {
        for off in *start..*end {
            c.read.insert(off);
        }
    }
    c
}

fn with_trace(rom: &RomImage, coverage: Coverage) -> Project {
    let mut project = Project::new(rom);
    project.add_trace(
        TraceRecord {
            source: "play.cdl".into(),
            format: "cdl".into(),
            executed_bytes: coverage.executed.count(),
            read_bytes: coverage.read.count(),
        },
        coverage,
    );
    project
}

fn run(rom: &RomImage, project: &Project) -> AnalysisSnapshot {
    analyze(rom, project, &AnalysisControl::silent()).unwrap()
}

fn kind_at(s: &AnalysisSnapshot, off: u32) -> RegionKind {
    s.region_at(FileOffset(off)).unwrap().kind
}

/// Observation beats inference: a trace outranks the descent's own 90, and
/// what it saw gets decoded, not merely coloured.
#[test]
fn a_trace_finds_code_the_walk_missed() {
    let rom = rom();
    let plain = run(&rom, &Project::new(&rom));
    // The two NOPs after the spin are unreachable statically.
    assert_ne!(kind_at(&plain, 0x0C), RegionKind::Code);

    // An emulator saw them run, and saw a subroutine at 0x10.
    let project = with_trace(
        &rom,
        executed(&rom, &[(0x0C, 0x0E, false), (0x10, 0x13, true)], &[]),
    );
    let s = run(&rom, &project);
    assert_eq!(kind_at(&s, 0x0C), RegionKind::Code);
    let region = s.region_at(FileOffset(0x10)).unwrap();
    assert_eq!(region.kind, RegionKind::Code);
    assert!(
        (region.confidence - 0.95).abs() < 1e-6,
        "a trace outranks the descent's 0.90, not the other way round"
    );
    assert_eq!(
        region.evidence,
        vec![Evidence::Trace {
            file: "play.cdl".into(),
            hits: 5,
        }]
    );
    // Decoded, not just painted: a byte called code that nothing decoded would
    // render as `db` under a "code" heading, which is a worse listing.
    assert!(
        s.instructions.iter().any(|r| r.offset == 0x0C),
        "the observed opcode was never decoded"
    );
    // A subroutine entry names itself.
    assert_eq!(
        s.auto_labels[&SnesAddress::new(0x00, 0x8010)].name,
        "SUB_008010"
    );
}

/// A byte only ever read is data, at a lower confidence than one executed,
/// and only where nothing else claimed it.
#[test]
fn read_bytes_fill_what_is_left() {
    let rom = rom();
    let project = with_trace(&rom, executed(&rom, &[], &[(0x200, 0x210)]));
    let s = run(&rom, &project);
    let region = s.region_at(FileOffset(0x200)).unwrap();
    assert_eq!(region.kind, RegionKind::Data(DataKind::Byte));
    assert!((region.confidence - 0.85).abs() < 1e-6);
    // The boot code was executed statically and stays code: a read record
    // never overwrites something already classified.
    assert_eq!(kind_at(&s, 0), RegionKind::Code);
}

/// The user outranks everything, including an observation.
#[test]
fn the_user_still_wins() {
    let rom = rom();
    let mut project = with_trace(&rom, executed(&rom, &[(0x0C, 0x0E, false)], &[]));
    project
        .apply(
            &rom,
            Command::MarkRegion {
                start: FileOffset(0x0C),
                len: 2,
                kind: OverrideKind::Data(DataKind::Palette),
            },
        )
        .unwrap();
    let s = run(&rom, &project);
    let region = s.region_at(FileOffset(0x0C)).unwrap();
    assert_eq!(region.kind, RegionKind::Data(DataKind::Palette));
    assert_eq!(region.evidence, vec![Evidence::User]);
}

/// The widths are the part the disassembler wants most: static descent loses
/// M and X after a `PLP`, and a recorded width is the answer. They stay
/// analyzer hints and never become user flag overrides.
#[test]
fn observed_widths_steer_the_decode_without_becoming_overrides() {
    // `PLP` leaves the widths unknown; the byte after it decodes differently
    // under 8-bit and 16-bit A.
    let code = [
        0x18, 0xFB, // CLC; XCE
        0x28, // PLP: widths now assumed
        0xA9, 0x01, 0x60, // LDA #$01; RTS   (8-bit A)
    ];
    let rom = RomImage::from_bytes(
        fixtures::build_with_code(
            romlens_core::MappingMode::LoRom,
            0x8000,
            false,
            &code,
            fixtures::FIXTURE_TITLE,
        ),
        "plp.sfc",
    )
    .unwrap();

    let mut coverage = Coverage::new(rom.len() as u32);
    coverage.flags.recorded = true;
    for off in [0x03u32, 0x05] {
        coverage.mark_opcode(off, false);
        coverage.flags.m8.insert(off);
        coverage.flags.x8.insert(off);
    }
    let project = with_trace(&rom, coverage);
    let s = run(&rom, &project);
    // `A9 01` is two bytes under 8-bit A and three under 16-bit.
    let lda = s
        .instructions
        .iter()
        .find(|r| r.offset == 0x03)
        .expect("the observed opcode was decoded");
    assert_eq!(lda.len, 2, "the recorded width was not used");
    assert!(
        s.instructions.iter().any(|r| r.offset == 0x05),
        "the next instruction is only aligned if the width was right"
    );
    // The hint is not an edit: it must not reach flags.json or the undo stack.
    assert!(
        project.flag_overrides.is_empty(),
        "an observation became a user flag override"
    );
}

/// The package carries one merged coverage file and a record per import, and
/// a package copied without its `traces/` directory still opens.
#[test]
fn traces_round_trip_through_a_package() {
    let rom = rom();
    let project = with_trace(
        &rom,
        executed(&rom, &[(0x0C, 0x0E, false)], &[(0x200, 0x204)]),
    );
    let mut files = to_files(&rom, &project);
    assert!(files.contains_key(COVERAGE_FILE));
    let back = from_files(&rom, &files).unwrap();
    assert_eq!(back.traces, project.traces);
    assert_eq!(back.coverage, project.coverage);

    files.remove(COVERAGE_FILE);
    let without = from_files(&rom, &files).unwrap();
    assert!(without.coverage.is_none());
    assert!(
        without.traces.is_empty(),
        "a record with no payload is dropped"
    );
}

/// Two sessions merge, and the second does not overwrite the first's widths.
#[test]
fn two_traces_merge() {
    let rom = rom();
    let mut project = with_trace(&rom, executed(&rom, &[(0x0C, 0x0E, false)], &[]));
    project.add_trace(
        TraceRecord {
            source: "second.cdl".into(),
            format: "cdl".into(),
            executed_bytes: 3,
            read_bytes: 0,
        },
        executed(&rom, &[(0x10, 0x13, true)], &[]),
    );
    assert_eq!(project.traces.len(), 2);
    let coverage = project.coverage.as_ref().unwrap();
    assert_eq!(coverage.executed.count(), 5);
    let s = run(&rom, &project);
    assert_eq!(kind_at(&s, 0x0C), RegionKind::Code);
    assert_eq!(kind_at(&s, 0x10), RegionKind::Code);
    // With two traces merged, naming one of them would be a lie.
    assert_eq!(
        s.region_at(FileOffset(0x10)).unwrap().evidence,
        vec![Evidence::Trace {
            file: "2 traces".into(),
            hits: 5,
        }]
    );
    // Re-importing the same source replaces its record rather than adding one.
    project.add_trace(
        TraceRecord {
            source: "second.cdl".into(),
            format: "cdl".into(),
            executed_bytes: 3,
            read_bytes: 0,
        },
        executed(&rom, &[(0x10, 0x13, true)], &[]),
    );
    assert_eq!(project.traces.len(), 2);
}

/// A usage map folds onto ROM offsets through every mirror, and both formats
/// reach the same place.
#[test]
fn both_formats_reach_the_same_coverage() {
    let rom = rom();
    let mut map = vec![0u8; usage_map::CPU_BLOCK];
    map[0x80_800C] = usage_map::EXEC | usage_map::OPCODE | usage_map::FLAG_E;
    map[0x00_800D] = usage_map::EXEC;
    let (_, from_map) = read_trace(&map, &rom, None).unwrap();

    let mut payload = vec![0u8; rom.len()];
    payload[0x0C] = cdl::CODE | cdl::JUMP_TARGET | cdl::MEMORY_MODE_8 | cdl::INDEX_MODE_8;
    payload[0x0D] = cdl::CODE;
    let (_, from_cdl) = read_trace(&payload, &rom, None).unwrap();

    assert_eq!(from_map.executed, from_cdl.executed);
    assert_eq!(from_map.opcode_start, from_cdl.opcode_start);
    assert_eq!(from_map.flags.m8, from_cdl.flags.m8);
    assert_eq!(from_map.flags.x8, from_cdl.flags.x8);
}
