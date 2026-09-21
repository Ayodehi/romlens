//! Hand-assembled homebrew ROMs used by the test suite, the CLI's `testrom`
//! command and the FFI's `make_test_rom`, so nothing needs a commercial ROM.
//!
//! Each fixture boots with the canonical prologue, forces a blank screen and
//! spins:
//!
//! ```text
//! $00:8000  78            SEI
//! $00:8001  18            CLC
//! $00:8002  FB            XCE          ; native mode
//! $00:8003  E2 30         SEP #$30     ; 8-bit A, X, Y
//! $00:8005  A9 80         LDA #$80
//! $00:8007  8D 00 21      STA $2100    ; INIDISP: force blank
//! $00:800A  80 FE         BRA $800A    ; spin
//! $00:800C  EA EA         NOP NOP
//! $00:800E  40            RTI          ; catch-all vector target
//! ```

use crate::memory::map::MappingMode;
use crate::rom::checksum::compute_checksum;
use crate::rom::header::TITLE_LEN;

pub const FIXTURE_TITLE: &str = "ROMLENS TEST";

/// Bytes of the boot routine, placed at `$00:8000`.
pub const BOOT_CODE: [u8; 15] = [
    0x78, 0x18, 0xFB, 0xE2, 0x30, 0xA9, 0x80, 0x8D, 0x00, 0x21, 0x80, 0xFE, 0xEA, 0xEA, 0x40,
];

const RESET_TARGET: u16 = 0x8000;
const CATCH_ALL: u16 = 0x800E;

/// Where `$00:8000` lives in the file for each mapping.
const fn boot_file_offset(mode: MappingMode) -> usize {
    match mode {
        MappingMode::LoRom => 0x0000,
        MappingMode::HiRom => 0x8000,
        MappingMode::ExHiRom => 0x40_8000,
    }
}

const fn rom_size_code(len: usize) -> u8 {
    // Smallest code whose 1 << (10 + code) covers len.
    let mut code = 0u8;
    while (1usize << (10 + code)) < len {
        code += 1;
    }
    code
}

/// Build a fixture of `len` bytes with the given mapping.
pub fn build(mode: MappingMode, len: usize, fast_rom: bool) -> Vec<u8> {
    let mut rom = vec![0x00u8; len];
    let boot = boot_file_offset(mode);
    rom[boot..boot + BOOT_CODE.len()].copy_from_slice(&BOOT_CODE);

    let h = mode.header_offset().as_usize();
    let mut title = [b' '; TITLE_LEN];
    title[..FIXTURE_TITLE.len()].copy_from_slice(FIXTURE_TITLE.as_bytes());
    rom[h..h + TITLE_LEN].copy_from_slice(&title);
    rom[h + 0x15] = 0x20 | mode.map_mode_nibble() | if fast_rom { 0x10 } else { 0x00 };
    rom[h + 0x16] = 0x00; // ROM only
    rom[h + 0x17] = rom_size_code(len);
    rom[h + 0x18] = 0x00; // no RAM
    rom[h + 0x19] = 0x01; // USA
    rom[h + 0x1A] = 0x00;
    rom[h + 0x1B] = 0x00;

    let vectors: [u16; 12] = [
        CATCH_ALL,
        CATCH_ALL,
        CATCH_ALL,
        CATCH_ALL,
        CATCH_ALL,
        CATCH_ALL, // native
        CATCH_ALL,
        CATCH_ALL,
        CATCH_ALL,
        CATCH_ALL,
        RESET_TARGET,
        CATCH_ALL, // emulation
    ];
    for (i, v) in vectors.iter().enumerate() {
        let at = h + 0x24 + i * 2 + if i >= 6 { 4 } else { 0 };
        rom[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }

    // Checksum: with complement = $0000 and checksum = $FFFF the four bytes
    // contribute the same as any valid pair, so one pass gives the answer.
    rom[h + 0x1C..h + 0x1E].copy_from_slice(&0x0000u16.to_le_bytes());
    rom[h + 0x1E..h + 0x20].copy_from_slice(&0xFFFFu16.to_le_bytes());
    let sum = compute_checksum(&rom);
    rom[h + 0x1C..h + 0x1E].copy_from_slice(&(!sum).to_le_bytes());
    rom[h + 0x1E..h + 0x20].copy_from_slice(&sum.to_le_bytes());
    rom
}

/// 32 KB LoROM, header at `0x7FC0`, map mode `$20`.
pub fn minimal_lorom() -> Vec<u8> {
    build(MappingMode::LoRom, 0x8000, false)
}

/// 64 KB HiROM, header at `0xFFC0`, map mode `$21`.
pub fn minimal_hirom() -> Vec<u8> {
    build(MappingMode::HiRom, 0x1_0000, false)
}

/// 4 MB + 64 KB ExHiROM, header at `0x40FFC0`, map mode `$25`.
pub fn minimal_exhirom() -> Vec<u8> {
    build(MappingMode::ExHiRom, 0x41_0000, false)
}

/// The fixture for a mapping, by name.
pub fn for_mapping(mode: MappingMode) -> Vec<u8> {
    match mode {
        MappingMode::LoRom => minimal_lorom(),
        MappingMode::HiRom => minimal_hirom(),
        MappingMode::ExHiRom => minimal_exhirom(),
    }
}

/// Conventional file name for a fixture.
pub fn file_name(mode: MappingMode) -> &'static str {
    match mode {
        MappingMode::LoRom => "romlens-test-lorom.sfc",
        MappingMode::HiRom => "romlens-test-hirom.sfc",
        MappingMode::ExHiRom => "romlens-test-exhirom.sfc",
    }
}
