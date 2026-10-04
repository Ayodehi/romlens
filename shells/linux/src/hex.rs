//! Hex rows as the core delivers them, and the text layout of one row.
//!
//! No GTK in here: this mirrors the macOS shell's `HexBatch` and
//! `HexRowLayout`, so the column geometry is testable and identical.

pub const BYTES_PER_ROW: usize = 16;
pub const ROWS_PER_BATCH: u32 = 256;

impl crate::model::batch_cache::RowBatch for HexBatch {
    const ROWS_PER_BATCH: u32 = ROWS_PER_BATCH;
    fn decode(start_row: u32, data: &[u8]) -> Result<Self, String> {
        HexBatch::decode(start_row, data).map_err(|e| format!("{e:?}"))
    }
}
const STRIDE: usize = 64;
const HEADER_LEN: usize = 8;
const UNMAPPED: u32 = 0xFFFF_FFFF;

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    TooShort,
    BadVersion(u16),
    BadStride(u16),
    Truncated,
}

/// One decoded 64-byte record (layout in the core's `viewmodel::hex_rows`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexRow {
    pub file_offset: u32,
    pub snes_address: Option<u32>,
    pub byte_count: usize,
    pub flags: u8,
    pub bytes: [u8; BYTES_PER_ROW],
    pub span_ids: [u8; BYTES_PER_ROW],
    pub ascii: [u8; BYTES_PER_ROW],
}

impl HexRow {
    pub fn is_last_row(&self) -> bool {
        self.flags & 0b10 != 0
    }
}

/// A run of rows fetched in one call.
pub struct HexBatch {
    pub start_row: u32,
    rows: Vec<HexRow>,
}

fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([d[at], d[at + 1]])
}

fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

impl HexBatch {
    pub fn decode(start_row: u32, data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < HEADER_LEN {
            return Err(DecodeError::TooShort);
        }
        let version = u16_at(data, 0);
        let stride = u16_at(data, 2);
        let count = u32_at(data, 4) as usize;
        if version != 1 {
            return Err(DecodeError::BadVersion(version));
        }
        if stride as usize != STRIDE {
            return Err(DecodeError::BadStride(stride));
        }
        if data.len() < HEADER_LEN + count * STRIDE {
            return Err(DecodeError::Truncated);
        }
        let rows = (0..count)
            .map(|i| {
                let base = HEADER_LEN + i * STRIDE;
                let snes = u32_at(data, base + 4);
                let lane = |from: usize| -> [u8; BYTES_PER_ROW] {
                    data[base + from..base + from + BYTES_PER_ROW]
                        .try_into()
                        .unwrap()
                };
                HexRow {
                    file_offset: u32_at(data, base),
                    snes_address: (snes != UNMAPPED).then_some(snes),
                    byte_count: data[base + 8] as usize,
                    flags: data[base + 9],
                    bytes: lane(12),
                    span_ids: lane(28),
                    ascii: lane(44),
                }
            })
            .collect();
        Ok(Self { start_row, rows })
    }

    pub fn row(&self, row: u32) -> Option<&HexRow> {
        row.checked_sub(self.start_row)
            .and_then(|i| self.rows.get(i as usize))
    }
}

/// Which address column(s) the hex view shows. Both values are in every row
/// record, so switching never refetches from the core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AddressStyle {
    #[default]
    Both,
    Snes,
    File,
}

impl AddressStyle {
    pub const ALL: [AddressStyle; 3] = [Self::Both, Self::Snes, Self::File];
}

/// Column geometry in character cells; the widget multiplies by the font's
/// advance. Same arithmetic as the macOS `HexRowLayout`.
#[derive(Debug, Clone, Copy)]
pub struct HexLayout {
    pub style: AddressStyle,
}

impl HexLayout {
    pub const LEFT_PADDING: f64 = 8.0;

    pub fn prefix_chars(&self) -> usize {
        if self.style == AddressStyle::Both {
            20
        } else {
            10
        }
    }

    /// `XX ` per byte plus one extra space after byte 8.
    pub fn hex_chars(&self) -> usize {
        BYTES_PER_ROW * 3 + 1
    }

    pub fn ascii_start(&self) -> usize {
        self.prefix_chars() + self.hex_chars() + 1
    }

    pub fn total_chars(&self) -> usize {
        self.ascii_start() + BYTES_PER_ROW + 1
    }

    pub fn hex_column(&self, byte: usize) -> usize {
        self.prefix_chars() + byte * 3 + usize::from(byte >= 8)
    }

    pub fn ascii_column(&self, byte: usize) -> usize {
        self.ascii_start() + byte
    }

    /// Which byte a character column falls on, hex area or ASCII area.
    pub fn byte_at_column(&self, c: usize) -> Option<usize> {
        let prefix = self.prefix_chars();
        if (prefix..prefix + self.hex_chars()).contains(&c) {
            let rel = c - prefix;
            // Skip the gap in the middle.
            let adjusted = if rel >= 25 { rel - 1 } else { rel };
            let i = adjusted / 3;
            return (i < BYTES_PER_ROW).then_some(i);
        }
        let ascii = self.ascii_start();
        (ascii..ascii + BYTES_PER_ROW)
            .contains(&c)
            .then(|| c - ascii)
    }

