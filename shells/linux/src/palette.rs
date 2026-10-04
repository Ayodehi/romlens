//! Span and region colours. The core says what a byte is; the shell picks the
//! colour (docs/08). These are the GNOME palette's mid tones, the nearest
//! equivalents of the macOS system colours, and read on light and dark.

use gtk::gdk::RGBA;
use romlens_ffi::SpanKind;

use crate::model::Document;

fn rgb(hex: u32) -> RGBA {
    RGBA::new(
        ((hex >> 16) & 0xFF) as f32 / 255.0,
        ((hex >> 8) & 0xFF) as f32 / 255.0,
        (hex & 0xFF) as f32 / 255.0,
        1.0,
    )
}

pub fn span_color(kind: SpanKind) -> RGBA {
    rgb(match kind {
        SpanKind::Title => 0x3584e4,
        SpanKind::MapMode => 0x9141ac,
        SpanKind::CartridgeType => 0x5b5bd6,
        SpanKind::RomSize => 0x2190a4,
        SpanKind::RamSize => 0x33b7f4,
        SpanKind::Region => 0x33a852,
        SpanKind::DeveloperId => 0x2ec27e,
        SpanKind::Version => 0x865e3c,
        SpanKind::ChecksumComplement => 0xe66100,
        SpanKind::Checksum => 0xe01b24,
        SpanKind::NativeVector => 0xd56199,
        SpanKind::EmulationVector => 0xf5c211,
        SpanKind::ExtendedHeader => 0x77767b,
    })
}

const CODE: u32 = 0x3584e4;
const DATA: u32 = 0xe66100;
const UNKNOWN: u32 = 0x77767b;

/// Blend `a` toward `b` by `fraction` of `b`.
fn blend(a: RGBA, b: RGBA, fraction: f32) -> RGBA {
    let mix = |x: f32, y: f32| x + (y - x) * fraction;
    RGBA::new(
        mix(a.red(), b.red()),
        mix(a.green(), b.green()),
        mix(a.blue(), b.blue()),
        1.0,
    )
}

/// A hex-lane byte: `0x80 | kind << 4 | confidence4`, kind 1 being code.
/// Returns `(is_code, confidence)`.
pub fn decode_lane(byte: u8) -> Option<(bool, f32)> {
    (byte & 0x80 != 0).then(|| ((byte >> 4) & 0x7 == 1, f32::from(byte & 0x0F) / 15.0))
}

/// Region colour, blended toward grey as confidence falls.
pub fn region_color(is_code: bool, confidence: f32) -> RGBA {
    let base = rgb(if is_code { CODE } else { DATA });
    blend(base, rgb(UNKNOWN), 1.0 - confidence.clamp(0.0, 1.0))
}

/// The tint behind a byte with this span-id lane value, alpha included.
pub fn span_tint(doc: &Document, id: u8) -> Option<RGBA> {
    if let Some((is_code, confidence)) = decode_lane(id) {
        let c = region_color(is_code, confidence);
        return Some(RGBA::new(
            c.red(),
            c.green(),
            c.blue(),
            0.08 + 0.14 * confidence,
        ));
    }
    doc.span_kind(id).map(|kind| {
        let c = span_color(kind);
        RGBA::new(c.red(), c.green(), c.blue(), 0.28)
    })
}

use crate::asm::{RegionKind, TokenKind};

/// Disassembly token colours. `fg` is the text colour, which the neutral
/// kinds dim; the others are tones that read on both light and dark.
pub fn token_color(kind: TokenKind, dark: bool, fg: &RGBA, accent: &RGBA) -> RGBA {
    let tone = |light: u32, dk: u32| rgb(if dark { dk } else { light });
    let dim = |a: f32| RGBA::new(fg.red(), fg.green(), fg.blue(), a);
    match kind {
        TokenKind::Mnemonic | TokenKind::Other => *fg,
        TokenKind::Punct
        | TokenKind::AutoLabel
        | TokenKind::AutoComment
        | TokenKind::AutoLabelDef
        | TokenKind::Directive => dim(0.6),
        TokenKind::Section => dim(0.45),
        TokenKind::DataValue => dim(0.8),
        TokenKind::Immediate => tone(0x813d9c, 0xdc8add),
        TokenKind::Number => tone(0x1a7f8e, 0x7fd6e0),
        TokenKind::UserLabel | TokenKind::UserLabelDef => *accent,
        TokenKind::HardwareRegister => tone(0xc2457f, 0xf2a1c8),
        TokenKind::Comment => tone(0x1c7a4c, 0x8ff0a4),
        TokenKind::Warning => tone(0xc64600, 0xffbe6f),
        TokenKind::Note => tone(0x4a4ac4, 0x9ea0ff),
    }
}

/// The region colour a disassembly line's tint and gutter use.
pub fn line_region_color(kind: RegionKind, confidence: f32) -> Option<RGBA> {
    match kind {
        RegionKind::Unknown => None,
        RegionKind::Code => Some(region_color(true, confidence)),
        RegionKind::Data => Some(region_color(false, confidence)),
    }
}

pub const WARNING_ORANGE: u32 = 0xe66100;

pub fn warning_color() -> RGBA {
    rgb(WARNING_ORANGE)
}

/// Overview strip colour by kind code. Code is the accent colour because it
/// is what a reader is looking for; unknown is a neutral wash, so a map full
/// of holes looks like one rather than like a decision.
pub fn strip_color(code: u8, accent: &RGBA, fg: &RGBA) -> RGBA {
    match code {
        0 => RGBA::new(fg.red(), fg.green(), fg.blue(), 0.08),
        1 => *accent,
        2 => rgb(0x77767b),
        3 => rgb(0x2190a4),
        4 => rgb(0x33b7f4),
        5 | 6 => rgb(0x9141ac),
        7 => rgb(0x33a852),
        8 => rgb(0xe66100),
        9 => rgb(0xf5c211),
        10 => rgb(0xd56199),
        11 => rgb(0xe01b24),
        13 => rgb(0x5b5bd6),
        _ => rgb(0x865e3c),
    }
}

use romlens_ffi::CTokenKind;

/// Token colours for the C, in the disassembly's palette where the two name
/// the same thing.
pub fn c_token_color(kind: CTokenKind, dark: bool, fg: &RGBA, accent: &RGBA) -> RGBA {
    let tone = |light: u32, dk: u32| rgb(if dark { dk } else { light });
    match kind {
        CTokenKind::Keyword => tone(0x813d9c, 0xdc8add),
        CTokenKind::Type => tone(0x4a4ac4, 0x9ea0ff),
        CTokenKind::Number => tone(0x1a7f8e, 0x7fd6e0),
        CTokenKind::Comment => tone(0x1c7a4c, 0x8ff0a4),
        CTokenKind::Function | CTokenKind::Label => *accent,
        CTokenKind::Variable => tone(0xc64600, 0xffbe6f),
        CTokenKind::Register => tone(0xc2457f, 0xf2a1c8),
        CTokenKind::Helper => RGBA::new(fg.red(), fg.green(), fg.blue(), 0.6),
        CTokenKind::Local => *fg,
        CTokenKind::GotoLabel => tone(0x865e3c, 0xd9a36b),
    }
}
