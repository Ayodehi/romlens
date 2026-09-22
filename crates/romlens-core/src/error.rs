//! Error types. Kept free of platform details so the FFI crate can map them
//! onto its own flat error enum.

use std::io;

use crate::memory::address::SnesAddress;

/// Failures while loading or interpreting a ROM image.
#[derive(Debug, thiserror::Error)]
pub enum RomError {
    #[error("could not read ROM: {0}")]
    Io(#[from] io::Error),
    /// Fewer bytes than the smallest header slot needs.
    #[error("ROM is too small ({len} bytes); the smallest LoROM image is 32 KB")]
    TooSmall { len: usize },
    /// Larger than the address space can map.
    #[error("ROM is too large ({len} bytes); ExHiROM tops out at 8 MB")]
    TooLarge { len: usize },
    /// No header slot scored above the acceptance floor.
    #[error("no valid SNES header found at 0x7FC0, 0xFFC0 or 0x40FFC0")]
    NoValidHeader,
    /// The header names a mapping (special chip) Phase 0 does not model.
    #[error("unsupported map mode ${0:02X}")]
    UnsupportedMapping(u8),
}

/// Failures while parsing or resolving an address expression.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("enter an address such as $80:841C or 0x41C")]
    Empty,
    #[error("not a hexadecimal address: {0:?}")]
    InvalidHex(String),
    #[error("address is out of range: {0:?}")]
    OutOfRange(String),
    /// A valid CPU address that does not map onto the ROM.
    #[error("{0} is not mapped to ROM")]
    Unmapped(SnesAddress),
    /// A file offset past the end of the image.
    #[error("file offset 0x{0:06X} is past the end of the ROM ({1} bytes)")]
    PastEnd(u32, usize),
}

/// Failures while editing or storing a project.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectError {
    #[error("could not access project: {0}")]
    Io(String),
    #[error("{file}: {msg}")]
    Json { file: String, msg: String },
    #[error("project is missing {0}")]
    MissingFile(String),
    #[error("not a Romlens project ({0})")]
    BadFormat(String),
    #[error("project was written by a newer Romlens (format version {0})")]
    NewerVersion(u32),
    #[error("project belongs to a different ROM (expected SHA-256 {expected}, found {found})")]
    RomMismatch { expected: String, found: String },
    #[error("{0}")]
    InvalidLabelName(String),
    #[error("range {0} is outside the ROM or empty")]
    BadRange(String),
}

impl From<io::Error> for ProjectError {
    fn from(e: io::Error) -> Self {
        ProjectError::Io(e.to_string())
    }
}
