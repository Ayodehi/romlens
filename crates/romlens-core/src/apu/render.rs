//! Sound from a recording (docs/23, A8): the machine from a frame's
//! snapshot, the recording's port writes fed in as they come, and the
//! samples it makes. Romlens plays these and never writes them out
//! (`12-content-policy.md` rule 11); the CLI prints their digest.

use sha2::{Digest, Sha256};

use super::Apu;
use crate::dsp::Frame;
use crate::recording::apu::ApuEventKind;
use crate::recording::{MachineStateSource, RecordingError, SpcState, StateRegion};

/// 32,000 samples a second, each 32 SPC700 cycles.
pub const SAMPLE_RATE: u32 = 32_000;

/// `samples` samples from the snapshot at `frame`. With `follow`, the
/// S-CPU's port writes from the frames after it land on their cycles, so
/// the driver hears the commands it heard; without, it plays on alone.
/// The DSP starts from rest: a note sounding at the snapshot is heard
/// from the next time the driver keys it.
pub fn render(
    src: &dyn MachineStateSource,
    frame: u64,
    samples: usize,
    follow: bool,
) -> Result<Vec<Frame>, RecordingError> {
    let s = src.state_at(frame)?;
    let get = |r: StateRegion| s.region(r).ok_or(RecordingError::MissingRegion(r.name()));
    let spc = SpcState::decode(get(StateRegion::SpcState)?);
    let mut apu = Apu::from_snapshot(
        get(StateRegion::Aram)?,
        get(StateRegion::DspRegisters)?,
        &spc,
    );
    if follow {
        let last = src.frame_count().unwrap_or(0);
        for f in frame..last {
            if let Some(e) = src.apu_events(f)? {
                for e in e.events.iter().filter(|e| e.kind == ApuEventKind::CpuPort) {
                    if e.spc_cycle >= spc.cycle {
                        apu.queue_port(e.spc_cycle, e.address, e.value);
                    }
                }
            }
            // Enough to cover the render.
            if f > frame + 1 + (samples as u64 * 60 / SAMPLE_RATE as u64) {
                break;
            }
        }
    }
    apu.bus.output = Some(Vec::with_capacity(samples));
    while apu.bus.output.as_ref().is_some_and(|o| o.len() < samples) {
        apu.cpu.step(&mut apu.bus);
    }
    let mut out = apu.bus.output.take().unwrap_or_default();
    out.truncate(samples);
    Ok(out)
}

/// The SHA-256 of the samples as 16-bit little-endian pairs, left first.
pub fn digest(frames: &[Frame]) -> String {
    let mut h = Sha256::new();
    for f in frames {
        h.update(f.left.to_le_bytes());
        h.update(f.right.to_le_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
