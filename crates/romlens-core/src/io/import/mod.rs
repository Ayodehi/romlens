//! Reading what other tools know about a ROM.

pub mod cdl;
pub mod symbols;
pub mod usage_map;

use crate::error::ProjectError;
use crate::model::coverage::Coverage;
use crate::rom::image::RomImage;

/// The trace formats this reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceFormat {
    /// Mesen2 `.cdl`.
    Cdl,
    /// bsnes-plus usage map.
    UsageMap,
}

impl TraceFormat {
    pub const fn name(self) -> &'static str {
        match self {
            TraceFormat::Cdl => "cdl",
            TraceFormat::UsageMap => "usage",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "cdl" => Some(TraceFormat::Cdl),
            "usage" | "usagemap" | "usage-map" => Some(TraceFormat::UsageMap),
            _ => None,
        }
    }
}

/// Identify a trace file by its contents and its size against the ROM.
///
/// Neither format carries a version or a type beyond the CDL's optional magic,
/// so detection is: a CDL says so, a usage map is the only thing 16 MB long,
/// and a bare file the size of the ROM is a header-less CDL.
pub fn detect(bytes: &[u8], rom: &RomImage) -> Result<TraceFormat, ProjectError> {
    let rom_len = rom.len() as u32;
    if bytes.starts_with(cdl::MAGIC) {
        return Ok(TraceFormat::Cdl);
    }
    if usage_map::looks_like(bytes) {
        return Ok(TraceFormat::UsageMap);
    }
    if bytes.len() as u32 == rom_len {
        return Ok(TraceFormat::Cdl);
    }
    Err(ProjectError::BadFormat(format!(
        "{} bytes is neither a Mesen2 CDL for this {rom_len}-byte ROM nor a bsnes-plus usage map. \
DiztinGUIsh projects are not read directly: export a bsnes usage map or a WLA `.sym` from Diz",
        bytes.len()
    )))
}

/// Read a trace, detecting the format when one is not given.
pub fn read_trace(
    bytes: &[u8],
    rom: &RomImage,
    format: Option<TraceFormat>,
) -> Result<(TraceFormat, Coverage), ProjectError> {
    let format = match format {
        Some(f) => f,
        None => detect(bytes, rom)?,
    };
    let coverage = match format {
        TraceFormat::Cdl => cdl::read(bytes, rom.len() as u32)?,
        TraceFormat::UsageMap => usage_map::read(bytes, rom)?,
    };
    Ok((format, coverage))
}

/// The folded form a project stores: one byte per ROM byte, whatever the
/// source was. A usage map is 16.8 MB of mostly nothing and a project must not
/// carry that (`12-content-policy.md` keeps packages small and ROM-free), so
/// everything is normalised to the CDL payload shape on the way in.
pub fn to_stored(coverage: &Coverage) -> Vec<u8> {
    cdl::write(coverage, 0)
}

pub fn from_stored(bytes: &[u8], rom_len: u32) -> Result<Coverage, ProjectError> {
    cdl::read(bytes, rom_len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    fn rom() -> RomImage {
        RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap()
    }

    #[test]
    fn detects_each_format() {
        let rom = rom();
        let mut headed = cdl::MAGIC.to_vec();
        headed.extend_from_slice(&[0u8; 4]);
        headed.extend_from_slice(&vec![0u8; rom.len()]);
        assert_eq!(detect(&headed, &rom).unwrap(), TraceFormat::Cdl);
        assert_eq!(
            detect(&vec![0u8; rom.len()], &rom).unwrap(),
            TraceFormat::Cdl,
            "a bare file the size of the ROM is a header-less CDL"
        );
        assert_eq!(
            detect(&vec![0u8; usage_map::CPU_BLOCK], &rom).unwrap(),
            TraceFormat::UsageMap
        );
        let err = detect(&[0u8; 100], &rom).unwrap_err();
        assert!(format!("{err}").contains("DiztinGUIsh"), "{err}");
    }

    #[test]
    fn a_usage_map_stores_as_one_byte_per_rom_byte() {
        let rom = rom();
        let mut map = vec![0u8; usage_map::CPU_BLOCK];
        map[0x00_8000] = usage_map::EXEC | usage_map::OPCODE;
        map[0x00_8005] = usage_map::READ;
        let (format, coverage) = read_trace(&map, &rom, None).unwrap();
        assert_eq!(format, TraceFormat::UsageMap);
        let stored = to_stored(&coverage);
        assert_eq!(stored.len(), cdl::HEADER_LEN + rom.len());
        assert_eq!(from_stored(&stored, rom.len() as u32).unwrap(), coverage);
    }

    #[test]
    fn format_names_round_trip() {
        for f in [TraceFormat::Cdl, TraceFormat::UsageMap] {
            assert_eq!(TraceFormat::parse(f.name()), Some(f));
        }
        assert_eq!(TraceFormat::parse("diz"), None);
    }
}
