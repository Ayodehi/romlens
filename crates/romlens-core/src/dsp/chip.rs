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
    /// Samples decoded ahead of the window, oldest first, which the window
    /// takes before the decoder decodes more. The chip decodes four at a
    /// time; this one decodes one when it needs it, so only a DSP resumed
    /// from a recording (see [`Dsp::resume`]) has any.
    pub queue: [i32; 8],
    pub queued: u8,
    /// The envelope has stepped for this sample already (a resumed DSP).
    pub stepped: bool,
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
        if std::mem::take(&mut voice.stepped) {
            // Resumed between the chip's envelope step and its pitch step.
        } else {
            self.envelope(v, regs);
        }
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
        if voice.queued > 0 {
            let s = voice.queue[0];
            voice.queue.copy_within(1.., 0);
            voice.queued -= 1;
            voice.window = [voice.window[1], voice.window[2], voice.window[3], s];
            return;
        }
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

/// The chip's step that Romlens's voice *n* runs at (3*n* + 2) is where the
/// chip does that voice's pitch counter and decoding (fullsnes's chart:
/// voice *n*'s BRR bytes at step 3*n* − 1, voice 0's at 31), three steps
/// on: a recording's step plus this is Romlens's.
pub const STEP_OFFSET: u8 = 3;

impl Dsp {
    /// The DSP as a recording had it inside (docs/23): each voice's place in
    /// its sample, its decoded samples, envelope and key-on wait, and the
    /// chip's counter, noise, KON latches and echo. Returns the step for
    /// the bus's DSP clock.
    ///
    /// The chip keeps twelve decoded samples and interpolates the four at
    /// the pitch counter's bits 12-14 past the next group's place; Romlens
    /// keeps those four as its window, the ones after them in the queue,
    /// and decodes on from where the chip would. Romlens does a sample's
    /// mixing, KON poll, counter, noise and echo at once at its step 31
    /// (the chip's 28), where the chip spreads them over steps 22-30; a
    /// recording taken in between is moved to match. Romlens polls KON on the chip's
    /// other samples (see `even` below).
    pub fn resume(
        inside: &crate::recording::DspInside,
        regs: &[u8; 128],
        aram: &[u8],
    ) -> (Dsp, u8) {
        let s = inside.step & 31;
        let mut d = Dsp {
            counter: inside.counter,
            // Romlens keys a voice on at the poll, where the chip keys
            // voices 1-7 on in the sample after; polling on the chip's other
            // samples makes up for it (the phase A8 measured from rest).
            even: !inside.polls,
            kon_latch: inside.kon_written & !inside.kon_taken,
            noise: inside.noise & 0x7FFF,
            echo_offset: inside.echo_offset,
            echo_length: if inside.echo_length == 0 {
                4
            } else {
                inside.echo_length
            },
            fir_at: inside.fir_at as usize & 7,
            ..Dsp::default()
        };
        for (e, pair) in inside.fir.iter().enumerate() {
            d.fir[0][e] = pair[0] as i32;
            d.fir[1][e] = pair[1] as i32;
        }
        // The chip mixes a sample from its step 31 to 21 and gives it out
        // at 26 and 27; Romlens gives its out at 28.
        if s <= 25 {
            d.main = [inside.mix[0] as i32, inside.mix[1] as i32];
            d.echo_in = [inside.echo_in[0] as i32, inside.echo_in[1] as i32];
        }
        // The FIR's input for this sample is in (steps 22 and 23) but
        // Romlens's step 31 has not run: it reads the same entry again.
        if (23..=28).contains(&s) {
            d.fir_at = (d.fir_at + 7) & 7;
        }
        // Romlens's step 31 has run but the chip's 29 and 30 have not: the
        // KON phase has flipped, the counter and noise moved and the echo
        // offset stepped.
        if (29..=30).contains(&s) {
            d.counter = if d.counter == 0 {
                COUNTER_WRAP - 1
            } else {
                d.counter - 1
            };
            if d.fires(regs[0x6C] & 0x1F) {
                let feedback = (d.noise ^ (d.noise >> 1)) & 1;
                d.noise = (d.noise >> 1) & 0x3FFF | feedback << 14;
            }
            if s == 29 {
                d.even = !d.even;
                if d.echo_offset == 0 {
                    let edl = regs[0x7D] & 0xF;
                    d.echo_length = if edl == 0 { 4 } else { edl as u16 * 2048 };
                }
                d.echo_offset += 4;
                if d.echo_offset >= d.echo_length {
                    d.echo_offset = 0;
                }
            }
        }
        for (n, v) in inside.voices.iter().enumerate() {
            d.voices[n] = resume_voice(n, v, regs, aram);
        }
        // The chip does a voice's output and envelope a step before its
        // pitch counter (voice n at 3n - 2, voice 0 at 30); Romlens does
        // all three at once. Taken between the two, the envelope has
        // stepped already.
        let between = match s {
            31 => Some(0),
            s if s >= 2 && (s + 1) % 3 == 0 && s <= 20 => Some((s as usize + 1) / 3),
            _ => None,
        };
        if let Some(n) = between {
            d.voices[n].stepped = true;
        }
        // Pitch modulation reads the voice before's output: OUTX's top
        // bits, or all of it for the voice the chip did last.
        let last = match s {
            0 | 1 | 31 => 0,
            _ => ((s as usize + 1) / 3).clamp(1, 7),
        };
        for n in 0..8 {
            d.voices[n].output = (regs[n << 4 | 9] as i8 as i32) << 7;
        }
        d.voices[last].output = inside.voice_output as i32 >> 1;
        (d, (s + STEP_OFFSET) & 31)
    }
}

