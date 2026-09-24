//! What the screen is set up to be at an instruction (docs/21): registers
//! through calls, paths that disagree, and where VRAM was filled from.

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::explain::screen::{Link, ScreenSetup, screen_at};
use romlens_core::explain::setup::{Reach, registers_at};
use romlens_core::explain::{Explanations, UploadIndex};
use romlens_core::fixtures;
use romlens_core::model::Project;
use romlens_core::{FileOffset, MappingMode, RomImage, SnesAddress};

/// RESET calls a setup routine, then either path sets BG2's tilemap:
///
/// ```text
/// $8000  SEI; CLC; XCE; PHK; PLB; SEP #$30
/// $8007  JSR $8040
/// $800A  LDA $10 / BEQ +7
/// $800E  LDA #$34 / STA $2108 / BRA +5    ; BG2SC one way
/// $8015  LDA #$38 / STA $2108             ; or the other
/// $801A  NOP                               ; <- the screen here
/// $801B  BRA $801B
///
/// $8040  LDA #$01 / STA $2105     ; mode 1
/// $8045  LDA #$21 / STA $2107     ; BG1: tilemap $2000, 64×32
/// $804A  STZ $210B                ; BG1/BG2 tiles at $0000
/// $804D  LDA #$11 / STA $212C     ; BG1 and sprites on the main screen
/// $8052  REP #$20
/// $8054  LDA #$0000 / STA $2116   ; VRAM $0000
/// $805A  LDA #$1801 / STA $4300
/// $8060  LDA #$9000 / STA $4302
/// $8066  LDA #$0800 / STA $4305
/// $806C  SEP #$20
/// $806E  STZ $4304
/// $8071  LDA #$01 / STA $420B     ; tiles from $00:9000
/// $8076  RTS
/// ```
fn rom() -> RomImage {
    let mut code = vec![0u8; 0x100];
    let mut put = |at: u16, b: &[u8]| {
        let i = (at - 0x8000) as usize;
        code[i..i + b.len()].copy_from_slice(b);
    };
    put(0x8000, &[0x78, 0x18, 0xFB, 0x4B, 0xAB, 0xE2, 0x30]);
    put(0x8007, &[0x20, 0x40, 0x80]);
    put(0x800A, &[0xA5, 0x10, 0xF0, 0x07]);
    put(0x800E, &[0xA9, 0x34, 0x8D, 0x08, 0x21, 0x80, 0x05]);
    put(0x8015, &[0xA9, 0x38, 0x8D, 0x08, 0x21]);
    put(0x801A, &[0xEA, 0x80, 0xFE]);
    put(0x8040, &[0xA9, 0x01, 0x8D, 0x05, 0x21]);
    put(0x8045, &[0xA9, 0x21, 0x8D, 0x07, 0x21]);
    put(0x804A, &[0x9C, 0x0B, 0x21]);
    put(0x804D, &[0xA9, 0x11, 0x8D, 0x2C, 0x21]);
    put(0x8052, &[0xC2, 0x20]);
    put(0x8054, &[0xA9, 0x00, 0x00, 0x8D, 0x16, 0x21]);
    put(0x805A, &[0xA9, 0x01, 0x18, 0x8D, 0x00, 0x43]);
    put(0x8060, &[0xA9, 0x00, 0x90, 0x8D, 0x02, 0x43]);
    put(0x8066, &[0xA9, 0x00, 0x08, 0x8D, 0x05, 0x43]);
    put(0x806C, &[0xE2, 0x20]);
    put(0x806E, &[0x9C, 0x04, 0x43]);
    put(0x8071, &[0xA9, 0x01, 0x8D, 0x0B, 0x42, 0x60]);
    put(0x80F0, &[0x40]);
    let mut vectors = [0x80F0; 12];
    vectors[10] = 0x8000;
    RomImage::from_bytes(
        fixtures::build_custom(MappingMode::LoRom, 0x8000, false, &code, "SCREEN", vectors),
        "s.sfc",
    )
    .unwrap()
}

fn off(rom: &RomImage, a: u16) -> FileOffset {
    rom.file_offset_for(SnesAddress::new(0, a)).unwrap()
}

fn setup(rom: &RomImage) -> ScreenSetup {
    let project = Project::new(rom);
    let snap = analyze(rom, &project, &AnalysisControl::silent()).unwrap();
    let x = Explanations::build(rom, &project, &snap);
    let u = UploadIndex { explain: &x, rom };
    screen_at(rom, &project, &snap, off(rom, 0x801A), Some(&u)).unwrap()
}

fn row<'a>(
    s: &'a ScreenSetup,
    section: &str,
    label: &str,
) -> &'a romlens_core::explain::screen::Row {
    s.sections
        .iter()
        .find(|x| x.title.starts_with(section))
        .and_then(|x| x.rows.iter().find(|r| r.label == label))
        .unwrap_or_else(|| panic!("no {section} / {label} in {s:#?}"))
}

#[test]
fn a_call_s_writes_reach_past_it() {
    let rom = rom();
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let r = registers_at(&rom, &snap, off(&rom, 0x801A)).unwrap();
    assert_eq!(r.get(0x2105), Some(Reach::Known(0x01, off(&rom, 0x8042))));
    // Two paths, two values.
    assert_eq!(r.get(0x2108), Some(Reach::Varies));
    // Never written.
    assert_eq!(r.get(0x2109), None);
}

#[test]
fn the_setup_reads_as_a_screen() {
    let rom = rom();
    let s = setup(&rom);
    assert!(
        row(&s, "Background mode", "Mode")
            .text
            .starts_with("mode 1:")
    );
    let map = row(&s, "BG1", "Tilemap");
    assert_eq!(map.text, "tilemap at VRAM $2000, 64×32 tiles");
    assert_eq!(map.set_at, [off(&rom, 0x8047)]);
    let tiles = row(&s, "BG1", "Tiles");
    assert_eq!(tiles.text, "tiles at VRAM $0000");
    // The DMA that writes there, and its source in ROM to view.
    assert_eq!(
        tiles.source.as_deref(),
        Some("VRAM here is written by the DMA at $00:8073: $0800 bytes from $00:9000")
    );
    assert_eq!(
        tiles.link,
        Some(Link::Tiles {
            rom: rom.file_offset_for(SnesAddress::new(0, 0x9000)).unwrap(),
            bpp: 4
        })
    );
    assert_eq!(
        row(&s, "BG1", "Shown on").text,
        "the main screen; the sub screen not set yet"
    );
    assert_eq!(
        row(&s, "BG2", "Tilemap").text,
        "set differently on each path here"
    );
    assert_eq!(
        row(&s, "BG2", "Shown on").text,
        "not the main screen; the sub screen not set yet"
    );
    assert_eq!(row(&s, "BG3", "Tilemap").text, "not set yet");
}
