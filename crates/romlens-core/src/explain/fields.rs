//! What each hardware register's bits mean (docs/20), so a write reads as
//! `NMITIMEN = $81: NMI on, joypad auto-read on` rather than just a name.
//!
//! The text is our own, checked field by field against fullsnes (Martin
//! Korth) and the SNESdev wiki's register pages. Registers whose bytes mean
//! whatever the program says (the APU ports, the data ports) have no fields,
//! only what they are for.

use crate::graphics::oam::OBJ_SIZES;
use crate::model::hardware_register;

/// How a field's bits read.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    /// One bit. The short form shows `off` only when `show_off` is set.
    Flag {
        on: &'static str,
        off: &'static str,
        show_off: bool,
    },
    /// The value picks one of these. With `quiet_zero` the short form
    /// leaves out the first, which means "none".
    Choice {
        names: &'static [&'static str],
        quiet_zero: bool,
    },
    /// A number, said in words.
    Number(fn(u32) -> String),
}

/// Bits `lo..=hi` of a register (or of a register pair, for 16-bit ones).
#[derive(Debug, Clone, Copy)]
pub struct Field {
    pub hi: u8,
    pub lo: u8,
    pub name: &'static str,
    pub kind: Kind,
}

impl Field {
    pub fn raw(&self, value: u32) -> u32 {
        let width = u32::from(self.hi - self.lo) + 1;
        (value >> self.lo) & ((1u32 << width) - 1)
    }

    /// The meaning of these bits in `value`.
    pub fn meaning(&self, value: u32) -> String {
        let raw = self.raw(value);
        match self.kind {
            Kind::Flag { on, off, .. } => if raw != 0 { on } else { off }.to_owned(),
            Kind::Choice { names, .. } => names
                .get(raw as usize)
                .map(|s| (*s).to_owned())
                .unwrap_or_else(|| format!("{raw}")),
            Kind::Number(f) => f(raw),
        }
    }

    /// The field in the short form, or nothing for a flag that is off.
    fn short(&self, value: u32) -> Option<String> {
        let zero = self.raw(value) == 0;
        match self.kind {
            Kind::Flag {
                show_off: false, ..
            }
            | Kind::Choice {
                quiet_zero: true, ..
            } if zero => None,
            _ => Some(self.meaning(value)),
        }
    }

    /// `7` or `0–3`.
    pub fn bits(&self) -> String {
        if self.hi == self.lo {
            format!("{}", self.lo)
        } else {
            format!("{}–{}", self.lo, self.hi)
        }
    }
}

/// A register's layout: what it is for and its fields.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    /// The register, or the low register of a pair.
    pub address: u16,
    /// For a pair written as one 16-bit value (`VMADD`); a single register
    /// uses its own name.
    pub pair: Option<&'static str>,
    pub about: &'static str,
    pub fields: &'static [Field],
    /// Written twice in a row, low byte then high (the scroll registers,
    /// the mode 7 matrix, CGDATA).
    pub twice: bool,
    /// Its bytes are data, not settings: the short form has no fields.
    pub data: bool,
}

/// One register (or pair) of a write, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub address: u16,
    pub name: String,
    /// 1 for a register, 2 for a pair.
    pub width: u8,
    pub value: Option<u32>,
    pub about: &'static str,
    pub twice: bool,
    pub fields: Vec<FieldRow>,
    /// The fields in a few words, when the value is known.
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldRow {
    pub bits: String,
    pub name: &'static str,
    pub raw: Option<u32>,
    pub meaning: Option<String>,
}

/// A store to the hardware, register by register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterWrite {
    /// The first register written.
    pub address: u16,
    /// Bytes stored: 1 or 2.
    pub width: u8,
    pub value: Option<u32>,
    pub parts: Vec<Part>,
}

/// About how long a comment may get before it crowds the listing.
pub const SHORT_LIMIT: usize = 60;

impl Part {
    /// `NMITIMEN = $81: NMI on, joypad auto-read on`.
    pub fn short(&self) -> String {
        let Some(v) = self.value else {
            return self.name.clone();
        };
        let digits = if self.width == 2 { 4 } else { 2 };
        let mut s = format!("{} = ${v:0digits$X}", self.name);
        if let Some(summary) = &self.summary {
            s.push_str(": ");
            s.push_str(summary);
        }
        s
    }
}

impl RegisterWrite {
    /// Every part's short form, capped at about [`SHORT_LIMIT`] characters.
    pub fn short(&self) -> String {
        let s = self
            .parts
            .iter()
            .map(Part::short)
            .collect::<Vec<_>>()
            .join("; ");
        cap(&s, SHORT_LIMIT)
    }
}

/// `s` cut at a word to about `limit` characters, with an ellipsis.
pub fn cap(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_owned();
    }
    let cut: String = s.chars().take(limit - 1).collect();
    let at = cut.rfind([' ', ',', ';']).filter(|&i| i > limit / 2);
    let mut out = at.map_or(cut.clone(), |i| cut[..i].to_owned());
    while out.ends_with([',', ';', ':', ' ']) {
        out.pop();
    }
    out.push('…');
    out
}

