//! An execution log's import: its coverage, the references and data types it
//! adds, and its life in a project package.

use romlens_core::analysis::{AnalysisControl, AnalysisSnapshot, analyze};
use romlens_core::fixtures;
use romlens_core::io::import::{self, TraceFormat, exec_log};
use romlens_core::io::{EXEC_LOG_FILE, from_files, to_files};
use romlens_core::model::exec_log::{
    Access, AccessRun, DmaRun, ExecInsn, ExecLog, Flow, FlowKind, MemKind,
};
use romlens_core::model::{DataKind, Evidence, Project, RegionKind, TraceRecord, XRefKind};
use romlens_core::{FileOffset, RomImage, SnesAddress};

/// 64 KB LoROM with a block of BGR15 colours at 0x1000 and a table of
/// addresses at 0x1400 (`fixtures::mixed_data_lorom`).
fn rom() -> RomImage {
    RomImage::from_bytes(fixtures::mixed_data_lorom(), "t.sfc").unwrap()
}

const BOOT: u32 = 0x80_8000;
/// Zero filler, so nothing static claims it.
const CALLEE: u32 = 0x80_A000;

/// What a short session would record: boot calls a routine, reads the
/// address table, and DMAs the colours to CGRAM.
fn session(rom: &RomImage) -> ExecLog {
    let insn = |pc: u32, abs: i32| ExecInsn {
        pc,
        abs,
        kind: MemKind::PrgRom,
        states: 1 << 3, // state 3: 8-bit A and X
        count: 1,
    };
    ExecLog {
        rom_crc32: romlens_core::io::crc32::crc32(rom.bytes()),
        rom_size: rom.len() as u32,
        insns: vec![insn(BOOT, 0), insn(CALLEE, 0x2000)],
        accesses: vec![AccessRun {
            pc: BOOT,
            addr: 0x80_9400,
            abs: 0x1400,
            len: 8,
            access: Access::Read,
            kind: MemKind::PrgRom,
            count: 8,
        }],
        flows: vec![Flow {
            from: BOOT,
            to: CALLEE,
            kind: FlowKind::IndirectCall,
            count: 1,
        }],
        dma: vec![DmaRun {
            pc: BOOT,
            addr: 0x80_9000,
            abs: 0x1000,
            len: 0x200,
            bbus: 0x22,
            to_a_bus: false,
            hdma: false,
            mode: 0,
            kind: MemKind::PrgRom,
            channel: 0,
            count: 0x200,
        }],
    }
}

fn import_into(project: &mut Project, rom: &RomImage, bytes: &[u8]) {
    let t = import::read(bytes, rom, None).unwrap();
    assert_eq!(t.format, TraceFormat::ExecLog);
    project.add_trace(
        TraceRecord {
            source: "play.mxlog".into(),
            format: t.format.name().into(),
            executed_bytes: t.coverage.executed.count(),
            read_bytes: t.coverage.read.count(),
        },
        t.coverage,
    );
    project.add_exec_log(t.exec_log.as_ref().unwrap());
}

fn run(rom: &RomImage, project: &Project) -> AnalysisSnapshot {
    analyze(rom, project, &AnalysisControl::silent()).unwrap()
}

#[test]
fn a_log_adds_the_references_the_game_made() {
    let rom = rom();
    let mut project = Project::new(&rom);
    import_into(&mut project, &rom, &exec_log::write(&session(&rom)));
    let s = run(&rom, &project);

    // An indirect call no disassembler could follow, now a named routine.
    let at = |a: u32| Project::canonical(&rom, SnesAddress::from_u24(a));
    let callee = at(CALLEE);
    let calls: Vec<_> = s
        .xrefs_to(callee)
        .iter()
        .filter(|x| x.kind == XRefKind::Call)
        .collect();
    assert_eq!(calls.len(), 1, "{:?}", s.xrefs_to(callee));
    assert!(calls[0].observed && calls[0].certain);
    assert_eq!(calls[0].from, FileOffset(0));
    assert!(
        s.auto_labels
            .get(&callee)
            .is_some_and(|l| l.name.starts_with("SUB_")),
        "{:?}",
        s.auto_labels.get(&callee)
    );

    // The table the boot code read, by the instruction that read it.
    let table = s.xrefs_to(at(0x80_9400));
    assert!(
        table
            .iter()
            .any(|x| x.kind == XRefKind::Read && x.observed && x.from == FileOffset(0)),
        "{table:?}"
    );
}

#[test]
fn dma_to_cgram_makes_a_palette() {
    let rom = rom();
    let mut project = Project::new(&rom);
    import_into(&mut project, &rom, &exec_log::write(&session(&rom)));
    let s = run(&rom, &project);
    let r = s.region_at(FileOffset(0x1000)).unwrap();
    assert_eq!(r.kind, RegionKind::Data(DataKind::Palette));
    assert_eq!((r.start, r.len), (FileOffset(0x1000), 0x200));
    assert!(
        r.evidence.contains(&Evidence::Observed(
            "sent to CGRAM by DMA from $80:8000".into()
        )),
        "{:?}",
        r.evidence
    );
}

#[test]
fn a_project_keeps_its_log_and_merges_another() {
    let rom = rom();
    let log = session(&rom);
    let mut project = Project::new(&rom);
    import_into(&mut project, &rom, &exec_log::write(&log));
    import_into(&mut project, &rom, &exec_log::write(&log));
    let merged = project.exec_log.as_deref().unwrap();
    assert_eq!(merged.insns.len(), 2, "the same instructions, once each");
    assert_eq!(merged.insns[0].count, 2, "their counts summed");
    assert_eq!(merged.dma[0].count, 0x400);

    let files = to_files(&rom, &project);
    assert!(files.contains_key(EXEC_LOG_FILE));
    let back = from_files(&rom, &files).unwrap();
    assert_eq!(back.exec_log, project.exec_log);
    assert_eq!(back.coverage, project.coverage);
}

#[test]
fn a_log_from_another_rom_is_refused() {
    let rom = rom();
    let mut log = session(&rom);
    log.rom_crc32 ^= 1;
    let err = import::read(&exec_log::write(&log), &rom, None).unwrap_err();
    assert!(format!("{err}").contains("CRC32"), "{err}");
}
