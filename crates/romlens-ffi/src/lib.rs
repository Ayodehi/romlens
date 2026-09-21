//! The public API of the core, as one UniFFI object plus a few free
//! functions. Records and enums mirror core types through `From` impls so the
//! core never depends on `uniffi`. Hot paths (`hex_rows`) return flat buffers
//! (docs/10); everything else returns typed records.

use std::sync::Arc;

use romlens_core::{
    AddressError, FileOffset, MappingMode, RomError, RomImage, SnesAddress, SpanIndex, encode_rows,
    fixtures, header_spans, interpret,
};

uniffi::setup_scaffolding!();

/// Flat error surface for shells.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum RomlensError {
    #[error("{msg}")]
    Io { msg: String },
    #[error("{msg}")]
    InvalidRom { msg: String },
    #[error("{msg}")]
    BadAddress { msg: String },
}

impl From<RomError> for RomlensError {
    fn from(e: RomError) -> Self {
        match e {
            RomError::Io(_) => RomlensError::Io { msg: e.to_string() },
            _ => RomlensError::InvalidRom { msg: e.to_string() },
        }
    }
}

impl From<AddressError> for RomlensError {
    fn from(e: AddressError) -> Self {
        RomlensError::BadAddress { msg: e.to_string() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Mapping {
    LoRom,
    HiRom,
    ExHiRom,
}

impl From<MappingMode> for Mapping {
    fn from(m: MappingMode) -> Self {
        match m {
            MappingMode::LoRom => Mapping::LoRom,
            MappingMode::HiRom => Mapping::HiRom,
            MappingMode::ExHiRom => Mapping::ExHiRom,
        }
    }
}

impl From<Mapping> for MappingMode {
    fn from(m: Mapping) -> Self {
        match m {
            Mapping::LoRom => MappingMode::LoRom,
            Mapping::HiRom => MappingMode::HiRom,
            Mapping::ExHiRom => MappingMode::ExHiRom,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Vectors {
    pub cop: u16,
    pub brk: u16,
    pub abort: u16,
    pub nmi: u16,
    pub reset: u16,
    pub irq: u16,
}

impl From<&romlens_core::Vectors> for Vectors {
    fn from(v: &romlens_core::Vectors) -> Self {
        Self {
            cop: v.cop,
            brk: v.brk,
            abort: v.abort,
            nmi: v.nmi,
            reset: v.reset,
            irq: v.irq,
        }
    }
}

/// Everything the header summary shows. Addresses are file offsets or
/// 24-bit SNES addresses as plain integers; format them with
/// [`format_file_offset`] and [`format_snes_address`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RomInfo {
    pub file_name: String,
    pub byte_len: u32,
    pub row_count: u32,
    pub has_copier_header: bool,
    pub mapping: Mapping,
    pub mapping_name: String,
    pub fast_rom: bool,
    pub header_offset: u32,
    pub title: String,
    pub map_mode: u8,
    pub cartridge_type: u8,
    pub cartridge_type_name: String,
    pub rom_size_code: u8,
    pub declared_rom_size: u64,
    pub ram_size_code: u8,
    pub declared_ram_size: u64,
    pub region: u8,
    pub region_name: String,
    pub developer_id: u8,
    pub version: u8,
    pub complement: u16,
    pub checksum: u16,
    pub complement_valid: bool,
    pub computed_checksum: u16,
    pub checksum_ok: bool,
    pub native: Vectors,
    pub emulation: Vectors,
    pub sha256: String,
}

impl From<romlens_core::RomInfo> for RomInfo {
    fn from(i: romlens_core::RomInfo) -> Self {
        let h = &i.header;
        Self {
            file_name: i.source_name.clone(),
            byte_len: i.byte_len,
            row_count: i.row_count,
            has_copier_header: i.has_copier_header,
            mapping: i.mapping.into(),
            mapping_name: i.mapping.name().to_owned(),
            fast_rom: i.fast_rom,
            header_offset: i.header_offset.value(),
            title: h.title.clone(),
            map_mode: h.map_mode,
            cartridge_type: h.cartridge_type,
            cartridge_type_name: h.cartridge_type_name().to_owned(),
            rom_size_code: h.rom_size_code,
            declared_rom_size: h.declared_rom_size(),
            ram_size_code: h.ram_size_code,
            declared_ram_size: h.declared_ram_size(),
            region: h.region,
            region_name: h.region_name().to_owned(),
            developer_id: h.developer_id,
            version: h.version,
            complement: h.complement,
            checksum: h.checksum,
            complement_valid: h.complement_valid(),
            computed_checksum: i.computed_checksum,
            checksum_ok: i.checksum_ok,
            native: (&h.native).into(),
            emulation: (&h.emulation).into(),
            sha256: i.sha256,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SpanKind {
    Title,
    MapMode,
    CartridgeType,
    RomSize,
    RamSize,
    Region,
    DeveloperId,
    Version,
    ChecksumComplement,
    Checksum,
    NativeVector,
    EmulationVector,
    ExtendedHeader,
}

impl From<romlens_core::SpanKind> for SpanKind {
    fn from(k: romlens_core::SpanKind) -> Self {
        use romlens_core::SpanKind as K;
        match k {
            K::Title => SpanKind::Title,
            K::MapMode => SpanKind::MapMode,
            K::CartridgeType => SpanKind::CartridgeType,
            K::RomSize => SpanKind::RomSize,
            K::RamSize => SpanKind::RamSize,
            K::Region => SpanKind::Region,
            K::DeveloperId => SpanKind::DeveloperId,
            K::Version => SpanKind::Version,
            K::ChecksumComplement => SpanKind::ChecksumComplement,
            K::Checksum => SpanKind::Checksum,
            K::NativeVector => SpanKind::NativeVector,
            K::EmulationVector => SpanKind::EmulationVector,
            K::ExtendedHeader => SpanKind::ExtendedHeader,
        }
    }
}

/// A named byte range the hex view overlays. `id` matches the per-byte span
/// ids in a hex-row batch.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Span {
    pub id: u32,
    pub start: u32,
    pub len: u32,
    pub kind: SpanKind,
    pub name: String,
    pub value_text: String,
    /// 24-bit SNES address a vector points at, when it maps to ROM.
    pub target: Option<u32>,
}

impl From<&romlens_core::Span> for Span {
    fn from(s: &romlens_core::Span) -> Self {
        Self {
            id: s.id,
            start: s.start.value(),
            len: s.len,
            kind: s.kind.into(),
            name: s.name.clone(),
            value_text: s.value_text.clone(),
            target: s.target.map(SnesAddress::as_u24),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ByteInterpretation {
    pub file_offset: u32,
    pub disk_offset: u32,
    pub snes_address: Option<u32>,
    pub mirrors: Vec<u32>,
    pub value_u8: u8,
    pub value_i8: i8,
    pub value_u16_le: Option<u16>,
    pub value_i16_le: Option<i16>,
    pub value_u24_le: Option<u32>,
    pub u16_as_address_in_bank: Option<u32>,
    pub u24_as_snes_address: Option<u32>,
    pub pointer_target_file_offset: Option<u32>,
    pub pointer_target_region: Option<String>,
    pub ascii: Option<String>,
    pub span_name: Option<String>,
    pub span_value: Option<String>,
}

impl From<romlens_core::ByteInterpretation> for ByteInterpretation {
    fn from(b: romlens_core::ByteInterpretation) -> Self {
        Self {
            file_offset: b.file_offset.value(),
            disk_offset: b.disk_offset,
            snes_address: b.snes_address.map(SnesAddress::as_u24),
            mirrors: b.mirrors.iter().map(|a| a.as_u24()).collect(),
            value_u8: b.u8,
            value_i8: b.i8,
            value_u16_le: b.u16_le,
            value_i16_le: b.i16_le,
            value_u24_le: b.u24_le,
            u16_as_address_in_bank: b.u16_as_address_in_bank.map(SnesAddress::as_u24),
            u24_as_snes_address: b.u24_as_snes_address.map(SnesAddress::as_u24),
            pointer_target_file_offset: b.pointer_target_file_offset.map(FileOffset::value),
            pointer_target_region: b.pointer_target_region.map(|r| format!("{r:?}")),
            ascii: b.ascii.map(String::from),
            span_name: b.span_name,
            span_value: b.span_value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ResolvedAddress {
    pub file_offset: u32,
    pub snes_address: Option<u32>,
    pub row: u32,
}

/// A loaded ROM. Immutable after construction, so it is safe to share
/// between threads; the shell keeps one per document.
#[derive(uniffi::Object)]
pub struct Rom {
    image: RomImage,
    spans: Vec<romlens_core::Span>,
    index: SpanIndex,
}

impl Rom {
    fn wrap(image: RomImage) -> Arc<Self> {
        let spans = header_spans(&image);
        let index = SpanIndex::new(&spans);
        Arc::new(Self {
            image,
            spans,
            index,
        })
    }
}

#[uniffi::export]
impl Rom {
    /// Read and identify a file.
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, RomlensError> {
        Ok(Self::wrap(RomImage::load(path)?))
    }

    /// Identify bytes the shell already read (the `NSDocument` path).
    #[uniffi::constructor]
    pub fn from_bytes(bytes: Vec<u8>, name: String) -> Result<Arc<Self>, RomlensError> {
        Ok(Self::wrap(RomImage::from_bytes(bytes, name)?))
    }

    pub fn info(&self) -> RomInfo {
        self.image.info().into()
    }

    pub fn row_count(&self) -> u32 {
        self.image.row_count()
    }

    pub fn byte_len(&self) -> u32 {
        self.image.len() as u32
    }

    /// Hex rows `[start_row, start_row + count)` as a flat batch; see the
    /// core's `viewmodel::hex_rows` for the layout.
    pub fn hex_rows(&self, start_row: u32, count: u32) -> Vec<u8> {
        encode_rows(&self.image, &self.index, start_row, count)
    }

    pub fn spans(&self) -> Vec<Span> {
        self.spans.iter().map(Span::from).collect()
    }

    pub fn inspect(&self, file_offset: u32) -> Option<ByteInterpretation> {
        interpret(&self.image, &self.spans, FileOffset(file_offset)).map(Into::into)
    }

    /// Parse and resolve `$80:841C`, `80841C` or `0x41C`.
    pub fn resolve(&self, text: String) -> Result<ResolvedAddress, RomlensError> {
        let r = self.image.resolve(&text)?;
        Ok(ResolvedAddress {
            file_offset: r.file_offset.value(),
            snes_address: r.snes_address.map(SnesAddress::as_u24),
            row: r.row,
        })
    }

    pub fn file_offset_for(&self, snes_address: u32) -> Option<u32> {
        self.image
            .file_offset_for(SnesAddress::from_u24(snes_address))
            .map(FileOffset::value)
    }

    pub fn snes_address_for(&self, file_offset: u32) -> Option<u32> {
        self.image
            .snes_address_for(FileOffset(file_offset))
            .map(SnesAddress::as_u24)
    }

    pub fn mirrors(&self, file_offset: u32) -> Vec<u32> {
        self.image
            .mirrors(FileOffset(file_offset))
            .iter()
            .map(|a| a.as_u24())
            .collect()
    }
}

/// Semantic version of the core API the library was built from.
#[uniffi::export]
pub fn api_version() -> String {
    romlens_core::API_VERSION.to_owned()
}

/// Bytes per record in a hex-row batch.
#[uniffi::export]
pub fn hex_row_stride() -> u16 {
    romlens_core::ROW_STRIDE
}

/// Bytes before the first record in a hex-row batch.
#[uniffi::export]
pub fn hex_batch_header_len() -> u16 {
    romlens_core::BATCH_HEADER_LEN as u16
}

/// One of the homebrew test ROMs the test suite assembles.
#[uniffi::export]
pub fn make_test_rom(mapping: Mapping) -> Vec<u8> {
    fixtures::for_mapping(mapping.into())
}

/// `$80:841C`
#[uniffi::export]
pub fn format_snes_address(address: u32) -> String {
    SnesAddress::from_u24(address).to_string()
}

/// `0x00041C`
#[uniffi::export]
pub fn format_file_offset(offset: u32) -> String {
    FileOffset(offset).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_round_trip() {
        let rom = Rom::from_bytes(make_test_rom(Mapping::LoRom), "t.sfc".into()).unwrap();
        assert_eq!(rom.info().title, "ROMLENS TEST");
        assert_eq!(rom.hex_rows(0, 4).len(), 8 + 4 * 64);
        assert_eq!(rom.resolve("$00:8000".into()).unwrap().file_offset, 0);
        assert!(matches!(
            rom.resolve("$7E:0000".into()),
            Err(RomlensError::BadAddress { .. })
        ));
        assert_eq!(rom.spans().len(), 22);
        assert_eq!(rom.inspect(0).unwrap().value_u8, 0x78);
        assert_eq!(format_snes_address(0x80841C), "$80:841C");
        assert_eq!(format_file_offset(0x41C), "0x00041C");
    }
}
