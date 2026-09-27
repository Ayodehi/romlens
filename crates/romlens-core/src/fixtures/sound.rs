//! A sound fixture of our own (docs/23): BRR samples made here, so tests
//! and goldens need no game's audio.

use crate::MappingMode;

/// Where [`sound_lorom`] puts [`brr_sample`].
pub const SAMPLE_OFFSET: usize = 0x4000;

pub const SOUND_TITLE: &str = "ROMLENS SOUND";

fn block(header: u8, nibbles: [i8; 16]) -> [u8; 9] {
    let mut b = [0u8; 9];
    b[0] = header;
    for (i, n) in nibbles.iter().enumerate() {
        let n = *n as u8 & 0xF;
        b[1 + i / 2] |= if i % 2 == 0 { n << 4 } else { n };
    }
    b
}

/// A four-block sample that shows each part of the format: a ramp with no
/// filter, a decay that filter 1 predicts, then a square wave whose two
/// blocks loop, the last with the loop and end flags.
pub fn brr_sample() -> Vec<u8> {
    let ramp: [i8; 16] = std::array::from_fn(|i| i as i8 - 8);
    let mut decay = [0i8; 16];
    decay[0] = 7;
    let square: [i8; 16] = std::array::from_fn(|i| if i < 8 { 5 } else { -5 });
    let mut out = Vec::new();
    out.extend(block(0xB0, ramp)); // shift 11, filter 0
    out.extend(block(0xA4, decay)); // shift 10, filter 1
    out.extend(block(0xC0, square)); // shift 12, filter 0: the loop point
    out.extend(block(0xC3, square)); // loop, end
    out
}

/// The loop point of [`brr_sample`], as a byte offset in it.
pub const SAMPLE_LOOP: usize = 18;

/// A LoROM image holding [`brr_sample`] at [`SAMPLE_OFFSET`].
pub fn sound_lorom() -> Vec<u8> {
    let mut rom = super::build_with_code(
        MappingMode::LoRom,
        0x8000,
        false,
        &super::BOOT_CODE,
        SOUND_TITLE,
    );
    let s = brr_sample();
    rom[SAMPLE_OFFSET..SAMPLE_OFFSET + s.len()].copy_from_slice(&s);
    rom
}

/// Where [`sound_upload_lorom`] keeps its block list: file `0x5000`,
/// `$00:D000`.
pub const UPLOAD_LIST: usize = 0x5000;

