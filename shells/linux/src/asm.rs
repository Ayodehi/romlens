//! Disassembly lines as the core delivers them, and the text layout of one
//! line. No GTK in here: this mirrors the macOS shell's `AsmBatch` and
//! `AsmLineLayout` (record layout in the core's `viewmodel::asm_lines`).

use crate::hex::AddressStyle;
use crate::model::batch_cache::RowBatch;

pub const LINES_PER_BATCH: u32 = 256;
const STRIDE: usize = 96;
const HEADER_LEN: usize = 16;
const NONE_ADDRESS: u32 = 0xFFFF_FFFF;
const MAX_TOKENS: usize = 6;

/// Line kinds (the core's `LineKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Instruction,
    Data,
    Label,
    Blank,
    Comment,
    Section,
    /// An idiom's note (docs/20), above its first instruction.
    Note,
}

impl LineKind {
    fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Instruction,
            2 => Self::Data,
            3 => Self::Label,
            5 => Self::Comment,
            6 => Self::Section,
            7 => Self::Note,
            _ => Self::Blank,
        }
    }

    pub fn is_content(self) -> bool {
        matches!(self, Self::Instruction | Self::Data)
    }
}

/// Region kinds as the batch encodes them (record byte 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    Unknown,
    Code,
    /// Anything typed as data: byte, word, table, graphics, and so on.
    Data,
}

impl RegionKind {
    fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::Unknown,
            1 => Self::Code,
            _ => Self::Data,
        }
    }
}

/// Token kinds (the core's `TokenKind`); `Other` catches future kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Mnemonic,
    Punct,
    Immediate,
    Number,
    AutoLabel,
    UserLabel,
    HardwareRegister,
    Comment,
    AutoComment,
    AutoLabelDef,
    UserLabelDef,
    Directive,
    DataValue,
    Section,
    Warning,
    Note,
    Other,
}

impl TokenKind {
    fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Mnemonic,
            2 => Self::Punct,
            3 => Self::Immediate,
            4 => Self::Number,
            5 => Self::AutoLabel,
            6 => Self::UserLabel,
            7 => Self::HardwareRegister,
            8 => Self::Comment,
            9 => Self::AutoComment,
            10 => Self::AutoLabelDef,
            11 => Self::UserLabelDef,
            12 => Self::Directive,
            13 => Self::DataValue,
            14 => Self::Section,
            15 => Self::Warning,
            16 => Self::Note,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// Byte range within the line's own text.
    pub start: usize,
    pub len: usize,
}

/// One decoded 96-byte record.
#[derive(Debug, Clone, PartialEq)]
pub struct AsmLine {
    pub file_offset: u32,
    pub snes_address: Option<u32>,
    pub kind: LineKind,
    pub byte_count: usize,
    pub region: RegionKind,
    /// 0..=100
    pub confidence: u8,
    pub flags_before: u8,
    pub line_flags: u8,
    pub text: String,
    pub target: Option<u32>,
    pub target_file_offset: Option<u32>,
    pub xref_in_count: usize,
    pub bytes: Vec<u8>,
    pub tokens: Vec<Token>,
}

impl AsmLine {
    pub fn has_warning(&self) -> bool {
        self.flags_before & 0x40 != 0
    }

    #[allow(dead_code)] // the inspector's evidence row uses these (L1 step 4)
    pub fn is_low_confidence(&self) -> bool {
        self.line_flags & 0x40 != 0
    }

    #[allow(dead_code)]
    pub fn has_user_override(&self) -> bool {
        self.line_flags & 0x80 != 0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    TooShort,
    BadVersion(u16),
    BadStride(u16),
    Truncated,
}

/// A run of lines fetched in one call.
pub struct AsmBatch {
    pub start_line: u32,
    lines: Vec<AsmLine>,
}

fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([d[at], d[at + 1]])
}

fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

fn optional(v: u32) -> Option<u32> {
    (v != NONE_ADDRESS).then_some(v)
}

