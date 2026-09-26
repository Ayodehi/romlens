//! Notes over time, from the DSP writes a recording (or later, the
//! emulator) makes: a key-on starts a voice's note with its pitch and
//! sample, a pitch write while it plays bends it, a key-off ends it.
//!
//! A pitch says how fast a sample plays, not which note it is: `$1000`
//! plays it at its own rate. Which note that is depends on the sample, so
//! [`sample_tuning`] estimates the frequency a sample makes at `$1000` from
//! its loop, and [`note_name`] names the result. It is an estimate and is
//! always shown as one.

use crate::dsp::brr::decode_sample;
use crate::recording::{MachineStateSource, RecordingError, StateRegion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    /// Keyed on: the note starts, its envelope from silence.
    On,
    /// A new pitch while it plays: a slide or vibrato.
    Pitch,
    /// Keyed off: the note fades out.
    Off,
}

impl NoteKind {
    pub const fn name(self) -> &'static str {
        match self {
            NoteKind::On => "on",
            NoteKind::Pitch => "pitch",
            NoteKind::Off => "off",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteEvent {
    pub voice: u8,
    pub frame: u64,
    pub spc_cycle: u64,
    pub kind: NoteKind,
    pub pitch: u16,
    pub source: u8,
}

/// The note events of frames `from..=to`. A voice's pitch and sample start
/// from the DSP's registers at the end of the frame before.
pub fn timeline(
    src: &dyn MachineStateSource,
    from: u64,
    to: u64,
) -> Result<Vec<NoteEvent>, RecordingError> {
    let start = if from > 0 { from - 1 } else { 0 };
    let dsp = src.region_at(start, StateRegion::DspRegisters)?;
    let mut pitch: [u16; 8] =
        std::array::from_fn(|v| u16::from_le_bytes([dsp[v << 4 | 2], dsp[v << 4 | 3]]) & 0x3FFF);
    let mut source: [u8; 8] = std::array::from_fn(|v| dsp[v << 4 | 4]);
    let mut on = [false; 8];
    let mut koff = if from > 0 { dsp[0x5C] } else { 0 };
    let mut out = Vec::new();
    for frame in from..=to {
        let Some(events) = src.apu_events(frame)? else {
            continue;
        };
        for w in events.dsp_writes() {
            let (hi, lo) = ((w.register >> 4) as usize, w.register & 0xF);
            let event =
                |voice: usize, kind: NoteKind, pitch: &[u16; 8], source: &[u8; 8]| NoteEvent {
                    voice: voice as u8,
                    frame,
                    spc_cycle: w.spc_cycle,
                    kind,
                    pitch: pitch[voice],
                    source: source[voice],
                };
            match (hi, lo) {
                (v, 2) => pitch[v] = (pitch[v] & 0x3F00) | w.value as u16,
                (v, 3) => {
                    pitch[v] = (pitch[v] & 0xFF) | ((w.value as u16 & 0x3F) << 8);
                    if on[v] {
                        out.push(event(v, NoteKind::Pitch, &pitch, &source));
                    }
                }
                (v, 4) => source[v] = w.value,
                (4, 0xC) => {
                    for (v, playing) in on.iter_mut().enumerate() {
                        if w.value & (1 << v) != 0 {
                            *playing = true;
                            out.push(event(v, NoteKind::On, &pitch, &source));
                        }
                    }
                }
                (5, 0xC) => {
                    for (v, playing) in on.iter_mut().enumerate() {
                        let bit = 1 << v;
                        if w.value & bit != 0 && koff & bit == 0 && *playing {
                            *playing = false;
                            out.push(event(v, NoteKind::Off, &pitch, &source));
                        }
                    }
                    koff = w.value;
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// The frequency a sample makes at pitch `$1000`, estimated from its loop:
/// the shortest lag at which the looped samples repeat. `None` for a
/// sample that does not loop, or whose loop is nearly flat.
pub fn sample_tuning(aram: &[u8], start: u16, loop_at: u16) -> Option<f64> {
    let s = decode_sample(aram, start as u32, Some(loop_at as u32), 4096);
    if !s.loops || s.unterminated {
        return None;
    }
    let first = s.loop_block?;
    let looped: Vec<f64> = s.blocks[first..]
        .iter()
        .flat_map(|b| b.samples())
        .map(|v| v as f64)
        .collect();
    let n = looped.len();
    if n < 4 {
        return None;
    }
    // Without its constant part; a loop that barely moves holds no wave,
    // only a tail held for the envelope to fade.
    let mean = looped.iter().sum::<f64>() / n as f64;
    let looped: Vec<f64> = looped.iter().map(|v| v - mean).collect();
    let energy: f64 = looped.iter().map(|v| v * v).sum();
    if (energy / n as f64).sqrt() < 64.0 {
        return None;
    }
    // The loop is seamless, so it holds a whole number of periods: n / k.
    // YIN's measure finds the fundamental: each lag's difference against
    // the mean difference of the lags before it, so a smooth wave, which
    // barely differs a sample on, still scores near 1 there and near 0
    // only at its period. The first lag under 0.1, at its dip, is taken,
    // then rounded to the loop's nearest whole division.
    let n_max = n.min(4096);
    let diff = |lag: usize| -> f64 {
        (0..n)
            .map(|i| {
                let e = looped[i] - looped[(i + lag) % n];
                e * e
            })
            .sum()
    };
    let d: Vec<f64> = (0..n_max)
        .map(|lag| if lag == 0 { 0.0 } else { diff(lag) })
        .collect();
    let mut running = 0.0;
    let mut normalized = vec![1.0; n_max];
    for lag in 1..n_max {
        running += d[lag];
        normalized[lag] = if running > 0.0 {
            d[lag] * lag as f64 / running
        } else {
            1.0
        };
    }
    let mut lag = None;
    let mut i = 2;
    while i < n_max {
        if normalized[i] < 0.1 {
            while i + 1 < n_max && normalized[i + 1] < normalized[i] {
                i += 1;
            }
            lag = Some(i);
            break;
        }
        i += 1;
    }
    let periods = match lag {
        Some(l) => (n as f64 / l as f64).round().max(1.0),
        None => 1.0,
    };
    let period = n as f64 / periods;
    Some(32000.0 / period)
}

/// A frequency as a note name and cents: `A4 +3`.
pub fn note_name(hz: f64) -> String {
    const NAMES: [&str; 12] = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    if hz <= 0.0 {
        return "–".to_owned();
    }
    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    let near = midi.round();
    let cents = ((midi - near) * 100.0).round() as i32;
    let n = near as i32;
    let name = NAMES[n.rem_euclid(12) as usize];
    let octave = n.div_euclid(12) - 1;
    if cents == 0 {
        format!("{name}{octave}")
    } else {
        format!("{name}{octave} {cents:+}")
    }
}
