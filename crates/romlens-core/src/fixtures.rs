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
    build_with_code(mode, len, fast_rom, &BOOT_CODE, FIXTURE_TITLE)
}

/// The default vector table: everything at the catch-all, RESET at `$8000`.
pub const DEFAULT_VECTORS: [u16; 12] = [
    CATCH_ALL,
    CATCH_ALL,
    CATCH_ALL,
    CATCH_ALL,
    CATCH_ALL,
    CATCH_ALL, // native COP BRK ABORT NMI (RESET) IRQ
    CATCH_ALL,
    CATCH_ALL,
    CATCH_ALL,
    CATCH_ALL,
    RESET_TARGET,
    CATCH_ALL, // emulation COP (BRK) ABORT NMI RESET IRQ
];

/// Build a fixture with `code` at `$00:8000` and the given title.
pub fn build_with_code(
    mode: MappingMode,
    len: usize,
    fast_rom: bool,
    code: &[u8],
    title_text: &str,
) -> Vec<u8> {
    build_custom(mode, len, fast_rom, code, title_text, DEFAULT_VECTORS)
}

/// Build a fixture with `code` at `$00:8000`, a title and a full vector
/// table (native COP BRK ABORT NMI RESET IRQ, then emulation likewise).
pub fn build_custom(
    mode: MappingMode,
    len: usize,
    fast_rom: bool,
    code: &[u8],
    title_text: &str,
    vectors: [u16; 12],
) -> Vec<u8> {
    let mut rom = vec![0x00u8; len];
    let boot = boot_file_offset(mode);
    rom[boot..boot + code.len()].copy_from_slice(code);

    let h = mode.header_offset().as_usize();
    let mut title = [b' '; TITLE_LEN];
    title[..title_text.len()].copy_from_slice(title_text.as_bytes());
    rom[h..h + TITLE_LEN].copy_from_slice(&title);
    rom[h + 0x15] = 0x20 | mode.map_mode_nibble() | if fast_rom { 0x10 } else { 0x00 };
    rom[h + 0x16] = 0x00; // ROM only
    rom[h + 0x17] = rom_size_code(len);
    rom[h + 0x18] = 0x00; // no RAM
    rom[h + 0x19] = 0x01; // USA
    rom[h + 0x1A] = 0x00;
    rom[h + 0x1B] = 0x00;

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

/// 32 KB LoROM holding the 256 opcodes in order at `$00:8000`, each followed
/// by operand bytes `12 34 56` truncated to its length under M = X = E = 0.
/// Drives the CLI `disasm-allopcodes` golden and the FFI smoke test.
pub fn all_opcodes_lorom() -> Vec<u8> {
    use crate::cpu65816::{FlagState, OPCODES};
    let flags = FlagState {
        m: false,
        x: false,
        e: false,
        ..FlagState::NATIVE_VECTOR
    };
    let mut code = Vec::with_capacity(256 * 4);
    for (op, info) in OPCODES.iter().enumerate() {
        code.push(op as u8);
        let n = info.mode.operand_len(flags) as usize;
        code.extend_from_slice(&[0x12, 0x34, 0x56][..n]);
    }
    build_with_code(MappingMode::LoRom, 0x8000, false, &code, ALL_OPCODES_TITLE)
}

pub const ALL_OPCODES_TITLE: &str = "ROMLENS OPCODES";

pub const DISPATCH_TITLE: &str = "ROMLENS DISPATCH";

/// 32 KB LoROM whose boot dispatches through two jump tables, so the resolver
/// and `romlens tables` have a golden that needs no commercial ROM
/// (`12-content-policy.md` rule 1).
///
/// ```text
/// $00:8000  18 FB E2 30   CLC; XCE; SEP #$30    ; native, 8-bit A and X
/// $00:8004  FC 20 80      JSR ($8020,X)         ; dispatch, eight entries
/// $00:8007  7C 60 80      JMP ($8060,X)         ; dispatch, two entries
/// $00:800A  60            RTS
/// $00:800E  40            RTI                   ; the catch-all vector target
/// $00:8020  eight entries over $8030 $8038 $8040 $8048
/// $00:8060  two entries   → $8030 $8038, then filler
/// $00:8030  A9 nn 60      LDA #nn; RTS          ; and at $8038 $8040 $8048
/// ```
///
/// The two tables stop for different reasons, which is the point: the first
/// runs up to `$00:8030`, the routine its own first entry names, while the
/// second is followed by zeroes, which are not addresses of anything.
pub fn dispatch_lorom() -> Vec<u8> {
    let mut code = vec![0u8; 0x80];
    let mut put = |at: usize, bytes: &[u8]| code[at..at + bytes.len()].copy_from_slice(bytes);
    put(0x00, &[0x18, 0xFB, 0xE2, 0x30]);
    put(0x04, &[0xFC, 0x20, 0x80]);
    put(0x07, &[0x7C, 0x60, 0x80]);
    put(0x0A, &[0x60]);
    put(0x0E, &[0x40]);
    let routines = [0x8030u16, 0x8038, 0x8040, 0x8048];
    for i in 0..8 {
        put(0x20 + i * 2, &routines[i % routines.len()].to_le_bytes());
    }
    for (i, target) in routines[..2].iter().enumerate() {
        put(0x60 + i * 2, &target.to_le_bytes());
    }
    for (i, at) in routines.iter().enumerate() {
        put(*at as usize - 0x8000, &[0xA9, i as u8 + 1, 0x60]);
    }
    build_with_code(MappingMode::LoRom, 0x8000, false, &code, DISPATCH_TITLE)
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
