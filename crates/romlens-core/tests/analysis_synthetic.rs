//! A hand-built LoROM exercising every edge the walker has.

use romlens_core::analysis::{AnalysisControl, WarningKind, analyze};
use romlens_core::fixtures;
use romlens_core::model::{
    Command, DataKind, Evidence, FlagOverride, OverrideKind, Project, RegionKind, XRefKind,
};
use romlens_core::{FileOffset, MappingMode, RomImage, SnesAddress};

/// Vectors: RESET → $8000, NMI → $8090, everything else → $80A0.
fn synthetic() -> RomImage {
    let mut code = vec![0u8; 0x100];
    let mut put = |at: usize, bytes: &[u8]| code[at..at + bytes.len()].copy_from_slice(bytes);
    put(0x00, &[0x18, 0xFB]); // CLC; XCE
    put(0x02, &[0xC2, 0x30]); // REP #$30
    put(0x04, &[0xA9, 0x34, 0x12]); // LDA #$1234
    put(0x07, &[0x20, 0x20, 0x80]); // JSR $8020
    put(0x0A, &[0x22, 0x40, 0x80, 0x00]); // JSL $008040
    put(0x0E, &[0xE2, 0x20]); // SEP #$20
    put(0x10, &[0xA9, 0x01]); // LDA #$01
    put(0x12, &[0xD0, 0x02]); // BNE $8016
    put(0x14, &[0xEA, 0xEA]); // NOP NOP
    put(0x16, &[0x6C, 0x60, 0x80]); // JMP ($8060)
    put(0x20, &[0x8D, 0x00, 0x90, 0x60]); // STA $9000; RTS
    put(0x24, &[0xA9, 0x05, 0xEA, 0x60]); // sweep candidate: LDA #$05EA... no: LDA #imm16 then RTS
    put(0x40, &[0x28, 0x6B]); // PLP; RTL
    put(0x60, &[0x70, 0x80]); // pointer → $8070
    put(0x70, &[0x7C, 0x80, 0x80]); // JMP ($8080,X)
    put(0x90, &[0xC2, 0x30, 0x4C, 0x10, 0x80]); // REP #$30; JMP $8010
    put(0xA0, &[0x40]); // RTI
    let v = |n: u16| n;
    let vectors = [
        v(0x80A0),
        v(0x80A0),
        v(0x80A0),
        v(0x8090),
        v(0x80A0),
        v(0x80A0), // native, NMI → $8090
        v(0x80A0),
        v(0x80A0),
        v(0x80A0),
        v(0x8090),
        v(0x8000),
        v(0x80A0), // emulation, NMI → $8090, RESET → $8000
    ];
    RomImage::from_bytes(
        fixtures::build_custom(
            MappingMode::LoRom,
            0x8000,
            false,
            &code,
            "SYNTHETIC",
            vectors,
        ),
        "synthetic.sfc",
    )
    .unwrap()
}

fn a(off: u16) -> SnesAddress {
    SnesAddress::new(0, off)
}

#[test]
fn walker_follows_every_static_edge() {
    let rom = synthetic();
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let at = |off: u32| snap.instruction_at(FileOffset(off)).copied();
    // Width change: LDA #$1234 is 3 bytes under M=0, LDA #$01 is 2 under M=1.
    assert_eq!(at(0x04).unwrap().len, 3);
    assert_eq!(at(0x10).unwrap().len, 2);
    assert!(!at(0x02).unwrap().flags_before().e, "native after CLC/XCE");
    // Calls were followed with depth, and the callee's RTS/RTL ended them.
    assert_eq!(at(0x20).unwrap().len, 3);
    assert_eq!(at(0x23).unwrap().opcode, 0x60);
    assert_eq!(at(0x40).unwrap().opcode, 0x28);
    let sub_b = snap.region_at(FileOffset(0x40)).unwrap();
    assert!(
        (sub_b.confidence - 0.7).abs() < 1e-6,
        "PLP lowers confidence"
    );
    assert_eq!(sub_b.evidence, vec![Evidence::VectorReach { depth: 1 }]);
    assert_eq!(
        snap.region_at(FileOffset(0x00)).unwrap().evidence,
        vec![Evidence::VectorReach { depth: 0 }]
    );
    // Branch: both edges; the NOPs are reached by fall-through.
    assert!(at(0x14).is_some() && at(0x15).is_some());
    // JMP (abs): the slot is a pointer region with a PTR label; the pointee was walked.
    let slot = snap.region_at(FileOffset(0x60)).unwrap();
    assert_eq!(slot.kind, RegionKind::Data(DataKind::Pointer));
    assert_eq!(slot.len, 2);
    assert!((slot.confidence - 0.9).abs() < 1e-6);
    assert_eq!(snap.auto_labels[&a(0x8060)].name, "PTR_008060");
    assert_eq!(at(0x70).unwrap().opcode, 0x7C);
    assert_eq!(snap.auto_labels[&a(0x8070)].name, "CODE_008070");
    // JMP (abs,X) warns and stops.
    assert!(
        snap.warnings
            .iter()
            .any(|w| w.kind == WarningKind::ComputedJump && w.offset == FileOffset(0x70))
    );
    assert!(at(0x73).is_none());
    // Data xref and seed: STA $9000 under M=0 (from RESET, DBR=0) → 2-byte word.
    let data = snap.region_at(FileOffset(0x1000)).unwrap();
    assert_eq!(data.kind, RegionKind::Data(DataKind::Word));
    assert_eq!(data.len, 2);
    assert_eq!(snap.auto_labels[&a(0x9000)].name, "DATA_009000");
    assert_eq!(snap.xrefs_to(a(0x9000))[0].kind, XRefKind::Write);
    // Labels: SUB for call targets, CODE for the branch target.
    assert_eq!(snap.auto_labels[&a(0x8020)].name, "SUB_008020");
    assert_eq!(snap.auto_labels[&a(0x8040)].name, "SUB_008040");
    assert_eq!(snap.auto_labels[&a(0x8016)].name, "CODE_008016");
    assert_eq!(snap.auto_labels[&a(0x8000)].name, "RESET_008000");
    assert_eq!(snap.auto_labels[&a(0x8090)].name, "NMI_008090");
    assert_eq!(snap.auto_labels[&a(0x80A0)].name, "IRQ_0080A0");
    // Conflict: NMI reaches $8010 with M=0 after the RESET walk had M=1.
    let conflict = snap
        .warnings
        .iter()
        .find(|w| w.kind == WarningKind::FlagConflict)
        .unwrap();
    assert_eq!(conflict.offset, FileOffset(0x10));
    assert_eq!(snap.stats.conflicts, 1);
    assert!(snap.region_at(FileOffset(0x10)).unwrap().confidence < 0.9);
    // Sweep: after the RTS at $8023 the gap holds LDA #$05EA; RTS → accepted at 0.3.
    let swept = snap.region_at(FileOffset(0x24)).unwrap();
    assert_eq!(swept.kind, RegionKind::Code);
    assert!((swept.confidence - 0.3).abs() < 1e-6);
    assert_eq!(
        swept.evidence,
        vec![Evidence::Heuristic {
            name: "linear sweep".into(),
            score: 0.3
        }]
    );
    assert_eq!(at(0x24).unwrap().len, 3);
    assert_eq!(at(0x27).unwrap().opcode, 0x60);
    // ... and the zeros after it (BRK) are rejected.
    assert_eq!(
        snap.region_at(FileOffset(0x28)).unwrap().kind,
        RegionKind::Unknown
    );
    assert!(
        !snap.auto_labels.contains_key(&a(0x8024)),
        "sweeps never label"
    );
    // Sorted, non-overlapping instructions.
    for w in snap.instructions.windows(2) {
        assert!(w[0].end() <= w[1].offset);
    }
    assert_eq!(
        snap.stats.unknown_bytes + snap.stats.code_bytes + snap.stats.data_bytes,
        0x8000
    );
}

