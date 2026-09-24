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
    assert_eq!(
        at(&rom, &x, 0x8009),
        "INIDISP = $8F: forced blank, brightness 15"
    );
    assert_eq!(at(&rom, &x, 0x800C), "NMITIMEN = $00: NMI off");
    // A 16-bit store to two registers, and one to a pair.
    assert!(
        at(&rom, &x, 0x8014).starts_with("DMAP0 = $01: 2 registers, alternating (VRAM); BBAD0")
    );
    assert_eq!(
        at(&rom, &x, 0x8026),
        "VMADD = $6000: VRAM word $6000 (byte $C000)"
    );
    assert!(at(&rom, &x, 0x8020).starts_with("DAS0 = $0800: 2048 bytes"));
    assert_eq!(at(&rom, &x, 0x8030), "A1B0 = $00: bank $00");
    assert_eq!(at(&rom, &x, 0x8035), "MDMAEN = $01: start DMA on channel 0");
    assert_eq!(
        at(&rom, &x, 0x8053),
        "NMITIMEN = $81: NMI on, joypad auto-read on"
    );
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
    assert_eq!(
        at(&rom, &x, 0x80A6),
        "INIDISP = $0F: display on, brightness 15"
    );
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
    assert_eq!(
        at(&rom, &x, 0x801E),
        "INIDISP = $04: display on, brightness 4"
    );
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

fn titles(rom: &RomImage, x: &Explanations, addr: u16) -> Vec<String> {
    let off = rom.file_offset_for(SnesAddress::new(0, addr)).unwrap();
    x.idioms_starting_at(off)
        .iter()
        .map(|i| format!("{}: {}", i.title, i.summary))
        .collect()
}

#[test]
fn the_fixture_s_idioms() {
    let rom = rom();
    let x = build(&rom, &Project::new(&rom));
    assert_eq!(
        titles(&rom, &x, 0x8014),
        ["DMA transfer: Channel 0 copies $0800 bytes from $00:9000 to VRAM word $6000."]
    );
    assert_eq!(
        titles(&rom, &x, 0x803A),
        [
            "Hardware multiply: 6 × 7 = 42, unsigned 8 × 8 bits; the 16-bit product is read from RDMPY."
        ]
    );
    assert_eq!(
        titles(&rom, &x, 0x8056),
        ["Wait for vertical blank: Reads HVBJOY until bit 7 is set."]
    );
    assert_eq!(
        titles(&rom, &x, 0x805D),
        ["Clear memory: Clears $7E:0200 to $7E:020F: 16 bytes, a byte at a time, indexed by X."]
    );
    assert_eq!(titles(&rom, &x, 0x8063).len(), 1);
    assert!(titles(&rom, &x, 0x8063)[0].starts_with("Decimal arithmetic"));
    assert_eq!(
        titles(&rom, &x, 0x806E),
        ["Wait for the sound CPU: Waits until the sound CPU puts $AA in APUIO0."]
    );
    assert!(
        titles(&rom, &x, 0x80A4)[0]
            .starts_with("A second way in: SUB_0080A0 runs on into SUB_0080A4")
    );
    // Every instruction of the DMA's setup is part of it.
    let off = rom.file_offset_for(SnesAddress::new(0, 0x8026)).unwrap();
    assert_eq!(x.idioms_at(off).len(), 1);
}

/// Look-alikes that are not the idiom.
#[test]
fn near_misses_are_not_idioms() {
    let mut code = vec![0u8; 0x60];
    let mut put = |at: usize, b: &[u8]| code[at..at + b.len()].copy_from_slice(b);
    put(0x00, &[0x78, 0x18, 0xFB, 0x4B, 0xAB, 0xE2, 0x30]);
    // A loop on a RAM flag, not the hardware.
    put(0x07, &[0xA5, 0x10, 0xF0, 0xFC]);
    // HVBJOY masked with two bits: which one it waits for is not clear.
    put(0x0B, &[0xAD, 0x12, 0x42, 0x29, 0x03, 0xF0, 0xF9]);
    // A loop storing two things: not a clear.
    put(
        0x12,
        &[
            0xA2, 0x0F, 0x9E, 0x00, 0x02, 0x9E, 0x00, 0x03, 0xCA, 0x10, 0xF7,
        ],
    );
    // MDMAEN = 0 starts nothing.
    put(0x1D, &[0x9C, 0x0B, 0x42]);
    // SED with no arithmetic before CLD.
    put(0x20, &[0xF8, 0xEA, 0xD8]);
    // A multiply whose product is never read.
    put(0x23, &[0xA9, 0x02, 0x8D, 0x02, 0x42, 0x8D, 0x03, 0x42]);
    put(0x2B, &[0x80, 0xFE]);
    let mut vectors = [0x8040; 12];
    vectors[10] = 0x8000;
    put(0x40, &[0x40]);
    let rom = RomImage::from_bytes(
        fixtures::build_custom(
            romlens_core::MappingMode::LoRom,
            0x8000,
            false,
            &code,
            "MISSES",
            vectors,
        ),
        "m.sfc",
    )
    .unwrap();
    let x = build(&rom, &Project::new(&rom));
    let found: Vec<String> = x.idioms().iter().map(|i| i.title.clone()).collect();
    assert!(found.is_empty(), "{found:?}");
}