fn resume_voice(
    n: usize,
    v: &crate::recording::dsp_inside::VoiceInside,
    regs: &[u8; 128],
    aram: &[u8],
) -> Voice {
    let mode = phase(n, v, regs);
    if v.key_on_wait > 0 {
        // Keyed on and waiting: Romlens starts the sample at the end of the
        // wait, from the directory.
        return Voice {
            block: dir_entry(regs, aram, n).0,
            delay: v.key_on_wait,
            mode: EnvelopeMode::Attack,
            ..Voice::default()
        };
    }
    // The ring, oldest group first from `ring_at`; the window starts the
    // pitch counter's bits 12-14 in.
    let at = v.ring_at as usize % 12;
    let skip = (v.pitch_counter >> 12) as usize & 7;
    let ring = |k: usize| v.ring[(at + k) % 12] as i32 >> 1;
    let mut queue = [0i32; 8];
    let queued = 8 - skip;
    for (k, q) in queue.iter_mut().enumerate().take(queued) {
        *q = ring(skip + 4 + k);
    }
    let block = v.block;
    Voice {
        block,
        header: aram[block as usize],
        nibble: (v.data_byte.saturating_sub(1) * 2) & 15,
        p1: ring(11),
        p2: ring(10),
        window: [ring(skip), ring(skip + 1), ring(skip + 2), ring(skip + 3)],
        fraction: v.pitch_counter & 0xFFF,
        delay: 0,
        mode,
        envelope: v.envelope & 0x7FF,
        output: 0,
        queue,
        queued: queued as u8,
        stepped: false,
    }
}

