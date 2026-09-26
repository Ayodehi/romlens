//! What the sound hardware's registers mean (docs/23, A3): the S-DSP's 128
//! registers and the SPC700's I/O registers at `$F0–$FF`, in the same shape
//! as the 65816's hardware registers ([`super::fields`]).
//!
//! The text is our own. Every field and every rate was checked on
//! 26 September 2026 against fullsnes ("SNES APU DSP BRR Samples", "BRR
//! Pitch", "ADSR/Gain Envelope", "Volume Registers", "Control Registers",
//! "Echo Registers", and "SPC700 I/O Ports"). The envelope times are worked
//! out from its step rules, not quoted.

use super::fields::{
    Field, Layout, Part, RegisterWrite, data, fields_part, flag, flag2, number, settings,
};

/// Samples at 32 kHz between envelope (or noise) steps, for each 5-bit
/// rate; rate 0 never steps (fullsnes, "ADSR/Gain (and Noise) Rates").
pub const RATE_PERIODS: [u32; 32] = [
    0, 2048, 1536, 1280, 1024, 768, 640, 512, 384, 320, 256, 192, 160, 128, 96, 80, 64, 48, 40, 32,
    24, 20, 16, 12, 10, 8, 6, 5, 4, 3, 2, 1,
];

/// The envelope's full level: 11 bits.
pub const ENVELOPE_MAX: u32 = 0x7FF;

/// The level at which the attack ends and the decay begins.
pub const ATTACK_END: u32 = 0x7E0;

/// Milliseconds as a person says them.
pub fn duration(ms: f64) -> String {
    if ms < 1.0 {
        "at once".to_owned()
    } else if ms < 10.0 {
        format!("{ms:.1} ms")
    } else if ms < 1000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}

fn steps_ms(steps: u32, rate: u32) -> Option<f64> {
    let period = RATE_PERIODS[rate as usize & 31];
    (period != 0).then(|| steps as f64 * period as f64 / 32.0)
}

/// One exponential step, as decay, sustain and GAIN mode 1 take it.
pub const fn exp_step(level: u32) -> u32 {
    level - (((level as i32 - 1) >> 8) + 1) as u32
}

/// Exponential steps from `from` until the level is at or below `to`.
fn exp_steps(from: u32, to: u32) -> u32 {
    let mut level = from;
    let mut n = 0;
    while level > to && level > 0 {
        level = exp_step(level);
        n += 1;
    }
    n
}

/// How long an ADSR attack at `a` (0–15) takes from silence to full.
pub fn attack_ms(a: u32) -> f64 {
    let rate = a * 2 + 1;
    let step = if rate == 31 { 1024 } else { 32 };
    let steps = ATTACK_END.div_ceil(step);
    steps_ms(steps, rate).unwrap_or(0.0)
}

/// How long a decay at `d` (0–7) takes from full to sustain level `sl`.
pub fn decay_ms(d: u32, sl: u32) -> f64 {
    steps_ms(exp_steps(ENVELOPE_MAX, (sl + 1) * 0x100), d * 2 + 16).unwrap_or(0.0)
}

/// How long a sustain at rate `sr` takes from level `sl` to silence, or
/// `None` when rate 0 holds it.
pub fn sustain_ms(sr: u32, sl: u32) -> Option<f64> {
    let from = ((sl + 1) * 0x100).min(ENVELOPE_MAX);
    steps_ms(exp_steps(from, 0), sr)
}

/// The rate a 5-bit rate gives, in words.
fn rate_words(rate: u32) -> String {
    match RATE_PERIODS[rate as usize & 31] {
        0 => "never steps".to_owned(),
        1 => "a step every sample".to_owned(),
        p => format!("a step every {p} samples"),
    }
}

