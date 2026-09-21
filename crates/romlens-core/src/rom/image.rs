//! A loaded ROM: payload, detected mapping, parsed header, identity.

use std::path::Path;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::error::{AddressError, RomError};
use crate::memory::address::{FileOffset, SnesAddress};
use crate::memory::map::{AddressMap, MappingMode};
use crate::memory::parse::{AddressExpr, parse_address_expr};
use crate::rom::checksum::compute_checksum;
use crate::rom::copier::{COPIER_HEADER_LEN, split_copier_header};
use crate::rom::header::{RomHeader, Vectors};
use crate::rom::scorer::detect_mapping;

/// Smallest image with a complete LoROM header and vector table.
pub const MIN_ROM_LEN: usize = 0x8000;
/// Largest image the ExHiROM map can address.
pub const MAX_ROM_LEN: usize = 0x80_0000;

/// A loaded, identified ROM image. Cheap to clone (the payload is shared).
#[derive(Debug, Clone)]
pub struct RomImage {
    payload: Arc<[u8]>,
    copier_header: Option<Arc<[u8]>>,
    header_offset: FileOffset,
    header: RomHeader,
    mapping: MappingMode,
    map: AddressMap,
    sha256: [u8; 32],
    computed_checksum: u16,
    source_name: String,
}

/// Plain facts about an image, shared by the CLI and the FFI records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomInfo {
    pub source_name: String,
    pub byte_len: u32,
    pub row_count: u32,
    pub has_copier_header: bool,
    pub mapping: MappingMode,
    pub fast_rom: bool,
    pub header_offset: FileOffset,
    pub header: RomHeader,
    pub computed_checksum: u16,
    pub checksum_ok: bool,
    pub sha256: String,
}

/// An address expression resolved against one image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    pub file_offset: FileOffset,
    /// Canonical CPU address, `None` for the rare unreachable ExHiROM byte.
    pub snes_address: Option<SnesAddress>,
    pub row: u32,
}

impl RomImage {
    /// Read and identify a `.sfc`/`.smc` file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, RomError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self::from_bytes(bytes, name)
    }

    /// Identify an in-memory dump. `source_name` is a display name (a file
    /// name, never a path, so CLI output is the same on every OS).
    pub fn from_bytes(bytes: Vec<u8>, source_name: impl Into<String>) -> Result<Self, RomError> {
        let (copier, payload) = split_copier_header(&bytes);
        if payload.len() < MIN_ROM_LEN {
            return Err(RomError::TooSmall { len: bytes.len() });
        }
        if payload.len() > MAX_ROM_LEN {
            return Err(RomError::TooLarge { len: bytes.len() });
        }
        let best = detect_mapping(payload)?;
        let sha256: [u8; 32] = Sha256::digest(payload).into();
        let computed_checksum = compute_checksum(payload);
        let copier_header = copier.map(Arc::from);
        let payload: Arc<[u8]> = Arc::from(payload);
        let map = AddressMap::new(best.mode, payload.len() as u32, best.header.is_fast_rom());
        Ok(Self {
            payload,
            copier_header,
            header_offset: best.mode.header_offset(),
            header: best.header,
            mapping: best.mode,
            map,
            sha256,
            computed_checksum,
            source_name: source_name.into(),
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.payload
    }

    pub fn payload_arc(&self) -> Arc<[u8]> {
        Arc::clone(&self.payload)
    }

    pub fn len(&self) -> usize {
        self.payload.len()
    }

    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }

    /// Number of 16-byte hex rows, the last possibly partial.
    pub fn row_count(&self) -> u32 {
        self.payload.len().div_ceil(16) as u32
    }

    pub fn has_copier_header(&self) -> bool {
        self.copier_header.is_some()
    }

    pub fn copier_header(&self) -> Option<&[u8]> {
        self.copier_header.as_deref()
    }

    /// Bytes to add to a payload offset to get the on-disk offset.
    pub fn disk_offset_delta(&self) -> u32 {
        if self.copier_header.is_some() {
            COPIER_HEADER_LEN as u32
        } else {
            0
        }
    }

    pub fn header_offset(&self) -> FileOffset {
        self.header_offset
    }

    pub fn header(&self) -> &RomHeader {
        &self.header
    }

    pub fn mapping(&self) -> MappingMode {
        self.mapping
    }

    pub fn map(&self) -> &AddressMap {
        &self.map
    }

    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    pub fn sha256(&self) -> &[u8; 32] {
        &self.sha256
    }

    pub fn sha256_hex(&self) -> String {
        self.sha256.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn computed_checksum(&self) -> u16 {
        self.computed_checksum
    }

    /// The header's checksum matches the mirrored sum and its complement.
    pub fn checksum_ok(&self) -> bool {
        self.header.complement_valid() && self.header.checksum == self.computed_checksum
    }

    pub fn native_vectors(&self) -> &Vectors {
        &self.header.native
    }

    pub fn emulation_vectors(&self) -> &Vectors {
        &self.header.emulation
    }

    pub fn info(&self) -> RomInfo {
        RomInfo {
            source_name: self.source_name.clone(),
            byte_len: self.payload.len() as u32,
            row_count: self.row_count(),
            has_copier_header: self.copier_header.is_some(),
            mapping: self.mapping,
            fast_rom: self.header.is_fast_rom(),
            header_offset: self.header_offset,
            header: self.header.clone(),
            computed_checksum: self.computed_checksum,
            checksum_ok: self.checksum_ok(),
            sha256: self.sha256_hex(),
        }
    }

    /// File offset for a CPU address under this image's mapping.
    pub fn file_offset_for(&self, addr: SnesAddress) -> Option<FileOffset> {
        self.map.file_offset(addr)
    }

    /// Canonical CPU address for a file offset.
    pub fn snes_address_for(&self, off: FileOffset) -> Option<SnesAddress> {
        self.map.canonical(off)
    }

    pub fn mirrors(&self, off: FileOffset) -> Vec<SnesAddress> {
        self.map.mirrors(off)
    }

    /// Parse and resolve an address expression against this image.
    pub fn resolve(&self, text: &str) -> Result<Resolved, AddressError> {
        let off = match parse_address_expr(text)? {
            AddressExpr::File(off) => {
                if off.as_usize() >= self.len() {
                    return Err(AddressError::PastEnd(off.0, self.len()));
                }
                off
            }
            AddressExpr::Snes(addr) => self
                .file_offset_for(addr)
                .ok_or(AddressError::Unmapped(addr))?,
        };
        Ok(Resolved {
            file_offset: off,
            snes_address: self.snes_address_for(off),
            row: off.row(),
        })
    }
}