impl AsmBatch {
    pub fn decode(start_line: u32, data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < HEADER_LEN {
            return Err(DecodeError::TooShort);
        }
        let version = u16_at(data, 0);
        let stride = u16_at(data, 2);
        let count = u32_at(data, 4) as usize;
        let text_off = u32_at(data, 8) as usize;
        let text_len = u32_at(data, 12) as usize;
        if version != 1 {
            return Err(DecodeError::BadVersion(version));
        }
        if stride as usize != STRIDE {
            return Err(DecodeError::BadStride(stride));
        }
        if data.len() < HEADER_LEN + count * STRIDE || data.len() < text_off + text_len {
            return Err(DecodeError::Truncated);
        }
        let lines = (0..count)
            .map(|i| Self::record(data, HEADER_LEN + i * STRIDE))
            .collect::<Result<_, _>>()?;
        Ok(Self { start_line, lines })
    }

    fn record(data: &[u8], base: usize) -> Result<AsmLine, DecodeError> {
        let byte_count = usize::from(data[base + 9]).min(16);
        let token_count = usize::from(data[base + 14]).min(MAX_TOKENS);
        let text_len = usize::from(u16_at(data, base + 18));
        let text_off = u32_at(data, base + 20) as usize;
        let text = data
            .get(text_off..text_off + text_len)
            .ok_or(DecodeError::Truncated)?;
        let tokens = (0..token_count)
            .map(|t| {
                let at = base + 52 + t * 6;
                Token {
                    kind: TokenKind::from_raw(data[at]),
                    start: usize::from(u16_at(data, at + 2)),
                    len: usize::from(u16_at(data, at + 4)),
                }
            })
            .collect();
        Ok(AsmLine {
            file_offset: u32_at(data, base),
            snes_address: optional(u32_at(data, base + 4)),
            kind: LineKind::from_raw(data[base + 8]),
            byte_count,
            region: RegionKind::from_raw(data[base + 10]),
            confidence: data[base + 11],
            flags_before: data[base + 12],
            line_flags: data[base + 13],
            text: String::from_utf8_lossy(text).into_owned(),
            target: optional(u32_at(data, base + 24)),
            target_file_offset: optional(u32_at(data, base + 28)),
            xref_in_count: usize::from(u16_at(data, base + 32)),
            bytes: data[base + 36..base + 36 + byte_count].to_vec(),
            tokens,
        })
    }

    pub fn line(&self, line: u32) -> Option<&AsmLine> {
        line.checked_sub(self.start_line)
            .and_then(|i| self.lines.get(i as usize))
    }
}

impl RowBatch for AsmBatch {
    const ROWS_PER_BATCH: u32 = LINES_PER_BATCH;
    fn decode(start_row: u32, data: &[u8]) -> Result<Self, String> {
        AsmBatch::decode(start_row, data).map_err(|e| format!("{e:?}"))
    }
}

/// Column geometry and text for disassembly lines. Content lines carry the
/// address column(s), a 12-character bytes column and the text; label and
/// section lines start at column 0; comment lines start at the text column.
#[derive(Debug, Clone, Copy)]
pub struct AsmLayout {
    pub style: AddressStyle,
}

impl AsmLayout {
    pub const LEFT_PADDING: f64 = 8.0;
    /// The region stripe in the gutter.
    pub const GUTTER_WIDTH: f64 = 6.0;
    /// The column for a line's warning mark, between the stripe and the text.
    pub const MARK_WIDTH: f64 = 14.0;
    pub const MAX_TEXT_CHARS: usize = 80;

    /// Characters used by the address column(s), including the trailing gap.
    pub fn address_chars(&self) -> usize {
        if self.style == AddressStyle::Both {
            20
        } else {
            10
        }
    }

    pub fn bytes_chars(&self) -> usize {
        12
    }

    /// Column where the mnemonic (or `db`) starts.
    pub fn text_column(&self) -> usize {
        self.address_chars() + self.bytes_chars() + 1
    }

    pub fn total_chars(&self) -> usize {
        self.text_column() + Self::MAX_TEXT_CHARS
    }

    pub fn mark_start() -> f64 {
        Self::LEFT_PADDING + Self::GUTTER_WIDTH + 2.0
    }

    pub fn text_start() -> f64 {
        Self::mark_start() + Self::MARK_WIDTH
    }

