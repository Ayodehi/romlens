//! What the screen is set up to be at an instruction (docs/21): the
//! registers reaching it, read together as a display, a background mode,
//! its layers, the sprites, colour math and interrupts.

use super::fields::{self, describe};
use super::setup::{self, Before, Reach, Registers};
use crate::analysis::snapshot::AnalysisSnapshot;
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::project::Project;
use crate::model::symbols::Symbols;
use crate::rom::image::RomImage;

/// A graphics view that shows what a row describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// Tiles in ROM, at this colour depth.
    Tiles {
        rom: FileOffset,
        bpp: u8,
    },
    Tilemap {
        rom: FileOffset,
    },
    Palette {
        rom: FileOffset,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub text: String,
    /// The stores that set it, ascending.
    pub set_at: Vec<FileOffset>,
    /// A DMA that writes the memory it describes, when one does (docs/21
    /// S3), and that DMA's instruction.
    pub source: Option<String>,
    pub source_at: Option<FileOffset>,
    pub link: Option<Link>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenSetup {
    pub routine: SnesAddress,
    pub sections: Vec<Section>,
    /// Registers not set on the way here were set, if at all, by this
    /// routine's callers.
    pub before: Before,
}

/// VRAM and palette uploads, for rows to say where their data came from.
pub trait Uploads {
    /// The DMA that filled VRAM at `word` (a word address), if one did: a
    /// description, its source in ROM and the transfer's instruction.
    fn vram(&self, word: u16) -> Option<(String, Option<FileOffset>, FileOffset)>;
    /// The DMA that filled the palette from colour 0, if one did.
    fn palette(&self) -> Option<(String, Option<FileOffset>, FileOffset)>;
}

/// The screen as set up when the instruction at `off` runs.
pub fn screen_at(
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    off: FileOffset,
    uploads: Option<&dyn Uploads>,
) -> Option<ScreenSetup> {
    let regs = setup::registers_at(rom, snap, off)?;
    let symbols = Symbols::new(rom, project, &snap.auto_labels);
    // What "not set here" means depends on how the routine is entered.
    let entry = regs.routine.offset();
    let reset = rom.emulation_vectors().reset;
    let interrupts = [
        rom.native_vectors().nmi,
        rom.native_vectors().irq,
        rom.emulation_vectors().irq,
    ];
    let unset = if regs.before != Before::Caller {
        "not set on the way here"
    } else if entry == reset {
        "not set yet"
    } else if interrupts.contains(&entry) {
        "not set in this handler (the code it interrupted set it)"
    } else {
        "not set in this routine (its callers may set it)"
    };
    let s = Reader {
        regs: &regs,
        name: &|a| super::address_name(rom, &symbols, a),
        unset,
    };
    let mut sections = Vec::new();

    // The display.
    let mut rows = Vec::new();
    rows.push(s.decoded("Picture", 0x2100));
    if s.set(0x2133) {
        rows.push(s.decoded("Screen mode", 0x2133));
    }
    if s.set(0x2106) {
        rows.push(s.decoded("Mosaic", 0x2106));
    }
    sections.push(Section {
        title: "Display".into(),
        rows,
    });

    // The mode, then its layers.
    let mode = s.byte(0x2105).map(|v| v & 7);
    sections.push(Section {
        title: "Background mode".into(),
        rows: vec![s.decoded("Mode", 0x2105)],
    });
    let depths: &[u8] = match mode {
        Some(0) => &[2, 2, 2, 2],
        Some(1) => &[4, 4, 2],
        Some(2) => &[4, 4],
        Some(3) => &[8, 4],
        Some(4) => &[8, 2],
        Some(5) => &[4, 2],
        Some(6) => &[4],
        Some(7) => &[8],
        _ => &[0, 0, 0, 0],
    };
    let layer_regs = [
        0x2107u16, 0x2108, 0x2109, 0x210A, 0x210B, 0x210C, 0x212C, 0x212D, 0x211A,
    ];
    let depths = if mode.is_none() && !layer_regs.iter().any(|r| s.set(*r)) {
        // Nothing about the layers is set here: one line says so.
        sections.push(Section {
            title: "Layers".into(),
            rows: vec![Row {
                label: "BG1–BG4".into(),
                text: s.unset.to_owned(),
                set_at: Vec::new(),
                source: None,
                source_at: None,
                link: None,
            }],
        });
        &[][..]
    } else {
        depths
    };
    for (i, &bpp) in depths.iter().enumerate() {
        let bg = i as u16 + 1;
        let title = match (bpp, mode) {
            (_, Some(7)) => "BG1 (mode 7, 256 colours)".to_owned(),
            (0, _) => format!("BG{bg}"),
            (b, _) => format!("BG{bg} ({} colours)", 1u32 << b),
        };
        let mut rows = Vec::new();
        if mode == Some(7) {
            let mut r = s.row("Map and tiles", &[]);
            r.text = "the 128×128-tile map and its 256 tiles share VRAM from $0000".into();
            if let Some(u) = uploads
                && let Some((what, src, at)) = u.vram(0)
            {
                r.source = Some(what);
                r.source_at = Some(at);
                r.link = src.map(|rom| Link::Tiles { rom, bpp: 7 });
            }
            rows.push(r);
            rows.push(s.decoded("Mode 7", 0x211A));
        } else {
            let sc = 0x2106 + bg;
            let mut map = s.field_row("Tilemap", sc, &["Base", "Size"]);
            if let (Some(v), Some(u)) = (s.byte(sc), uploads) {
                let word = (u16::from(v) & 0xFC) << 8;
                if let Some((what, src, at)) = u.vram(word) {
                    map.source = Some(what);
                    map.source_at = Some(at);
                    map.link = src.map(|rom| Link::Tilemap { rom });
                }
            }
            rows.push(map);
            let nba = if bg <= 2 { 0x210B } else { 0x210C };
            let nibble = if bg % 2 == 1 { "BG1 base" } else { "BG2 base" };
            let nibble = match bg {
                3 => "BG3 base",
                4 => "BG4 base",
                _ => nibble,
            };
            let mut tiles = s.field_row("Tiles", nba, &[nibble]);
            tiles.text = tiles
                .text
                .replace(&format!("BG{bg} tiles at "), "tiles at VRAM ");
            if let (Some(v), Some(u)) = (s.byte(nba), uploads) {
                let n = if bg % 2 == 1 { v & 0x0F } else { v >> 4 };
                let word = u16::from(n) << 12;
                if let Some((what, src, at)) = u.vram(word) {
                    tiles.source = Some(what);
                    tiles.source_at = Some(at);
                    tiles.link = src.map(|rom| Link::Tiles {
                        rom,
                        bpp: bpp.max(2),
                    });
                }
            }
            rows.push(tiles);
            let size_field = format!("BG{bg} tiles");
            let mut size = s.field_row("Tile size", 0x2105, &[size_field.as_str()]);
            // The heading already names the layer.
            size.text = size.text.replace(&format!("BG{bg} "), "");
            rows.push(size);
        }
        rows.push(s.screens(bg as u8 - 1));
        sections.push(Section { title, rows });
    }

    // Sprites.
    let mut rows = vec![
        s.field_row("Sizes", 0x2101, &["Sizes"]),
        s.field_row("Tiles", 0x2101, &["Name base", "Name select"]),
    ];
    if let (Some(v), Some(u)) = (s.byte(0x2101), uploads) {
        let word = u16::from(v & 7) << 13;
        if let Some((what, src, at)) = u.vram(word) {
            rows[1].source = Some(what);
            rows[1].source_at = Some(at);
            rows[1].link = src.map(|rom| Link::Tiles { rom, bpp: 4 });
        }
    }
    rows.push(s.screens(4));
    sections.push(Section {
        title: "Sprites (16 colours)".into(),
        rows,
    });

    // The palette, colour math and interrupts.
    if let Some(u) = uploads
        && let Some((what, src, at)) = u.palette()
    {
        sections.push(Section {
            title: "Palette".into(),
            rows: vec![Row {
                label: "Colours".into(),
                text: what,
                set_at: Vec::new(),
                source: None,
                source_at: Some(at),
                link: src.map(|rom| Link::Palette { rom }),
            }],
        });
    }
    let mut rows = Vec::new();
    if s.set(0x2130) {
        rows.push(s.decoded("Where", 0x2130));
    }
    if s.set(0x2131) {
        rows.push(s.decoded("Layers", 0x2131));
    }
    if !rows.is_empty() {
        sections.push(Section {
            title: "Colour math".into(),
            rows,
        });
    }
    sections.push(Section {
        title: "Interrupts".into(),
        rows: vec![s.decoded("NMITIMEN", 0x4200)],
    });
    Some(ScreenSetup {
        routine: regs.routine,
        sections,
        before: regs.before,
    })
}

struct Reader<'a> {
    regs: &'a Registers,
    name: &'a dyn Fn(SnesAddress) -> String,
    unset: &'a str,
}

