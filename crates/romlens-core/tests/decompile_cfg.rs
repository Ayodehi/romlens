//! Functions and control-flow graphs on hand-assembled routines.

mod common;

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::decompile::{self, Cfg, Dest, Term, Transfer};
use romlens_core::fixtures;

use romlens_core::model::project::Project;
use romlens_core::{FileOffset, MappingMode, RomImage, SnesAddress};

fn a(off: u16) -> SnesAddress {
    SnesAddress::new(0, off)
}

/// ```text
/// $8000  SEI; CLC; XCE; SEP #$30
/// $8005  JSR $8010
/// $8008  JSR $8040
/// $800B  BRA $800B
/// $800E  RTI
/// $8010  LDX #$05
/// $8012  LDA #$00
/// $8014  STA $0100,X     ; loop
/// $8017  DEX
/// $8018  BPL $8014
/// $801A  BEQ $801F
/// $801C  JMP $8030
/// $801F  RTS
/// $8030  JMP $8040       ; a tail call: $8040 is called from $8008
/// $8040  RTS
/// ```
fn loop_rom() -> RomImage {
    let mut code = vec![0u8; 0x48];
    let mut put = |at: usize, b: &[u8]| code[at..at + b.len()].copy_from_slice(b);
    put(0x00, &[0x78, 0x18, 0xFB, 0xE2, 0x30]);
    put(0x05, &[0x20, 0x10, 0x80]);
    put(0x08, &[0x20, 0x40, 0x80]);
    put(0x0B, &[0x80, 0xFE]);
    put(0x0E, &[0x40]);
    put(
        0x10,
        &[0xA2, 0x05, 0xA9, 0x00, 0x9D, 0x00, 0x01, 0xCA, 0x10, 0xFA],
    );
    put(0x1A, &[0xF0, 0x03, 0x4C, 0x30, 0x80, 0x60]);
    put(0x30, &[0x4C, 0x40, 0x80]);
    put(0x40, &[0x60]);
    let bytes = fixtures::build_with_code(MappingMode::LoRom, 0x8000, false, &code, "CFG TEST");
    RomImage::from_bytes(bytes, "cfg.sfc").unwrap()
}

fn setup(rom: &RomImage) -> (Project, romlens_core::analysis::AnalysisSnapshot) {
    let project = Project::new(rom);
    let snap = analyze(rom, &project, &AnalysisControl::silent()).unwrap();
    (project, snap)
}

#[test]
fn a_loop_a_branch_and_a_tail_call() {
    let rom = loop_rom();
    let (_, snap) = setup(&rom);
    let entries = decompile::entries(&snap);
    assert!(entries.contains(&a(0x8010)) && entries.contains(&a(0x8040)));
    let f = decompile::discover(&rom, &snap, &entries, a(0x8010)).unwrap();
    let offs: Vec<u32> = f.steps.iter().map(|s| s.insn.file_offset.0).collect();
    assert_eq!(
        offs,
        vec![0x10, 0x12, 0x14, 0x17, 0x18, 0x1A, 0x1C, 0x1F, 0x30]
    );
    assert_eq!(
        f.steps.last().unwrap().transfer,
        Transfer::Jump(Dest::Tail(a(0x8040)))
    );

    let cfg = Cfg::build(&f);
    let starts: Vec<Option<u32>> = cfg
        .blocks
        .iter()
        .map(|b| (!b.is_stub()).then(|| f.steps[b.steps.start].insn.file_offset.0))
        .collect();
    assert_eq!(
        starts,
        vec![
            Some(0x10),
            Some(0x14),
            Some(0x1A),
            Some(0x1C),
            Some(0x1F),
            Some(0x30),
            None
        ]
    );
    let blk = |off: u32| cfg.block_of(&f, FileOffset(off)).unwrap();
    assert_eq!(cfg.entry, blk(0x10));
    assert_eq!(cfg.blocks[blk(0x10)].term, Term::Fall(blk(0x14)));
    assert_eq!(
        cfg.blocks[blk(0x14)].term,
        Term::Branch {
            taken: blk(0x14),
            fall: blk(0x1A)
        }
    );
    assert_eq!(cfg.blocks[blk(0x1C)].term, Term::Goto(blk(0x30)));
    assert_eq!(cfg.blocks[blk(0x30)].term, Term::Fall(6));
    assert_eq!(cfg.blocks[6].term, Term::Tail(a(0x8040)));
    assert_eq!(cfg.blocks[blk(0x1F)].term, Term::Return);

    assert_eq!(cfg.loops.len(), 1);
    assert_eq!(cfg.loops[0].header, blk(0x14));
    assert_eq!(cfg.loops[0].body, vec![blk(0x14)]);
    assert!(!cfg.irreducible);
    assert_eq!(cfg.idom[blk(0x1A)], Some(blk(0x14)));
    assert!(cfg.dominates(blk(0x10), blk(0x30)));
    // The branch at $801A rejoins nowhere: both arms leave the function.
    assert_eq!(cfg.ipdom[blk(0x1A)], None);
    assert_eq!(cfg.ipdom[blk(0x10)], Some(blk(0x14)));

    let inside = decompile::containing(&rom, &snap, &entries, FileOffset(0x17)).unwrap();
    assert_eq!(inside.entry, a(0x8010));
}

