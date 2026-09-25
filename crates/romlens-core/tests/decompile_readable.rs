//! What makes the C read like C: registers by name even where the data
//! bank is not known, a pointer built a byte at a time as one store, and
//! unnamed RAM as `ADDR_7E0000` until a variable names it.

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::decompile::{self, DecompileOptions};
use romlens_core::fixtures;
use romlens_core::model::{Command, Origin, Project, VarType, VarWidth};
use romlens_core::{MappingMode, RomImage, SnesAddress};

/// The NMI handler at `$00:8020`, entered with the data bank unknown:
///
/// ```text
/// $8020  SEP #$20
/// $8022  LDA #$00 / STA $0000     ; a pointer to $0E:8000, a byte at a time
/// $8027  LDA #$80 / STA $0001
/// $802C  LDA #$0E / STA $0002
/// $8031  STZ $2140 / STZ $2141    ; two APU ports: never merged
/// $8037  RTI
/// ```
fn rom() -> RomImage {
    let mut code = vec![0u8; 0x40];
    code[..fixtures::BOOT_CODE.len()].copy_from_slice(&fixtures::BOOT_CODE);
    code[0x20..0x38].copy_from_slice(&[
        0xE2, 0x20, 0xA9, 0x00, 0x8D, 0x00, 0x00, 0xA9, 0x80, 0x8D, 0x01, 0x00, 0xA9, 0x0E, 0x8D,
        0x02, 0x00, 0x9C, 0x40, 0x21, 0x9C, 0x41, 0x21, 0x40,
    ]);
    let mut vectors = fixtures::DEFAULT_VECTORS;
    vectors[3] = 0x8020; // native NMI
    let bytes = fixtures::build_custom(
        MappingMode::LoRom,
        0x8000,
        false,
        &code,
        "READABLE",
        vectors,
    );
    RomImage::from_bytes(bytes, "r.sfc").unwrap()
}

fn c(project: &Project, rom: &RomImage) -> String {
    let snap = analyze(rom, project, &AnalysisControl::silent()).unwrap();
    decompile::decompile(
        rom,
        project,
        &snap,
        SnesAddress::new(0, 0x8020),
        &DecompileOptions::default(),
    )
    .unwrap()
    .text
}

#[test]
fn registers_are_named_and_a_pointer_is_one_store() {
    let rom = rom();
    let text = c(&Project::new(&rom), &rom);
    assert!(text.contains("ADDR_7E0000 = 0x0E8000;"), "{text}");
    assert!(
        text.contains("extern u32 ADDR_7E0000; /* 24-bit */"),
        "{text}"
    );
    assert!(text.contains("APUIO0 = 0;\n    APUIO1 = 0;"), "{text}");
    assert!(text.contains("the data bank is not known"), "{text}");
}

#[test]
fn a_long_variable_names_the_pointer() {
    let rom = rom();
    let mut project = Project::new(&rom);
    project
        .apply_batch(
            &rom,
            vec![
                Command::SetLabel {
                    address: SnesAddress::new(0x7E, 0),
                    name: Some("SpcSource".into()),
                },
                Command::SetVariable {
                    address: SnesAddress::new(0x7E, 0),
                    ty: Some(VarType::scalar(VarWidth::Long)),
                },
            ],
            Origin::User,
        )
        .unwrap();
    let text = c(&project, &rom);
    assert!(text.contains("SpcSource = 0x0E8000;"), "{text}");
    assert!(
        text.contains("extern u32 SpcSource; /* 24-bit */"),
        "{text}"
    );
    assert!(
        !text.contains("ADDR_7E0000"),
        "the variable replaces the placeholder: {text}"
    );
}

/// A test that reads the flag an instruction with its own line set is the
/// branch's alone; one made of a compare keeps the compare:
///
/// ```text
/// $8000  SEI; CLC; XCE; SEP #$30; JSR $8020; STA $30; BRA self
/// $8020  LDA $10 / SEC / SBC $12 / BMI $802C   ; A kept: its own line
/// $8027  CMP #$05 / BCS $802C                  ; the compare is the test
/// $802B  INC A
/// $802C  RTS
/// ```
#[test]
fn a_test_names_only_the_instructions_it_shows() {
    let mut code = vec![0u8; 0x100];
    let mut put = |at: usize, b: &[u8]| code[at..at + b.len()].copy_from_slice(b);
    put(
        0x00,
        &[
            0x78, 0x18, 0xFB, 0xE2, 0x30, 0x20, 0x20, 0x80, 0x85, 0x30, 0x80, 0xFE,
        ],
    );
    put(
        0x20,
        &[
            0xA5, 0x10, 0x38, 0xE5, 0x12, 0x30, 0x05, 0xC9, 0x05, 0xB0, 0x01, 0x1A, 0x60,
        ],
    );
    put(0xF0, &[0x40]);
    let mut vectors = [0x80F0; 12];
    vectors[10] = 0x8000;
    let rom = RomImage::from_bytes(
        fixtures::build_custom(MappingMode::LoRom, 0x8000, false, &code, "TESTS", vectors),
        "t.sfc",
    )
    .unwrap();
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let d = decompile::decompile(
        &rom,
        &project,
        &snap,
        SnesAddress::new(0, 0x8020),
        &DecompileOptions::default(),
    )
    .unwrap();
    let text: Vec<&str> = d.text.lines().collect();
    let off = |a: u16| rom.file_offset_for(SnesAddress::new(0, a)).unwrap();
    let shown = |a: u16| -> Vec<&str> {
        d.lines_for(off(a))
            .iter()
            .map(|&i| text[i].trim())
            .collect()
    };
    // The SBC is its own line only; the BMI is the first test of the if.
    let sbc = shown(0x8023);
    assert!(
        sbc.len() == 1 && sbc[0].contains(" - ") && !sbc[0].starts_with("if"),
        "{sbc:?}\n{}",
        d.text
    );
    assert!(shown(0x8025)[0].starts_with("if ((s8)a >= 0"), "{}", d.text);
    // The compare is in the test it became.
    let cmp = shown(0x8027);
    assert!(
        cmp.iter().any(|l| l.starts_with("if") && l.contains("5")),
        "{cmp:?}\n{}",
        d.text
    );
}
