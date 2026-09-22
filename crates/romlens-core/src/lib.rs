//! Romlens core: SNES ROM loading, memory mapping, header parsing and the
//! presentation-ready view models shared by the CLI, the FFI crate and every
//! shell. No UI, no platform types, no I/O beyond reading a ROM file.
//!
//! Phase 0 scope ("open and see"): copier-header detection, LoROM / HiROM /
//! ExHiROM detection by header scoring, header and vector parsing, the
//! mirrored-sum checksum, file-offset ↔ SNES-address translation, hex-row
//! batches, header spans and the byte inspector.

pub mod analysis;
pub mod cpu65816;
pub mod error;
pub mod fixtures;
pub mod io;
pub mod memory;
pub mod model;
pub mod rom;
pub mod viewmodel;

pub use error::{AddressError, ProjectError, RomError};
pub use memory::address::{FileOffset, SnesAddress};
pub use memory::map::{AddressMap, MappingMode, MemoryClass, mirror_offset};
pub use memory::parse::{AddressExpr, parse_address_expr};
pub use rom::header::{ExtendedHeader, RomHeader, Vectors};
pub use rom::image::{Resolved, RomImage, RomInfo};
pub use viewmodel::hex_rows::{
    AddressStyle, BATCH_HEADER_LEN, BYTES_PER_ROW, ROW_STRIDE, ROW_VERSION, encode_rows,
    format_rows_text,
};
pub use viewmodel::inspector::{ByteInterpretation, interpret};
pub use viewmodel::spans::{Span, SpanIndex, SpanKind, header_spans};

/// Semantic version of the core's public surface. The FFI crate and the CLI
/// both print it so a shell can pin the version it was built against
/// (docs/08 rule 6).
pub const API_VERSION: &str = "0.2.0";