/// A voice's envelope phase: as recorded, or, from a Mesen that does not
/// export it, worked out from the envelope and the value the chip last
/// computed for it before clamping. That value is the level's next step by
/// the phase's own rule (attack adds 32 or 1024, decay and sustain take
/// 1/256), or the level itself when the step was just taken (a level on
/// attack's steps of 32 then taken for attack); release computes none, so
/// a level no rule explains is releasing. Under a gain mode only release
/// matters, and a voice keyed on is otherwise in attack. On the five games
/// of docs/23's *Measured* this agrees with the phase recorded in 99.6% of
/// the voice-frames with a level or more.
fn phase(
    n: usize,
    v: &crate::recording::dsp_inside::VoiceInside,
    regs: &[u8; 128],
) -> EnvelopeMode {
    use crate::recording::dsp_inside::PHASE_UNKNOWN;
    if v.phase != PHASE_UNKNOWN {
        return match v.phase {
            1 => EnvelopeMode::Attack,
            2 => EnvelopeMode::Decay,
            3 => EnvelopeMode::Sustain,
            _ => EnvelopeMode::Release,
        };
    }
    if v.key_on_wait > 0 {
        return EnvelopeMode::Attack;
    }
    let (e, u) = (v.envelope as i32, v.unclamped as i32);
    let adsr1 = vreg(regs, n, 5);
    let adsr2 = vreg(regs, n, 6);
    let gain = vreg(regs, n, 7);
    let exp = |x: i32| x - (((x - 1) >> 8) + 1);
    if adsr1 & 0x80 == 0 {
        // Gain modes ignore the phase but for release.
        let explained = if gain & 0x80 == 0 {
            u == (gain as i32 & 0x7F) << 4
        } else {
            let step = match (gain >> 5) & 3 {
                0 => e - 32,
                1 => exp(e),
                2 => e + 32,
                _ => e + if u < 0x600 { 32 } else { 8 },
            };
            u == step || u == e
        };
        // A voice keyed on stays in attack under a gain mode until its
        // level clips.
        return if explained {
            EnvelopeMode::Attack
        } else {
            EnvelopeMode::Release
        };
    }
    let sustain = (adsr2 >> 5) as i32;
    let by_level = || {
        if u >> 8 <= sustain {
            EnvelopeMode::Sustain
        } else {
            EnvelopeMode::Decay
        }
    };
    if u == e + 32 || u == e + 1024 || (e == 0x7FF && u > 0x7FF) {
        if e == 0x7FF {
            EnvelopeMode::Decay
        } else {
            EnvelopeMode::Attack
        }
    } else if u == exp(e) {
        by_level()
    } else if u == e {
        if e < 0x7FF && e & 31 == 0 {
            EnvelopeMode::Attack
        } else {
            by_level()
        }
    } else {
        EnvelopeMode::Release
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::DspInside;
    use crate::recording::dsp_inside::{PHASE_UNKNOWN, VoiceInside};

    fn inside_with(v: VoiceInside, step: u8) -> DspInside {
        let mut d = DspInside {
            step,
            counter: 0x1234,
            noise: 0x4000,
            echo_offset: 0x100,
            echo_length: 0x800,
            fir_at: 5,
            phases_recorded: true,
            ..DspInside::default()
        };
        d.voices[1] = v;
        d
    }

    #[test]
    fn a_resumed_voice_takes_the_chips_window_and_decodes_on_from_it() {
        // The ring's next group goes at 8, so 8-11 are the oldest; the
        // pitch counter's bits 12-14 put the window two further on.
        let ring: [i16; 12] = std::array::from_fn(|k| (k as i16 + 1) * 100);
        let v = VoiceInside {
            block: 0x4009,
            data_byte: 5,
            ring_at: 8,
            pitch_counter: 0x2345,
            phase: 3,
            envelope: 0x400,
            ring,
            ..VoiceInside::default()
        };
        let aram = vec![0u8; 0x10000];
        let (d, clock) = Dsp::resume(&inside_with(v, 10), &[0; 128], &aram);
        assert_eq!(clock, 13, "the chip's step plus three");
        let r = &d.voices[1];
        // Ring slots 10, 11, 0, 1, halved to 15 bits.
        assert_eq!(r.window, [550, 600, 50, 100]);
        // Then 2-7: six decoded ahead of the window.
        assert_eq!(r.queued, 6);
        assert_eq!(r.queue[..6], [150, 200, 250, 300, 350, 400]);
        // The decoder carries on after the newest (slot 7), from nibble 8.
        assert_eq!((r.p1, r.p2), (400, 350));
        assert_eq!((r.block, r.nibble, r.fraction), (0x4009, 8, 0x345));
        assert_eq!((r.mode, r.envelope), (EnvelopeMode::Sustain, 0x400));
        assert_eq!(
            (d.counter, d.noise, d.echo_offset, d.fir_at),
            (0x1234, 0x4000, 0x100, 5)
        );
    }

    #[test]
    fn a_resume_between_the_chips_steps_moves_to_romlenss() {
        let aram = vec![0u8; 0x10000];
        let regs = [0u8; 128];
        let v = VoiceInside::default();
        // After the chip's FIR input (22) and before Romlens's step 31 (28):
        // Romlens reads the same entry again.
        let (d, _) = Dsp::resume(&inside_with(v, 25), &regs, &aram);
        assert_eq!(d.fir_at, 4);
        // After Romlens's step 31 (28) but before the chip's 29: the
        // counter, the KON phase and the echo offset have moved on.
        let (d, clock) = Dsp::resume(&inside_with(v, 29), &regs, &aram);
        assert_eq!((clock, d.counter, d.echo_offset), (0, 0x1233, 0x104));
        assert!(!d.even, "the poll phase moved on");
        // Between voice 1's envelope (step 1) and its pitch counter (2).
        let (d, _) = Dsp::resume(&inside_with(v, 2), &regs, &aram);
        assert!(d.voices[1].stepped && !d.voices[2].stepped);
    }

    #[test]
    fn a_phase_not_recorded_is_worked_out_from_the_envelope() {
        let mut regs = [0u8; 128];
        // Voice 1 by ADSR, sustain level 4.
        regs[0x15] = 0x8F;
        regs[0x16] = 0x80;
        let aram = vec![0u8; 0x10000];
        let at = |envelope: u16, unclamped: i16| {
            let v = VoiceInside {
                phase: PHASE_UNKNOWN,
                envelope,
                unclamped,
                ..VoiceInside::default()
            };
            let (d, _) = Dsp::resume(&inside_with(v, 10), &regs, &aram);
            d.voices[1].mode
        };
        assert_eq!(at(0x200, 0x220), EnvelopeMode::Attack);
        assert_eq!(at(0x7FF, 0x7FF + 1024), EnvelopeMode::Decay);
        // 1/256 down: decay above the sustain level, sustain at it.
        assert_eq!(at(0x600, 0x600 - 6), EnvelopeMode::Decay);
        assert_eq!(at(0x480, 0x480 - 4 - 1), EnvelopeMode::Sustain);
        // Neither rule: releasing.
        assert_eq!(at(0x300, 0x5A0), EnvelopeMode::Release);
    }
}
