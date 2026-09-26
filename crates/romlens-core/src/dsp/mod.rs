//! The S-DSP, the sound chip (docs/23): its sample format, and the chip
//! itself.

pub mod brr;
pub mod chip;
pub mod gauss;

pub use chip::{Dsp, EnvelopeMode, Frame, Voice};
