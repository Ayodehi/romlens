//! The S-DSP (docs/23, A8), each case worked by hand from fullsnes, anomie's
//! S-DSP document and the SNESdev wiki's envelope page.

use romlens_core::dsp::chip::{KEY_ON_DELAY, counter_after};
use romlens_core::dsp::{Dsp, EnvelopeMode};

/// Registers and audio RAM with one sample: at `$1000`, one block that
/// holds 16 samples of +7 at shift 12 and filter 0 (each (7 << 12) >> 1 =
/// $3800), looping to itself when `loops`, else ending there; entry 0 of
/// the directory at `$0200` points at it.
fn machine(loops: bool) -> ([u8; 128], Vec<u8>) {
    let mut aram = vec![0u8; 0x10000];
    aram[0x0200..0x0204].copy_from_slice(&[0x00, 0x10, 0x00, 0x10]);
    aram[0x1000] = 0xC0 | if loops { 0x03 } else { 0x01 };
    aram[0x1001..0x1009].fill(0x77);
    let mut regs = [0u8; 128];
    regs[0x5D] = 0x02; // DIR
    regs[0x6C] = 0x20; // FLG: echo writes off
    regs[0x02] = 0x00; // V0 pitch $1000
    regs[0x03] = 0x10;
    (regs, aram)
}

/// Key voice 0 on and run through the poll and the wait, so the next
/// sample is its first.
fn keyed(dsp: &mut Dsp, regs: &mut [u8; 128], aram: &mut [u8]) {
    dsp.write_kon(0x01);
    // KON is taken on every other sample: within two.
    let mut n = 0;
    while dsp.voices[0].delay == 0 {
        dsp.sample(regs, aram);
        n += 1;
        assert!(n <= 2, "KON not taken");
    }
    assert_eq!(dsp.voices[0].delay, KEY_ON_DELAY);
    while dsp.voices[0].delay > 0 {
        dsp.sample(regs, aram);
    }
}

#[test]
fn the_global_counter_counts_down_from_7800() {
    assert_eq!(counter_after(0), 0);
    assert_eq!(counter_after(1), 0x77FF);
    assert_eq!(counter_after(0x7800), 0);
    assert_eq!(counter_after(0x7801), 0x77FF);
}

#[test]
fn attack_15_adds_1024_a_sample_then_decays() {
    let (mut regs, mut aram) = machine(true);
    regs[0x05] = 0x8F; // ADSR, attack 15, decay 0
    regs[0x06] = 0xE0; // sustain level 7
    let mut dsp = Dsp::default();
    keyed(&mut dsp, &mut regs, &mut aram);
    assert_eq!(dsp.voices[0].mode, EnvelopeMode::Attack);
    // Each sample goes out at the level so far, then the envelope steps.
    dsp.sample(&mut regs, &mut aram);
    assert_eq!((regs[0x08], dsp.voices[0].envelope), (0x00, 1024));
    dsp.sample(&mut regs, &mut aram);
    assert_eq!(regs[0x08], 0x40, "1024 >> 4");
    // 2048 is past 11 bits: clipped to $7FF, and the attack is over.
    assert_eq!(dsp.voices[0].envelope, 0x7FF);
    assert_eq!(dsp.voices[0].mode, EnvelopeMode::Decay);
}

#[test]
fn direct_gain_sets_the_level_and_release_takes_8_a_sample() {
    let (mut regs, mut aram) = machine(true);
    regs[0x07] = 0x40; // direct gain: $40 << 4 = $400
    let mut dsp = Dsp::default();
    keyed(&mut dsp, &mut regs, &mut aram);
    dsp.sample(&mut regs, &mut aram);
    assert_eq!(dsp.voices[0].envelope, 0x400);
    // Key off: polled on every other sample, then down by 8 each.
    regs[0x5C] = 0x01;
    while dsp.voices[0].mode != EnvelopeMode::Release {
        dsp.sample(&mut regs, &mut aram);
    }
    let from = dsp.voices[0].envelope;
    dsp.sample(&mut regs, &mut aram);
    dsp.sample(&mut regs, &mut aram);
    assert_eq!(from - dsp.voices[0].envelope, 16);
}

#[test]
fn linear_gain_steps_at_its_rate() {
    // Linear increase at rate 31, every sample: +32 each.
    let (mut regs, mut aram) = machine(true);
    regs[0x07] = 0xDF;
    let mut dsp = Dsp::default();
    keyed(&mut dsp, &mut regs, &mut aram);
    for _ in 0..4 {
        dsp.sample(&mut regs, &mut aram);
    }
    assert_eq!(dsp.voices[0].envelope, 4 * 32);
    // Rate 0 never steps.
    regs[0x07] = 0xC0;
    dsp.sample(&mut regs, &mut aram);
    assert_eq!(dsp.voices[0].envelope, 4 * 32);
}

#[test]
fn the_pitch_counter_takes_one_sample_a_step_at_1000() {
    let (mut regs, mut aram) = machine(true);
    regs[0x07] = 0x7F;
    let mut dsp = Dsp::default();
    keyed(&mut dsp, &mut regs, &mut aram);
    // Four samples go in at the start.
    assert_eq!(dsp.voices[0].nibble, 4);
    dsp.sample(&mut regs, &mut aram);
    assert_eq!(dsp.voices[0].nibble, 5);
    // Twice as fast: two a sample.
    regs[0x03] = 0x20;
    dsp.sample(&mut regs, &mut aram);
    assert_eq!(dsp.voices[0].nibble, 7);
    // Half as fast: one every other sample, the fraction carrying.
    regs[0x03] = 0x08;
    dsp.sample(&mut regs, &mut aram);
    dsp.sample(&mut regs, &mut aram);
    assert_eq!(dsp.voices[0].nibble, 8);
}