/// Decode a store of `width` bytes (1 or 2) to the register at bank offset
/// `address`, with the value when it is known. `None` when `address` is
/// not a hardware register.
pub fn describe(address: u16, value: Option<u32>, width: u8) -> Option<RegisterWrite> {
    hardware_register(address)?;
    let width = width.clamp(1, 2);
    let mut parts = Vec::new();
    if width == 2
        && let Some(l) = layout(address).filter(|l| l.pair.is_some())
    {
        parts.push(part(address, &l, value.map(|v| v & 0xFFFF), 2));
    } else {
        for i in 0..u16::from(width) {
            let a = address.wrapping_add(i);
            let Some(r) = hardware_register(a) else { break };
            let v = value.map(|v| (v >> (8 * i)) & 0xFF);
            let l = layout(a)
                .filter(|l| l.pair.is_none())
                .or_else(|| layout(a).map(|l| half_of(&l, a)))
                .unwrap_or(Layout {
                    address: a,
                    pair: None,
                    about: r.description,
                    fields: &[],
                    twice: false,
                    data: true,
                });
            parts.push(part(a, &l, v, 1));
        }
    }
    Some(RegisterWrite {
        address,
        width,
        value: value.map(|v| v & if width == 2 { 0xFFFF } else { 0xFF }),
        parts,
    })
}

/// The layout of the register at `address`, or the pair it starts.
pub fn layout(address: u16) -> Option<Layout> {
    let a = if (0x4300..=0x437F).contains(&address) {
        0x4300 | (address & 0x0F)
    } else {
        address
    };
    let base = LAYOUTS.iter().find(|l| l.address == a)?;
    Some(Layout { address, ..*base })
}

/// One byte of a pair written on its own: the pair's text, no fields.
fn half_of(pair: &Layout, address: u16) -> Layout {
    Layout {
        address,
        pair: None,
        about: pair.about,
        fields: &[],
        twice: false,
        data: true,
    }
}

fn part(address: u16, l: &Layout, value: Option<u32>, width: u8) -> Part {
    let name = l
        .pair
        .map(str::to_owned)
        .or_else(|| hardware_register(address).map(|r| r.name.to_owned()))
        .unwrap_or_default();
    let name = if l.pair.is_some() && (0x4300..=0x437F).contains(&address) {
        // A1T, DAS, A2A: the channel number goes before nothing else.
        format!("{name}{}", (address >> 4) & 7)
    } else {
        name
    };
    let fields = l
        .fields
        .iter()
        .map(|f| FieldRow {
            bits: f.bits(),
            name: f.name,
            raw: value.map(|v| f.raw(v)),
            meaning: value.map(|v| f.meaning(v)),
        })
        .collect();
    let summary = value.and_then(|v| {
        if l.data || l.fields.is_empty() {
            return None;
        }
        let parts: Vec<String> = l.fields.iter().filter_map(|f| f.short(v)).collect();
        Some(if parts.is_empty() {
            "all off".to_owned()
        } else {
            parts.join(", ")
        })
    });
    Part {
        address,
        name,
        width,
        value,
        about: l.about,
        twice: l.twice,
        fields,
        summary,
    }
}

// ---- field constructors ----

const fn flag(bit: u8, name: &'static str, on: &'static str, off: &'static str) -> Field {
    Field {
        hi: bit,
        lo: bit,
        name,
        kind: Kind::Flag {
            on,
            off,
            show_off: false,
        },
    }
}

/// A flag whose off state is worth saying even in the short form.
const fn flag2(bit: u8, name: &'static str, on: &'static str, off: &'static str) -> Field {
    Field {
        hi: bit,
        lo: bit,
        name,
        kind: Kind::Flag {
            on,
            off,
            show_off: true,
        },
    }
}

