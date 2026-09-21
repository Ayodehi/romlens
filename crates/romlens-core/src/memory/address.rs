//! The two address types every other module speaks in.

use std::fmt;

/// A 24-bit CPU address `$BB:AAAA` (bank, offset within bank).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnesAddress(u32);

impl SnesAddress {
    /// Build from a bank and a 16-bit offset within the bank.
    pub const fn new(bank: u8, offset: u16) -> Self {
        Self(((bank as u32) << 16) | offset as u32)
    }

    /// Build from a 24-bit value; the top byte is ignored.
    pub const fn from_u24(value: u32) -> Self {
        Self(value & 0x00FF_FFFF)
    }

    pub const fn bank(self) -> u8 {
        (self.0 >> 16) as u8
    }

    pub const fn offset(self) -> u16 {
        (self.0 & 0xFFFF) as u16
    }

    pub const fn as_u24(self) -> u32 {
        self.0
    }

    /// Same offset in another bank.
    pub const fn with_bank(self, bank: u8) -> Self {
        Self::new(bank, self.offset())
    }
}

impl fmt::Display for SnesAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${:02X}:{:04X}", self.bank(), self.offset())
    }
}

/// Offset into the ROM payload, with any copier header already stripped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct FileOffset(pub u32);

impl FileOffset {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u32 {
        self.0
    }

    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// The 16-byte hex row this offset falls in.
    pub const fn row(self) -> u32 {
        self.0 / 16
    }
}

impl fmt::Display for FileOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:06X}", self.0)
    }
}

impl From<u32> for FileOffset {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snes_address_round_trips_bank_and_offset() {
        let a = SnesAddress::new(0x80, 0x841C);
        assert_eq!(a.bank(), 0x80);
        assert_eq!(a.offset(), 0x841C);
        assert_eq!(a.as_u24(), 0x80841C);
        assert_eq!(SnesAddress::from_u24(0xFF80841C), a);
        assert_eq!(a.with_bank(0x00), SnesAddress::new(0x00, 0x841C));
    }

    #[test]
    fn display_formats() {
        assert_eq!(SnesAddress::new(0x80, 0x841C).to_string(), "$80:841C");
        assert_eq!(SnesAddress::new(0x00, 0x0000).to_string(), "$00:0000");
        assert_eq!(FileOffset(0x41C).to_string(), "0x00041C");
        assert_eq!(FileOffset(0x40FFC0).to_string(), "0x40FFC0");
    }
}