/// The C carries the same explanations as comments, and without them the
/// code is the same.
#[test]
fn the_c_says_the_same() {
    use romlens_core::decompile::{self, DecompileOptions};
    let rom = rom();
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let c = |explain| {
        decompile::decompile(
            &rom,
            &project,
            &snap,
            SnesAddress::new(0, 0x8000),
            &DecompileOptions {
                explain,
                ..Default::default()
            },
        )
        .unwrap()
    };
    let with = c(true);
    let without = c(false);
    assert!(
        with.text
            .contains("INIDISP = 0x8F; /* forced blank, brightness 15 */"),
        "{}",
        with.text
    );
    assert!(
        with.text.contains(
            "    /* ▸ Wait for vertical blank: Reads HVBJOY until bit 7 is set. */\n    do {"
        ),
        "{}",
        with.text
    );
    assert!(
        with.text
            .contains("/* ▸ DMA transfer: Channel 0 copies $0800 bytes")
    );
    // Taking the comments out gives the plain text back.
    let stripped: Vec<String> = with
        .text
        .lines()
        .filter(|l| !l.trim_start().starts_with("/* ▸"))
        .map(|l| match l.find(" /* ") {
            Some(i) if l.starts_with("    ") && !l.trim_start().starts_with("/*") => {
                l[..i].to_owned()
            }
            _ => l.to_owned(),
        })
        .collect();
    let plain: Vec<&str> = without.text.lines().collect();
    assert_eq!(stripped, plain);
    // The line map still lines up with the text, note lines mapping to
    // nothing.
    assert_eq!(with.lines.len(), with.text.lines().count());
    // Tokens stay inside the text and on character boundaries.
    for t in &with.tokens {
        let end = (t.start + t.len) as usize;
        assert!(end <= with.text.len());
        assert!(with.text.is_char_boundary(t.start as usize) && with.text.is_char_boundary(end));
    }
}

/// A DMA whose channel is set on two paths: what they set differently
/// says so, rather than guessing one.
#[test]
fn a_dma_set_up_on_two_paths() {
    let mut code = vec![0u8; 0x40];
    let mut put = |at: usize, b: &[u8]| code[at..at + b.len()].copy_from_slice(b);
    put(0x00, &[0x78, 0x18, 0xFB, 0x4B, 0xAB, 0xE2, 0x30]);
    put(0x07, &[0xA9, 0x01, 0x8D, 0x00, 0x43]); // DMAP0 = $01
    put(0x0C, &[0xA5, 0x10, 0xF0, 0x07]); // BEQ $8017
    put(0x10, &[0xA9, 0x18, 0x8D, 0x01, 0x43, 0x80, 0x05]); // BBAD0 = $18; BRA $801C
    put(0x17, &[0xA9, 0x22, 0x8D, 0x01, 0x43]); // BBAD0 = $22
    put(0x1C, &[0xA9, 0x01, 0x8D, 0x0B, 0x42, 0x80, 0xFE]); // MDMAEN = $01
    put(0x30, &[0x40]);
    let mut vectors = [0x8030; 12];
    vectors[10] = 0x8000;
    let rom = RomImage::from_bytes(
        fixtures::build_custom(
            romlens_core::MappingMode::LoRom,
            0x8000,
            false,
            &code,
            "PATHS",
            vectors,
        ),
        "p.sfc",
    )
    .unwrap();
    let x = build(&rom, &Project::new(&rom));
    let dma: Vec<&str> = x
        .idioms()
        .iter()
        .filter(|i| i.title == "DMA transfer")
        .map(|i| i.summary.as_str())
        .collect();
    assert_eq!(
        dma,
        [
            "Channel 0 copies bytes from an address set by the caller to a register set differently on each path here; the byte count is set by the caller."
        ]
    );
}

/// A write-only register stored with a copy in RAM names the copy; a read
/// back register, or a store with something else in between, does not.
#[test]
fn shadow_copies_of_registers() {
    let mut code = vec![0u8; 0x40];
    let mut put = |at: usize, b: &[u8]| code[at..at + b.len()].copy_from_slice(b);
    put(0x00, &[0x78, 0x18, 0xFB, 0x4B, 0xAB, 0xE2, 0x30]);
    put(0x07, &[0xA9, 0x17, 0x8D, 0x2C, 0x21, 0x8D, 0x69, 0x00]); // TM, then $0069
    put(0x0F, &[0x9C, 0x2D, 0x21, 0x9C, 0x6B, 0x00]); // TS = 0, then $006B
    put(0x15, &[0xA9, 0x01, 0x8D, 0x40, 0x21, 0x8D, 0x10, 0x00]); // APUIO0: not write-only
    put(
        0x1D,
        &[0xA9, 0x0F, 0x8D, 0x00, 0x21, 0x1A, 0x8D, 0x11, 0x00],
    ); // INC between
    put(0x26, &[0x80, 0xFE]);
    put(0x30, &[0x40]);
    let mut vectors = [0x8030; 12];
    vectors[10] = 0x8000;
    let rom = RomImage::from_bytes(
        fixtures::build_custom(
            romlens_core::MappingMode::LoRom,
            0x8000,
            false,
            &code,
            "SHADOW",
            vectors,
        ),
        "s.sfc",
    )
    .unwrap();
    let x = build(&rom, &Project::new(&rom));
    let shadows: Vec<_> = x
        .idioms()
        .iter()
        .filter(|i| i.kind == romlens_core::explain::IdiomKind::ShadowRegister)
        .collect();
    assert_eq!(shadows.len(), 1);
    assert_eq!(shadows[0].summary, "Keeps a RAM copy of 2 registers.");
    let t = shadows[0].table.as_ref().unwrap();
    let cells: Vec<&[String]> = t.rows.iter().map(|r| r.cells.as_slice()).collect();
    assert_eq!(
        cells,
        [
            &["TM".to_owned(), "$7E:0069".to_owned()][..],
            &["TS".to_owned(), "$7E:006B".to_owned()][..]
        ]
    );
    assert_eq!(t.note, None);
    // Each row names its two stores.
    assert_eq!(t.rows[0].offsets.len(), 2);
}