/// What pitch `p` (14 bits) does: `$1000` plays a sample at 32 kHz, its
/// own rate; each doubling is an octave up.
pub fn pitch_words(p: u32) -> String {
    let p = p & 0x3FFF;
    if p == 0 {
        return "stopped".to_owned();
    }
    let semis = 12.0 * (p as f64 / 4096.0).log2();
    format!(
        "{} Hz sample rate, ×{:.3} ({}{:.2} semitones)",
        32000 * p / 4096,
        p as f64 / 4096.0,
        if semis >= 0.0 { "+" } else { "" },
        semis
    )
}

// ---- numbers in words ----

fn signed(v: u32) -> i32 {
    v as u8 as i8 as i32
}

fn volume(v: u32) -> String {
    let s = signed(v);
    let pct = s as f64 * 100.0 / 128.0;
    if s < 0 {
        format!("{s} ({pct:.0}%, phase inverted)")
    } else {
        format!("{s} ({pct:.0}%)")
    }
}

fn voices(v: u32) -> String {
    let list: Vec<String> = (0..8)
        .filter(|i| v & (1 << i) != 0)
        .map(|i| i.to_string())
        .collect();
    match list.len() {
        0 => "no voices".to_owned(),
        1 => format!("voice {}", list[0]),
        8 => "all voices".to_owned(),
        _ => format!("voices {}", list.join(", ")),
    }
}

fn modulated(v: u32) -> String {
    let v = v & 0xFE;
    if v == 0 {
        return "no voices".to_owned();
    }
    let list: Vec<String> = (1..8)
        .filter(|i| v & (1 << i) != 0)
        .map(|i| format!("voice {i} follows voice {}", i - 1))
        .collect();
    list.join(", ")
}

fn dir_page(v: u32) -> String {
    format!("sample directory at ${:04X}", v << 8)
}

fn esa_page(v: u32) -> String {
    format!("echo buffer at ${:04X}", v << 8)
}

fn pmon(v: u32) -> String {
    modulated(v << 1)
}

fn edl(v: u32) -> String {
    let v = v & 0xF;
    if v == 0 {
        "4 bytes of echo buffer, no delay".to_owned()
    } else {
        format!("{} ms of echo, {} KB of buffer", v * 16, v * 2)
    }
}

fn source(v: u32) -> String {
    format!("sample {v}, the directory's entry at +${:03X}", v * 4)
}

fn noise(v: u32) -> String {
    match RATE_PERIODS[v as usize & 31] {
        0 => "noise stopped".to_owned(),
        p => format!("noise clock {:.0} Hz", 32000.0 / p as f64),
    }
}

fn attack(v: u32) -> String {
    let ms = attack_ms(v);
    if ms < 1.0 {
        format!("attack {v}: full at once")
    } else {
        format!("attack {v}: full in {}", duration(ms))
    }
}

fn decay(v: u32) -> String {
    let half = steps_ms(exp_steps(ENVELOPE_MAX, ENVELOPE_MAX / 2), v * 2 + 16).unwrap_or(0.0);
    format!("decay {v}: halves in {}", duration(half))
}

fn sustain_level(v: u32) -> String {
    format!("sustain at {}/8", v + 1)
}

fn sustain_rate(v: u32) -> String {
    format!("sustain rate {v}: {}", rate_words(v))
}

fn gain(v: u32) -> String {
    if v & 0x80 == 0 {
        return format!("fixed level {}/127", v & 0x7F);
    }
    let rate = v & 0x1F;
    let mode = [
        "linear decrease",
        "exponential decrease",
        "linear increase",
        "bent increase",
    ][(v >> 5) as usize & 3];
    format!("{mode}, rate {rate}: {}", rate_words(rate))
}

fn envx(v: u32) -> String {
    format!("envelope {}/127", v & 0x7F)
}

fn outx(v: u32) -> String {
    format!("sample high byte {}", signed(v))
}

fn coefficient(v: u32) -> String {
    format!("{}", signed(v))
}

fn divider(v: u32) -> String {
    format!("divide by {}", if v == 0 { 256 } else { v })
}

