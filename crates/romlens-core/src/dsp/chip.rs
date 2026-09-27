//! The S-DSP as a machine (docs/23, A8): eight voices, the noise
//! generator, the envelopes, echo and the mix, one stereo sample every 32
//! SPC700 cycles (32 kHz).
//!
//! Written from fullsnes's DSP chapters, anomie's S-DSP document and the
//! SNESdev wiki's envelope page (the global counter and its offsets). The
//! 128 registers the SPC700 sees stay in the bus; this holds what the chip
//! keeps inside: where each voice is in its sample, its envelope, the echo
//! buffer's position and the FIR's history.
//!
//! The chip works through a sample in 32 steps (fullsnes's timing chart);
//! this does the whole sample at once, in the chart's order (voices 0 to 7,
//! then echo and the mix), and polls KON and KOFF every other sample as
//! the chip does.

use super::brr::Header;
use super::gauss::GAUSS;

/// Samples between steps, by rate (fullsnes, anomie): 0 never.
pub const RATE_PERIODS: [u16; 32] = [
    0, 2048, 1536, 1280, 1024, 768, 640, 512, 384, 320, 256, 192, 160, 128, 96, 80, 64, 48, 40, 32,
    24, 20, 16, 12, 10, 8, 6, 5, 4, 3, 2, 1,
];

/// Each rate's offset against the global counter (the SNESdev wiki's
/// "DSP Period Offset" table): the three columns of the table, 1×, 3× and
/// 5× a power of two, step at different points of the counter.
const fn rate_offset(rate: u8) -> u16 {
    match rate % 3 {
        0 => 536,
        1 => 0,
        _ => 1040,
    }
}

/// The global counter's length: it counts down one a sample from here.
const COUNTER_WRAP: u16 = 0x7800;

/// The global counter `samples` after power on: it starts at 0 and counts
/// down one a sample, the first wrapping it.
pub fn counter_after(samples: u64) -> u16 {
    let wrap = COUNTER_WRAP as u64;
    ((wrap - samples % wrap) % wrap) as u16
}

/// Samples a voice waits after key on before its sample starts.
pub const KEY_ON_DELAY: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvelopeMode {
    Attack,
    Decay,
    Sustain,
    #[default]
    Release,
}

/// One voice's inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Voice {
    /// The block being decoded, its header, and the next nibble in it.
    pub block: u16,
    pub header: u8,
    pub nibble: u8,
    /// The decoder's last two results (15 bits).
    pub p1: i32,
    pub p2: i32,
    /// The four newest decoded samples, oldest first, 15 bits each.
    pub window: [i32; 4],
    /// The pitch counter's fraction: bits 4-11 pick the interpolation.
    pub fraction: u16,
    /// Samples left of the key-on wait.
    pub delay: u8,
    pub mode: EnvelopeMode,
    /// 11 bits.
    pub envelope: u16,
    /// The sample after the envelope, 15 bits: what OUTX shows the top of
    /// and what the next voice's pitch modulation reads.
    pub output: i32,
}