impl Reader<'_> {
    fn set(&self, reg: u16) -> bool {
        self.regs.get(reg).is_some()
    }

    fn byte(&self, reg: u16) -> Option<u8> {
        self.regs.get(reg).and_then(Reach::known)
    }

    fn row(&self, label: &str, regs: &[u16]) -> Row {
        let mut set_at: Vec<FileOffset> = regs
            .iter()
            .filter_map(|r| self.regs.get(*r).and_then(Reach::at))
            .collect();
        set_at.sort_by_key(|o| o.0);
        set_at.dedup();
        Row {
            label: label.to_owned(),
            text: String::new(),
            set_at,
            source: None,
            source_at: None,
            link: None,
        }
    }

    /// Why a register's value is not known here.
    fn unknown(&self, reg: u16) -> String {
        match self.regs.get(reg) {
            Some(Reach::From(a, _)) => format!("from {}", (self.name)(a)),
            Some(Reach::Computed(_)) => "worked out at run time".into(),
            Some(Reach::Varies) => "set differently on each path here".into(),
            Some(Reach::Known(..)) => unreachable!(),
            None => self.unset.to_owned(),
        }
    }

    /// A register's summary: `forced blank, brightness 15`.
    fn decoded(&self, label: &str, reg: u16) -> Row {
        let mut r = self.row(label, &[reg]);
        r.text = match self.byte(reg) {
            Some(v) => describe(reg, Some(u32::from(v)), 1)
                .and_then(|w| w.parts.into_iter().next())
                .and_then(|p| p.summary)
                .unwrap_or_else(|| format!("${v:02X}")),
            None => self.unknown(reg),
        };
        r
    }

    /// Some of a register's fields, by name.
    fn field_row(&self, label: &str, reg: u16, names: &[&str]) -> Row {
        let mut r = self.row(label, &[reg]);
        r.text = match self.byte(reg) {
            Some(v) => {
                let parts: Vec<String> = fields::layout(reg)
                    .map(|l| {
                        names
                            .iter()
                            .filter_map(|n| l.fields.iter().find(|f| f.name == *n))
                            .map(|f| f.meaning(u32::from(v)))
                            .collect()
                    })
                    .unwrap_or_default();
                parts.join(", ")
            }
            None => self.unknown(reg),
        };
        r
    }

    /// Whether a layer (0–3, or 4 for sprites) is on the main screen and
    /// the sub screen.
    fn screens(&self, bit: u8) -> Row {
        let mut r = self.row("Shown on", &[0x212C, 0x212D]);
        let on = |reg: u16| self.byte(reg).map(|v| v & (1 << bit) != 0);
        r.text = match (on(0x212C), on(0x212D)) {
            (Some(true), Some(true)) => "the main screen and the sub screen".into(),
            (Some(true), Some(false)) => "the main screen".into(),
            (Some(false), Some(true)) => "the sub screen only".into(),
            (Some(false), Some(false)) => "neither screen (off)".into(),
            (Some(m), None) => format!(
                "{}; the sub screen {}",
                if m {
                    "the main screen"
                } else {
                    "not the main screen"
                },
                self.unknown(0x212D)
            ),
            (None, None) => self.unknown(0x212C),
            (None, _) => format!("main screen {}", self.unknown(0x212C)),
        };
        r
    }
}