    /// The row's text. Built into a byte buffer, never `format!` per cell.
    pub fn text(&self, r: &HexRow) -> String {
        const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
        let mut out = vec![b' '; self.total_chars()];
        let mut p = 0;
        let hex = |out: &mut Vec<u8>, p: &mut usize, v: u32, digits: usize| {
            for d in (0..digits).rev() {
                out[*p] = DIGITS[((v >> (d * 4)) & 0xF) as usize];
                *p += 1;
            }
        };
        if self.style != AddressStyle::Snes {
            out[p..p + 2].copy_from_slice(b"0x");
            p += 2;
            hex(&mut out, &mut p, r.file_offset, 6);
            p += 2;
        }
        if self.style != AddressStyle::File {
            match r.snes_address {
                Some(a) => {
                    out[p] = b'$';
                    p += 1;
                    hex(&mut out, &mut p, a >> 16, 2);
                    out[p] = b':';
                    p += 1;
                    hex(&mut out, &mut p, a & 0xFFFF, 4);
                }
                None => {
                    out[p..p + 7].copy_from_slice(b"--:----");
                    p += 7;
                }
            }
            p += 2;
        }
        for i in 0..BYTES_PER_ROW {
            if i == 8 {
                p += 1;
            }
            if i < r.byte_count {
                out[p] = DIGITS[(r.bytes[i] >> 4) as usize];
                out[p + 1] = DIGITS[(r.bytes[i] & 0xF) as usize];
            }
            p += 3;
        }
        out[p] = b'|';
        p += 1;
        for i in 0..BYTES_PER_ROW {
            out[p] = if i < r.byte_count { r.ascii[i] } else { b' ' };
            p += 1;
        }
        out[p] = b'|';
        // The core's ASCII lane is printable ASCII, so this never replaces.
        String::from_utf8_lossy(&out).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> HexRow {
        let mut bytes = [0u8; 16];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = 0xA0 + i as u8;
        }
        HexRow {
            file_offset: 0x7FC0,
            snes_address: Some(0x80FFC0),
            byte_count: 16,
            flags: 0,
            bytes,
            span_ids: [0; 16],
            ascii: *b"ABCDEFGHIJKLMNOP",
        }
    }

    #[test]
    fn both_style_text_matches_the_macos_layout() {
        let layout = HexLayout {
            style: AddressStyle::Both,
        };
        let text = layout.text(&row());
        assert_eq!(text.len(), layout.total_chars());
        assert!(text.starts_with("0x007FC0  $80:FFC0  A0 A1 A2 A3 A4 A5 A6 A7  A8 A9"));
        assert!(text.ends_with("|ABCDEFGHIJKLMNOP|"));
        // Every byte's column holds that byte's digits.
        for i in 0..16 {
            let c = layout.hex_column(i);
            assert_eq!(&text[c..c + 2], format!("{:02X}", row().bytes[i]));
            assert_eq!(layout.byte_at_column(c), Some(i));
            assert_eq!(layout.byte_at_column(c + 1), Some(i));
        }
    }

    #[test]
    fn single_style_and_unmapped_text() {
        let mut r = row();
        r.snes_address = None;
        let snes = HexLayout {
            style: AddressStyle::Snes,
        }
        .text(&r);
        assert!(snes.starts_with("--:----  A0"));
        let file = HexLayout {
            style: AddressStyle::File,
        }
        .text(&row());
        assert!(file.starts_with("0x007FC0  A0"));
    }

    #[test]
    fn ascii_columns_hit_test() {
        let layout = HexLayout {
            style: AddressStyle::File,
        };
        assert_eq!(layout.byte_at_column(layout.ascii_column(5)), Some(5));
        assert_eq!(layout.byte_at_column(0), None);
    }

    #[test]
    fn batch_decodes_and_rejects_bad_input() {
        let mut data = vec![0u8; 8 + 64];
        data[0..2].copy_from_slice(&1u16.to_le_bytes());
        data[2..4].copy_from_slice(&64u16.to_le_bytes());
        data[4..8].copy_from_slice(&1u32.to_le_bytes());
        data[8..12].copy_from_slice(&0x30u32.to_le_bytes());
        data[12..16].copy_from_slice(&UNMAPPED.to_le_bytes());
        data[16] = 3;
        data[17] = 0b10;
        let batch = HexBatch::decode(5, &data).unwrap();
        let r = batch.row(5).unwrap();
        assert_eq!(
            (r.file_offset, r.snes_address, r.byte_count),
            (0x30, None, 3)
        );
        assert!(r.is_last_row());
        assert!(batch.row(4).is_none() && batch.row(6).is_none());
        assert_eq!(
            HexBatch::decode(0, &data[..4]).err(),
            Some(DecodeError::TooShort)
        );
        data[0] = 2;
        assert_eq!(
            HexBatch::decode(0, &data).err(),
            Some(DecodeError::BadVersion(2))
        );
        data[0] = 1;
        data.truncate(40);
        assert_eq!(
            HexBatch::decode(0, &data).err(),
            Some(DecodeError::Truncated)
        );
    }
}
