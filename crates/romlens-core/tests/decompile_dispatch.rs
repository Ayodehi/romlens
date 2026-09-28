//! Routines behind a jump no table explains, which an execution log saw
//! (Super Mario World's game modes, through `JSL JumpTableLong`), and what
//! the C says a routine uses of what it is given.

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::decompile::{self, DecompileOptions};
use romlens_core::fixtures;
use romlens_core::io::import::{self, exec_log};
use romlens_core::model::exec_log::{ExecInsn, ExecLog, Flow, FlowKind, MemKind};
use romlens_core::model::{Project, TraceRecord};
use romlens_core::{MappingMode, RomImage, SnesAddress};

/// ```text
/// $8020  SEP #$20 / JSR $8070 / JMP ($0010)   ; the NMI: a call, then a dispatch
/// $8050  JSR $8070 / RTS                      ; a handler only the log saw
/// $8070  LDA $12 / BEQ $8075 / CLC            ; leaves the carry alone on one way
/// $8075  RTS
/// ```
fn rom() -> RomImage {
    let mut code = vec![0u8; 0x80];
    code[..fixtures::BOOT_CODE.len()].copy_from_slice(&fixtures::BOOT_CODE);
    code[0x20..0x28].copy_from_slice(&[0xE2, 0x20, 0x20, 0x70, 0x80, 0x6C, 0x10, 0x00]);
    code[0x50..0x54].copy_from_slice(&[0x20, 0x70, 0x80, 0x60]);
    code[0x70..0x76].copy_from_slice(&[0xA5, 0x12, 0xF0, 0x01, 0x18, 0x60]);
    let mut vectors = fixtures::DEFAULT_VECTORS;
    vectors[3] = 0x8020;
    let bytes = fixtures::build_custom(
        MappingMode::LoRom,
        0x8000,
        false,
        &code,
        "DISPATCH",
        vectors,
    );
    RomImage::from_bytes(bytes, "d.sfc").unwrap()
}

/// The dispatch as a session saw it: the `JMP ($0010)` went to `$8050`.
fn project(rom: &RomImage) -> Project {
    let insn = |pc: u32| ExecInsn {
        pc,
        abs: (pc & 0x7FFF) as i32,
        kind: MemKind::PrgRom,
        states: 1 << 3,
        count: 1,
    };
    let log = ExecLog {
        rom_crc32: romlens_core::io::crc32::crc32(rom.bytes()),
        rom_size: rom.len() as u32,
        insns: [
            0x8020, 0x8022, 0x8025, 0x8050, 0x8053, 0x8070, 0x8072, 0x8074, 0x8075,
        ]
        .into_iter()
        .map(insn)
        .collect(),
        accesses: vec![],
        flows: vec![Flow {
            from: 0x8025,
            to: 0x8050,
            kind: FlowKind::IndirectJump,
            count: 1,
        }],
        dma: vec![],
    };
    let mut p = Project::new(rom);
    let t = import::read(&exec_log::write(&log), rom, None).unwrap();
    p.add_trace(
        TraceRecord {
            source: "play.mxlog".into(),
            format: t.format.name().into(),
            executed_bytes: t.coverage.executed.count(),
            read_bytes: t.coverage.read.count(),
        },
        t.coverage,
    );
    p.add_exec_log(t.exec_log.as_ref().unwrap());
    p
}

#[test]
fn a_handler_a_log_saw_is_a_routine_and_its_calls_count() {
    let rom = rom();
    let project = project(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let c = |at: u16| {
        decompile::decompile(
            &rom,
            &project,
            &snap,
            SnesAddress::new(0, at),
            &DecompileOptions::default(),
        )
        .unwrap()
        .text
    };
    let nmi = c(0x8020);
    let decl = nmi
        .lines()
        .find(|l| l.contains("SUB_008070(") && l.ends_with("*/"))
        .unwrap_or_else(|| panic!("{nmi}"));
    // Both its callers are known now: the NMI, and the handler.
    assert!(!decl.contains("callers are not known"), "{decl}");
    // It does nothing with the carry; it can only give it back.
    assert!(
        decl.contains("Reads nothing; the carry comes back as given where it leaves it alone"),
        "{decl}"
    );
    // The handler is a routine of its own, which the dispatch reaches.
    let handler = c(0x8050);
    assert!(handler.contains("SUB_008070("), "{handler}");
}

/// `LDA`, `XBA`, `LDA` builds A from two bytes, as Super Mario World's
/// palette upload does: whatever A's high byte was on entry is gone before
/// anything reads it.
///
/// ```text
/// $8020  JSR $8040 / RTI
/// $8040  SEP #$20 / LDA $10 / XBA / LDA $11 / REP #$30 / TAY / STY $14 / RTS
/// ```
#[test]
fn a_byte_swapped_away_is_not_read() {
    let mut code = vec![0u8; 0x60];
    code[..fixtures::BOOT_CODE.len()].copy_from_slice(&fixtures::BOOT_CODE);
    code[0x20..0x24].copy_from_slice(&[0x20, 0x40, 0x80, 0x40]);
    code[0x40..0x4E].copy_from_slice(&[
        0xE2, 0x20, 0xA5, 0x10, 0xEB, 0xA5, 0x11, 0xC2, 0x30, 0xA8, 0x84, 0x14, 0x60, 0x00,
    ]);
    let mut vectors = fixtures::DEFAULT_VECTORS;
    vectors[3] = 0x8020;
    let bytes = fixtures::build_custom(MappingMode::LoRom, 0x8000, false, &code, "XBA", vectors);
    let rom = RomImage::from_bytes(bytes, "x.sfc").unwrap();
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let nmi = decompile::decompile(
        &rom,
        &project,
        &snap,
        SnesAddress::new(0, 0x8020),
        &DecompileOptions::default(),
    )
    .unwrap()
    .text;
    let decl = nmi
        .lines()
        .find(|l| l.contains("SUB_008040(") && l.ends_with("*/"))
        .unwrap_or_else(|| panic!("{nmi}"));
    assert!(decl.contains("/* Reads nothing;"), "{decl}");
}
