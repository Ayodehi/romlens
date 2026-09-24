//! Explanations (docs/20): the value each hardware store writes, and what
//! it means.

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::explain::Explanations;
use romlens_core::fixtures;
use romlens_core::model::{Command, Project};
use romlens_core::{FileOffset, RomImage, SnesAddress};

fn rom() -> RomImage {
    RomImage::from_bytes(fixtures::explain_lorom(), "e.sfc").unwrap()
}

fn build(rom: &RomImage, project: &Project) -> Explanations {
    let snap = analyze(rom, project, &AnalysisControl::silent()).unwrap();
    Explanations::build(rom, project, &snap)
}

fn at(rom: &RomImage, x: &Explanations, addr: u16) -> String {
    let off = rom.file_offset_for(SnesAddress::new(0, addr)).unwrap();
    x.write_at(off)
        .unwrap_or_else(|| panic!("no explanation at ${addr:04X}"))
        .short()
}

#[test]
fn constants_reach_their_stores() {
    let rom = rom();
    let x = build(&rom, &Project::new(&rom));
    assert_eq!(at(&rom, &x, 0x8009), "INIDISP = $8F: forced blank, brightness 15");
    assert_eq!(at(&rom, &x, 0x800C), "NMITIMEN = $00: NMI off");
    // A 16-bit store to two registers, and one to a pair.
    assert!(at(&rom, &x, 0x8014).starts_with("DMAP0 = $01: 2 registers, alternating; BBAD0"));
    assert_eq!(at(&rom, &x, 0x8026), "VMADD = $6000: VRAM word $6000 (byte $0C000)");
    assert!(at(&rom, &x, 0x8020).starts_with("DAS0 = $0800: 2048 bytes"));
    assert_eq!(at(&rom, &x, 0x8030), "A1B0 = $00: bank $00");
    assert_eq!(at(&rom, &x, 0x8035), "MDMAEN = $01: start DMA on channel 0");
    assert_eq!(at(&rom, &x, 0x8053), "NMITIMEN = $81: NMI on, joypad auto-read on");
}

#[test]
fn an_unknown_value_says_where_it_came_from() {
    let rom = rom();
    let x = build(&rom, &Project::new(&rom));
    assert_eq!(at(&rom, &x, 0x804E), "INIDISP ← $7E:0DAE");
    let mut project = Project::new(&rom);
    project
        .apply(
            &rom,
            Command::SetLabel {
                address: SnesAddress::new(0x7E, 0x0DAE),
                name: Some("Brightness".into()),
            },
        )
        .unwrap();
    let x = build(&rom, &project);
    assert_eq!(at(&rom, &x, 0x804E), "INIDISP ← Brightness ($7E:0DAE)");
}

#[test]
fn a_store_reached_two_ways_keeps_what_they_agree_on() {
    let rom = rom();
    let x = build(&rom, &Project::new(&rom));
    // $80A6 is reached from $80A0 and from its own entry at $80A4: A is
    // $0F either way.
    assert_eq!(at(&rom, &x, 0x80A6), "INIDISP = $0F: display on, brightness 15");
    let s = x.stats();
    assert!(s.stores >= 12, "{s:?}");
    assert_eq!(x.write_at(FileOffset::new(0)), None);
}

/// Where two paths meet with different values, and after a call, the value
/// is unknown; a push and pull within the block keep it.
#[test]
fn joins_calls_and_the_stack() {
    let mut code = vec![0u8; 0x40];
    let mut put = |at: usize, b: &[u8]| code[at..at + b.len()].copy_from_slice(b);
    put(0x00, &[0x78, 0x18, 0xFB, 0xE2, 0x30]);
    put(0x05, &[0xA9, 0x01, 0xA6, 0x10, 0xF0, 0x02, 0xA9, 0x02]);
    put(0x0D, &[0x8D, 0x00, 0x21]); // A is $01 or $02
    put(0x10, &[0xA9, 0x03, 0x20, 0x30, 0x80]);
    put(0x15, &[0x8D, 0x00, 0x21]); // after a call
    put(0x18, &[0xA9, 0x04, 0x48, 0xA9, 0x00, 0x68]);
    put(0x1E, &[0x8D, 0x00, 0x21, 0x80, 0xFE]); // pulled back: $04
    put(0x30, &[0x60, 0x40]);
    let mut vectors = [0x8031; 12];
    vectors[10] = 0x8000;
    let rom = RomImage::from_bytes(
        fixtures::build_custom(
            romlens_core::MappingMode::LoRom,
            0x8000,
            false,
            &code,
            "JOINS",
            vectors,
        ),
        "j.sfc",
    )
    .unwrap();
    let x = build(&rom, &Project::new(&rom));
    assert_eq!(at(&rom, &x, 0x800D), "INIDISP");
    assert_eq!(at(&rom, &x, 0x8015), "INIDISP");
    assert_eq!(at(&rom, &x, 0x801E), "INIDISP = $04: display on, brightness 4");
}

mod common;

/// Every routine of the development ROM: no panic, and the counts.
#[test]
fn dev_rom_explains_without_panicking() {
    let Some(rom) = common::dev_rom() else { return };
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let started = std::time::Instant::now();
    let x = Explanations::build(&rom, &project, &snap);
    let s = x.stats();
    eprintln!("{s:?} in {:?}", started.elapsed());
    assert!(s.stores > 100, "{s:?}");
}
