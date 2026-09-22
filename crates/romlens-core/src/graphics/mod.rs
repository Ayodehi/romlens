//! SNES graphics formats.
//!
//! Track 2B builds the decoders and views here (`07-memory-to-screen.md`).
//! What exists now is the part track 2A needs: two scoring functions that say
//! how much a byte range looks like a palette or like bitplane tile data. They
//! live here rather than in `analysis::heuristics` so the bitplane rule and the
//! BGR15 expansion are written once — the decoders will read the same
//! constants, and a classifier that disagreed with the decoder would be worse
//! than no classifier.

pub mod palette;
pub mod tile;