/// The 65816 side of [`sound_upload_lorom`], at `$00:8000`, written for
/// Romlens from the upload protocol (`apu::ipl`): reset points the long
/// pointer at `$00` to the block list and calls the upload routine at
/// `$8020`, which waits for `$BBAA`, sends each block (its address on
/// ports 2 and 3, a command on port 0, then each byte on port 1 with its
/// index on port 0, waiting for the echo), and ends with port 1 zero and
/// the entry.
#[rustfmt::skip]
pub const UPLOAD_CODE: [u8; 0x88] = [
    // $8000 reset
    0x78,             // SEI
    0x18,             // CLC
    0xFB,             // XCE          native
    0xC2, 0x20,       // REP #$20     16-bit A
    0xA9, 0x00, 0xD0, // LDA #$D000   the list
    0x85, 0x00,       // STA $00
    0xE2, 0x20,       // SEP #$20
    0xA9, 0x00,       // LDA #$00     bank 0
    0x85, 0x02,       // STA $02
    0x20, 0x20, 0x80, // JSR $8020
    0x80, 0xFE,       // BRA *        then wait for ever
    0xEA, 0xEA, 0xEA, 0xEA, 0xEA, 0xEA, 0xEA, 0xEA, 0xEA, 0xEA, 0xEA, // to $8020
    // $8020 upload
    0x08,             // PHP
    0xC2, 0x30,       // REP #$30     16-bit A, X, Y
    0xA0, 0x00, 0x00, // LDY #$0000
    0xA9, 0xAA, 0xBB, // LDA #$BBAA   the boot program's ready
    0xCD, 0x40, 0x21, // CMP $2140
    0xD0, 0xFB,       // BNE -3
    0xE2, 0x20,       // SEP #$20
    0xA9, 0xCC,       // LDA #$CC     the first command
    0x85, 0x04,       // STA $04
    // $8034 block: its length and address
    0xC2, 0x20,       // REP #$20
    0xB7, 0x00,       // LDA [$00],Y  length
    0xC8, 0xC8,       // INY; INY
    0xAA,             // TAX
    0xB7, 0x00,       // LDA [$00],Y  audio RAM address
    0xC8, 0xC8,       // INY; INY
    0x8D, 0x42, 0x21, // STA $2142    ports 2 and 3
    0xE2, 0x20,       // SEP #$20
    0xE0, 0x00, 0x00, // CPX #$0000
    0xF0, 0x30,       // BEQ $8079    length zero: the end
    0xA9, 0x01,       // LDA #$01
    0x8D, 0x41, 0x21, // STA $2141    port 1 non-zero: a block
    0xA5, 0x04,       // LDA $04
    0x8D, 0x40, 0x21, // STA $2140    the command
    0xCD, 0x40, 0x21, // CMP $2140
    0xD0, 0xFB,       // BNE -3       until it is echoed
    0x64, 0x05,       // STZ $05      index 0
    // $805A each byte
    0xB7, 0x00,       // LDA [$00],Y
    0xC8,             // INY
    0x8D, 0x41, 0x21, // STA $2141    the byte
    0xA5, 0x05,       // LDA $05
    0x8D, 0x40, 0x21, // STA $2140    its index
    0xCD, 0x40, 0x21, // CMP $2140
    0xD0, 0xFB,       // BNE -3       until it is echoed
    0xE6, 0x05,       // INC $05
    0xCA,             // DEX
    0xD0, 0xEB,       // BNE $805A
    // the next command: the index after the last, plus one, never zero
    0xA5, 0x05,       // LDA $05
    0x1A,             // INC A
    0xD0, 0x01,       // BNE +1
    0x1A,             // INC A
    0x85, 0x04,       // STA $04
    0x80, 0xBB,       // BRA $8034
    // $8079 the end: port 1 zero, the entry on ports 2 and 3
    0x9C, 0x41, 0x21, // STZ $2141
    0xA5, 0x04,       // LDA $04
    0x8D, 0x40, 0x21, // STA $2140
    0xCD, 0x40, 0x21, // CMP $2140
    0xD0, 0xFB,       // BNE -3
    0x28,             // PLP
    0x60,             // RTS
];

/// A LoROM image that uploads a sound driver of Romlens's own
/// (`recording::mesen::stream::encode::FIXTURE_PLAYER`), the fixture's
/// sample directory and [`brr_sample`] through the boot program, the way a
/// game does: [`UPLOAD_CODE`] at `$00:8000` and the block list at
/// [`UPLOAD_LIST`].
pub fn sound_upload_lorom() -> Vec<u8> {
    let mut rom =
        super::build_with_code(MappingMode::LoRom, 0x8000, false, &UPLOAD_CODE, SOUND_TITLE);
    let driver = crate::spc700::assemble(crate::recording::mesen::stream::encode::FIXTURE_PLAYER)
        .expect("the fixture driver assembles");
    let (at, code) = &driver.chunks[0];
    let mut list = Vec::new();
    let mut block = |aram: u16, bytes: &[u8]| {
        list.extend((bytes.len() as u16).to_le_bytes());
        list.extend(aram.to_le_bytes());
        list.extend_from_slice(bytes);
    };
    block(*at, code);
    block(0x3C00, &[0x00, 0x40, 0x12, 0x40]);
    block(0x4000, &brr_sample());
    list.extend([0, 0]);
    list.extend(at.to_le_bytes());
    rom[UPLOAD_LIST..UPLOAD_LIST + list.len()].copy_from_slice(&list);
    let s = brr_sample();
    rom[SAMPLE_OFFSET..SAMPLE_OFFSET + s.len()].copy_from_slice(&s);
    rom
}
