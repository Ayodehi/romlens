//! The sound hardware's registers and the SPC700's code explained
//! (docs/23, A3). Each value is checked by hand against fullsnes's
//! register descriptions.

use romlens_core::explain::sound::{
    attack_ms, describe_dsp, describe_spc_io, dsp_register_name, dsp_register_named, pitch_words,
    sustain_ms, timer_period_ms,
};
use romlens_core::explain::spc::{SpcIdiomKind, explain_spc};
use romlens_core::spc700::{aram, assemble};

fn dsp(reg: u8, v: u8) -> String {
    describe_dsp(reg, Some(v)).parts[0].short()
}

fn io(a: u16, v: u8) -> String {
    describe_spc_io(a, Some(v)).unwrap().parts[0].short()
}

#[test]
fn dsp_registers_have_fullsnes_names() {
    assert_eq!(dsp_register_name(0x00), "V0VOLL");
    assert_eq!(dsp_register_name(0x42), "V4PITCHL");
    assert_eq!(dsp_register_name(0x75), "V7ADSR1");
    assert_eq!(dsp_register_name(0x4C), "KON");
    assert_eq!(dsp_register_name(0x5C), "KOFF");
    assert_eq!(dsp_register_name(0x7C), "ENDX");
    assert_eq!(dsp_register_name(0x2D), "PMON");
    assert_eq!(dsp_register_name(0x7D), "EDL");
    assert_eq!(dsp_register_name(0x3F), "FIR3");
    assert_eq!(dsp_register_name(0x1D), "$1D");
    assert_eq!(dsp_register_name(0x0A), "$0A");
    assert_eq!(dsp_register_named("kon"), Some(0x4C));
    assert_eq!(dsp_register_named("V2SRCN"), Some(0x24));
}

#[test]
fn global_dsp_writes_read_as_their_meaning() {
    assert_eq!(dsp(0x4C, 0x01), "KON = $01: voice 0");
    assert_eq!(dsp(0x4C, 0x81), "KON = $81: voices 0, 7");
    assert_eq!(dsp(0x5C, 0xFF), "KOFF = $FF: all voices");
    assert_eq!(dsp(0x5D, 0x3C), "DIR = $3C: sample directory at $3C00");
    assert_eq!(dsp(0x6D, 0xD0), "ESA = $D0: echo buffer at $D000");
    assert_eq!(dsp(0x7D, 0x05), "EDL = $05: 80 ms of echo, 10 KB of buffer");
    assert_eq!(
        dsp(0x7D, 0x00),
        "EDL = $00: 4 bytes of echo buffer, no delay"
    );
    // FLG's reset value: every voice off, muted, echo writes off.
    assert_eq!(
        dsp(0x6C, 0xE0),
        "FLG = $E0: soft reset: every voice off, envelopes 0, output muted, echo writes off, noise stopped"
    );
    // The listing's comment is capped.
    assert!(describe_dsp(0x6C, Some(0xE0)).short().ends_with('…'));
    assert_eq!(dsp(0x6C, 0x20), "FLG = $20: echo writes off, noise stopped");
    // Noise rates as the envelope's: 1 is a step every 2048 samples, 16 Hz.
    assert_eq!(
        dsp(0x6C, 0x01),
        "FLG = $01: echo writes on, noise clock 16 Hz"
    );
    assert_eq!(
        dsp(0x6C, 0x1F),
        "FLG = $1F: echo writes on, noise clock 32000 Hz"
    );
    assert_eq!(dsp(0x0D, 0x40), "EFB = $40: 64 (50%)");
    assert_eq!(
        dsp(0x2D, 0x06),
        "PMON = $06: voice 1 follows voice 0, voice 2 follows voice 1"
    );
    assert_eq!(dsp(0x7F, 0x7F), "FIR7 = $7F: 127");
    assert_eq!(dsp(0x0C, 0x80), "MVOLL = $80: -128 (-100%, phase inverted)");
}

#[test]
fn voice_writes_read_as_their_meaning() {
    assert_eq!(
        dsp(0x04, 0x03),
        "V0SRCN = $03: sample 3, the directory's entry at +$00C"
    );
    assert_eq!(dsp(0x20, 0x7F), "V2VOLL = $7F: 127 (99%)");
    // ADSR1: bit 7 on, decay 0, attack 15: at once.
    assert_eq!(
        dsp(0x05, 0x8F),
        "V0ADSR1 = $8F: ADSR, decay 0: halves in 326 ms, attack 15: full at once"
    );
    // Attack 0 is the slowest: 63 steps of 32, a step every 2048 samples.
    assert_eq!(attack_ms(0), 63.0 * 2048.0 / 32.0);
    assert_eq!(attack_ms(10), 63.0 * 20.0 / 32.0);
    assert_eq!(
        dsp(0x15, 0x0A),
        "V1ADSR1 = $0A: GAIN (the GAIN register sets it), decay 0: halves in 326 ms, attack 10: full in 39 ms"
    );
    // ADSR2: sustain level 7 (8/8), sustain rate 0 never falls.
    assert_eq!(
        dsp(0x06, 0xE0),
        "V0ADSR2 = $E0: sustain at 8/8, sustain rate 0: never steps"
    );
    assert_eq!(sustain_ms(0, 7), None);
    assert!(sustain_ms(31, 0).unwrap() < 10.0);
    // GAIN: direct, or a slope.
    assert_eq!(dsp(0x07, 0x7F), "V0GAIN = $7F: fixed level 127/127");
    assert_eq!(
        dsp(0x07, 0xDF),
        "V0GAIN = $DF: linear increase, rate 31: a step every sample"
    );
    assert_eq!(
        dsp(0x07, 0xA0),
        "V0GAIN = $A0: exponential decrease, rate 0: never steps"
    );
    assert_eq!(dsp(0x08, 0x40), "V0ENVX = $40: envelope 64/127");
    assert_eq!(dsp(0x09, 0xF0), "V0OUTX = $F0: sample high byte -16");
    // The pitch's bytes are data; the pair is worked out where both are known.
    assert_eq!(dsp(0x02, 0x00), "V0PITCHL = $00");
    assert_eq!(
        pitch_words(0x1000),
        "32000 Hz sample rate, ×1.000 (+0.00 semitones)"
    );
    assert_eq!(
        pitch_words(0x2000),
        "64000 Hz sample rate, ×2.000 (+12.00 semitones)"
    );
    assert_eq!(
        pitch_words(0x0800),
        "16000 Hz sample rate, ×0.500 (-12.00 semitones)"
    );
    assert_eq!(pitch_words(0), "stopped");
}