    /// Width of the whole line for a given glyph advance.
    pub fn total_width(&self, char_width: f64) -> f64 {
        Self::text_start() + self.total_chars() as f64 * char_width + Self::LEFT_PADDING
    }

    /// Column at which a line's own text begins.
    pub fn text_origin(&self, line: &AsmLine) -> usize {
        match line.kind {
            LineKind::Label | LineKind::Section | LineKind::Blank => 0,
            _ => self.text_column(),
        }
    }

    /// The token under a character column, if any.
    #[allow(dead_code)] // token hover and click (L1 step 4)
    pub fn token_at(&self, column: usize, line: &AsmLine) -> Option<Token> {
        let rel = column.checked_sub(self.text_origin(line))?;
        line.tokens
            .iter()
            .copied()
            .find(|t| (t.start..t.start + t.len).contains(&rel))
    }

    /// The full display line.
    pub fn text(&self, line: &AsmLine) -> String {
        const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
        let mut out = String::with_capacity(self.text_column() + line.text.len());
        let hex = |out: &mut String, v: u32, digits: usize| {
            for d in (0..digits).rev() {
                out.push(DIGITS[((v >> (d * 4)) & 0xF) as usize] as char);
            }
        };
        let pad_to = |out: &mut String, column: usize| {
            while out.len() < column {
                out.push(' ');
            }
        };
        match line.kind {
            LineKind::Blank => {}
            LineKind::Label | LineKind::Section => out.push_str(&line.text),
            LineKind::Comment | LineKind::Note => {
                pad_to(&mut out, self.text_column());
                out.push_str(&line.text);
            }
            LineKind::Instruction | LineKind::Data => {
                if self.style != AddressStyle::Snes {
                    out.push_str("0x");
                    hex(&mut out, line.file_offset, 6);
                    out.push_str("  ");
                }
                if self.style != AddressStyle::File {
                    match line.snes_address {
                        Some(a) => {
                            out.push('$');
                            hex(&mut out, a >> 16, 2);
                            out.push(':');
                            hex(&mut out, a & 0xFFFF, 4);
                        }
                        None => out.push_str("--:----"),
                    }
                    out.push_str("  ");
                }
                for (i, b) in line.bytes.iter().take(4).enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    hex(&mut out, u32::from(*b), 2);
                }
                pad_to(&mut out, self.text_column());
                out.push_str(&line.text);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: u8, offset: u32, text: &str, bytes: &[u8]) -> Vec<u8> {
        // One-line batch laid out as the core encodes it.
        let text_off = HEADER_LEN + STRIDE;
        let mut d = vec![0u8; text_off];
        d[0..2].copy_from_slice(&1u16.to_le_bytes());
        d[2..4].copy_from_slice(&(STRIDE as u16).to_le_bytes());
        d[4..8].copy_from_slice(&1u32.to_le_bytes());
        d[8..12].copy_from_slice(&(text_off as u32).to_le_bytes());
        d[12..16].copy_from_slice(&(text.len() as u32).to_le_bytes());
        let r = &mut d[HEADER_LEN..];
        r[0..4].copy_from_slice(&offset.to_le_bytes());
        r[4..8].copy_from_slice(&0x0080_0000u32.to_le_bytes());
        r[8] = kind;
        r[9] = bytes.len() as u8;
        r[10] = 1;
        r[11] = 90;
        r[12] = 0x40;
        r[14] = 2;
        r[18..20].copy_from_slice(&(text.len() as u16).to_le_bytes());
        r[20..24].copy_from_slice(&(text_off as u32).to_le_bytes());
        r[24..28].copy_from_slice(&NONE_ADDRESS.to_le_bytes());
        r[28..32].copy_from_slice(&0x30u32.to_le_bytes());
        r[36..36 + bytes.len()].copy_from_slice(bytes);
        r[52] = 1; // mnemonic
        r[54..56].copy_from_slice(&0u16.to_le_bytes());
        r[56..58].copy_from_slice(&3u16.to_le_bytes());
        r[58] = 4; // number
        r[60..62].copy_from_slice(&4u16.to_le_bytes());
        r[62..64].copy_from_slice(&3u16.to_le_bytes());
        d.extend_from_slice(text.as_bytes());
        d
    }

    fn line(style: AddressStyle) -> (AsmLayout, AsmLine) {
        let data = record(1, 0x12, "LDA $12", &[0xA5, 0x12]);
        let batch = AsmBatch::decode(7, &data).unwrap();
        (AsmLayout { style }, batch.line(7).unwrap().clone())
    }

    #[test]
    fn decodes_a_record() {
        let (_, l) = line(AddressStyle::Both);
        assert_eq!(l.kind, LineKind::Instruction);
        assert_eq!(l.file_offset, 0x12);
        assert_eq!(l.snes_address, Some(0x80_0000));
        assert_eq!(l.target, None);
        assert_eq!(l.target_file_offset, Some(0x30));
        assert_eq!(l.region, RegionKind::Code);
        assert_eq!(l.confidence, 90);
        assert!(l.has_warning());
        assert_eq!(l.bytes, vec![0xA5, 0x12]);
        assert_eq!(l.text, "LDA $12");
        assert_eq!(l.tokens.len(), 2);
        assert_eq!(
            l.tokens[1],
            Token {
                kind: TokenKind::Number,
                start: 4,
                len: 3
            }
        );
    }

    #[test]
    fn text_follows_the_address_style() {
        let (layout, l) = line(AddressStyle::Both);
        let t = layout.text(&l);
        assert_eq!(t, "0x000012  $80:0000  A5 12        LDA $12");
        assert_eq!(t.find("LDA"), Some(layout.text_column()));
        let (layout, l) = line(AddressStyle::Snes);
        assert_eq!(layout.text(&l), "$80:0000  A5 12        LDA $12");
        let (layout, l) = line(AddressStyle::File);
        assert_eq!(layout.text(&l), "0x000012  A5 12        LDA $12");
    }

    #[test]
    fn tokens_map_to_columns() {
        let (layout, l) = line(AddressStyle::Both);
        let col = layout.text_column();
        assert_eq!(
            layout.token_at(col, &l).map(|t| t.kind),
            Some(TokenKind::Mnemonic)
        );
        assert_eq!(
            layout.token_at(col + 4, &l).map(|t| t.kind),
            Some(TokenKind::Number)
        );
        assert_eq!(layout.token_at(col + 3, &l), None);
        assert_eq!(layout.token_at(2, &l), None);
    }

    #[test]
    fn label_lines_start_at_column_zero() {
        let data = record(3, 0x12, "Boot:", &[]);
        let l = AsmBatch::decode(0, &data).unwrap().line(0).unwrap().clone();
        let layout = AsmLayout {
            style: AddressStyle::Both,
        };
        assert_eq!(layout.text(&l), "Boot:");
        assert_eq!(layout.text_origin(&l), 0);
        let data = record(5, 0x12, "; note", &[]);
        let l = AsmBatch::decode(0, &data).unwrap().line(0).unwrap().clone();
        assert!(layout.text(&l).ends_with("; note"));
        assert_eq!(layout.text(&l).find(';'), Some(layout.text_column()));
    }

    #[test]
    fn rejects_bad_batches() {
        let data = record(1, 0, "NOP", &[0xEA]);
        assert_eq!(
            AsmBatch::decode(0, &data[..8]).err(),
            Some(DecodeError::TooShort)
        );
        let mut bad = data.clone();
        bad[0] = 2;
        assert_eq!(
            AsmBatch::decode(0, &bad).err(),
            Some(DecodeError::BadVersion(2))
        );
        let mut bad = data.clone();
        bad[2] = 64;
        assert_eq!(
            AsmBatch::decode(0, &bad).err(),
            Some(DecodeError::BadStride(64))
        );
        assert_eq!(
            AsmBatch::decode(0, &data[..HEADER_LEN + 40]).err(),
            Some(DecodeError::Truncated)
        );
    }

    #[test]
    fn the_core_agrees_on_the_record_geometry() {
        assert_eq!(romlens_ffi::workbench::asm_line_stride() as usize, STRIDE);
        assert_eq!(
            romlens_ffi::workbench::asm_batch_header_len() as usize,
            HEADER_LEN
        );
        assert_eq!(romlens_ffi::workbench::asm_none_address(), NONE_ADDRESS);
    }
}
