//! The explanations' records (docs/20): what a hardware write does, field
//! by field, and the idioms an instruction is part of.

use romlens_core::explain::{self, Explained, Idiom, IdiomKind, RegisterWrite};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FieldRowInfo {
    /// `7` or `0–3`.
    pub bits: String,
    pub name: String,
    /// With a known value.
    pub raw: Option<u32>,
    pub meaning: Option<String>,
}

/// One register (or a pair written as one value) of an access.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RegisterPartInfo {
    /// The register's bank offset (`$2100`).
    pub address: u32,
    pub name: String,
    /// 1, or 2 for a pair such as VMADD.
    pub width: u8,
    pub value: Option<u32>,
    /// What the register is for.
    pub about: String,
    /// Written twice in a row, low byte then high.
    pub twice: bool,
    /// The fields in a few words, when the value is known.
    pub summary: Option<String>,
    /// `NMITIMEN = $81: NMI on, joypad auto-read on`.
    pub short: String,
    pub fields: Vec<FieldRowInfo>,
}

/// An instruction's access to a hardware register.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RegisterAccessInfo {
    /// A store, with the value it writes when known; else a read (or a
    /// store the analysis did not reach), described without values.
    pub store: bool,
    /// The listing's comment.
    pub short: String,
    pub value: Option<u32>,
    /// Bytes: 1 or 2.
    pub width: u8,
    pub parts: Vec<RegisterPartInfo>,
    /// Where an unknown value was loaded from (SNES address).
    pub source: Option<u32>,
    pub source_name: Option<String>,
    /// Through an index the analysis does not know: only the base register.
    pub indexed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum IdiomKindInfo {
    Wait,
    Dma,
    Hdma,
    Multiply,
    Divide,
    ClearMemory,
    BlockMove,
    ApuHandshake,
    Decimal,
    SharedEntry,
}

impl From<IdiomKind> for IdiomKindInfo {
    fn from(k: IdiomKind) -> Self {
        match k {
            IdiomKind::Wait => Self::Wait,
            IdiomKind::Dma => Self::Dma,
            IdiomKind::Hdma => Self::Hdma,
            IdiomKind::Multiply => Self::Multiply,
            IdiomKind::Divide => Self::Divide,
            IdiomKind::ClearMemory => Self::ClearMemory,
            IdiomKind::BlockMove => Self::BlockMove,
            IdiomKind::ApuHandshake => Self::ApuHandshake,
            IdiomKind::Decimal => Self::Decimal,
            IdiomKind::SharedEntry => Self::SharedEntry,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct IdiomInfo {
    pub kind: IdiomKindInfo,
    pub title: String,
    /// What it does here, in its values.
    pub summary: String,
    /// Why SNES games do this.
    pub why: String,
    /// The instructions it spans, ascending (file offsets).
    pub offsets: Vec<u32>,
    /// Where its note line goes.
    pub note_at: u32,
}

impl From<&Idiom> for IdiomInfo {
    fn from(i: &Idiom) -> Self {
        IdiomInfo {
            kind: i.kind.into(),
            title: i.title.clone(),
            summary: i.summary.clone(),
            why: i.why.to_owned(),
            offsets: i.offsets.iter().map(|o| o.0).collect(),
            note_at: i.first().0,
        }
    }
}

/// Everything explained about one instruction.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ExplanationInfo {
    pub register: Option<RegisterAccessInfo>,
    pub idioms: Vec<IdiomInfo>,
}

fn parts(w: &RegisterWrite) -> Vec<RegisterPartInfo> {
    w.parts
        .iter()
        .map(|p| RegisterPartInfo {
            address: u32::from(p.address),
            name: p.name.clone(),
            width: p.width,
            value: p.value,
            about: p.about.to_owned(),
            twice: p.twice,
            summary: p.summary.clone(),
            short: p.short(),
            fields: p
                .fields
                .iter()
                .map(|f| FieldRowInfo {
                    bits: f.bits.clone(),
                    name: f.name.to_owned(),
                    raw: f.raw,
                    meaning: f.meaning.clone(),
                })
                .collect(),
        })
        .collect()
}

pub fn store_info(e: &Explained) -> RegisterAccessInfo {
    RegisterAccessInfo {
        store: true,
        short: e.short(),
        value: e.write.value,
        width: e.write.width,
        parts: parts(&e.write),
        source: e.source.map(|a| a.as_u24()),
        source_name: e.source_name.clone(),
        indexed: e.indexed,
    }
}

/// A register an instruction touches, described without a value.
pub fn access_info(address: u16, width: u8, store: bool) -> Option<RegisterAccessInfo> {
    let w = explain::describe(address, None, width)?;
    Some(RegisterAccessInfo {
        store,
        short: w.short(),
        value: None,
        width: w.width,
        parts: parts(&w),
        source: None,
        source_name: None,
        indexed: false,
    })
}

/// A graphics view of what a screen row describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ScreenLinkInfo {
    /// Tiles in ROM at this file offset, at this depth (7 for mode 7).
    Tiles {
        rom: u32,
        bpp: u8,
    },
    Tilemap {
        rom: u32,
    },
    Palette {
        rom: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScreenRowInfo {
    pub label: String,
    pub text: String,
    /// File offsets of the stores that set it.
    pub set_at: Vec<u32>,
    /// A DMA that writes the memory it describes.
    pub source: Option<String>,
    pub source_at: Option<u32>,
    pub link: Option<ScreenLinkInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScreenSectionInfo {
    pub title: String,
    pub rows: Vec<ScreenRowInfo>,
}

/// What the screen is set up to be at an instruction (docs/21).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScreenSetupInfo {
    /// The routine's entry (SNES address).
    pub routine: u32,
    pub sections: Vec<ScreenSectionInfo>,
}

impl From<romlens_core::explain::screen::ScreenSetup> for ScreenSetupInfo {
    fn from(s: romlens_core::explain::screen::ScreenSetup) -> Self {
        use romlens_core::explain::screen::Link;
        ScreenSetupInfo {
            routine: s.routine.as_u24(),
            sections: s
                .sections
                .into_iter()
                .map(|sec| ScreenSectionInfo {
                    title: sec.title,
                    rows: sec
                        .rows
                        .into_iter()
                        .map(|r| ScreenRowInfo {
                            label: r.label,
                            text: r.text,
                            set_at: r.set_at.iter().map(|o| o.0).collect(),
                            source: r.source,
                            source_at: r.source_at.map(|o| o.0),
                            link: r.link.map(|l| match l {
                                Link::Tiles { rom, bpp } => {
                                    ScreenLinkInfo::Tiles { rom: rom.0, bpp }
                                }
                                Link::Tilemap { rom } => ScreenLinkInfo::Tilemap { rom: rom.0 },
                                Link::Palette { rom } => ScreenLinkInfo::Palette { rom: rom.0 },
                            }),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}