#[test]
fn user_marks_and_flag_overrides_steer_the_walk() {
    let rom = synthetic();
    let mut project = Project::new(&rom);
    // A data mark on the NOPs blocks the fall-through.
    project
        .apply(
            &rom,
            Command::MarkRegion {
                start: FileOffset(0x14),
                len: 2,
                kind: OverrideKind::Data(DataKind::Byte),
            },
        )
        .unwrap();
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    assert!(snap.instruction_at(FileOffset(0x14)).is_none());
    let blocked = snap
        .warnings
        .iter()
        .find(|w| w.kind == WarningKind::WalkedIntoUserData)
        .unwrap();
    assert_eq!(blocked.offset, FileOffset(0x14));
    let marked = snap.region_at(FileOffset(0x14)).unwrap();
    assert_eq!(marked.kind, RegionKind::Data(DataKind::Byte));
    assert_eq!(marked.evidence, vec![Evidence::User]);
    assert!((marked.confidence - 1.0).abs() < 1e-6);
    // The branch target is still reached through the taken edge.
    assert!(snap.instruction_at(FileOffset(0x16)).is_some());

    // A flag override at $8010 pins M=0: LDA #$01 D0 becomes a 3-byte immediate.
    let mut project = Project::new(&rom);
    project
        .apply(
            &rom,
            Command::SetFlagOverride {
                offset: FileOffset(0x10),
                flags: Some(FlagOverride {
                    m: Some(false),
                    ..Default::default()
                }),
            },
        )
        .unwrap();
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let lda = snap.instruction_at(FileOffset(0x10)).unwrap();
    assert_eq!(lda.len, 3);
    assert!(!lda.flags_before().m);
    assert!(
        !snap
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::FlagConflict),
        "NMI now agrees: {:?}",
        snap.warnings
    );

    // A code mark is an entry point of its own, at confidence 1.0.
    let mut project = Project::new(&rom);
    project
        .apply(
            &rom,
            Command::MarkRegion {
                start: FileOffset(0x80),
                len: 1,
                kind: OverrideKind::Code,
            },
        )
        .unwrap();
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    assert!(snap.instruction_at(FileOffset(0x80)).is_some());
    assert!((snap.region_at(FileOffset(0x80)).unwrap().confidence - 1.0).abs() < 1e-6);
    // A table mark keeps its stride through the region list.
    let mut project = Project::new(&rom);
    project
        .apply(
            &rom,
            Command::MarkRegion {
                start: FileOffset(0x1100),
                len: 8,
                kind: OverrideKind::Data(DataKind::Table { stride: 4 }),
            },
        )
        .unwrap();
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    assert_eq!(
        snap.region_at(FileOffset(0x1102)).unwrap().kind,
        RegionKind::Data(DataKind::Table { stride: 4 })
    );
}

#[test]
fn cancellation_is_honoured() {
    let rom = synthetic();
    let control = AnalysisControl::silent();
    control
        .cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    // The fixture is too small to hit a check inside the walk, but the
    // post-descent check fires.
    assert!(analyze(&rom, &Project::new(&rom), &control).is_err());
}