const fn choice(hi: u8, lo: u8, name: &'static str, names: &'static [&'static str]) -> Field {
    Field {
        hi,
        lo,
        name,
        kind: Kind::Choice {
            names,
            quiet_zero: false,
        },
    }
}

/// A choice whose first entry means "none", left out of the short form.
const fn choice0(hi: u8, lo: u8, name: &'static str, names: &'static [&'static str]) -> Field {
    Field {
        hi,
        lo,
        name,
        kind: Kind::Choice {
            names,
            quiet_zero: true,
        },
    }
}

const fn number(hi: u8, lo: u8, name: &'static str, f: fn(u32) -> String) -> Field {
    Field {
        hi,
        lo,
        name,
        kind: Kind::Number(f),
    }
}

const fn settings(address: u16, about: &'static str, fields: &'static [Field]) -> Layout {
    Layout {
        address,
        pair: None,
        about,
        fields,
        twice: false,
        data: false,
    }
}

const fn pair(
    address: u16,
    name: &'static str,
    about: &'static str,
    fields: &'static [Field],
) -> Layout {
    Layout {
        address,
        pair: Some(name),
        about,
        fields,
        twice: false,
        data: false,
    }
}

const fn twice(address: u16, about: &'static str) -> Layout {
    Layout {
        address,
        pair: None,
        about,
        fields: &[],
        twice: true,
        data: true,
    }
}

const fn data(address: u16, about: &'static str) -> Layout {
    Layout {
        address,
        pair: None,
        about,
        fields: &[],
        twice: false,
        data: true,
    }
}

// ---- numbers in words ----

fn brightness(v: u32) -> String {
    format!("brightness {v}")
}

fn obj_sizes(v: u32) -> String {
    let ((sw, sh), (lw, lh)) = OBJ_SIZES[v as usize & 7];
    format!("sprites {sw}×{sh} and {lw}×{lh}")
}

fn obj_gap(v: u32) -> String {
    format!("second table ${:04X} words on", (v + 1) << 12)
}

fn obj_base(v: u32) -> String {
    format!("sprite tiles at VRAM ${:04X}", v << 13)
}

fn mosaic_size(v: u32) -> String {
    if v == 0 {
        "mosaic 1×1 (none)".to_owned()
    } else {
        format!("mosaic {0}×{0}", v + 1)
    }
}

fn tilemap_base(v: u32) -> String {
    format!("tilemap at VRAM ${:04X}", v << 10)
}

fn char_base(v: u32) -> String {
    format!("tiles at ${:04X}", v << 12)
}

fn vram_word(v: u32) -> String {
    format!("VRAM word ${v:04X} (byte ${:05X})", v << 1)
}

fn oam_word(v: u32) -> String {
    let v = v & 0x1FF;
    if v < 0x100 {
        format!("OAM entry {} (byte ${:03X})", v / 2, v << 1)
    } else {
        format!("OAM high table (byte ${:03X})", v << 1)
    }
}

fn colour_index(v: u32) -> String {
    if v < 0x80 {
        format!(
            "colour {v} (background palette {}, entry {})",
            v / 16,
            v % 16
        )
    } else {
        format!(
            "colour {v} (sprite palette {}, entry {})",
            (v - 0x80) / 16,
            v % 16
        )
    }
}

fn position(v: u32) -> String {
    format!("x = {v}")
}

fn intensity(v: u32) -> String {
    format!("intensity {v}")
}

fn hex16(v: u32) -> String {
    format!("${v:04X}")
}

fn dividend(v: u32) -> String {
    format!("dividend {v}")
}

fn h_dot(v: u32) -> String {
    format!("dot {}", v & 0x1FF)
}

fn v_line(v: u32) -> String {
    format!("line {}", v & 0x1FF)
}

fn channels(v: u32) -> String {
    let list: Vec<String> = (0..8)
        .filter(|c| v & (1 << c) != 0)
        .map(|c| c.to_string())
        .collect();
    match list.len() {
        0 => "no channels".to_owned(),
        1 => format!("channel {}", list[0]),
        _ => format!("channels {}", list.join(", ")),
    }
}

fn cpu_version(v: u32) -> String {
    format!("CPU version {v}")
}

fn bbus(v: u32) -> String {
    let addr = 0x2100 | v as u16;
    let name = hardware_register(addr).map_or("?", |r| r.name);
    let what = match v {
        0x18 | 0x19 => ": VRAM",
        0x22 => ": CGRAM (palette)",
        0x04 => ": OAM (sprites)",
        0x80 => ": WRAM",
        0x40..=0x43 => ": the sound CPU",
        _ => "",
    };
    format!("to ${addr:04X} {name}{what}")
}

fn byte_count(v: u32) -> String {
    let n = if v == 0 { 65536 } else { v };
    format!("{n} bytes (or an HDMA address)")
}

fn line_count(v: u32) -> String {
    format!("{v} lines")
}

fn bank(v: u32) -> String {
    format!("bank ${v:02X}")
}

fn wram_low(v: u32) -> String {
    format!("WRAM $xx:{v:04X} (low 16 bits)")
}

// ---- the tables ----

const MODES: &[&str] = &[
    "mode 0: four 4-colour layers",
    "mode 1: two 16-colour layers and one 4-colour",
    "mode 2: two 16-colour layers, offset per tile",
    "mode 3: one 256-colour layer and one 16-colour",
    "mode 4: 256- and 4-colour layers, offset per tile",
    "mode 5: high-res, 16- and 4-colour layers",
    "mode 6: high-res 16-colour layer, offset per tile",
    "mode 7: one rotated and scaled layer",
];

const TILEMAP_SIZES: &[&str] = &["32×32 tiles", "64×32 tiles", "32×64 tiles", "64×64 tiles"];

const WINDOW_LOGIC: &[&str] = &["OR", "AND", "XOR", "XNOR"];

const FORCE_BLACK: &[&str] = &[
    "main screen shown",
    "main screen black outside the colour window",
    "main screen black inside the colour window",
    "main screen always black",
];

const MATH_WHERE: &[&str] = &[
    "colour math everywhere",
    "colour math inside the colour window only",
    "colour math outside the colour window only",
    "colour math off",
];

const IRQ_MODES: &[&str] = &[
    "no timer IRQ",
    "IRQ at dot HTIME on every line",
    "IRQ at the start of line VTIME",
    "IRQ at dot HTIME on line VTIME",
];

const VRAM_STEP: &[&str] = &[
    "step 1 word",
    "step 32 words",
    "step 128 words",
    "step 128 words",
];

const VRAM_REMAP: &[&str] = &[
    "no address remapping",
    "remap for 2bpp tiles",
    "remap for 4bpp tiles",
    "remap for 8bpp tiles",
];

const M7_OUTSIDE: &[&str] = &[
    "outside the map: wrap",
    "outside the map: wrap",
    "outside the map: transparent",
    "outside the map: tile 0",
];

const A_STEP: &[&str] = &[
    "source address counts up",
    "source address fixed",
    "source address counts down",
    "source address fixed",
];

const PATTERNS: &[&str] = &[
    "1 register",
    "2 registers, alternating",
    "1 register, written twice",
    "2 registers, each twice",
    "4 registers",
    "2 registers, alternating twice",
    "1 register, written twice",
    "2 registers, each twice",
];

macro_rules! window_sel {
    ($a:literal, $b:literal) => {
        [
            flag(
                0,
                concat!($a, " window 1 invert"),
                concat!($a, " window 1 inverted"),
                concat!($a, " window 1 not inverted"),
            ),
            flag(
                1,
                concat!($a, " window 1"),
                concat!($a, " in window 1"),
                concat!($a, " not in window 1"),
            ),
            flag(
                2,
                concat!($a, " window 2 invert"),
                concat!($a, " window 2 inverted"),
                concat!($a, " window 2 not inverted"),
            ),
            flag(
                3,
                concat!($a, " window 2"),
                concat!($a, " in window 2"),
                concat!($a, " not in window 2"),
            ),
            flag(
                4,
                concat!($b, " window 1 invert"),
                concat!($b, " window 1 inverted"),
                concat!($b, " window 1 not inverted"),
            ),
            flag(
                5,
                concat!($b, " window 1"),
                concat!($b, " in window 1"),
                concat!($b, " not in window 1"),
            ),
            flag(
                6,
                concat!($b, " window 2 invert"),
                concat!($b, " window 2 inverted"),
                concat!($b, " window 2 not inverted"),
            ),
            flag(
                7,
                concat!($b, " window 2"),
                concat!($b, " in window 2"),
                concat!($b, " not in window 2"),
            ),
        ]
    };
}

const fn layers(what: &'static [&'static str; 5]) -> [Field; 5] {
    [
        flag(0, "BG1", what[0], "BG1 off"),
        flag(1, "BG2", what[1], "BG2 off"),
        flag(2, "BG3", what[2], "BG3 off"),
        flag(3, "BG4", what[3], "BG4 off"),
        flag(4, "Sprites", what[4], "sprites off"),
    ]
}

static W12: [Field; 8] = window_sel!("BG1", "BG2");
static W34: [Field; 8] = window_sel!("BG3", "BG4");
static WOBJ: [Field; 8] = window_sel!("sprites", "colour");
static LAYERS_ON: [Field; 5] = layers(&["BG1", "BG2", "BG3", "BG4", "sprites"]);
static LAYERS_MASKED: [Field; 5] = layers(&[
    "BG1 masked",
    "BG2 masked",
    "BG3 masked",
    "BG4 masked",
    "sprites masked",
]);

static LAYOUTS: &[Layout] = &[
    settings(
        0x2100,
        "Turns the picture on or off and sets its brightness. While forced blank is on the screen is black and VRAM, CGRAM and OAM can be written at any time, which is why games set it before loading graphics.",
        &[
            flag2(7, "Forced blank", "forced blank", "display on"),
            number(3, 0, "Brightness", brightness),
        ],
    ),
    settings(
        0x2101,
        "Chooses the two sprite sizes a game can use and where in VRAM the sprite tiles are. Every sprite is one of the two sizes, picked by a bit in OAM.",
        &[
            number(7, 5, "Sizes", obj_sizes),
            number(4, 3, "Name select", obj_gap),
            number(2, 0, "Name base", obj_base),
        ],
    ),
    pair(
        0x2102,
        "OAMADD",
        "Where the next OAMDATA write goes in sprite memory. The low 9 bits pick a word; bit 15 makes the sprite at this address draw in front of the others.",
        &[
            number(8, 0, "Address", oam_word),
            flag(
                15,
                "Priority rotation",
                "priority starts at this sprite",
                "priority from sprite 0",
            ),
        ],
    ),
    data(
        0x2104,
        "Writes a byte to sprite memory (OAM) at OAMADD, which then moves on. Usually filled by DMA from a copy in RAM each frame.",
    ),
    settings(
        0x2105,
        "Picks the background mode, which fixes how many layers there are and how many colours each can use, and the tile size of each layer.",
        &[
            choice(2, 0, "Mode", MODES),
            flag(
                3,
                "BG3 priority",
                "BG3 in front (mode 1)",
                "BG3 normal priority",
            ),
            flag(4, "BG1 tiles", "BG1 16×16 tiles", "BG1 8×8 tiles"),
            flag(5, "BG2 tiles", "BG2 16×16 tiles", "BG2 8×8 tiles"),
            flag(6, "BG3 tiles", "BG3 16×16 tiles", "BG3 8×8 tiles"),
            flag(7, "BG4 tiles", "BG4 16×16 tiles", "BG4 8×8 tiles"),
        ],
    ),
    settings(
        0x2106,
        "Makes layers blocky: each block of pixels shows the colour of its top-left pixel. Games use it for screen transitions.",
        &[
            number(7, 4, "Size", mosaic_size),
            flag(0, "BG1", "BG1 mosaic", "BG1 normal"),
            flag(1, "BG2", "BG2 mosaic", "BG2 normal"),
            flag(2, "BG3", "BG3 mosaic", "BG3 normal"),
            flag(3, "BG4", "BG4 mosaic", "BG4 normal"),
        ],
    ),
    settings(
        0x2107,
        "Where BG1's tilemap is in VRAM and how many tiles it spans. The tilemap says which tile goes in each cell of the layer.",
        &[
            number(7, 2, "Base", tilemap_base),
            choice(1, 0, "Size", TILEMAP_SIZES),
        ],
    ),
    settings(
        0x2108,
        "Where BG2's tilemap is in VRAM and how many tiles it spans.",
        &[
            number(7, 2, "Base", tilemap_base),
            choice(1, 0, "Size", TILEMAP_SIZES),
        ],
    ),
    settings(
        0x2109,
        "Where BG3's tilemap is in VRAM and how many tiles it spans.",
        &[
            number(7, 2, "Base", tilemap_base),
            choice(1, 0, "Size", TILEMAP_SIZES),
        ],
    ),
    settings(
        0x210A,
        "Where BG4's tilemap is in VRAM and how many tiles it spans.",
        &[
            number(7, 2, "Base", tilemap_base),
            choice(1, 0, "Size", TILEMAP_SIZES),
        ],
    ),
    settings(
        0x210B,
        "Where the tiles (character data) for BG1 and BG2 start in VRAM, in steps of $1000 words.",
        &[
            number(3, 0, "BG1 base", |v| format!("BG1 {}", char_base(v))),
            number(7, 4, "BG2 base", |v| format!("BG2 {}", char_base(v))),
        ],
    ),
    settings(
        0x210C,
        "Where the tiles (character data) for BG3 and BG4 start in VRAM, in steps of $1000 words.",
        &[
            number(3, 0, "BG3 base", |v| format!("BG3 {}", char_base(v))),
            number(7, 4, "BG4 base", |v| format!("BG4 {}", char_base(v))),
        ],
    ),
    twice(
        0x210D,
        "BG1's horizontal scroll. Written twice: the low 8 bits, then the high bits. In mode 7 it also sets the mode 7 scroll.",
    ),
    twice(
        0x210E,
        "BG1's vertical scroll. Written twice: the low 8 bits, then the high bits.",
    ),
    twice(
        0x210F,
        "BG2's horizontal scroll. Written twice: low byte, then high.",
    ),
    twice(
        0x2110,
        "BG2's vertical scroll. Written twice: low byte, then high.",
    ),
    twice(
        0x2111,
        "BG3's horizontal scroll. Written twice: low byte, then high.",
    ),
    twice(
        0x2112,
        "BG3's vertical scroll. Written twice: low byte, then high.",
    ),
    twice(
        0x2113,
        "BG4's horizontal scroll. Written twice: low byte, then high.",
    ),
    twice(
        0x2114,
        "BG4's vertical scroll. Written twice: low byte, then high.",
    ),
    settings(
        0x2115,
        "How the VRAM address moves on after each VMDATA write: by how much, and after the low or the high byte. Most games step one word after the high byte, so a 16-bit write fills one word.",
        &[
            flag2(
                7,
                "Step after",
                "step after the high byte",
                "step after the low byte",
            ),
            choice0(3, 2, "Remap", VRAM_REMAP),
            choice(1, 0, "Step", VRAM_STEP),
        ],
    ),
    pair(
        0x2116,
        "VMADD",
        "The VRAM word address the next VMDATA write goes to. VRAM holds 32K words (64 KB): tiles, tilemaps and sprite graphics.",
        &[number(15, 0, "Address", vram_word)],
    ),
    pair(
        0x2118,
        "VMDATA",
        "Writes a word to VRAM at VMADD; the address then steps as VMAIN says. Usually fed by DMA.",
        &[number(15, 0, "Data", hex16)],
    ),
    settings(
        0x211A,
        "Mode 7 settings: what shows outside the 1024×1024 map, and whether it is flipped.",
        &[
            choice(7, 6, "Outside", M7_OUTSIDE),
            flag(
                1,
                "Vertical flip",
                "flipped vertically",
                "not flipped vertically",
            ),
            flag(
                0,
                "Horizontal flip",
                "flipped horizontally",
                "not flipped horizontally",
            ),
        ],
    ),
    twice(
        0x211B,
        "Mode 7 matrix A (cosine × scale, 8.8 fixed point). Written twice: low byte, then high. Its value also multiplies by an 8-bit write to M7B, giving MPYL/M/H.",
    ),
    twice(
        0x211C,
        "Mode 7 matrix B (sine × scale, 8.8 fixed point). Written twice. An 8-bit write here also multiplies by M7A.",
    ),
    twice(
        0x211D,
        "Mode 7 matrix C (8.8 fixed point). Written twice: low byte, then high.",
    ),
    twice(
        0x211E,
        "Mode 7 matrix D (8.8 fixed point). Written twice: low byte, then high.",
    ),
    twice(
        0x211F,
        "Mode 7 centre of rotation, X (13 bits). Written twice: low byte, then high.",
    ),
    twice(
        0x2120,
        "Mode 7 centre of rotation, Y (13 bits). Written twice: low byte, then high.",
    ),
    settings(
        0x2121,
        "Picks the palette colour the next CGDATA writes change. Colours 0–127 are for backgrounds and 128–255 for sprites.",
        &[number(7, 0, "Colour", colour_index)],
    ),
    twice(
        0x2122,
        "Writes a colour to the palette at CGADD. Each colour is 15 bits, blue-green-red, written as two bytes: low, then high.",
    ),
    settings(
        0x2123,
        "Which of the two windows cut BG1 and BG2, and whether each is inverted.",
        &W12,
    ),
    settings(
        0x2124,
        "Which of the two windows cut BG3 and BG4, and whether each is inverted.",
        &W34,
    ),
    settings(
        0x2125,
        "Which of the two windows cut the sprites and the colour window, and whether each is inverted.",
        &WOBJ,
    ),
    settings(
        0x2126,
        "Window 1's left edge.",
        &[number(7, 0, "Left", position)],
    ),
    settings(
        0x2127,
        "Window 1's right edge.",
        &[number(7, 0, "Right", position)],
    ),
    settings(
        0x2128,
        "Window 2's left edge.",
        &[number(7, 0, "Left", position)],
    ),
    settings(
        0x2129,
        "Window 2's right edge.",
        &[number(7, 0, "Right", position)],
    ),
    settings(
        0x212A,
        "How the two windows combine for each background layer when both are on.",
        &[
            choice(1, 0, "BG1", WINDOW_LOGIC),
            choice(3, 2, "BG2", WINDOW_LOGIC),
            choice(5, 4, "BG3", WINDOW_LOGIC),
            choice(7, 6, "BG4", WINDOW_LOGIC),
        ],
    ),
    settings(
        0x212B,
        "How the two windows combine for the sprites and the colour window.",
        &[
            choice(1, 0, "Sprites", WINDOW_LOGIC),
            choice(3, 2, "Colour", WINDOW_LOGIC),
        ],
    ),
    settings(
        0x212C,
        "Which layers are drawn on the main screen, the picture you see.",
        &LAYERS_ON,
    ),
    settings(
        0x212D,
        "Which layers are drawn on the sub screen, which colour math can blend with the main screen.",
        &LAYERS_ON,
    ),
    settings(
        0x212E,
        "Which main-screen layers the windows cut.",
        &LAYERS_MASKED,
    ),
    settings(
        0x212F,
        "Which sub-screen layers the windows cut.",
        &LAYERS_MASKED,
    ),
    settings(
        0x2130,
        "Colour math, part one: where it happens and what it blends with. Games use it for transparency, shadows and fades.",
        &[
            choice0(7, 6, "Force black", FORCE_BLACK),
            choice(5, 4, "Math where", MATH_WHERE),
            flag2(
                1,
                "Blend with",
                "blend with the sub screen",
                "blend with the fixed colour",
            ),
            flag(
                0,
                "Direct colour",
                "direct colour for 256-colour layers",
                "palette colour",
            ),
        ],
    ),
    settings(
        0x2131,
        "Colour math, part two: add or subtract, halve the result, and which layers take part.",
        &[
            flag2(7, "Operation", "subtract", "add"),
            flag(6, "Half", "halve the result", "full result"),
            flag(0, "BG1", "on BG1", "not BG1"),
            flag(1, "BG2", "on BG2", "not BG2"),
            flag(2, "BG3", "on BG3", "not BG3"),
            flag(3, "BG4", "on BG4", "not BG4"),
            flag(4, "Sprites", "on sprites (palettes 4–7)", "not sprites"),
            flag(5, "Backdrop", "on the backdrop", "not the backdrop"),
        ],
    ),
    settings(
        0x2132,
        "Sets the fixed colour colour math can blend with. Each write sets the intensity of the channels whose bits are set, so a game writes it up to three times.",
        &[
            flag(7, "Blue", "blue", "blue unchanged"),
            flag(6, "Green", "green", "green unchanged"),
            flag(5, "Red", "red", "red unchanged"),
            number(4, 0, "Intensity", intensity),
        ],
    ),
    settings(
        0x2133,
        "Screen settings: interlace, the 239-line overscan mode, pseudo high-res and mode 7's second layer.",
        &[
            flag(7, "External sync", "external sync", "internal sync"),
            flag(6, "EXTBG", "mode 7 EXTBG layer", "no EXTBG"),
            flag(3, "Pseudo high-res", "pseudo high-res", "normal width"),
            flag(2, "Overscan", "239 lines", "224 lines"),
            flag(
                1,
                "Sprite interlace",
                "sprite interlace",
                "no sprite interlace",
            ),
            flag(0, "Interlace", "interlace", "no interlace"),
        ],
    ),
    data(
        0x2140,
        "A byte for the sound CPU (SPC700). The four ports are the only way the two CPUs talk; what the byte means is up to the game's sound driver.",
    ),
    data(0x2141, "A byte for the sound CPU (SPC700), through port 1."),
    data(0x2142, "A byte for the sound CPU (SPC700), through port 2."),
    data(0x2143, "A byte for the sound CPU (SPC700), through port 3."),
    data(
        0x2180,
        "Writes a byte to work RAM at WMADD, which then moves on. Mostly used as a DMA target to fill or copy RAM.",
    ),
    pair(
        0x2181,
        "WMADD",
        "The work RAM address for WMDATA, low 16 bits; WMADDH holds the bank bit.",
        &[number(15, 0, "Address", wram_low)],
    ),
    settings(
        0x2183,
        "The work RAM address for WMDATA, bank bit: $7E or $7F.",
        &[flag2(0, "Bank", "bank $7F", "bank $7E")],
    ),
    settings(
        0x4016,
        "Writing 1 then 0 latches the controllers' buttons so they can be read one bit at a time. Games that let the hardware read the pads (NMITIMEN bit 0) rarely touch it.",
        &[flag2(
            0,
            "Latch",
            "latch the controllers",
            "release the latch",
        )],
    ),
    settings(
        0x4200,
        "Turns interrupts on and off: the NMI at the start of vertical blank, the timer IRQ, and the automatic reading of the controllers each frame.",
        &[
            flag2(7, "NMI", "NMI on", "NMI off"),
            choice0(5, 4, "Timer IRQ", IRQ_MODES),
            flag(0, "Joypad", "joypad auto-read on", "joypad auto-read off"),
        ],
    ),
    data(
        0x4201,
        "The programmable I/O port. Bit 7 also latches the H/V counters when it goes from 1 to 0; light guns use it.",
    ),
    settings(
        0x4202,
        "The first number for the hardware multiplier (unsigned, 8 bits).",
        &[number(7, 0, "Multiplicand", |v| format!("{v} × WRMPYB"))],
    ),
    settings(
        0x4203,
        "The second number for the hardware multiplier. Writing it starts the multiply; the 16-bit product is in RDMPYL/H 8 CPU cycles later.",
        &[number(7, 0, "Multiplier", |v| {
            format!("WRMPYA × {v}, into RDMPY")
        })],
    ),
    pair(
        0x4204,
        "WRDIV",
        "The 16-bit number for the hardware divider to divide.",
        &[number(15, 0, "Dividend", dividend)],
    ),
    settings(
        0x4206,
        "What to divide WRDIV by. Writing it starts the divide; the quotient is in RDDIVL/H and the remainder in RDMPYL/H 16 CPU cycles later.",
        &[number(7, 0, "Divisor", |v| {
            format!("WRDIV ÷ {v}, into RDDIV")
        })],
    ),
    pair(
        0x4207,
        "HTIME",
        "The dot on a line where the timer IRQ fires (0–339), when NMITIMEN asks for it.",
        &[number(8, 0, "Dot", h_dot)],
    ),
    pair(
        0x4209,
        "VTIME",
        "The line where the timer IRQ fires, when NMITIMEN asks for it. Games use it to change settings partway down the screen.",
        &[number(8, 0, "Line", v_line)],
    ),
    settings(
        0x420B,
        "Starts a DMA transfer on each channel whose bit is set, one after another. The CPU stops until they are done. Each channel's $43x0–$43x6 say what to copy where.",
        &[number(7, 0, "Channels", |v| {
            if v == 0 {
                "no transfer started".to_owned()
            } else {
                format!("start DMA on {}", channels(v))
            }
        })],
    ),
    settings(
        0x420C,
        "Turns HDMA on for each channel whose bit is set. HDMA writes a register at the start of each line, from a table, so a setting can change down the screen (waves, gradients, split scrolling).",
        &[number(7, 0, "Channels", |v| {
            if v == 0 {
                "HDMA off".to_owned()
            } else {
                format!("HDMA on {}", channels(v))
            }
        })],
    ),
    settings(
        0x420D,
        "FastROM: whether banks $80–$FF are read at 3.58 MHz instead of 2.68 MHz. Only works with a cartridge whose ROM is fast enough.",
        &[flag2(0, "FastROM", "FastROM on", "FastROM off")],
    ),
    settings(
        0x4210,
        "Bit 7 is set when vertical blank starts, and reading the register clears it. A game waiting for a frame can read it until the bit is set.",
        &[
            flag(
                7,
                "NMI flag",
                "vertical blank has started",
                "no new vertical blank",
            ),
            number(3, 0, "Version", cpu_version),
        ],
    ),
    settings(
        0x4211,
        "Bit 7 is set when the timer IRQ fires, and reading the register clears it.",
        &[flag(7, "IRQ flag", "the timer IRQ fired", "no timer IRQ")],
    ),
    settings(
        0x4212,
        "Where the picture is being drawn: in vertical blank, in horizontal blank, and whether the automatic controller read is still running. Games poll it to wait for blanking.",
        &[
            flag(7, "VBlank", "in vertical blank", "drawing the picture"),
            flag(6, "HBlank", "in horizontal blank", "drawing a line"),
            flag(0, "Joypad busy", "auto-read running", "auto-read done"),
        ],
    ),
    settings(
        0x4300,
        "How this DMA channel copies: which way, how the source address moves, and which pattern of B-bus registers it writes.",
        &[
            flag(
                7,
                "Direction",
                "from the B bus to memory",
                "from memory to the B bus",
            ),
            flag(
                6,
                "HDMA indirect",
                "HDMA indirect table",
                "HDMA direct table",
            ),
            choice0(4, 3, "Source step", A_STEP),
            choice(2, 0, "Pattern", PATTERNS),
        ],
    ),
    settings(
        0x4301,
        "Which B-bus register ($21xx) this channel writes: $18 is VRAM, $22 the palette, $04 sprite memory, $80 work RAM.",
        &[number(7, 0, "Register", bbus)],
    ),
    pair(
        0x4302,
        "A1T",
        "The source address in the bank A1B for this DMA channel, or its HDMA table's start.",
        &[number(15, 0, "Address", |v| {
            format!("source ${v:04X} in bank A1B")
        })],
    ),
    settings(
        0x4304,
        "The bank of this channel's source address or HDMA table.",
        &[number(7, 0, "Bank", bank)],
    ),
    pair(
        0x4305,
        "DAS",
        "For DMA, how many bytes to copy (0 means 65536). For indirect HDMA, the address the hardware fetched from the table.",
        &[number(15, 0, "Count", byte_count)],
    ),
    settings(
        0x4307,
        "For indirect HDMA, the bank of the data the table points at.",
        &[number(7, 0, "Bank", bank)],
    ),
    pair(
        0x4308,
        "A2A",
        "The HDMA table's current address; the hardware moves it on each line.",
        &[number(15, 0, "Address", |v| {
            format!("table now at ${v:04X}")
        })],
    ),
    settings(
        0x430A,
        "HDMA's line counter: how many lines are left for the current table entry, and whether it writes on every one of them.",
        &[
            flag(7, "Repeat", "write every line", "write once"),
            number(6, 0, "Lines", line_count),
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn short(address: u16, value: u32, width: u8) -> String {
        describe(address, Some(value), width).unwrap().short()
    }

    #[test]
    fn decodes_the_common_writes() {
        assert_eq!(
            short(0x4200, 0x81, 1),
            "NMITIMEN = $81: NMI on, joypad auto-read on"
        );
        assert_eq!(short(0x4200, 0x00, 1), "NMITIMEN = $00: NMI off");
        assert_eq!(
            short(0x4200, 0xB1, 1),
            "NMITIMEN = $B1: NMI on, IRQ at dot HTIME on line VTIME…"
        );
        assert_eq!(
            short(0x2100, 0x8F, 1),
            "INIDISP = $8F: forced blank, brightness 15"
        );
        assert_eq!(
            short(0x2100, 0x0F, 1),
            "INIDISP = $0F: display on, brightness 15"
        );
        assert_eq!(
            short(0x2107, 0x21, 1),
            "BG1SC = $21: tilemap at VRAM $2000, 64×32 tiles"
        );
        assert_eq!(
            short(0x210B, 0x42, 1),
            "BG12NBA = $42: BG1 tiles at $2000, BG2 tiles at $4000"
        );
        assert_eq!(
            short(0x420B, 0x03, 1),
            "MDMAEN = $03: start DMA on channels 0, 1"
        );
        assert_eq!(short(0x420C, 0x00, 1), "HDMAEN = $00: HDMA off");
        assert_eq!(short(0x420B, 0x00, 1), "MDMAEN = $00: no transfer started");
        assert_eq!(short(0x212C, 0x13, 1), "TM = $13: BG1, BG2, sprites");
        assert_eq!(short(0x212C, 0x00, 1), "TM = $00: all off");
        assert_eq!(
            short(0x2121, 0x81, 1),
            "CGADD = $81: colour 129 (sprite palette 0, entry 1)"
        );
    }

    #[test]
    fn field_rows() {
        let w = describe(0x2101, Some(0x63), 1).unwrap();
        let rows = &w.parts[0].fields;
        assert_eq!(rows[0].meaning.as_deref(), Some("sprites 16×16 and 32×32"));
        assert_eq!(rows[1].bits, "3–4");
        assert_eq!(
            rows[2].meaning.as_deref(),
            Some("sprite tiles at VRAM $6000")
        );
        let w = describe(0x2105, Some(0x09), 1).unwrap();
        assert_eq!(
            w.parts[0].fields[0].meaning.as_deref(),
            Some("mode 1: two 16-colour layers and one 4-colour")
        );
        assert_eq!(
            w.parts[0].fields[1].meaning.as_deref(),
            Some("BG3 in front (mode 1)")
        );
    }

    #[test]
    fn sixteen_bit_stores() {
        // DMAP0 and BBAD0 in one store: two registers.
        let w = describe(0x4300, Some(0x1801), 2).unwrap();
        assert_eq!(w.parts.len(), 2);
        assert_eq!(w.parts[0].name, "DMAP0");
        assert_eq!(
            w.parts[0].summary.as_deref(),
            Some("2 registers, alternating")
        );
        assert_eq!(w.parts[1].name, "BBAD0");
        assert_eq!(
            w.parts[1].summary.as_deref(),
            Some("to $2118 VMDATAL: VRAM")
        );
        // A pair: one value.
        let w = describe(0x2116, Some(0x6000), 2).unwrap();
        assert_eq!(w.parts.len(), 1);
        assert_eq!(
            w.parts[0].short(),
            "VMADD = $6000: VRAM word $6000 (byte $0C000)"
        );
        let w = describe(0x4375, Some(0x0800), 2).unwrap();
        assert_eq!(w.parts[0].name, "DAS7");
        assert!(w.parts[0].short().starts_with("DAS7 = $0800: 2048 bytes"));
        // One half of a pair on its own: named, not decoded.
        let w = describe(0x2117, Some(0x30), 1).unwrap();
        assert_eq!(w.short(), "VMADDH = $30");
    }

    #[test]
    fn unknown_values_and_non_registers() {
        let w = describe(0x2100, None, 1).unwrap();
        assert_eq!(w.short(), "INIDISP");
        assert!(w.parts[0].fields.iter().all(|f| f.raw.is_none()));
        assert!(describe(0x0100, Some(1), 1).is_none());
        // The APU ports have no fields.
        assert_eq!(short(0x2140, 0xAA, 1), "APUIO0 = $AA");
    }

    #[test]
    fn every_layout_names_a_register_and_its_fields_fit() {
        for l in LAYOUTS {
            assert!(hardware_register(l.address).is_some(), "{:04X}", l.address);
            let bits = if l.pair.is_some() { 16 } else { 8 };
            for f in l.fields {
                assert!(f.lo <= f.hi && f.hi < bits, "{:04X} {}", l.address, f.name);
                if let Kind::Choice { names, .. } = f.kind {
                    assert_eq!(names.len(), 1 << (f.hi - f.lo + 1), "{}", f.name);
                }
            }
        }
    }

    #[test]
    fn caps_long_text() {
        let s = cap(
            "one two three four five six seven eight nine ten eleven twelve",
            30,
        );
        assert!(s.chars().count() <= 30, "{s}");
        assert!(s.ends_with('…'));
    }
}
