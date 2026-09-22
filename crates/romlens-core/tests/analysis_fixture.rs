//! The homebrew fixture through the analyzer.

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::fixtures;
use romlens_core::model::{DataKind, Project, RegionKind, XRefKind, hardware_register};
use romlens_core::{FileOffset, RomImage, SnesAddress};

#[test]
fn boot_code_is_walked_and_labelled() {
    let rom = RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap();
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    // RESET: SEI CLC XCE SEP LDA STA BRA (7 instructions, 12 bytes); the
    // NOPs are unreached; RTI at $800E from the other eleven slots.
    let offsets: Vec<u32> = snap.instructions.iter().map(|r| r.offset).collect();
    assert_eq!(offsets, vec![0, 1, 2, 3, 5, 7, 10, 14]);
    assert_eq!(snap.stats.code_bytes, 13);
    assert_eq!(snap.stats.instructions, 8);
    assert_eq!(snap.stats.blocks, 2);
    let code = snap.region_at(FileOffset(5)).unwrap();
    assert_eq!(code.kind, RegionKind::Code);
    assert_eq!((code.start, code.len), (FileOffset(0), 12));
    assert!((code.confidence - 0.9).abs() < 1e-6);
    // The two `NOP`s between the spin and the `RTI` are not reached, and since
    // Phase 2 the entropy heuristic claims the near-empty window around them
    // as data rather than leaving it unknown. It is only allowed to because
    // nothing else claimed those bytes.
    let filler = snap.region_at(FileOffset(12)).unwrap();
    assert_eq!(filler.kind, RegionKind::Data(DataKind::Byte));
    assert!(filler.confidence <= 0.55, "a guess must not look certain");
    assert_eq!(
        snap.region_at(FileOffset(14)).unwrap().kind,
        RegionKind::Code
    );
    let header = snap.region_at(FileOffset(0x7FC0)).unwrap();
    assert_eq!(header.kind, RegionKind::Data(DataKind::Struct));
    assert_eq!((header.start, header.len), (FileOffset(0x7FC0), 0x40));
    // Regions cover the image without gaps.
    let mut pos = 0;
    for r in &snap.regions {
        assert_eq!(r.start.0, pos);
        pos += r.len;
    }
    assert_eq!(pos, rom.len() as u32);

    // Labels: RESET target, the catch-all (one label, NMI wins), the BRA loop.
    let names: Vec<&str> = snap.auto_labels.values().map(|l| l.name.as_str()).collect();
    assert_eq!(names, vec!["RESET_008000", "CODE_00800A", "NMI_00800E"]);
    let catch_all = snap.xrefs_to(SnesAddress::new(0, 0x800E));
    assert_eq!(catch_all.len(), 11);
    assert!(catch_all.iter().all(|x| x.kind == XRefKind::Vector));
    // STA $2100: a write to a named register.
    let inidisp = snap.xrefs_to(SnesAddress::new(0, 0x2100));
    assert_eq!(inidisp.len(), 1);
    assert_eq!(inidisp[0].kind, XRefKind::Write);
    assert_eq!(inidisp[0].from, FileOffset(7));
    assert_eq!(inidisp[0].to_offset, None);
    assert_eq!(hardware_register(0x2100).unwrap().name, "INIDISP");
    let from_bra = snap.xrefs_from(FileOffset(10));
    assert_eq!(from_bra.len(), 1);
    assert_eq!(from_bra[0].kind, XRefKind::Jump, "BRA is unconditional");
    assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
    // Re-decoding a record reproduces the instruction.
    let rec = snap.instruction_at(FileOffset(6)).unwrap();
    assert_eq!(rec.offset, 5);
    let insn = snap.decode_at(&rom, rec).unwrap();
    assert_eq!(insn.mnemonic.as_str(), "LDA");
    assert_eq!(insn.len, 2, "M=1 after SEP #$30");
    assert!(!rec.flags_before().e);
}

#[test]
fn all_three_mappings_analyze() {
    for mode in romlens_core::MappingMode::all() {
        let rom = RomImage::from_bytes(fixtures::for_mapping(mode), "t.sfc").unwrap();
        let snap = analyze(&rom, &Project::new(&rom), &AnalysisControl::silent()).unwrap();
        assert_eq!(snap.stats.instructions, 8, "{mode}");
        assert_eq!(snap.stats.code_bytes, 13, "{mode}");
        assert!(
            snap.auto_labels
                .values()
                .any(|l| l.name.starts_with("RESET_"))
        );
    }
}
