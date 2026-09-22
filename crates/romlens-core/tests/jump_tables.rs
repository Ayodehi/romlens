//! `JMP (abs,X)` / `JSR (abs,X)` dispatch: bounding the table, following the
//! entries, and failing legibly when the table is not in ROM.

use romlens_core::analysis::jumptable::StopReason;
use romlens_core::analysis::{AnalysisControl, AnalysisSnapshot, WarningKind, analyze};
use romlens_core::fixtures;
use romlens_core::model::{DataKind, Evidence, Project, RegionKind, XRefKind};
use romlens_core::{FileOffset, MappingMode, RomImage, SnesAddress};

/// A LoROM whose reset routine dispatches through a table. Everything is at a
/// file offset equal to its `$00:8xxx` address minus `$8000`.
fn build(code: &[(usize, &[u8])]) -> RomImage {
    let mut bytes = vec![0u8; 0x200];
    for (at, b) in code {
        bytes[*at..*at + b.len()].copy_from_slice(b);
    }
    // Every vector at the catch-all RTI except RESET.
    let mut vectors = [0x80FFu16; 12];
    vectors[10] = 0x8000; // emulation RESET; everything else at the catch-all
    bytes[0xFF] = 0x40; // RTI
    let rom = fixtures::build_custom(
        MappingMode::LoRom,
        0x8000,
        false,
        &bytes,
        fixtures::FIXTURE_TITLE,
        vectors,
    );
    RomImage::from_bytes(rom, "tables.sfc").unwrap()
}

fn run(rom: &RomImage) -> AnalysisSnapshot {
    analyze(rom, &Project::new(rom), &AnalysisControl::silent()).unwrap()
}

/// A routine that decodes under 8-bit A: `LDA #imm`, `RTS`.
const ROUTINE: [u8; 3] = [0xA9, 0x01, 0x60];

/// The prologue every fixture here shares: native mode, 8-bit A and X, so the
/// flags at the dispatch are unambiguous.
const PROLOGUE: [u8; 4] = [0x18, 0xFB, 0xE2, 0x30];

fn label(snapshot: &AnalysisSnapshot, bank: u8, off: u16) -> &str {
    snapshot.auto_labels[&SnesAddress::new(bank, off)]
        .name
        .as_str()
}

fn region_at(snapshot: &AnalysisSnapshot, off: u32) -> &romlens_core::model::Region {
    snapshot
        .regions
        .iter()
        .find(|r| r.contains(off))
        .expect("every byte is in a region")
}

#[test]
fn a_table_is_bounded_by_the_first_routine_it_points_at() {
    // $00:8004 dispatches through a table at $00:8020. The table's own first
    // target sits at $00:8030, so it cannot be longer than eight entries even
    // though the bytes after it read as more entries.
    let mut table = Vec::new();
    for target in [
        0x8030u16, 0x8040, 0x8050, 0x8030, 0x8030, 0x8030, 0x8030, 0x8030,
    ] {
        table.extend_from_slice(&target.to_le_bytes());
    }
    // Two more well-formed entries past the floor, which must not be taken.
    table.extend_from_slice(&0x8030u16.to_le_bytes());
    table.extend_from_slice(&0x8040u16.to_le_bytes());
    let rom = build(&[
        (0x00, &PROLOGUE),
        (0x04, &[0x7C, 0x20, 0x80]), // JMP ($8020,X)
        (0x20, &table),
        (0x30, &ROUTINE),
        (0x40, &ROUTINE),
        (0x50, &ROUTINE),
    ]);
    let s = run(&rom);

    let warning = s
        .warnings
        .iter()
        .find(|w| w.kind == WarningKind::JumpTable)
        .expect("the dispatch resolved");
    assert_eq!(warning.offset, FileOffset(0x04));
    assert_eq!(
        warning.text,
        format!(
            "$00:8004: 8 entries at $00:8020 (16 bytes), bounded by {}",
            StopReason::TargetFloor.name()
        )
    );
    assert!(
        !s.warnings
            .iter()
            .any(|w| w.kind == WarningKind::ComputedJump),
        "a resolved site no longer reports a computed jump"
    );

    // The table is data, with evidence naming the dispatcher.
    let region = region_at(&s, 0x20);
    assert_eq!(region.start, FileOffset(0x20));
    assert_eq!(region.len, 16, "eight two-byte entries, not ten");
    assert_eq!(region.kind, RegionKind::Data(DataKind::Table { stride: 2 }));
    assert_eq!(region.confidence, StopReason::TargetFloor.confidence());
    assert_eq!(
        region.evidence,
        vec![Evidence::Heuristic {
            name: "jump table, 8 entries, dispatched from $00:8004".into(),
            score: StopReason::TargetFloor.confidence(),
        }]
    );
    assert_eq!(region.end(), 0x30, "the table runs up to its first target");

    // Every distinct target was walked.
    for target in [0x30u32, 0x40, 0x50] {
        assert_eq!(
            region_at(&s, target).kind,
            RegionKind::Code,
            "the routine at {target:#06X} was not reached"
        );
    }
    // One xref per entry, plus the dispatcher's own reference to the base.
    let to_base: Vec<_> = s
        .xrefs_by_target
        .iter()
        .filter(|x| x.kind == XRefKind::JumpTable)
        .collect();
    assert_eq!(to_base.len(), 1);
    assert_eq!(to_base[0].from, FileOffset(0x04));
    assert!(!to_base[0].certain, "the index is a run-time value");
    let entry_xrefs = s
        .xrefs_to(SnesAddress::new(0x00, 0x8030))
        .iter()
        .filter(|x| x.kind == XRefKind::Jump && x.from.0 >= 0x20 && x.from.0 < 0x30)
        .count();
    assert_eq!(entry_xrefs, 6, "one per slot holding $8030");

    // The base is labelled as a table, the targets as code.
    assert_eq!(label(&s, 0x00, 0x8020), "JTBL_008020");
    assert_eq!(label(&s, 0x00, 0x8030), "CODE_008030");
}

