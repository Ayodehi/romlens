//! What the S-DSP keeps inside at a frame's end (docs/23), beyond its 128
//! registers: where each voice is in its sample, the samples it has
//! decoded, its envelope, and the chip's counter, noise and echo. With it a
//! replay starts the DSP where the recording had it, not from rest.
//!
//! The layout follows how the chip works rather than any emulator's
//! variables: the decoder reads a BRR block two data bytes (four samples)
//! at a time into a ring of twelve, the pitch counter's top bits pick
//! which four of them are interpolated (fullsnes, "BRR Pitch"; anomie,
//! "Pitch adjustments"), and the chip runs through a sample in 32 steps.
//! `rec pack` fills it from Mesen's `spc.dsp.` state fields.
//!
//! 384 bytes, little-endian. Each voice, 40 bytes at 40 × *n*:
//!
//! | Offset | Size | What |
//! |---|---|---|
//! | 0 | 2 | the BRR block being decoded |
//! | 2 | 1 | the next data byte in it, 1, 3, 5 or 7 |
//! | 3 | 1 | where the ring's next four samples go: 0, 4 or 8 |
//! | 4 | 2 | the pitch counter, 15 bits: bits 12–14 the ring offset, 4–11 the interpolation |
//! | 6 | 1 | samples left of the key-on wait |
//! | 7 | 1 | the envelope's phase: 0 release, 1 attack, 2 decay, 3 sustain, `$FF` unknown |
//! | 8 | 2 | the envelope, 11 bits |
//! | 10 | 2 | the envelope's last value before clamping (signed) |
//! | 12 | 1 | ENVX as the chip will next show it |
//! | 13 | 3 | reserved, zero |
//! | 16 | 24 | the ring: twelve decoded samples, signed, doubled (16-bit) |
//!
//! Then at 320:
//!
//! | Offset | Size | What |
//! |---|---|---|
//! | 320 | 1 | the step the chip does next, 0–31 |
//! | 321 | 2 | the global counter (counts down from `$77FF`) |
//! | 323 | 1 | bit 0: this is a sample that polls KON and KOFF |
//! | 324 | 1 | KON as written, not yet taken |
//! | 325 | 1 | KON as taken at the last poll |
//! | 326 | 1 | KOFF as taken at the last poll |
//! | 327 | 2 | the noise generator, 15 bits |
//! | 329 | 2 | the echo buffer: the offset in it |
//! | 331 | 2 | the echo buffer's length in bytes |
//! | 333 | 1 | the FIR's newest entry, 0–7 |
//! | 334 | 32 | the FIR's eight inputs, left then right each (signed) |
//! | 366 | 4 | the sample's mix so far, left and right (signed) |
//! | 370 | 4 | its echo input so far, left and right (signed) |
//! | 374 | 2 | the last voice's output after its envelope (signed) |
//! | 376 | 1 | bit 0: the envelope phases were recorded, not inferred |
//! | 377 | 7 | reserved, zero |

pub const SIZE: usize = 384;
const VOICE: usize = 40;
const GLOBAL: usize = 320;

