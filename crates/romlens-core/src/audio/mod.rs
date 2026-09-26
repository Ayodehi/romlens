//! The sound side read for a student (docs/23, A5): what the eight voices
//! are doing, what each part of audio RAM holds, the notes over time, and
//! the two CPUs' messages. Everything here reads the hardware (the DSP's
//! registers, the sample directory, the ports), not a particular driver,
//! so it works for any game.

pub mod aram_map;
pub mod notes;
pub mod ports;
pub mod voices;

pub use aram_map::{AramPart, DirEntry, PartKind, aram_map, directory};
pub use notes::{NoteEvent, NoteKind, note_name, sample_tuning, timeline};
pub use ports::{PortMessage, port_messages};
pub use voices::{Voice, voices};
