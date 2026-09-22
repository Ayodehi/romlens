//! Regions: what the analyzer (or the user) believes a byte range is.

use crate::memory::address::FileOffset;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataKind {
    Byte,
    Word,
    Long,
    Pointer,
    Table { stride: u8 },
    String,
    Graphics { bpp: u8 },
    Tilemap,
    Palette,
    Compressed,
    Struct,
}

impl DataKind {
    pub const fn name(self) -> &'static str {
        match self {
            DataKind::Byte => "byte",
            DataKind::Word => "word",
            DataKind::Long => "long",
            DataKind::Pointer => "pointer",
            DataKind::Table { .. } => "table",
            DataKind::String => "string",
            DataKind::Graphics { .. } => "graphics",
            DataKind::Tilemap => "tilemap",
            DataKind::Palette => "palette",
            DataKind::Compressed => "compressed",
            DataKind::Struct => "struct",
        }
    }

    /// Parse a CLI/JSON name; `table` and `graphics` take their parameter
    /// separately.
    pub fn parse(name: &str, stride: Option<u8>, bpp: Option<u8>) -> Option<DataKind> {
        Some(match name {
            "byte" => DataKind::Byte,
            "word" => DataKind::Word,
            "long" => DataKind::Long,
            "pointer" => DataKind::Pointer,
            "table" => DataKind::Table {
                stride: stride.unwrap_or(2),
            },
            "string" => DataKind::String,
            "graphics" => DataKind::Graphics {
                bpp: bpp.unwrap_or(4),
            },
            "tilemap" => DataKind::Tilemap,
            "palette" => DataKind::Palette,
            "compressed" => DataKind::Compressed,
            "struct" => DataKind::Struct,
            _ => return None,
        })
    }

    /// Bytes per element for the sized kinds.
    pub const fn element_len(self) -> u32 {
        match self {
            DataKind::Word | DataKind::Pointer | DataKind::Palette | DataKind::Tilemap => 2,
            DataKind::Long => 3,
            DataKind::Table { stride } => stride as u32,
            _ => 1,
        }
    }

    /// The batch encoding (`asm_lines` record byte 10).
    pub const fn code(self) -> u8 {
        match self {
            DataKind::Byte => 2,
            DataKind::Word => 3,
            DataKind::Long => 4,
            DataKind::Pointer => 5,
            DataKind::Table { .. } => 6,
            DataKind::String => 7,
            DataKind::Graphics { .. } => 8,
            DataKind::Tilemap => 9,
            DataKind::Palette => 10,
            DataKind::Compressed => 11,
            DataKind::Struct => 12,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegionKind {
    Unknown,
    Code,
    Data(DataKind),
}

impl RegionKind {
    pub fn name(self) -> &'static str {
        match self {
            RegionKind::Unknown => "unknown",
            RegionKind::Code => "code",
            RegionKind::Data(d) => d.name(),
        }
    }

    /// The batch encoding: 0 unknown, 1 code, 2.. data kinds.
    pub const fn code(self) -> u8 {
        match self {
            RegionKind::Unknown => 0,
            RegionKind::Code => 1,
            RegionKind::Data(d) => d.code(),
        }
    }
}

/// Why the analyzer believes a region is what it is.
#[derive(Debug, Clone, PartialEq)]
pub enum Evidence {
    /// Reached from a vector through `depth` calls.
    VectorReach {
        depth: u32,
    },
    Heuristic {
        name: String,
        score: f32,
    },
    User,
    Imported(String),
    Trace {
        file: String,
        hits: u32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub start: FileOffset,
    pub len: u32,
    pub kind: RegionKind,
    /// 0.0..=1.0
    pub confidence: f32,
    pub evidence: Vec<Evidence>,
}

impl Region {
    pub fn end(&self) -> u32 {
        self.start.0 + self.len
    }

    pub fn contains(&self, off: u32) -> bool {
        off >= self.start.0 && off < self.end()
    }
}

/// A user's classification of a byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverrideKind {
    Code,
    Data(DataKind),
    Unknown,
}

impl OverrideKind {
    pub fn region_kind(self) -> RegionKind {
        match self {
            OverrideKind::Code => RegionKind::Code,
            OverrideKind::Data(d) => RegionKind::Data(d),
            OverrideKind::Unknown => RegionKind::Unknown,
        }
    }

    pub fn name(self) -> &'static str {
        self.region_kind().name()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionOverride {
    pub start: FileOffset,
    pub len: u32,
    pub kind: OverrideKind,
}

impl RegionOverride {
    pub fn end(&self) -> u32 {
        self.start.0 + self.len
    }
}