/// `JSR (abs,X)` dispatches to subroutines, so its targets rank as `SUB`.
#[test]
fn a_called_table_labels_its_targets_as_subroutines() {
    let mut table = Vec::new();
    table.extend_from_slice(&0x8030u16.to_le_bytes());
    table.extend_from_slice(&0x8040u16.to_le_bytes());
    let rom = build(&[
        (0x00, &PROLOGUE),
        (0x04, &[0xFC, 0x20, 0x80]), // JSR ($8020,X)
        (0x07, &[0x60]),             // RTS
        (0x20, &table),
        (0x30, &ROUTINE),
        (0x40, &ROUTINE),
    ]);
    let s = run(&rom);
    assert_eq!(label(&s, 0x00, 0x8030), "SUB_008030");
    assert_eq!(region_at(&s, 0x40).kind, RegionKind::Code);
}

/// A table whose targets dispatch through tables of their own. This is what
/// the multi-pass loop exists for: pass one sees only the outer dispatcher.
#[test]
fn nested_dispatch_resolves() {
    let outer = 0x8020u16.to_le_bytes();
    let inner = 0x8060u16.to_le_bytes();
    let rom = build(&[
        (0x00, &PROLOGUE),
        (0x04, &[0x7C, 0x10, 0x80]), // JMP ($8010,X) → outer table
        (0x10, &outer),              // → $00:8020
        (0x20, &[0x7C, 0x50, 0x80]), // JMP ($8050,X) → inner table
        (0x50, &inner),              // → $00:8060
        (0x60, &ROUTINE),
    ]);
    let s = run(&rom);
    assert_eq!(
        s.warnings
            .iter()
            .filter(|w| w.kind == WarningKind::JumpTable)
            .count(),
        2,
        "both dispatchers resolved"
    );
    assert_eq!(
        region_at(&s, 0x60).kind,
        RegionKind::Code,
        "the inner table's target was never reached"
    );
}

/// A user's data mark is a wall: the table stops at it, and the bytes behind
/// it keep the kind the user gave them.
#[test]
fn a_user_data_mark_bounds_a_table() {
    use romlens_core::model::{Command, OverrideKind};
    let mut table = Vec::new();
    for _ in 0..8 {
        table.extend_from_slice(&0x8060u16.to_le_bytes());
    }
    let rom = build(&[
        (0x00, &PROLOGUE),
        (0x04, &[0x7C, 0x20, 0x80]), // JMP ($8020,X)
        (0x20, &table),
        (0x60, &ROUTINE),
    ]);
    // Without the mark the table runs to the floor at $00:8060.
    let plain = run(&rom);
    assert_eq!(region_at(&plain, 0x20).len, 16);

    let mut project = Project::new(&rom);
    project
        .apply(
            &rom,
            Command::MarkRegion {
                start: FileOffset(0x24),
                len: 4,
                kind: OverrideKind::Data(DataKind::Byte),
            },
        )
        .unwrap();
    let s = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let region = region_at(&s, 0x20);
    assert_eq!(region.len, 4, "two entries, then the user's wall");
    assert_eq!(
        s.warnings
            .iter()
            .find(|w| w.kind == WarningKind::JumpTable)
            .map(|w| w.text.as_str()),
        Some("$00:8004: 2 entries at $00:8020 (4 bytes), bounded by user data mark")
    );
    assert_eq!(region_at(&s, 0x24).kind, RegionKind::Data(DataKind::Byte));
}

/// A table built in RAM cannot be read statically. The warning has to name the
/// address, because that is what makes the manual fix a one-line job.
#[test]
fn a_table_outside_rom_fails_by_name() {
    let rom = build(&[
        (0x00, &PROLOGUE),
        (0x04, &[0x7C, 0x12, 0x00]), // JMP ($0012,X) — direct page, in RAM
    ]);
    let s = run(&rom);
    let w = s
        .warnings
        .iter()
        .find(|w| w.kind == WarningKind::ComputedJump)
        .expect("an unreadable table still reports");
    assert_eq!(
        w.text,
        "$00:8004: table base $00:0012 is not in ROM; the table is built at run time"
    );
    assert!(!s.warnings.iter().any(|w| w.kind == WarningKind::JumpTable));
}

/// Bytes that merely read as addresses are not a table: every entry has to
/// decode as a routine under the dispatcher's own flags.
#[test]
fn filler_is_not_a_table() {
    let rom = build(&[
        (0x00, &PROLOGUE),
        (0x04, &[0x7C, 0x20, 0x80]), // JMP ($8020,X)
        // $00:8020 onwards is zero: `$00:0000` is RAM in LoROM, never a target.
        (0x40, &ROUTINE),
    ]);
    let s = run(&rom);
    let w = s
        .warnings
        .iter()
        .find(|w| w.kind == WarningKind::ComputedJump)
        .expect("nothing resolved");
    assert!(
        w.text.contains("no entry at $00:8020 reads as a routine"),
        "unexpected text: {}",
        w.text
    );
    assert_eq!(region_at(&s, 0x20).kind, RegionKind::Unknown);
}
