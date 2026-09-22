//! One-frame recordings from loose memory dumps: the path that always works,
//! whatever produced the dumps (`13-recording-format.md`, "Savestate
//! import"). Mesen2 `.mss` parsing is later and optional.

use crate::recording::{MachineState, RecordingError, StateRegion};

/// Build a state from dumps. Each must be exactly its region's size, except
/// that a 512-byte OAM dump (the low table alone) is accepted with the high
/// table zeroed, since some tools export only that.
pub fn state_from_dumps(dumps: &[(StateRegion, Vec<u8>)]) -> Result<MachineState, RecordingError> {
    let mut state = MachineState::default();
    for (r, bytes) in dumps {
        let mut bytes = bytes.clone();
        if *r == StateRegion::Oam && bytes.len() == 512 {
            bytes.resize(544, 0);
        }
        if bytes.len() != r.size() {
            return Err(RecordingError::BadFormat(format!(
                "a {} dump is {} bytes, not {}",
                r.name(),
                bytes.len(),
                r.size()
            )));
        }
        state.regions.insert(*r, bytes);
    }
    if state.regions.is_empty() {
        return Err(RecordingError::BadFormat(
            "give at least one of --vram, --cgram, --oam, --wram or --ppu".to_owned(),
        ));
    }
    Ok(state)
}
