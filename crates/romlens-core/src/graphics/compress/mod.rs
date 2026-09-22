//! Graphics compression formats. Super Metroid's is the only one in Phase 2:
//! it is what makes ROM → tiles work on the development ROM without an
//! emulator, and a successful decompression is the strongest evidence the
//! compressed-data heuristic can have.

pub mod sm_lz;
