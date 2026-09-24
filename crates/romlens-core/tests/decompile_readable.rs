//! What makes the C read like C: registers by name even where the data
//! bank is not known, and a pointer built a byte at a time as one store.

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
    assert!(text.contains("SET24(0x0000, 0x0E8000);"), "{text}");
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
    assert!(text.contains("SET24(SpcSource, 0x0E8000);"), "{text}");
    assert!(text.contains("extern u8 SpcSource[3];"), "{text}");
}
