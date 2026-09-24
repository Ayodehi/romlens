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

use crate::analysis::accuracy::TruthRange;
use crate::memory::map::MappingMode;
use crate::model::region::{BankRule, DataKind, RegionKind, TableElem};
use crate::rom::checksum::compute_checksum;
use crate::rom::header::TITLE_LEN;

pub mod graphics;

pub use graphics::{GRAPHICS_TITLE, graphics_lorom, truth_for_graphics};

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

pub const ROUTINES_TITLE: &str = "ROMLENS ROUTINES";

/// 32 KB LoROM whose boot calls small routines, one per shape the
/// decompiler has to recover (`docs/18-decompiler.md`). The goldens and the
/// semantic tests run on it.
///
/// ```text
/// $00:8000  SEI; CLC; XCE; SEP #$30   ; native, 8-bit A, X, Y
/// $00:8005  JSR $8020; JSR $8030
/// $00:800B  BRA $8010
/// $00:800E  RTI                       ; the catch-all vector target
/// $00:8010  JSR $8040; JSR $8050
/// $00:8016  BRA $8016                 ; spin
///
/// $00:8020  LDX #$0F                  ; clear $0200-$020F, a counted loop
/// $00:8022  STZ $0200,X
/// $00:8025  DEX
/// $00:8026  BPL $8022
/// $00:8028  RTS
///
/// $00:8030  REP #$20                  ; $14 = $10 + $12, 16-bit
/// $00:8032  LDA $10
/// $00:8034  CLC
/// $00:8035  ADC $12
/// $00:8037  STA $14
/// $00:8039  SEP #$20
/// $00:803B  RTS
///
/// $00:8040  LDA $20                   ; $22 = the larger of $20 and $21
/// $00:8042  CMP $21
/// $00:8044  BCS $8048
/// $00:8046  LDA $21
/// $00:8048  STA $22
/// $00:804A  RTS
///
/// $00:8050  PHX                       ; $0300 = bit X, from a table
/// $00:8051  LDA $8070,X
/// $00:8054  STA $0300
/// $00:8057  PLX
/// $00:8058  RTS
///
/// $00:8070  01 02 04 08 10 20 40 80
/// ```
pub fn routines_lorom() -> Vec<u8> {
    let mut code = vec![0u8; 0x78];
    let mut put = |at: usize, bytes: &[u8]| code[at..at + bytes.len()].copy_from_slice(bytes);
    put(0x00, &[0x78, 0x18, 0xFB, 0xE2, 0x30]);
    put(
        0x05,
        &[0x20, 0x20, 0x80, 0x20, 0x30, 0x80, 0x80, 0x03, 0xEA, 0x40],
    );
    put(0x10, &[0x20, 0x40, 0x80, 0x20, 0x50, 0x80, 0x80, 0xFE]);
    put(
        0x20,
        &[0xA2, 0x0F, 0x9E, 0x00, 0x02, 0xCA, 0x10, 0xFA, 0x60],
    );
    put(
        0x30,
        &[
            0xC2, 0x20, 0xA5, 0x10, 0x18, 0x65, 0x12, 0x85, 0x14, 0xE2, 0x20, 0x60,
        ],
    );
    put(
        0x40,
        &[
            0xA5, 0x20, 0xC5, 0x21, 0xB0, 0x02, 0xA5, 0x21, 0x85, 0x22, 0x60,
        ],
    );
    put(
        0x50,
        &[0xDA, 0xBD, 0x70, 0x80, 0x8D, 0x00, 0x03, 0xFA, 0x60],
    );
    put(0x70, &[0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80]);
    build_with_code(MappingMode::LoRom, 0x8000, false, &code, ROUTINES_TITLE)
}

pub const MIXED_DATA_TITLE: &str = "ROMLENS DATA";

/// 64 KB LoROM holding one recognisable block per data heuristic, so the
/// classifier and `romlens heuristics` have a golden that needs no commercial
/// ROM (`12-content-policy.md` rule 1).
///
/// | File offset | Contents |
/// |---|---|
/// | `0x1000` | 512 bytes of BGR15 colours: sixteen distinct, two rows |
/// | `0x1200` | printable ASCII, repeated to fill a window |
/// | `0x1400` | 128 in-bank 16-bit addresses |
/// | `0x1600` | 256 bytes from a linear congruential generator, the shape of compressed data |
///
/// Everything else is zero, which is what filler looks like.
pub fn mixed_data_lorom() -> Vec<u8> {
    let mut rom = build_with_code(
        MappingMode::LoRom,
        0x1_0000,
        false,
        &BOOT_CODE,
        MIXED_DATA_TITLE,
    );
    for i in 0..0x100usize {
        let colour = ((i as u16 % 16) * 0x0421 + 0x0400).to_le_bytes();
        rom[0x1000 + i * 2..0x1002 + i * 2].copy_from_slice(&colour);
    }
    let text = b"ROMLENS TEST STRING DATA FOR THE ASCII HEURISTIC TO FIND HERE OK ";
    for i in 0..4 {
        rom[0x1200 + i * text.len()..0x1200 + (i + 1) * text.len()].copy_from_slice(text);
    }
    for i in 0..128usize {
        let target = 0x8000u16 + (i as u16 * 8);
        rom[0x1400 + i * 2..0x1402 + i * 2].copy_from_slice(&target.to_le_bytes());
    }
    let mut state = 0x1234_5678u32;
    for b in &mut rom[0x1600..0x1700] {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *b = (state >> 24) as u8;
    }
    fix_checksum(&mut rom, MappingMode::LoRom);
    rom
}