fn io_waits(v: u32) -> String {
    format!("{} waits on I/O and ROM", [0, 1, 4, 9][v as usize & 3])
}

fn ram_waits(v: u32) -> String {
    format!("{} waits on RAM", [0, 1, 4, 9][v as usize & 3])
}

fn counter(v: u32) -> String {
    format!("{} ticks since the last read", v & 0xF)
}

fn dsp_index(v: u32) -> String {
    let name = dsp_register_name((v & 0x7F) as u8);
    if v & 0x80 != 0 {
        format!("{name}, read only (a mirror)")
    } else {
        name
    }
}

// ---- the DSP ----

/// The voice registers' names, by their low nibble.
const VOICE_NAMES: [&str; 10] = [
    "VOLL", "VOLR", "PITCHL", "PITCHH", "SRCN", "ADSR1", "ADSR2", "GAIN", "ENVX", "OUTX",
];

/// A DSP register's name: `V3PITCHL`, `KON`, `FIR2`, or `$1A` when unused.
pub fn dsp_register_name(reg: u8) -> String {
    let (hi, lo) = (reg >> 4 & 7, reg & 0xF);
    match lo {
        0..=9 => format!("V{hi}{}", VOICE_NAMES[lo as usize]),
        0xC => [
            "MVOLL", "MVOLR", "EVOLL", "EVOLR", "KON", "KOFF", "FLG", "ENDX",
        ][hi as usize]
            .to_owned(),
        0xD => match hi {
            0 => "EFB".to_owned(),
            2 => "PMON".to_owned(),
            3 => "NON".to_owned(),
            4 => "EON".to_owned(),
            5 => "DIR".to_owned(),
            6 => "ESA".to_owned(),
            7 => "EDL".to_owned(),
            _ => format!("${reg:02X}"),
        },
        0xF => format!("FIR{hi}"),
        _ => format!("${reg:02X}"),
    }
}

/// The DSP register that `name` names, for the assembler and the CLI.
pub fn dsp_register_named(name: &str) -> Option<u8> {
    (0..0x80u8).find(|r| dsp_register_name(*r).eq_ignore_ascii_case(name))
}

static VOL: [Field; 1] = [number(7, 0, "volume", volume)];
static SRCN: [Field; 1] = [number(7, 0, "sample", source)];
static ADSR1: [Field; 3] = [
    flag2(7, "envelope", "ADSR", "GAIN (the GAIN register sets it)"),
    number(6, 4, "decay", decay),
    number(3, 0, "attack", attack),
];
static ADSR2: [Field; 2] = [
    number(7, 5, "sustain level", sustain_level),
    number(4, 0, "sustain rate", sustain_rate),
];
static GAIN: [Field; 1] = [number(7, 0, "gain", gain)];
static ENVX: [Field; 1] = [number(6, 0, "envelope", envx)];
static OUTX: [Field; 1] = [number(7, 0, "sample", outx)];
static VOICE_FLAGS: [Field; 1] = [number(7, 0, "voices", voices)];
static PMON: [Field; 1] = [number(7, 1, "voices", pmon)];
static FLG: [Field; 4] = [
    flag(
        7,
        "reset",
        "soft reset: every voice off, envelopes 0",
        "no reset",
    ),
    flag(6, "mute", "output muted", "not muted"),
    flag2(5, "echo writes", "echo writes off", "echo writes on"),
    number(4, 0, "noise", noise),
];
static EFB: [Field; 1] = [number(7, 0, "feedback", volume)];
static DIR: [Field; 1] = [number(7, 0, "directory", dir_page)];
static ESA: [Field; 1] = [number(7, 0, "buffer", esa_page)];
static EDL: [Field; 1] = [number(3, 0, "delay", edl)];
static FIR: [Field; 1] = [number(7, 0, "coefficient", coefficient)];