/// What a sample produced, for listening and for scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Frame {
    pub left: i16,
    pub right: i16,
    /// Each voice's output after its envelope, before its volume.
    pub voices: [i16; 8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dsp {
    pub voices: [Voice; 8],
    /// Counts down one a sample, wrapping at `$7800`.
    pub counter: u16,
    /// KON is polled on every other sample.
    pub even: bool,
    /// Voices keyed on since the last poll.
    pub kon_latch: u8,
    /// The noise generator's 15 bits.
    pub noise: u16,
    /// Where in the echo buffer, in bytes, and how long the buffer is.
    pub echo_offset: u16,
    pub echo_length: u16,
    /// The FIR's last eight inputs, each side.
    pub fir: [[i32; 8]; 2],
    pub fir_at: usize,
    /// Voices to leave out of the mix (for listening to some alone); they
    /// still run and still show in their registers.
    pub muted: u8,
    /// The sample so far: the voices mixed, and what goes to echo.
    main: [i32; 2],
    echo_in: [i32; 2],
    frame: Frame,
}

impl Default for Dsp {
    fn default() -> Self {
        Dsp {
            voices: [Voice::default(); 8],
            counter: 0,
            even: false,
            kon_latch: 0,
            noise: 0x4000,
            echo_offset: 0,
            echo_length: 0,
            fir: [[0; 8]; 2],
            fir_at: 0,
            muted: 0,
            main: [0; 2],
            echo_in: [0; 2],
            frame: Frame::default(),
        }
    }
}

fn clamp16(v: i32) -> i32 {
    v.clamp(-0x8000, 0x7FFF)
}

/// A voice register.
fn vreg(regs: &[u8; 128], v: usize, r: usize) -> u8 {
    regs[v << 4 | r]
}

impl Dsp {
    /// Whether an envelope or the noise steps this sample at `rate`.
    fn fires(&self, rate: u8) -> bool {
        let period = RATE_PERIODS[rate as usize & 31];
        period != 0 && (self.counter + rate_offset(rate)).is_multiple_of(period)
    }

    /// The SPC700 wrote KON: the voices key on at the next poll.
    pub fn write_kon(&mut self, value: u8) {
        self.kon_latch |= value;
    }

    /// One sample: the 32 steps at once.
    pub fn sample(&mut self, regs: &mut [u8; 128], aram: &mut [u8]) -> Frame {
        for t in 0..31 {
            self.step(t, regs, aram);
        }
        self.step(31, regs, aram).unwrap()
    }

    /// One of the sample's 32 steps, one per SPC700 cycle: voice *n* runs
    /// at step 3*n* + 2, where fullsnes's chart has the chip write its
    /// ENVX; the last step polls KON and KOFF, moves the global counter and
    /// the noise, runs the echo and gives out the sample. `regs` are the
    /// 128 registers, `aram` audio RAM, which the chip reads samples from
    /// and writes echo into.
    pub fn step(&mut self, t: u8, regs: &mut [u8; 128], aram: &mut [u8]) -> Option<Frame> {
        if (2..=23).contains(&t) && (t - 2).is_multiple_of(3) {
            let v = ((t - 2) / 3) as usize;
            if regs[0x6C] & 0x80 != 0 {
                let voice = &mut self.voices[v];
                voice.mode = EnvelopeMode::Release;
                voice.envelope = 0;
            }
            let previous = if v > 0 { self.voices[v - 1].output } else { 0 };
            let s = self.run_voice(v, regs, aram, previous);
            self.frame.voices[v] = s as i16;
            if self.muted & (1 << v) == 0 {
                let vol = [vreg(regs, v, 0) as i8 as i32, vreg(regs, v, 1) as i8 as i32];
                let eon = regs[0x4D] & (1 << v) != 0;
                for (side, vol) in vol.into_iter().enumerate() {
                    let part = (s * vol) >> 6;
                    self.main[side] = clamp16(self.main[side] + part);
                    if eon {
                        self.echo_in[side] = clamp16(self.echo_in[side] + part);
                    }
                }
            }
            return None;
        }
        if t != 31 {
            return None;
        }
        self.counter = if self.counter == 0 {
            COUNTER_WRAP - 1
        } else {
            self.counter - 1
        };
        let flg = regs[0x6C];
        // KON and KOFF on every other sample.
        self.even = !self.even;
        if self.even {
            let koff = regs[0x5C];
            for v in 0..8 {
                let bit = 1 << v;
                if koff & bit != 0 {
                    self.voices[v].mode = EnvelopeMode::Release;
                }
                if self.kon_latch & bit != 0 {
                    self.key_on(v, regs, aram);
                }
            }
            self.kon_latch = 0;
        }
        if self.fires(flg & 0x1F) {
            let feedback = (self.noise ^ (self.noise >> 1)) & 1;
            self.noise = (self.noise >> 1) & 0x3FFF | feedback << 14;
        }
        let mut out = std::mem::take(&mut self.frame);
        let main = std::mem::take(&mut self.main);
        let echo = std::mem::take(&mut self.echo_in);

        // Echo: the oldest entry out of the buffer into the FIR, and, when
        // writes are on, the new echo in its place.
        let esa = (regs[0x6D] as u16) << 8;
        if self.echo_offset == 0 {
            let edl = regs[0x7D] & 0xF;
            self.echo_length = if edl == 0 { 4 } else { edl as u16 * 2048 };
        }
        let at = esa.wrapping_add(self.echo_offset);
        self.fir_at = (self.fir_at + 1) & 7;
        let mut fir_out = [0i32; 2];
        for (side, out) in fir_out.iter_mut().enumerate() {
            let a = at.wrapping_add(side as u16 * 2);
            let sample =
                i16::from_le_bytes([aram[a as usize], aram[a.wrapping_add(1) as usize]]) as i32;
            self.fir[side][self.fir_at] = sample >> 1;
            // FIR0 on the oldest; the first seven add up wrapping in 16
            // bits, the last is clamped.
            let mut sum = 0i32;
            for tap in 0..7 {
                let h = self.fir[side][(self.fir_at + 1 + tap) & 7];
                sum += (h * regs[tap << 4 | 0xF] as i8 as i32) >> 6;
            }
            sum = sum as i16 as i32;
            *out = clamp16(sum + ((self.fir[side][self.fir_at] * regs[0x7F] as i8 as i32) >> 6));
        }
        let mvol = [regs[0x0C] as i8 as i32, regs[0x1C] as i8 as i32];
        let evol = [regs[0x2C] as i8 as i32, regs[0x3C] as i8 as i32];
        let efb = regs[0x0D] as i8 as i32;
        let mut sides = [0i32; 2];
        for side in 0..2 {
            let s = clamp16(((main[side] * mvol[side]) >> 7) + ((fir_out[side] * evol[side]) >> 7));
            sides[side] = if flg & 0x40 != 0 { 0 } else { s };
            if flg & 0x20 == 0 {
                let e = clamp16(echo[side] + ((fir_out[side] * efb) >> 7)) & !1;
                let a = at.wrapping_add(side as u16 * 2);
                let [lo, hi] = (e as i16).to_le_bytes();
                aram[a as usize] = lo;
                aram[a.wrapping_add(1) as usize] = hi;
            }
        }
        self.echo_offset += 4;
        if self.echo_offset >= self.echo_length {
            self.echo_offset = 0;
        }
        out.left = sides[0] as i16;
        out.right = sides[1] as i16;
        Some(out)
    }

    fn key_on(&mut self, v: usize, regs: &mut [u8; 128], aram: &[u8]) {
        let entry = dir_entry(regs, aram, v);
        let voice = &mut self.voices[v];
        *voice = Voice {
            block: entry.0,
            delay: KEY_ON_DELAY,
            mode: EnvelopeMode::Attack,
            ..Voice::default()
        };
        regs[0x7C] &= !(1 << v);
    }

    /// One voice's sample: its output after the envelope (15 bits).
    fn run_voice(&mut self, v: usize, regs: &mut [u8; 128], aram: &[u8], previous: i32) -> i32 {
        let noise_on = regs[0x3D] & (1 << v) != 0;
        let pmon = v > 0 && regs[0x2D] & (1 << v) != 0 && !noise_on;
        let pitch = u16::from_le_bytes([vreg(regs, v, 2), vreg(regs, v, 3)]) & 0x3FFF;
        let noise = ((self.noise << 1) as i16 >> 1) as i32;

        if self.voices[v].delay > 0 {
            let voice = &mut self.voices[v];
            voice.delay -= 1;
            if voice.delay == 0 {
                // The first four samples go in before the first output.
                for _ in 0..4 {
                    self.decode_next(v, regs, aram);
                }
            }
            let voice = &mut self.voices[v];
            voice.output = 0;
            regs[v << 4 | 8] = 0;
            regs[v << 4 | 9] = 0;
            return 0;
        }

        // Interpolate between the four newest samples.
        let voice = &self.voices[v];
        let i = ((voice.fraction >> 4) & 0xFF) as usize;
        let [s0, s1, s2, s3] = voice.window;
        let sample = if noise_on {
            noise
        } else {
            let mut o = (GAUSS[0xFF - i] * s0) >> 10;
            o += (GAUSS[0x1FF - i] * s1) >> 10;
            o += (GAUSS[0x100 + i] * s2) >> 10;
            o = o as i16 as i32;
            o = clamp16(o + ((GAUSS[i] * s3) >> 10));
            o >> 1
        };

        // The sample goes out at the envelope's level so far; the envelope
        // steps after.
        let voice = &mut self.voices[v];
        let output = (sample * voice.envelope as i32) >> 11;
        voice.output = output;
        regs[v << 4 | 8] = (voice.envelope >> 4) as u8;
        regs[v << 4 | 9] = (output >> 7) as u8;
        self.envelope(v, regs);
        let voice = &mut self.voices[v];

        // The pitch counter, and the samples it passes.
        let mut step = pitch as i32;
        if pmon {
            let factor = (previous >> 4) + 0x400;
            step = ((step * factor) >> 10).min(0x7FFF);
        }
        let total = voice.fraction as i32 + step;
        voice.fraction = (total & 0xFFF) as u16;
        for _ in 0..(total >> 12) {
            self.decode_next(v, regs, aram);
        }
        output
    }

    /// Decode the next BRR sample into the window, moving through the
    /// blocks and to the loop point.
    fn decode_next(&mut self, v: usize, regs: &mut [u8; 128], aram: &[u8]) {
        let voice = &mut self.voices[v];
        if voice.nibble == 0 {
            voice.header = aram[voice.block as usize];
            let h = Header::from_byte(voice.header);
            if h.end {
                // Set as the block starts; one that does not loop also
                // ends the note there.
                regs[0x7C] |= 1 << v;
                if !h.loops {
                    voice.mode = EnvelopeMode::Release;
                    voice.envelope = 0;
                }
            }
        }
        let h = Header::from_byte(voice.header);
        let byte = aram[voice.block.wrapping_add(1 + voice.nibble as u16 / 2) as usize];
        let raw = if voice.nibble.is_multiple_of(2) {
            byte >> 4
        } else {
            byte & 0xF
        };
        let nibble = (((raw << 4) as i8) >> 4) as i32;
        let shifted = if h.shift <= 12 {
            (nibble << h.shift) >> 1
        } else if nibble < 0 {
            -2048
        } else {
            0
        };
        let (p1, p2) = (voice.p1, voice.p2);
        let prediction = match h.filter {
            0 => 0,
            1 => p1 + ((-p1) >> 4),
            2 => 2 * p1 + ((-3 * p1) >> 5) - p2 + (p2 >> 4),
            _ => 2 * p1 + ((-13 * p1) >> 6) - p2 + ((3 * p2) >> 4),
        };
        let clamped = clamp16(shifted + prediction);
        let result = (((clamped << 1) as i16) >> 1) as i32;
        voice.p2 = p1;
        voice.p1 = result;
        voice.window = [voice.window[1], voice.window[2], voice.window[3], result];
        voice.nibble += 1;
        if voice.nibble == 16 {
            voice.nibble = 0;
            voice.block = if h.end {
                dir_entry(regs, aram, v).1
            } else {
                voice.block.wrapping_add(9)
            };
        }
    }

    fn envelope(&mut self, v: usize, regs: &[u8; 128]) {
        let adsr1 = vreg(regs, v, 5);
        let adsr2 = vreg(regs, v, 6);
        let gain = vreg(regs, v, 7);
        let mode = self.voices[v].mode;
        let env = self.voices[v].envelope as i32;
        let exp = |e: i32| e - (((e - 1) >> 8) + 1);
        let (rate, next) = if mode == EnvelopeMode::Release {
            (31, env - 8)
        } else if adsr1 & 0x80 != 0 {
            match mode {
                EnvelopeMode::Attack => {
                    let a = adsr1 & 0xF;
                    if a == 0xF {
                        (31, env + 1024)
                    } else {
                        (a * 2 + 1, env + 32)
                    }
                }
                EnvelopeMode::Decay => (((adsr1 >> 4) & 7) * 2 + 16, exp(env)),
                _ => (adsr2 & 0x1F, exp(env)),
            }
        } else if gain & 0x80 == 0 {
            // Direct: the level is set, whatever the rate.
            self.voices[v].envelope = (gain as u16 & 0x7F) << 4;
            return;
        } else {
            let r = gain & 0x1F;
            match (gain >> 5) & 3 {
                0 => (r, env - 32),
                1 => (r, exp(env)),
                2 => (r, env + 32),
                _ => (r, env + if env < 0x600 { 32 } else { 8 }),
            }
        };
        if !self.fires(rate) {
            return;
        }
        let e = next.clamp(0, 0x7FF);
        let voice = &mut self.voices[v];
        voice.envelope = e as u16;
        // The ADSR's phases move on as the level passes their marks, in
        // gain modes too (fullsnes, "Gain Notes").
        match voice.mode {
            EnvelopeMode::Attack if next >= 0x7E0 => voice.mode = EnvelopeMode::Decay,
            EnvelopeMode::Decay => {
                let level = if adsr1 & 0x80 != 0 {
                    adsr2 >> 5
                } else {
                    gain >> 5
                };
                if e <= (level as i32 + 1) * 0x100 {
                    voice.mode = EnvelopeMode::Sustain;
                }
            }
            _ => {}
        }
    }
}

/// A voice's directory entry: where its sample starts and loops.
fn dir_entry(regs: &[u8; 128], aram: &[u8], v: usize) -> (u16, u16) {
    let at = ((regs[0x5D] as usize) << 8) + vreg(regs, v, 4) as usize * 4;
    let w = |i: usize| u16::from_le_bytes([aram[(at + i) & 0xFFFF], aram[(at + i + 1) & 0xFFFF]]);
    (w(0), w(2))
}

/// The envelope a voice with these settings goes through, run on the DSP
/// itself: keyed on, held `hold` samples, then keyed off for `release`
/// samples, the level (0–`$7FF`) taken every `stride` samples. What the
/// Voices view draws.
pub fn envelope_curve(
    adsr1: u8,
    adsr2: u8,
    gain: u8,
    hold: usize,
    release: usize,
    stride: usize,
) -> Vec<u16> {
    let stride = stride.max(1);
    // A looping block of silence at $1000; the envelope does not care
    // what the sample holds.
    let mut aram = vec![0u8; 0x10000];
    aram[0x0200..0x0204].copy_from_slice(&[0x00, 0x10, 0x00, 0x10]);
    aram[0x1000] = 0x03;
    let mut regs = [0u8; 128];
    regs[0x5D] = 0x02;
    regs[0x6C] = 0x20;
    regs[0x03] = 0x10;
    regs[0x05] = adsr1;
    regs[0x06] = adsr2;
    regs[0x07] = gain;
    let mut dsp = Dsp::default();
    dsp.write_kon(0x01);
    // Through the poll and the key-on wait, so the curve starts at the
    // note's first sample.
    for _ in 0..2 {
        if dsp.voices[0].delay > 0 {
            break;
        }
        dsp.sample(&mut regs, &mut aram);
    }
    while dsp.voices[0].delay > 0 {
        dsp.sample(&mut regs, &mut aram);
    }
    let mut out = Vec::with_capacity((hold + release) / stride + 1);
    for i in 0..hold + release {
        if i == hold {
            regs[0x5C] = 0x01;
        }
        if i % stride == 0 {
            out.push(dsp.voices[0].envelope);
        }
        dsp.sample(&mut regs, &mut aram);
    }
    out
}
