//! The byte inspector: every reading of the selected byte(s) a student might
//! want, computed once in the core so all shells agree.

use crate::memory::address::{FileOffset, SnesAddress};
use crate::memory::map::Region;
use crate::rom::image::RomImage;
use crate::viewmodel::spans::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteInterpretation {
    pub file_offset: FileOffset,
    /// Offset in the file on disk (differs when a copier header exists).
    pub disk_offset: u32,
    pub snes_address: Option<SnesAddress>,
    pub mirrors: Vec<SnesAddress>,
    pub u8: u8,
    pub i8: i8,
    /// `None` when fewer bytes remain than the reading needs.
    pub u16_le: Option<u16>,
    pub i16_le: Option<i16>,
    pub u24_le: Option<u32>,
    /// The 16-bit value as an address in the selected byte's bank.
    pub u16_as_address_in_bank: Option<SnesAddress>,
    pub u24_as_snes_address: Option<SnesAddress>,
    /// File offset the 24-bit value points at, when it maps to ROM.
    pub pointer_target_file_offset: Option<FileOffset>,
    /// Region the 24-bit value points into.
    pub pointer_target_region: Option<Region>,
    pub ascii: Option<char>,
    pub span_name: Option<String>,
    pub span_value: Option<String>,
}

/// Interpret the byte at `off`. `spans` is consulted for the span name.
pub fn interpret(rom: &RomImage, spans: &[Span], off: FileOffset) -> Option<ByteInterpretation> {
    let bytes = rom.bytes();
    let i = off.as_usize();
    let b = *bytes.get(i)?;
    let u16_le = bytes
        .get(i..i + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]));
    let u24_le = bytes
        .get(i..i + 3)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], 0]));
    let snes_address = rom.snes_address_for(off);
    let u24_as_snes_address = u24_le.map(SnesAddress::from_u24);
    let span = spans.iter().find(|s| s.contains(off));
    Some(ByteInterpretation {
        file_offset: off,
        disk_offset: off.0 + rom.disk_offset_delta(),
        snes_address,
        mirrors: rom.mirrors(off),
        u8: b,
        i8: b as i8,
        u16_le,
        i16_le: u16_le.map(|v| v as i16),
        u24_le,
        u16_as_address_in_bank: snes_address
            .and_then(|a| u16_le.map(|v| SnesAddress::new(a.bank(), v))),
        u24_as_snes_address,
        pointer_target_file_offset: u24_as_snes_address.and_then(|a| rom.file_offset_for(a)),
        pointer_target_region: u24_as_snes_address.map(|a| rom.map().classify(a)),
        ascii: (0x20..0x7F).contains(&b).then_some(b as char),
        span_name: span.map(|s| s.name.clone()),
        span_value: span.map(|s| s.value_text.clone()),
    })
}