#[test]
fn a_flat_sample_comes_out_at_its_level() {
    // Four samples of $3800 interpolate to $3800 at any point, give or
    // take the table's rounding (its four weights sum to $7FF-$801); at
    // full envelope ($7F0 by direct gain) OUTX is its top byte.
    let (mut regs, mut aram) = machine(true);
    regs[0x07] = 0x7F;
    let mut dsp = Dsp::default();
    keyed(&mut dsp, &mut regs, &mut aram);
    for _ in 0..3 {
        dsp.sample(&mut regs, &mut aram);
    }
    let out = dsp.voices[0].output;
    let want = (0x3800 * 0x7F0) >> 11;
    assert!((out - want).abs() <= 16, "{out:#x} against {want:#x}");
    assert_eq!(regs[0x09], (out >> 7) as u8);
}

#[test]
fn an_end_block_sets_endx_and_one_that_does_not_loop_ends_the_note() {
    let (mut regs, mut aram) = machine(false);
    regs[0x07] = 0x7F;
    let mut dsp = Dsp::default();
    dsp.write_kon(0x01);
    for _ in 0..3 {
        dsp.sample(&mut regs, &mut aram);
    }
    // Keying on clears ENDX; the block's start sets it again.
    while dsp.voices[0].delay > 0 {
        dsp.sample(&mut regs, &mut aram);
    }
    assert_eq!(regs[0x7C] & 1, 1);
    assert_eq!(dsp.voices[0].mode, EnvelopeMode::Release);
    assert_eq!(dsp.voices[0].envelope, 0);

    // A looping one keeps playing and goes back to its loop point.
    let (mut regs, mut aram) = machine(true);
    regs[0x07] = 0x7F;
    let mut dsp = Dsp::default();
    keyed(&mut dsp, &mut regs, &mut aram);
    for _ in 0..13 {
        dsp.sample(&mut regs, &mut aram);
    }
    assert_eq!(regs[0x7C] & 1, 1);
    assert_ne!(dsp.voices[0].mode, EnvelopeMode::Release);
    assert_eq!(dsp.voices[0].block, 0x1000);
}

#[test]
fn the_noise_shifts_in_the_xor_of_its_low_bits() {
    let (mut regs, mut aram) = machine(true);
    regs[0x6C] = 0x3F; // noise at rate 31: every sample
    let mut dsp = Dsp::default();
    assert_eq!(dsp.noise, 0x4000);
    dsp.sample(&mut regs, &mut aram);
    // $4000: bits 0 and 1 are clear, so a 0 comes in at the top.
    assert_eq!(dsp.noise, 0x2000);
    dsp.noise = 0x0001;
    dsp.sample(&mut regs, &mut aram);
    assert_eq!(dsp.noise, 0x4000);
}

#[test]
fn volumes_scale_the_voice_into_the_mix() {
    // Voice 0 at +64 each side, master at +64: a quarter of the voice
    // ((s × 64) >> 6, then (× 64) >> 7 is half).
    let (mut regs, mut aram) = machine(true);
    regs[0x07] = 0x7F;
    regs[0x00] = 0x40;
    regs[0x01] = 0xC0; // -64: the right side's phase inverted
    regs[0x0C] = 0x40;
    regs[0x1C] = 0x40;
    let mut dsp = Dsp::default();
    keyed(&mut dsp, &mut regs, &mut aram);
    dsp.sample(&mut regs, &mut aram);
    dsp.sample(&mut regs, &mut aram);
    let f = dsp.sample(&mut regs, &mut aram);
    let s = f.voices[0] as i32;
    assert_eq!(f.left as i32, (((s * 64) >> 6) * 64) >> 7);
    assert_eq!(f.right as i32, (((s * -64) >> 6) * 64) >> 7);
    // Muted by FLG, nothing comes out, and the voice still runs.
    regs[0x6C] |= 0x40;
    let f = dsp.sample(&mut regs, &mut aram);
    assert_eq!((f.left, f.right), (0, 0));
    assert_ne!(f.voices[0], 0);
}

#[test]
fn echo_goes_into_the_buffer_and_comes_back_through_the_fir() {
    // A 4-byte buffer (EDL 0) at $8000, FIR7 (the newest tap) at $40 and
    // the rest zero, echo volume +127 each side, writes on.
    let (mut regs, mut aram) = machine(true);
    regs[0x07] = 0x7F;
    regs[0x00] = 0x7F;
    regs[0x01] = 0x7F;
    regs[0x4D] = 0x01; // EON: voice 0
    regs[0x6D] = 0x80; // ESA
    regs[0x7D] = 0x00; // EDL
    regs[0x7F] = 0x40; // FIR7
    regs[0x2C] = 0x7F;
    regs[0x3C] = 0x7F;
    regs[0x6C] = 0x00;
    let mut dsp = Dsp::default();
    keyed(&mut dsp, &mut regs, &mut aram);
    let f = dsp.sample(&mut regs, &mut aram);
    let f = if f.voices[0] == 0 {
        dsp.sample(&mut regs, &mut aram)
    } else {
        f
    };
    let s = f.voices[0] as i32;
    let written = i16::from_le_bytes([aram[0x8000], aram[0x8001]]) as i32;
    assert_eq!(
        written,
        ((s * 0x7F) >> 6) & !1,
        "the voice's echo part, bit 0 clear"
    );
    // The next sample reads it back: halved into the FIR, × $40 >> 6.
    let g = dsp.sample(&mut regs, &mut aram);
    let fir = ((written >> 1) * 0x40) >> 6;
    assert_eq!(
        g.left as i32,
        (fir * 0x7F) >> 7,
        "master volume 0: only the echo"
    );
}