/// The layout of DSP register `reg` (`$00–$7F`).
pub fn dsp_layout(reg: u8) -> Layout {
    let reg = reg & 0x7F;
    let a = reg as u16;
    match (reg >> 4, reg & 0xF) {
        (_, 0) => settings(
            a,
            "The voice's left volume; negative inverts the wave.",
            &VOL,
        ),
        (_, 1) => settings(
            a,
            "The voice's right volume; negative inverts the wave.",
            &VOL,
        ),
        (_, 2) => data(
            a,
            "The pitch's low byte. With PITCHH, 14 bits: $1000 plays the sample at 32 kHz, its own rate; $2000 an octave up.",
        ),
        (_, 3) => data(
            a,
            "The pitch's high six bits. With PITCHL, 14 bits: $1000 plays the sample at 32 kHz, its own rate; $2000 an octave up.",
        ),
        (_, 4) => settings(
            a,
            "Which sample the voice plays: an entry of the sample directory, read when the voice is keyed on or loops.",
            &SRCN,
        ),
        (_, 5) => settings(
            a,
            "The envelope's shape: ADSR or GAIN, and the attack and decay rates. Key on starts the attack from silence.",
            &ADSR1,
        ),
        (_, 6) => settings(
            a,
            "The level the decay falls to, and how fast the sustain then fades.",
            &ADSR2,
        ),
        (_, 7) => settings(
            a,
            "The envelope when ADSR1 picks GAIN: a fixed level, or a rising or falling slope.",
            &GAIN,
        ),
        (_, 8) => settings(
            a,
            "The envelope's level now; the DSP rewrites it every sample.",
            &ENVX,
        ),
        (_, 9) => settings(
            a,
            "The voice's sample now, after its envelope and before its volume.",
            &OUTX,
        ),
        (0 | 1, 0xC) => settings(a, "The master volume, over every voice.", &VOL),
        (2 | 3, 0xC) => settings(a, "The echo's volume in the output.", &VOL),
        (4, 0xC) => settings(
            a,
            "Key on: start these voices' samples from the beginning, envelope from silence.",
            &VOICE_FLAGS,
        ),
        (5, 0xC) => settings(
            a,
            "Key off: these voices fade out, a step every sample, until keyed on again.",
            &VOICE_FLAGS,
        ),
        (6, 0xC) => settings(
            a,
            "Reset, mute, echo writes, and the noise generator's clock.",
            &FLG,
        ),
        (7, 0xC) => settings(
            a,
            "The voices whose sample reached a block with the end flag. Any write clears them all.",
            &VOICE_FLAGS,
        ),
        (0, 0xD) => settings(
            a,
            "How much of the echo is fed back into it; 0 is a single echo.",
            &EFB,
        ),
        (2, 0xD) => settings(
            a,
            "Pitch modulation: each voice's pitch follows the voice before's wave.",
            &PMON,
        ),
        (3, 0xD) => settings(
            a,
            "These voices play the noise generator instead of their sample.",
            &VOICE_FLAGS,
        ),
        (4, 0xD) => settings(a, "These voices are also sent into the echo.", &VOICE_FLAGS),
        (5, 0xD) => settings(
            a,
            "Where the sample directory is: four bytes a sample, its start and its loop point.",
            &DIR,
        ),
        (6, 0xD) => settings(a, "Where the echo's ring buffer is in audio RAM.", &ESA),
        (7, 0xD) => settings(a, "The echo's delay, which is its buffer's size.", &EDL),
        (_, 0xF) => settings(
            a,
            "A tap of the echo's 8-tap FIR filter; the eight usually add to 128.",
            &FIR,
        ),
        _ => data(a, "Not used: a byte of plain memory."),
    }
}

/// A write of `value` to DSP register `reg`.
pub fn describe_dsp(reg: u8, value: Option<u8>) -> RegisterWrite {
    let reg = reg & 0x7F;
    let l = dsp_layout(reg);
    let value = value.map(u32::from);
    let part: Part = fields_part(reg as u16, dsp_register_name(reg), &l, value, 1);
    RegisterWrite {
        address: reg as u16,
        width: 1,
        value,
        parts: vec![part],
    }
}

