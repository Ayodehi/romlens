//! Hex-row batches: the hot path across the FFI, so a flat little-endian
//! buffer rather than records (docs/10).
//!
//! Batch layout, all little-endian:
//!
//! ```text
//! header (8 bytes):  u16 version = 1, u16 stride = 64, u32 rows
//! record (64 bytes): u32 file_offset
//!                    u32 canonical SNES address (0xFFFF_FFFF when unmapped)
//!                    u8  byte_count (16, or fewer on the last row)
//!                    u8  flags (bit 0: row carries a span, bit 1: last row)
//!                    u16 reserved
//!                    16 × u8 bytes (zero padded)
//!                    16 × u8 span id (0 = none; Phase 2 reuses for region ids)
//!                    16 × u8 ASCII ('.' for non-printable)
//!                    4 reserved
//! ```
//!
//! Both address forms in the record mean the shell's address-style toggle
//! never refetches.

use std::fmt::Write;

use crate::memory::address::FileOffset;
use crate::rom::image::RomImage;
use crate::viewmodel::spans::SpanIndex;

pub const ROW_VERSION: u16 = 1;
pub const ROW_STRIDE: u16 = 64;
pub const BATCH_HEADER_LEN: usize = 8;
pub const BYTES_PER_ROW: usize = 16;
pub const UNMAPPED_ADDRESS: u32 = 0xFFFF_FFFF;
pub const FLAG_HAS_SPAN: u8 = 0b01;
pub const FLAG_LAST_ROW: u8 = 0b10;

const OFF_BYTES: usize = 12;
const OFF_SPANS: usize = 28;
const OFF_ASCII: usize = 44;

/// Which address column(s) the text formatter prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AddressStyle {
    #[default]
    Both,
    Snes,
    File,
}

const fn ascii(b: u8) -> u8 {
    if b >= 0x20 && b < 0x7F { b } else { b'.' }
}

/// Encode rows `[start_row, start_row + count)` clipped to the image.
pub fn encode_rows(rom: &RomImage, spans: &SpanIndex, start_row: u32, count: u32) -> Vec<u8> {
    let total = rom.row_count();
    let n = count.min(total.saturating_sub(start_row));
    let mut out = Vec::with_capacity(BATCH_HEADER_LEN + n as usize * ROW_STRIDE as usize);
    out.extend_from_slice(&ROW_VERSION.to_le_bytes());
    out.extend_from_slice(&ROW_STRIDE.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    let bytes = rom.bytes();
    for row in start_row..start_row + n {
        let off = row * BYTES_PER_ROW as u32;
        let src = &bytes[off as usize..(off as usize + BYTES_PER_ROW).min(bytes.len())];
        let mut rec = [0u8; ROW_STRIDE as usize];
        rec[0..4].copy_from_slice(&off.to_le_bytes());
        let addr = rom
            .snes_address_for(FileOffset(off))
            .map_or(UNMAPPED_ADDRESS, |a| a.as_u24());
        rec[4..8].copy_from_slice(&addr.to_le_bytes());
        rec[8] = src.len() as u8;
        let mut flags = 0;
        if spans.any_in(off, src.len() as u32) {
            flags |= FLAG_HAS_SPAN;
        }
        if row + 1 == total {
            flags |= FLAG_LAST_ROW;
        }
        rec[9] = flags;
        for (i, &b) in src.iter().enumerate() {
            rec[OFF_BYTES + i] = b;
            rec[OFF_SPANS + i] = spans.id_at(off + i as u32);
            rec[OFF_ASCII + i] = ascii(b);
        }
        out.extend_from_slice(&rec);
    }
    out
}

/// One line per row, the shape the CLI prints and the goldens pin:
///
/// ```text
/// 0x00041C  $80:841C  78 18 FB 5C 23 84 80 E2  20 A9 01 8D 0D 42 85 86  |x..\#... ....B...|
/// ```
pub fn format_rows_text(
    rom: &RomImage,
    spans: &SpanIndex,
    start_row: u32,
    count: u32,
    style: AddressStyle,
) -> String {
    let total = rom.row_count();
    let n = count.min(total.saturating_sub(start_row));
    let mut s = String::with_capacity(n as usize * 96);
    let bytes = rom.bytes();
    for row in start_row..start_row + n {
        let off = row * BYTES_PER_ROW as u32;
        let src = &bytes[off as usize..(off as usize + BYTES_PER_ROW).min(bytes.len())];
        let snes = rom.snes_address_for(FileOffset(off));
        match style {
            AddressStyle::Both | AddressStyle::File => {
                let _ = write!(s, "{}", FileOffset(off));
            }
            AddressStyle::Snes => {}
        }
        if matches!(style, AddressStyle::Both) {
            s.push_str("  ");
        }
        match style {
            AddressStyle::Both | AddressStyle::Snes => match snes {
                Some(a) => {
                    let _ = write!(s, "{a}");
                }
                None => s.push_str("--:----"),
            },
            AddressStyle::File => {}
        }
        s.push_str("  ");
        for i in 0..BYTES_PER_ROW {
            if i == 8 {
                s.push(' ');
            }
            match src.get(i) {
                Some(b) => {
                    let _ = write!(s, "{b:02X}");
                }
                None => s.push_str("  "),
            }
            s.push(' ');
        }
        s.push(' ');
        let marked = (off..off + src.len() as u32).any(|o| spans.id_at(o) != 0);
        s.push(if marked { '*' } else { ' ' });
        s.push('|');
        for &b in src {
            s.push(ascii(b) as char);
        }
        s.push_str("|\n");
    }
    s
}
