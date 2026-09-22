//! [`MemorySource`]: frames held in memory. The test double the conformance
//! suite runs beside [`super::RomrecSource`], and the shape a live source
//! will have.

use crate::recording::delta::{Run, diff, union};
use crate::recording::{
    Layers, MachineState, MachineStateSource, RecordingError, RecordingIdentity, StateRegion,
};

#[derive(Debug, Clone)]
pub struct MemorySource {
    pub identity: RecordingIdentity,
    pub frames: Vec<MachineState>,
}

impl MemorySource {
    pub fn new(identity: RecordingIdentity, frames: Vec<MachineState>) -> Self {
        let frames = frames
            .into_iter()
            .enumerate()
            .map(|(i, mut s)| {
                s.frame = i as u64;
                s
            })
            .collect();
        MemorySource { identity, frames }
    }
}

impl MachineStateSource for MemorySource {
    fn identity(&self) -> &RecordingIdentity {
        &self.identity
    }

    fn frame_count(&self) -> Option<u64> {
        Some(self.frames.len() as u64)
    }

    fn regions(&self) -> Vec<StateRegion> {
        self.frames
            .first()
            .map(|f| f.regions.keys().copied().collect())
            .unwrap_or_default()
    }

    fn state_at(&self, frame: u64) -> Result<MachineState, RecordingError> {
        self.frames
            .get(frame as usize)
            .cloned()
            .ok_or(RecordingError::NoSuchFrame {
                frame,
                count: self.frames.len() as u64,
            })
    }

    fn changes(&self, from: u64, to: u64, region: StateRegion) -> Result<Vec<Run>, RecordingError> {
        self.state_at(to)?;
        let mut out = Vec::new();
        for f in from..to {
            let (a, b) = (&self.frames[f as usize], &self.frames[f as usize + 1]);
            match (a.region(region), b.region(region)) {
                (Some(x), Some(y)) => out = union(&out, &diff(x, y)),
                _ => return Err(RecordingError::MissingRegion(region.name())),
            }
        }
        Ok(out)
    }

    fn layers(&self) -> Layers {
        Layers::default()
    }
}