// ---- the SPC700's I/O registers ----

static TEST: [Field; 6] = [
    number(7, 6, "I/O and ROM waits", io_waits),
    number(5, 4, "RAM waits", ram_waits),
    flag2(3, "timers", "timers run", "timers stopped"),
    flag(2, "crash", "the SPC700 crashes", "normal"),
    flag2(1, "RAM writes", "RAM writable", "RAM read only"),
    flag(0, "timer enable", "timers stopped", "normal"),
];
static CONTROL: [Field; 6] = [
    flag2(7, "boot ROM", "boot ROM at $FFC0", "RAM at $FFC0"),
    flag(
        5,
        "clear ports 2/3",
        "clear ports 2 and 3",
        "keep ports 2 and 3",
    ),
    flag(
        4,
        "clear ports 0/1",
        "clear ports 0 and 1",
        "keep ports 0 and 1",
    ),
    flag(2, "timer 2", "timer 2 on", "timer 2 off"),
    flag(1, "timer 1", "timer 1 on", "timer 1 off"),
    flag(0, "timer 0", "timer 0 on", "timer 0 off"),
];
static DSPADDR: [Field; 1] = [number(7, 0, "register", dsp_index)];
static DIV: [Field; 1] = [number(7, 0, "divider", divider)];
static OUT: [Field; 1] = [number(3, 0, "count", counter)];

/// The layout of the I/O register at `address` (`$F0–$FF`).
pub fn spc_io_layout(address: u16) -> Option<Layout> {
    let a = address;
    Some(match address {
        0xF0 => settings(
            a,
            "Test settings. Every driver leaves it at $0A, the value at reset.",
            &TEST,
        ),
        0xF1 => settings(
            a,
            "Starts and stops the three timers, clears the ports' input from the S-CPU, and maps the boot ROM over the top 64 bytes.",
            &CONTROL,
        ),
        0xF2 => settings(
            a,
            "Picks the DSP register that DSPDATA reads and writes.",
            &DSPADDR,
        ),
        0xF3 => data(
            a,
            "The DSP register that DSPADDR picked: a write here is a write to the DSP.",
        ),
        0xF4..=0xF7 => data(
            a,
            "A port to the S-CPU: reading gets the byte it wrote last, writing leaves a byte it can read. What the bytes mean is the sound driver's own protocol.",
        ),
        0xF8 | 0xF9 => data(a, "Not connected on the SNES: a byte of plain memory."),
        0xFA | 0xFB => settings(
            a,
            "How many 8 kHz ticks (125 µs) make one of this timer's counts.",
            &DIV,
        ),
        0xFC => settings(
            a,
            "How many 64 kHz ticks (15.6 µs) make one of this timer's counts.",
            &DIV,
        ),
        0xFD..=0xFF => settings(
            a,
            "The timer's count, four bits, cleared when read. A driver reads it to keep its tempo.",
            &OUT,
        ),
        _ => return None,
    })
}

/// A write (or read) of `value` at I/O register `address`.
pub fn describe_spc_io(address: u16, value: Option<u8>) -> Option<RegisterWrite> {
    let l = spc_io_layout(address)?;
    let name = crate::spc700::io_register(address)?.name.to_owned();
    let value = value.map(u32::from);
    Some(RegisterWrite {
        address,
        width: 1,
        value,
        parts: vec![fields_part(address, name, &l, value, 1)],
    })
}

/// A timer divider's period, for text: `T0DIV = 16` ticks 500 times a
/// second (every 2 ms).
pub fn timer_period_ms(timer: u8, div: u8) -> f64 {
    let n = if div == 0 { 256.0 } else { div as f64 };
    if timer == 2 { n / 64.0 } else { n / 8.0 }
}