/// Recompute the header checksum after writing a payload: it covers every
/// byte, so it is always the last thing a builder does.
fn fix_checksum(rom: &mut [u8], mode: MappingMode) {
    let h = mode.header_offset().as_usize();
    rom[h + 0x1C..h + 0x1E].copy_from_slice(&0x0000u16.to_le_bytes());
    rom[h + 0x1E..h + 0x20].copy_from_slice(&0xFFFFu16.to_le_bytes());
    let sum = crate::rom::checksum::compute_checksum(rom);
    rom[h + 0x1C..h + 0x1E].copy_from_slice(&(!sum).to_le_bytes());
    rom[h + 0x1E..h + 0x20].copy_from_slice(&sum.to_le_bytes());
}

/// What the builder knows it wrote into [`build`]'s image, as ground truth.
///
/// This is the only ground truth the repository can hold. Truth for a
/// commercial ROM is derived from that ROM and never ships
/// (`12-content-policy.md`); truth for a fixture we assembled ourselves is
/// ours to state, which is what lets CI print an accuracy number at all.
///
/// Only ranges the builder wrote on purpose are named. The filler between them
/// is left unlabelled and scores nothing either way: a byte nobody has an
/// opinion about is not evidence that the classifier is right or wrong.
pub fn truth_for(mode: MappingMode) -> Vec<TruthRange> {
    let boot = boot_file_offset(mode) as u32;
    let h = mode.header_offset().as_usize() as u32;
    vec![
        // SEI CLC XCE SEP LDA STA BRA: reached from RESET.
        TruthRange {
            start: boot,
            end: boot + 12,
            kind: RegionKind::Code,
        },
        // The two NOPs are filler nothing reaches; left unlabelled.
        TruthRange {
            start: boot + 14,
            end: boot + 15,
            kind: RegionKind::Code,
        },
        TruthRange {
            start: h,
            end: h + 0x40,
            kind: RegionKind::Data(DataKind::Struct),
        },
    ]
}

/// Ground truth for [`dispatch_lorom`]: the two dispatchers are code and the
/// two tables they read are data.
pub fn truth_for_dispatch() -> Vec<TruthRange> {
    vec![
        // CLC, XCE, SEP, then the two dispatchers. `JMP (abs,X)` does not fall
        // through, so nothing after $00:800A is reachable.
        TruthRange {
            start: 0x00,
            end: 0x0A,
            kind: RegionKind::Code,
        },
        TruthRange {
            start: 0x0E,
            end: 0x0F,
            kind: RegionKind::Code,
        },
        TruthRange {
            start: 0x20,
            end: 0x30,
            kind: RegionKind::Data(DataKind::Table {
                stride: 2,
                elem: TableElem::Code(BankRule::SameBank),
            }),
        },
        TruthRange {
            start: 0x60,
            end: 0x64,
            kind: RegionKind::Data(DataKind::Table {
                stride: 2,
                elem: TableElem::Code(BankRule::SameBank),
            }),
        },
        TruthRange {
            start: MappingMode::LoRom.header_offset().as_usize() as u32,
            end: MappingMode::LoRom.header_offset().as_usize() as u32 + 0x40,
            kind: RegionKind::Data(DataKind::Struct),
        },
    ]
}

/// Ground truth for [`mixed_data_lorom`], which is where the data heuristics
/// are actually measured: every block in it was written to be a specific kind.
pub fn truth_for_mixed_data() -> Vec<TruthRange> {
    let mut ranges = truth_for(MappingMode::LoRom);
    ranges.extend([
        TruthRange {
            start: 0x1000,
            end: 0x1200,
            kind: RegionKind::Data(DataKind::Palette),
        },
        TruthRange {
            start: 0x1200,
            end: 0x1300,
            kind: RegionKind::Data(DataKind::String),
        },
        TruthRange {
            start: 0x1400,
            end: 0x1500,
            kind: RegionKind::Data(DataKind::Pointer {
                bank: BankRule::SameBank,
            }),
        },
        TruthRange {
            start: 0x1600,
            end: 0x1700,
            kind: RegionKind::Data(DataKind::Compressed),
        },
    ]);
    ranges.sort_by_key(|r| r.start);
    ranges
}

/// The built-in ground truth for a fixture, chosen by its title.
///
/// `None` for anything else, including the all-opcodes ROM: that one holds
/// every opcode in order with no control flow, so there is no honest answer to
/// what is code in it. A caller must not fall back to a *different* fixture's
/// truth — scoring against the wrong answers is worse than not scoring.
pub fn truth_for_title(title: &str, mode: MappingMode) -> Option<Vec<TruthRange>> {
    match title.trim() {
        MIXED_DATA_TITLE => Some(truth_for_mixed_data()),
        GRAPHICS_TITLE => Some(truth_for_graphics()),
        DISPATCH_TITLE => Some(truth_for_dispatch()),
        FIXTURE_TITLE => Some(truth_for(mode)),
        _ => None,
    }
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
