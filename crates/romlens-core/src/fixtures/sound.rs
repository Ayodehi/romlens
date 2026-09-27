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

/// Audio RAM laid out as Nintendo's N-SPC driver keeps it, written by
/// hand from the format's description (`audio::nspc`): the older
/// version's length table at `$0FC2` (plus one, as Super Mario World keeps
/// it), a song table at `$1360` of two songs, and song 1 playing its
/// second block.
pub fn nspc_aram() -> Vec<u8> {
    use crate::audio::nspc::OLD_COMMANDS;
    let mut a = vec![0u8; 0x10000];
    let lengths: Vec<u8> = OLD_COMMANDS.iter().map(|c| c.params + 1).collect();
    a[0x0FC2..0x0FC2 + lengths.len()].copy_from_slice(&lengths);
    let put = |a: &mut Vec<u8>, at: usize, words: &[u16]| {
        for (i, w) in words.iter().enumerate() {
            a[at + i * 2..at + i * 2 + 2].copy_from_slice(&w.to_le_bytes());
        }
    };
    put(&mut a, 0x1360, &[0x1400, 0x1420]);
    // Song 1: block A, block B twice more, then back to the start for ever.
    put(
        &mut a,
        0x1400,
        &[0x1500, 0x1510, 0x0002, 0x1402, 0x00FF, 0x1400],
    );
    // Song 2: block A, then the end.
    put(&mut a, 0x1420, &[0x1500, 0x0000]);
    put(&mut a, 0x1500, &[0x2000, 0, 0, 0, 0, 0, 0, 0x2100]);
    put(&mut a, 0x1510, &[0x2200, 0, 0, 0, 0, 0, 0, 0]);
    let t1 = [
        0xDA, 0x04, // instrument 4
        0x18, 0x7F, // an eighth, quantize 7, velocity 15
        0xA4, // C3 ($80 + 36: C0 is $80)
        0xC6, // tie
        0x30, // a quarter
        0xC7, // rest
        0xE9, 0x00, 0x23, 0x02, // call $2300 twice
        0xD2, // percussion 2
        0x00,
    ];
    a[0x2000..0x2000 + t1.len()].copy_from_slice(&t1);
    a[0x2100..0x2103].copy_from_slice(&[0x0C, 0x80, 0x00]);
    a[0x2200..0x2203].copy_from_slice(&[0x30, 0xB9, 0x00]);
    a[0x2300..0x2302].copy_from_slice(&[0x80, 0x00]);
    // The driver in block B: the list pointer past it, voice 0 part way.
    put(&mut a, 0x40, &[0x1404]);
    put(&mut a, 0x30, &[0x2201]);
    a
}

/// Where [`sound_upload_banked_lorom`] keeps its pointer table: `$00:9000`.
pub const BANKED_TABLE: usize = 0x1000;

/// A LoROM image whose upload is shaped as Super Metroid's (docs/23, A15):
/// the list chosen from a table of three-byte pointers by index
/// (`LDA $00:9000,X` into `$00`, `LDA $00:9001,X` into `$01`), a routine
/// at `$8030` that takes the offset into Y and the bank into the data bank
/// (`LDY $00`, `LDA $02`, `PHA`, `PLB`) and calls one at `$8050` that
/// waits for `$BBAA`, kicks with `$CC` and reads `LDA $0000,Y`. The
/// table's first list, at `$01:FFF0`, runs across into bank `$02`; its
/// second is at `$00:A000`. A decoy at `$8070` points `$00–$02` at a
/// third list and calls nothing: it is not an upload. Written for static
/// tracing, not to run.
pub fn sound_upload_banked_lorom() -> Vec<u8> {
    #[rustfmt::skip]
    let code: Vec<u8> = [
        // $8000 reset
        &[0x78, 0x18, 0xFB, 0xC2, 0x30][..],  // SEI; CLC; XCE; REP #$30
        &[0xA2, 0x00, 0x00],                  // LDX #$0000
        &[0xBF, 0x00, 0x90, 0x00],            // LDA $00:9000,X
        &[0x85, 0x00],                        // STA $00
        &[0xBF, 0x01, 0x90, 0x00],            // LDA $00:9001,X
        &[0x85, 0x01],                        // STA $01
        &[0x20, 0x30, 0x80],                  // JSR $8030
        &[0x20, 0x70, 0x80],                  // JSR $8070 (the decoy)
        &[0x80, 0xFE],                        // BRA *
    ]
    .concat();
    #[rustfmt::skip]
    let routine: [u8; 11] = [
        0xE2, 0x20,       // SEP #$20
        0xA4, 0x00,       // LDY $00
        0xA5, 0x02,       // LDA $02
        0x48,             // PHA
        0xAB,             // PLB
        0x20, 0x50, 0x80, // JSR $8050
    ];
    #[rustfmt::skip]
    let send: [u8; 20] = [
        0xC2, 0x20,       // REP #$20
        0xA9, 0xAA, 0xBB, // LDA #$BBAA
        0xCD, 0x40, 0x21, // CMP $2140
        0xD0, 0xFB,       // BNE -5
        0xE2, 0x20,       // SEP #$20
        0xA9, 0xCC,       // LDA #$CC
        0x8D, 0x40, 0x21, // STA $2140
        0xB9, 0x00, 0x00, // LDA $0000,Y
    ];
    #[rustfmt::skip]
    let decoy: [u8; 12] = [
        0xA9, 0x00, 0xB0, // LDA #$B000
        0x85, 0x00,       // STA $00
        0xA9, 0x00, 0x00, // LDA #$0000
        0x85, 0x02,       // STA $02
        0x60, 0x00,       // RTS
    ];
    let mut rom = super::build_with_code(MappingMode::LoRom, 0x20000, false, &code, SOUND_TITLE);
    rom[0x0030..0x0030 + routine.len()].copy_from_slice(&routine);
    rom[0x003B] = 0x60; // RTS
    rom[0x0050..0x0050 + send.len()].copy_from_slice(&send);
    rom[0x0064] = 0x60; // RTS
    rom[0x0070..0x0070 + decoy.len()].copy_from_slice(&decoy);
    // The table: $01:FFF0, $00:A000, then nothing.
    rom[BANKED_TABLE..BANKED_TABLE + 6].copy_from_slice(&[0xF0, 0xFF, 0x01, 0x00, 0xA0, 0x00]);
    let list = |rom: &mut Vec<u8>, at: usize, aram: u16, bytes: &[u8], entry: u16| {
        let mut l = Vec::new();
        l.extend((bytes.len() as u16).to_le_bytes());
        l.extend(aram.to_le_bytes());
        l.extend_from_slice(bytes);
        l.extend([0, 0]);
        l.extend(entry.to_le_bytes());
        rom[at..at + l.len()].copy_from_slice(&l);
    };
    let across: Vec<u8> = (0..32u8).collect();
    list(&mut rom, 0xFFF0, 0x0300, &across, 0x0300);
    list(&mut rom, 0x2000, 0x0400, &[1, 2, 3, 4], 0x0300);
    list(&mut rom, 0x3000, 0x0500, &[9, 9], 0x0300);
    rom
}