#[test]
fn spc700_io_writes_read_as_their_meaning() {
    // CONTROL's reset value maps the boot ROM and clears the ports.
    assert_eq!(
        io(0xF1, 0xB0),
        "CONTROL = $B0: boot ROM at $FFC0, clear ports 2 and 3, clear ports 0 and 1"
    );
    assert_eq!(io(0xF1, 0x01), "CONTROL = $01: RAM at $FFC0, timer 0 on");
    assert_eq!(
        io(0xF0, 0x0A),
        "TEST = $0A: 0 waits on I/O and ROM, 0 waits on RAM, timers run, RAM writable"
    );
    assert_eq!(io(0xF2, 0x4C), "DSPADDR = $4C: KON");
    assert_eq!(io(0xF2, 0xCC), "DSPADDR = $CC: KON, read only (a mirror)");
    assert_eq!(io(0xFA, 0x10), "T0DIV = $10: divide by 16");
    assert_eq!(io(0xFC, 0x00), "T2DIV = $00: divide by 256");
    assert_eq!(timer_period_ms(0, 16), 2.0);
    assert_eq!(timer_period_ms(2, 0), 4.0);
    assert_eq!(io(0xF4, 0x55), "CPUIO0 = $55");
    assert!(describe_spc_io(0xEF, None).is_none());
}

#[test]
fn the_pass_follows_the_register_and_the_value() {
    let program = assemble(
        "
        .org $0400
        start:  MOV T0DIV,#$10
                MOV CONTROL,#$01
                MOV DSPADDR,#$5D
        dir:    MOV DSPDATA,#$3C
                MOV A,#$4C
                MOV Y,#$01
        kon:    MOVW DSPADDR,YA
                MOV DSPADDR,#$6C
                MOV A,#$20
        flg:    MOV DSPDATA,A
                INC DSPADDR
        endx:   MOV DSPDATA,A
                CALL !sub
        lost:   MOV DSPDATA,A
        tick:   MOV A,T0OUT
                BEQ tick
        wait:   CMP A,CPUIO0
                BEQ wait
        echo:   MOV CPUIO0,A
                BRA tick
        sub:    RET
        ",
    )
    .unwrap();
    let image = program.image();
    let l = |n: &str| program.label(n).unwrap();
    let walk = aram::walk(&image, &[l("start")]);
    let e = explain_spc(&image, &walk);
    let short = |n: &str| e.writes.get(&l(n)).map(|w| w.write.short());
    assert_eq!(short("start").as_deref(), Some("T0DIV = $10: divide by 16"));
    assert_eq!(
        short("dir").as_deref(),
        Some("DIR = $3C: sample directory at $3C00")
    );
    assert!(e.writes[&l("dir")].dsp);
    assert_eq!(
        short("kon").as_deref(),
        Some("KON = $01: voice 0"),
        "MOVW $F2,YA"
    );
    assert_eq!(
        short("flg").as_deref(),
        Some("FLG = $20: echo writes off, noise stopped")
    );
    assert_eq!(
        short("endx").as_deref(),
        Some("ESA = $20: echo buffer at $2000"),
        "INC $F2 moves on a register"
    );
    // After a call nothing is known: the write is to DSPDATA, register unknown.
    assert_eq!(short("lost").as_deref(), Some("DSPDATA"));
    assert!(!e.writes[&l("lost")].dsp);
    assert_eq!(short("echo").as_deref(), Some("CPUIO0"));

    let timer = e.idiom_at(l("tick")).unwrap();
    assert_eq!(timer.kind, SpcIdiomKind::TimerWait);
    assert_eq!(timer.title, "Wait for timer 0");
    assert_eq!(
        timer.summary,
        "Reads T0OUT until timer 0 ticks, every 2.0 ms (T0DIV = 16)."
    );
    let port = e.idiom_at(l("wait")).unwrap();
    assert_eq!(port.kind, SpcIdiomKind::PortWait);
    assert_eq!(
        port.summary,
        "Reads CPUIO0 until the S-CPU writes a new byte to port 0."
    );
    assert_eq!(
        e.idioms.len(),
        2,
        "the loop back to tick is not short enough to be a wait"
    );
}
