//! The stack by offset in the C (docs/22, L1): `LDA $03,S` reading what
//! the routine pushed is that push's variable, and `LDA ($01,S),Y` reads
//! through a pushed pointer.

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::decompile::{self, DecompileOptions, Level};
use romlens_core::fixtures;
use romlens_core::model::Project;
use romlens_core::{MappingMode, RomImage, SnesAddress};

/// ```text
/// $8000  SEI; CLC; XCE; REP #$30
/// $8005  JSR $8020; JSR $8040
/// $800B  BRA $800B
///
/// $8020  LDA #$1234; PHA          ; a word on the stack
/// $8024  SEP #$20
/// $8026  LDA #$01; CLC
/// $8029  ADC $02,S; STA $02,S     ; its high byte, plus one
/// $802D  REP #$20; PLA; STA $10   ; $10 = $1334
/// $8032  RTS
///
/// $8040  PEA $8070                ; a pointer on the stack
/// $8043  LDY #$0002
/// $8046  LDA ($01,S),Y; STA $12   ; the word at $8072
/// $804A  PLX; RTS
/// ```
fn rom() -> RomImage {
    let mut code = vec![0u8; 0x78];
    let mut put = |at: usize, b: &[u8]| code[at..at + b.len()].copy_from_slice(b);
    put(0x00, &[0x78, 0x18, 0xFB, 0xC2, 0x30, 0x20, 0x20, 0x80, 0x20, 0x40, 0x80, 0x80, 0xFE]);
    put(
        0x20,
        &[
            0xA9, 0x34, 0x12, 0x48, 0xE2, 0x20, 0xA9, 0x01, 0x18, 0x63, 0x02, 0x83, 0x02, 0xC2,
            0x20, 0x68, 0x85, 0x10, 0x60,
        ],
    );
    put(
        0x40,
        &[0xF4, 0x70, 0x80, 0xA0, 0x02, 0x00, 0xB3, 0x01, 0x85, 0x12, 0xFA, 0x60],
    );
    put(0x70, &[1, 2, 3, 4, 5, 6, 7, 8]);
    let bytes = fixtures::build_with_code(MappingMode::LoRom, 0x8000, false, &code, "STACK");
    RomImage::from_bytes(bytes, "s.sfc").unwrap()
}

fn c(level: Level, at: u16) -> String {
    let rom = rom();
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let opts = DecompileOptions {
        level,
        ..DecompileOptions::default()
    };
    decompile::decompile(&rom, &project, &snap, SnesAddress::new(0, at), &opts)
        .unwrap()
        .text
}

#[test]
fn a_pushed_word_updated_in_place_is_a_variable() {
    for level in [Level::Clean, Level::Full] {
        let text = c(level, 0x8020);
        assert!(!text.contains("STACK"), "{text}");
        assert!(!text.contains("push16"), "{text}");
        assert!(!text.contains("note: the stack"), "{text}");
    }
    // At lift the stack stays as the CPU sees it.
    let lift = c(Level::Lift, 0x8020);
    assert!(lift.contains("STACK8(S + 2)"), "{lift}");
}

#[test]
fn a_pushed_pointer_is_read_through() {
    let text = c(Level::Clean, 0x8040);
    assert!(!text.contains("STACK"), "{text}");
    // The pushed pointer plus Y, folded: the word at $8072.
    assert!(text.contains("MEM16(0x8072)"), "{text}");
}