/// The envelope's phase is not known (a Mesen that does not export it).
pub const PHASE_UNKNOWN: u8 = 0xFF;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VoiceInside {
    pub block: u16,
    pub data_byte: u8,
    pub ring_at: u8,
    pub pitch_counter: u16,
    pub key_on_wait: u8,
    pub phase: u8,
    pub envelope: u16,
    pub unclamped: i16,
    pub envx: u8,
    pub ring: [i16; 12],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DspInside {
    pub voices: [VoiceInside; 8],
    pub step: u8,
    pub counter: u16,
    pub polls: bool,
    pub kon_written: u8,
    pub kon_taken: u8,
    pub koff_taken: u8,
    pub noise: u16,
    pub echo_offset: u16,
    pub echo_length: u16,
    pub fir_at: u8,
    /// `[entry][side]`.
    pub fir: [[i16; 2]; 8],
    pub mix: [i16; 2],
    pub echo_in: [i16; 2],
    pub voice_output: i16,
    pub phases_recorded: bool,
}

impl DspInside {
    pub fn encode(&self) -> [u8; SIZE] {
        let mut b = [0u8; SIZE];
        let put16 =
            |b: &mut [u8], at: usize, v: u16| b[at..at + 2].copy_from_slice(&v.to_le_bytes());
        for (n, v) in self.voices.iter().enumerate() {
            let o = n * VOICE;
            put16(&mut b, o, v.block);
            b[o + 2] = v.data_byte;
            b[o + 3] = v.ring_at;
            put16(&mut b, o + 4, v.pitch_counter);
            b[o + 6] = v.key_on_wait;
            b[o + 7] = v.phase;
            put16(&mut b, o + 8, v.envelope);
            put16(&mut b, o + 10, v.unclamped as u16);
            b[o + 12] = v.envx;
            for (k, s) in v.ring.iter().enumerate() {
                put16(&mut b, o + 16 + k * 2, *s as u16);
            }
        }
        let g = GLOBAL;
        b[g] = self.step;
        put16(&mut b, g + 1, self.counter);
        b[g + 3] = self.polls as u8;
        b[g + 4] = self.kon_written;
        b[g + 5] = self.kon_taken;
        b[g + 6] = self.koff_taken;
        put16(&mut b, g + 7, self.noise);
        put16(&mut b, g + 9, self.echo_offset);
        put16(&mut b, g + 11, self.echo_length);
        b[g + 13] = self.fir_at;
        for (k, e) in self.fir.iter().enumerate() {
            put16(&mut b, g + 14 + k * 4, e[0] as u16);
            put16(&mut b, g + 16 + k * 4, e[1] as u16);
        }
        for side in 0..2 {
            put16(&mut b, g + 46 + side * 2, self.mix[side] as u16);
            put16(&mut b, g + 50 + side * 2, self.echo_in[side] as u16);
        }
        put16(&mut b, g + 54, self.voice_output as u16);
        b[g + 56] = self.phases_recorded as u8;
        b
    }

    pub fn decode(b: &[u8]) -> Self {
        let mut p = [0u8; SIZE];
        let n = b.len().min(SIZE);
        p[..n].copy_from_slice(&b[..n]);
        let w = |at: usize| u16::from_le_bytes([p[at], p[at + 1]]);
        let voices = std::array::from_fn(|n| {
            let o = n * VOICE;
            VoiceInside {
                block: w(o),
                data_byte: p[o + 2],
                ring_at: p[o + 3],
                pitch_counter: w(o + 4),
                key_on_wait: p[o + 6],
                phase: p[o + 7],
                envelope: w(o + 8),
                unclamped: w(o + 10) as i16,
                envx: p[o + 12],
                ring: std::array::from_fn(|k| w(o + 16 + k * 2) as i16),
            }
        });
        let g = GLOBAL;
        DspInside {
            voices,
            step: p[g],
            counter: w(g + 1),
            polls: p[g + 3] & 1 != 0,
            kon_written: p[g + 4],
            kon_taken: p[g + 5],
            koff_taken: p[g + 6],
            noise: w(g + 7),
            echo_offset: w(g + 9),
            echo_length: w(g + 11),
            fir_at: p[g + 13],
            fir: std::array::from_fn(|k| [w(g + 14 + k * 4) as i16, w(g + 16 + k * 4) as i16]),
            mix: [w(g + 46) as i16, w(g + 48) as i16],
            echo_in: [w(g + 50) as i16, w(g + 52) as i16],
            voice_output: w(g + 54) as i16,
            phases_recorded: p[g + 56] & 1 != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_round_trips() {
        let mut d = DspInside {
            step: 13,
            counter: 0x7123,
            polls: true,
            kon_written: 0x81,
            noise: 0x4000,
            echo_offset: 0x640,
            echo_length: 0x1000,
            fir_at: 7,
            mix: [-5, 9],
            voice_output: -1234,
            phases_recorded: true,
            ..Default::default()
        };
        d.fir[3] = [-100, 200];
        d.voices[5] = VoiceInside {
            block: 0x8124,
            data_byte: 5,
            ring_at: 8,
            pitch_counter: 0x5ABC,
            key_on_wait: 2,
            phase: 3,
            envelope: 0x7FF,
            unclamped: -3,
            envx: 0x7F,
            ring: std::array::from_fn(|k| k as i16 * -300),
        };
        assert_eq!(DspInside::decode(&d.encode()), d);
    }
}