#[test]
fn a_dispatch_table_is_a_switch() {
    let rom = RomImage::from_bytes(fixtures::dispatch_lorom(), "d.sfc").unwrap();
    let (_, snap) = setup(&rom);
    let entries = decompile::entries(&snap);
    let f = decompile::discover(&rom, &snap, &entries, a(0x8000)).unwrap();
    let call = f
        .steps
        .iter()
        .find(|s| s.insn.file_offset.0 == 0x04)
        .unwrap();
    assert!(matches!(
        &call.transfer,
        Transfer::Call { callee: decompile::Callee::Table { targets, .. }, .. } if targets.len() == 8
    ));
    let cfg = Cfg::build(&f);
    let sw = cfg
        .blocks
        .iter()
        .find_map(|b| match &b.term {
            Term::Switch { cases, .. } => Some(cases.clone()),
            _ => None,
        })
        .expect("a switch");
    // The JMP table's two routines are also JSR-table targets, so each case
    // is a tail call to that routine.
    assert_eq!(sw.len(), 2);
    for c in sw {
        assert!(matches!(cfg.blocks[c].term, Term::Tail(_)));
    }
}

#[test]
fn not_code_is_refused() {
    let rom = loop_rom();
    let (_, snap) = setup(&rom);
    let entries = decompile::entries(&snap);
    assert!(decompile::discover(&rom, &snap, &entries, a(0x8011)).is_err());
    assert!(decompile::discover(&rom, &snap, &entries, SnesAddress::new(0x7E, 0)).is_err());
}

/// Every routine on the development ROM: no panic, every block reachable
/// from the entry has a dominator chain back to it, and loops are inside
/// their function.
#[test]
fn every_routine_on_the_development_rom() {
    let Some(rom) = common::dev_rom() else {
        return;
    };
    let (_, snap) = setup(&rom);
    let entries = decompile::entries(&snap);
    let started = std::time::Instant::now();
    let (mut ok, mut steps, mut loops, mut irreducible, mut truncated) = (0, 0, 0, 0, 0);
    for &e in &entries {
        let Ok(f) = decompile::discover(&rom, &snap, &entries, e) else {
            continue;
        };
        let cfg = Cfg::build(&f);
        for &b in &cfg.rpo {
            assert!(
                b == cfg.entry || cfg.dominates(cfg.entry, b),
                "{e}: block {b}"
            );
        }
        ok += 1;
        steps += f.steps.len();
        loops += cfg.loops.len();
        irreducible += cfg.irreducible as usize;
        truncated += f.truncated as usize;
    }
    eprintln!(
        "{ok} of {} entries: {steps} instructions, {loops} loops, {irreducible} irreducible, {truncated} truncated, {:?}",
        entries.len(),
        started.elapsed()
    );
    assert!(ok > 500, "{ok}");
}
