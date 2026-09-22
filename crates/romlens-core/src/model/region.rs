//! Regions: what the analyzer (or the user) believes a byte range is.

use crate::memory::address::{FileOffset, SnesAddress};

/// Which bank a 16-bit pointer's target is in.
///
/// A 16-bit entry names an offset, not an address, so something has to supply
/// the bank. A dispatch table read by `JMP (abs,X)` uses the program bank; a
/// table of graphics pointers usually names one fixed bank; and a table of
/// three-byte entries carries its own. Getting this wrong is the difference
/// between `dw CODE_808423` and a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BankRule {
    /// The bank the table itself is in. The right default, and what a
    /// program-bank dispatch actually does.
    SameBank,
    /// One bank for every entry.
    Fixed(u8),
    /// The entry carries its own bank, so it is at least three bytes wide.
    FromEntry,
}

impl BankRule {
    pub fn name(self) -> String {
        match self {
            BankRule::SameBank => "same".to_owned(),
            BankRule::Fixed(b) => format!("${b:02X}"),
            BankRule::FromEntry => "entry".to_owned(),
        }
    }

    /// Parse `same`, `entry` or a bank number (`$C0`, `C0`, `0xC0`).
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "same" => Some(BankRule::SameBank),
            "entry" => Some(BankRule::FromEntry),
            other => {
                let t = other.trim_start_matches('$').trim_start_matches("0x");
                u8::from_str_radix(t, 16).ok().map(BankRule::Fixed)
            }
        }
    }

    /// The address one entry names, given where the table lives.
    ///
    /// `None` when the entry is too narrow for the rule — a `FromEntry` rule
    /// needs three bytes, and inventing a bank for a two-byte entry would be
    /// worse than saying nothing.
    pub fn target(self, bytes: &[u8], width: u32, table_bank: u8) -> Option<SnesAddress> {
        match (self, width) {
            (BankRule::FromEntry, 3..) if bytes.len() >= 3 => {
                Some(SnesAddress::from_u24(u32::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], 0,
                ])))
            }
            (BankRule::FromEntry, _) => None,
            (rule, 2..) if bytes.len() >= 2 => Some(SnesAddress::new(
                rule.bank_for(table_bank),
                u16::from_le_bytes([bytes[0], bytes[1]]),
            )),
            _ => None,
        }
    }

    /// The bank to read an entry's target in, given where the table lives.
    pub fn bank_for(self, table_bank: u8) -> u8 {
        match self {
            BankRule::SameBank => table_bank,
            BankRule::Fixed(b) => b,
            // The caller reads the bank out of the entry; this is the fallback
            // when it did not.
            BankRule::FromEntry => table_bank,
        }
    }
}

/// What one element of a table is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableElem {
    /// Bytes with no further meaning.
    Raw,
    /// An address of data.
    Pointer(BankRule),
    /// An address of code, so it is rendered with its label and makes an xref.
    Code(BankRule),
}

impl TableElem {
    pub fn name(self) -> &'static str {
        match self {
            TableElem::Raw => "raw",
            TableElem::Pointer(_) => "pointer",
            TableElem::Code(_) => "code",
        }
    }

    pub fn parse(name: &str, bank: BankRule) -> Option<Self> {
        match name {
            "raw" => Some(TableElem::Raw),
            "pointer" => Some(TableElem::Pointer(bank)),
            "code" => Some(TableElem::Code(bank)),
            _ => None,
        }
    }

    /// The bank rule, for the kinds that have one.
    pub fn bank(self) -> Option<BankRule> {
        match self {
            TableElem::Raw => None,
            TableElem::Pointer(b) | TableElem::Code(b) => Some(b),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataKind {
    Byte,
    Word,
    Long,
    Pointer { bank: BankRule },
    Table { stride: u8, elem: TableElem },
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
            DataKind::Pointer { .. } => "pointer",
            DataKind::Table { .. } => "table",
            DataKind::String => "string",
            DataKind::Graphics { .. } => "graphics",
            DataKind::Tilemap => "tilemap",
            DataKind::Palette => "palette",
            DataKind::Compressed => "compressed",
            DataKind::Struct => "struct",
        }
    }

    /// Parse a CLI/JSON name; `table`, `pointer` and `graphics` take their
    /// parameters separately. The defaults are the commonest real shapes: a
    /// two-byte stride, raw elements, and a pointer into the table's own bank.
    pub fn parse(name: &str, stride: Option<u8>, bpp: Option<u8>) -> Option<DataKind> {
        Self::parse_with(name, stride, bpp, None, None)
    }

    /// [`parse`](Self::parse) with the pointer and table parameters.
    pub fn parse_with(
        name: &str,
        stride: Option<u8>,
        bpp: Option<u8>,
        elem: Option<&str>,
        bank: Option<BankRule>,
    ) -> Option<DataKind> {
        let bank = bank.unwrap_or(BankRule::SameBank);
        Some(match name {
            "byte" => DataKind::Byte,
            "word" => DataKind::Word,
            "long" => DataKind::Long,
            "pointer" => DataKind::Pointer { bank },
            "table" => DataKind::Table {
                stride: stride.unwrap_or(2),
                elem: match elem {
                    Some(e) => TableElem::parse(e, bank)?,
                    None => TableElem::Raw,
                },
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

    /// For the kinds whose elements name an address: the entry width, the
    /// bank rule, and whether the target is code. `None` for everything else,
    /// which is what says "render these as numbers".
    pub const fn entry_rule(self) -> Option<(u32, BankRule, bool)> {
        match self {
            DataKind::Pointer { bank } => Some((
                if matches!(bank, BankRule::FromEntry) {
                    3
                } else {
                    2
                },
                bank,
                false,
            )),
            DataKind::Table { stride, elem } => match elem {
                TableElem::Raw => None,
                TableElem::Pointer(bank) => Some((stride as u32, bank, false)),
                TableElem::Code(bank) => Some((stride as u32, bank, true)),
            },
            _ => None,
        }
    }

    /// Bytes per element for the sized kinds.
    pub const fn element_len(self) -> u32 {
        match self {
            // A pointer that carries its own bank is three bytes wide, which
            // is what keeps a row of them from straddling entries.
            DataKind::Pointer {
                bank: BankRule::FromEntry,
            } => 3,
            DataKind::Word | DataKind::Pointer { .. } | DataKind::Palette | DataKind::Tilemap => 2,
            DataKind::Long => 3,
            DataKind::Table { stride, .. } => stride as u32,
            _ => 1,
        }
    }

    /// The batch encoding (`asm_lines` record byte 10).
    pub const fn code(self) -> u8 {
        match self {
            DataKind::Byte => 2,
            DataKind::Word => 3,
            DataKind::Long => 4,
            DataKind::Pointer { .. } => 5,
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
