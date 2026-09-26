//! The eight voices at one moment, from the DSP's registers.

use crate::explain::sound::{describe_dsp, pitch_words};

/// One voice, as its registers set it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voice {
    pub index: u8,
    pub volume: (i8, i8),
    /// 14 bits: `$1000` plays the sample at its own rate.
    pub pitch: u16,
    /// The sample directory entry.
    pub source: u8,
    pub adsr1: u8,
    pub adsr2: u8,
    pub gain: u8,
    /// The envelope now, 0–127.
    pub envx: u8,
    /// The sample now, after the envelope: its high byte.
    pub outx: i8,
    pub echo: bool,
    pub noise: bool,
    /// Its pitch follows the voice before's wave.
    pub modulated: bool,
    /// Its sample reached a block with the end flag.
    pub ended: bool,
    /// KOFF holds it off.
    pub keyed_off: bool,
    /// The directory's start and loop for its sample, when audio RAM is known.
    pub sample: Option<(u16, u16)>,
}

impl Voice {
    /// Heard now: its envelope is above silence.
    pub fn sounding(&self) -> bool {
        self.envx > 0
    }

    /// Its envelope's settings in words: ADSR's four parts, or GAIN's.
    pub fn envelope(&self) -> String {
        let words = |reg: u8, v: u8| {
            describe_dsp(reg, Some(v)).parts[0]
                .summary
                .clone()
                .unwrap_or_default()
        };
        let base = self.index << 4;
        if self.adsr1 & 0x80 != 0 {
            let a1 = words(base | 5, self.adsr1);
            let a1 = a1.trim_start_matches("ADSR, ");
            format!("ADSR: {a1}, {}", words(base | 6, self.adsr2))
        } else {
            format!("GAIN: {}", words(base | 7, self.gain))
        }
    }

    pub fn pitch_words(&self) -> String {
        pitch_words(self.pitch as u32)
    }
}

/// The voices from 128 DSP registers, and their samples from the sample
/// directory when audio RAM is given.
pub fn voices(dsp: &[u8], aram: Option<&[u8]>) -> Vec<Voice> {
    let r = |i: usize| dsp.get(i).copied().unwrap_or(0);
    let bit = |reg: usize, v: u8| r(reg) & (1 << v) != 0;
    let dir = (r(0x5D) as usize) << 8;
    (0..8u8)
        .map(|v| {
            let b = (v as usize) << 4;
            let source = r(b | 4);
            let sample = aram.map(|a| {
                let at = dir + source as usize * 4;
                let w = |i: usize| {
                    u16::from_le_bytes([
                        a.get((at + i) & 0xFFFF).copied().unwrap_or(0),
                        a.get((at + i + 1) & 0xFFFF).copied().unwrap_or(0),
                    ])
                };
                (w(0), w(2))
            });
            Voice {
                index: v,
                volume: (r(b) as i8, r(b | 1) as i8),
                pitch: u16::from_le_bytes([r(b | 2), r(b | 3)]) & 0x3FFF,
                source,
                adsr1: r(b | 5),
                adsr2: r(b | 6),
                gain: r(b | 7),
                envx: r(b | 8) & 0x7F,
                outx: r(b | 9) as i8,
                echo: bit(0x4D, v),
                noise: bit(0x3D, v),
                modulated: v > 0 && bit(0x2D, v),
                ended: bit(0x7C, v),
                keyed_off: bit(0x5C, v),
                sample,
            }
        })
        .collect()
}
