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
