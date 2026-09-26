//! BRR decoding (docs/23, A2), each case worked by hand from the formulas
//! in fullsnes and anomie's S-DSP document.

use romlens_core::dsp::brr::{Header, History, decode_block, decode_sample, encode_block_plain};

fn block(header: u8, nibbles: [u8; 16]) -> [u8; 9] {
    let mut b = [0u8; 9];
    b[0] = header;
    for (i, n) in nibbles.iter().enumerate() {
        b[1 + i / 2] |= if i % 2 == 0 { n << 4 } else { n & 0xF };
    }
    b
}

fn first(header: u8, nibble: u8, p1: i32, p2: i32) -> romlens_core::dsp::brr::Step {
    let mut h = History { p1, p2 };
    let mut n = [0u8; 16];
    n[0] = nibble;
    decode_block(&block(header, n), &mut h).steps[0]
}

#[test]
fn the_header_splits_into_its_fields() {
    let h = Header::from_byte(0xB7);
    assert_eq!((h.shift, h.filter, h.loops, h.end), (11, 1, true, true));
    assert_eq!(h.byte(), 0xB7);
    assert_eq!(Header::from_byte(0xC8).filter, 2);
}

#[test]
fn the_shift_scales_each_nibble() {
    // Filter 0, shift 12: 7 → (7 << 12) >> 1 = 14336, played as 28672.
    let s = first(0xC0, 0x7, 0, 0);
    assert_eq!(
        (s.nibble, s.shifted, s.result, s.output),
        (7, 14336, 14336, 28672)
    );
    // -8 → -16384, the loudest negative: -32768.
    let s = first(0xC0, 0x8, 0, 0);
    assert_eq!((s.nibble, s.shifted, s.output), (-8, -16384, -32768));
    // Shift 0 halves: 5 → 2.
    assert_eq!(first(0x00, 0x5, 0, 0).shifted, 2);
    // Shifts 13-15 are not valid: 0, or -2048 for a negative nibble.
    assert_eq!(first(0xD0, 0x3, 0, 0).shifted, 0);
    assert_eq!(first(0xF0, 0xF, 0, 0).shifted, -2048);
}

#[test]
fn each_filter_predicts_from_the_last_two() {
    // Filter 1: 1000 + (-1000 >> 4) = 1000 - 63 = 937.
    assert_eq!(first(0x04, 0, 1000, 500).prediction, 937);
    // Filter 2: 2000 + (-3000 >> 5) - 500 + (500 >> 4) = 2000 - 94 - 500 + 31.
    assert_eq!(first(0x08, 0, 1000, 500).prediction, 1437);
    // Filter 3: 2000 + (-13000 >> 6) - 500 + (1500 >> 4) = 2000 - 204 - 500 + 93.
    assert_eq!(first(0x0C, 0, 1000, 500).prediction, 1389);
    // Filter 0 ignores them.
    assert_eq!(first(0x00, 0, 1000, 500).prediction, 0);
}

#[test]
fn the_history_carries_from_sample_to_sample() {
    // Filter 1 with nothing added decays: 1000, 937, 878, ...
    let mut h = History { p1: 1000, p2: 0 };
    let b = decode_block(&block(0x04, [0; 16]), &mut h);
    assert_eq!(b.steps[0].result, 937);
    assert_eq!(b.steps[1].p1, 937);
    assert_eq!(b.steps[1].p2, 1000);
    assert_eq!(b.steps[1].result, 878);
    assert_eq!(h.p1, b.steps[15].result);
    assert_eq!(h.p2, b.steps[14].result);
}

#[test]
fn a_sum_past_fifteen_bits_wraps_and_past_sixteen_clips() {
    // Filter 1, shift 12, nibble 7, p1 16000: 14336 + 15000 = 29336, which
    // fits 16 bits but not 15, so it wraps to -3432 and plays as -6864.
    let s = first(0xC4, 0x7, 16000, 0);
    assert_eq!(
        (s.prediction, s.clamped, s.result, s.output),
        (15000, 29336, -3432, -6864)
    );
    assert!(s.wrapped() && !s.clipped());
    // Filter 2 from the extremes: 14336 + 46590 = 60926, clamped to 32767,
    // then wrapped to -1.
    let s = first(0xC8, 0x7, 16383, -16384);
    assert_eq!((s.prediction, s.clamped, s.result), (46590, 32767, -1));
    assert!(s.clipped() && s.wrapped());
}

#[test]
fn a_sample_runs_to_its_end_block_and_marks_its_loop() {
    let mut mem = vec![0xAAu8; 5];
    mem.extend(block(0xB0, [1; 16]));
    mem.extend(block(0xB0, [2; 16]));
    mem.extend(block(0xB3, [3; 16])); // loop and end
    mem.extend(block(0xB0, [4; 16])); // not part of the sample
    let s = decode_sample(&mem, 5, Some(14), 100);
    assert_eq!(s.starts, [5, 14, 23]);
    assert_eq!(s.loop_block, Some(1));
    assert!(s.loops && !s.unterminated);
    assert_eq!(s.len(), 27);
    assert_eq!(s.samples().len(), 48);
    // Shift 11, filter 0: nibble n plays as n << 11.
    assert_eq!(s.samples()[0], 1 << 11);
    assert_eq!(s.samples()[47], 3 << 11);
    // No end flag before memory runs out.
    let s = decode_sample(&mem[..31], 5, None, 100);
    assert!(s.unterminated);
    assert_eq!(s.blocks.len(), 2);
}

#[test]
fn the_plain_encoder_round_trips() {
    let mut samples = [0i16; 16];
    for (i, s) in samples.iter_mut().enumerate() {
        *s = if i < 8 { 0x3000 } else { -0x3000 };
    }
    let b = encode_block_plain(&samples, 12, false, true);
    let mut h = History::default();
    assert_eq!(decode_block(&b, &mut h).samples(), samples);
}
