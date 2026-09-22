//! The development ROM: the docs/04 boot listing, the never-happen handler
//! every spare vector points at, and the time budget. Opt-in through
//! `ROMLENS_ROM_DIR`.

mod common;

use std::time::Instant;

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::model::{Project, XRefKind};
use romlens_core::{FileOffset, SnesAddress};

/// docs/04, the first 30 instructions from `$80:841C`.
const LISTING: [&str; 30] = [
    "78",
    "18",
    "FB",
    "5C 23 84 80",
    "E2 20",
    "A9 01",
    "8D 0D 42",
    "85 86",
    "C2 30",
    "A2 FF 1F",
    "9A",
    "A9 00 00",
    "5B",
    "4B",
    "AB",
    "E2 30",
    "A2 04",
    "AD 12 42",
    "10 FB",
    "AD 12 42",
    "30 FB",
    "CA",
    "D0 F3",
    "C2 30",
    "A2 FE 1F",
    "9E 00 00",
    "CA",
    "CA",
    "10 F9",
    "22 46 91 8B",
];

#[test]
fn boot_listing_vectors_and_budget() {
    let Some(rom) = common::dev_rom() else { return };
    let started = Instant::now();
    let snap = analyze(&rom, &Project::new(&rom), &AnalysisControl::silent()).unwrap();
    let elapsed = started.elapsed();
    eprintln!(
        "dev ROM: {} instructions, {} code / {} data / {} unknown bytes, {} labels, {} xrefs, {} warnings in {} ms",
        snap.stats.instructions,
        snap.stats.code_bytes,
        snap.stats.data_bytes,
        snap.stats.unknown_bytes,
        snap.stats.labels,
        snap.stats.xrefs,
        snap.stats.warnings,
        elapsed.as_millis()
    );
    for w in &snap.warnings {
        eprintln!("  warning {} {}: {}", w.offset, w.kind.name(), w.text);
    }
    let start = snap.instruction_index_from(0x41C);
    for (i, expected) in LISTING.iter().enumerate() {
        let rec = snap.instructions[start + i];
        let bytes = &rom.bytes()[rec.offset as usize..rec.end() as usize];
        let text: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
        assert_eq!(
            text.join(" "),
            *expected,
            "instruction {i} at {}",
            FileOffset(rec.offset)
        );
    }
    assert_eq!(snap.instructions[start].offset, 0x41C);
    // Native COP/BRK/ABORT, emulation COP/ABORT/NMI/IRQ, plus the two unused
    // slots (native RESET, emulation BRK) all hold $8573.
    let never = snap.xrefs_to(SnesAddress::new(0x80, 0x8573));
    assert_eq!(
        never.iter().filter(|x| x.kind == XRefKind::Vector).count(),
        9
    );
    assert_eq!(
        snap.auto_labels[&SnesAddress::new(0x80, 0x8573)].name,
        "NMI_808573"
    );
    assert_eq!(
        snap.auto_labels[&SnesAddress::new(0x80, 0x841C)].name,
        "RESET_80841C"
    );
    assert_eq!(
        snap.auto_labels[&SnesAddress::new(0x80, 0x8423)].name,
        "CODE_808423"
    );
    // Static descent alone reaches only what static edges reach; jump tables
    // are Phase 2 (docs/10 records the numbers).
    assert!(snap.stats.code_bytes > 5_000, "{}", snap.stats.code_bytes);
    if !cfg!(debug_assertions) {
        assert!(elapsed.as_secs_f64() < 2.0, "{elapsed:?}");
    }
}
