//! Per-mapping translation between CPU addresses and file offsets.
//!
//! Rules (docs/04, docs/14):
//!
//! * **LoROM**: ROM in the upper half (`$8000–$FFFF`) of every bank except
//!   `$7E`/`$7F` (WRAM). `raw = (bank & 0x7F) * 0x8000 + (addr & 0x7FFF)`.
//! * **HiROM**: whole banks `$40–$7D` and `$C0–$FF`, plus the upper halves of
//!   `$00–$3F` and `$80–$BF`. `raw = (bank & 0x3F) << 16 | addr`.
//! * **ExHiROM**: `$C0–$FF` → first 4 MB, `$40–$7D` → `0x400000 +`; upper
//!   halves of `$80–$BF` → first 4 MB, of `$00–$3F` → `0x400000 +`. So the
//!   CPU's `$00:FFC0` header slot is file `0x40FFC0`.
//!
//! A `raw` offset past the end of the image is folded back with
//! [`mirror_offset`], the same rule the checksum uses.

use crate::memory::address::{FileOffset, SnesAddress};

/// The three cartridge mappings Phase 0 understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MappingMode {
    LoRom,
    HiRom,
    ExHiRom,
}

impl MappingMode {
    /// File offset of the 32-byte internal header for this mapping.
    pub const fn header_offset(self) -> FileOffset {
        FileOffset(match self {
            MappingMode::LoRom => 0x7FC0,
            MappingMode::HiRom => 0xFFC0,
            MappingMode::ExHiRom => 0x40_FFC0,
        })
    }

    /// The low nibble of the header's map-mode byte for this mapping.
    pub const fn map_mode_nibble(self) -> u8 {
        match self {
            MappingMode::LoRom => 0x0,
            MappingMode::HiRom => 0x1,
            MappingMode::ExHiRom => 0x5,
        }
    }

    /// Mapping named by the low nibble of the header's map-mode byte.
    pub const fn from_map_mode_nibble(nibble: u8) -> Option<Self> {
        match nibble & 0x0F {
            0x0 => Some(MappingMode::LoRom),
            0x1 => Some(MappingMode::HiRom),
            0x5 => Some(MappingMode::ExHiRom),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            MappingMode::LoRom => "LoROM",
            MappingMode::HiRom => "HiROM",
            MappingMode::ExHiRom => "ExHiROM",
        }
    }

    pub const fn all() -> [MappingMode; 3] {
        [MappingMode::LoRom, MappingMode::HiRom, MappingMode::ExHiRom]
    }
}

impl std::fmt::Display for MappingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// What a CPU address points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    Rom,
    /// Banks `$7E`/`$7F`.
    Wram,
    /// The first 8 KB of WRAM mirrored into `$0000–$1FFF` of the system banks.
    LowRam,
    /// PPU, APU, CPU and DMA registers at `$2000–$5FFF` of the system banks.
    Hardware,
    Sram,
    OpenBus,
}

/// Translation rules for one image: its mapping, its length, and whether the
/// header asks for FastROM (which decides the canonical bank).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressMap {
    pub mode: MappingMode,
    pub rom_len: u32,
    pub fast_rom: bool,
}

/// Fold an offset that may be past the end of the image back into it.
///
/// Power-of-two images simply wrap. Otherwise the image is split at its
/// largest power of two `big`: offsets below `big` map directly, and the rest
/// recurses into the remainder, so a 3 MB image sees `0x3FFFFF → 0x2FFFFF`.
pub fn mirror_offset(off: u32, len: u32) -> u32 {
    debug_assert!(len > 0, "mirror_offset on an empty image");
    if len == 0 {
        return 0;
    }
    if len.is_power_of_two() {
        return off & (len - 1);
    }
    let big = 1u32 << (31 - len.leading_zeros());
    let off = off & (big * 2 - 1); // the whole (big + remainder) region repeats at 2*big
    if off < big {
        off
    } else {
        big + mirror_offset(off - big, len - big)
    }
}

impl AddressMap {
    pub const fn new(mode: MappingMode, rom_len: u32, fast_rom: bool) -> Self {
        Self {
            mode,
            rom_len,
            fast_rom,
        }
    }

    /// Raw (un-mirrored) ROM offset for a CPU address, `None` when the
    /// address is not ROM under this mapping.
    fn raw_offset(&self, addr: SnesAddress) -> Option<u32> {
        let bank = addr.bank();
        let off = addr.offset();
        let upper = off >= 0x8000;
        match self.mode {
            MappingMode::LoRom => {
                if bank == 0x7E || bank == 0x7F || !upper {
                    return None;
                }
                Some((bank as u32 & 0x7F) * 0x8000 + (off as u32 & 0x7FFF))
            }
            MappingMode::HiRom => {
                let in_bank = (bank as u32 & 0x3F) << 16 | off as u32;
                match bank {
                    0x40..=0x7D | 0xC0..=0xFF => Some(in_bank),
                    0x00..=0x3F | 0x80..=0xBF if upper => Some(in_bank),
                    _ => None,
                }
            }
            MappingMode::ExHiRom => {
                let in_bank = (bank as u32 & 0x3F) << 16 | off as u32;
                match bank {
                    0xC0..=0xFF => Some(in_bank),
                    0x40..=0x7D => Some(0x40_0000 + in_bank),
                    0x80..=0xBF if upper => Some(in_bank),
                    0x00..=0x3F if upper => Some(0x40_0000 + in_bank),
                    _ => None,
                }
            }
        }
    }

    /// File offset a CPU address reads from, or `None` for anything that is
    /// not ROM (WRAM, hardware, SRAM, open bus).
    pub fn file_offset(&self, addr: SnesAddress) -> Option<FileOffset> {
        let raw = self.raw_offset(addr)?;
        if self.rom_len == 0 {
            return None;
        }
        Some(FileOffset(mirror_offset(raw, self.rom_len)))
    }

    /// The address to display for a file offset: the FastROM bank when the
    /// header asks for it, else the slow bank. `None` past the image end or
    /// for the few ExHiROM offsets no bank can reach.
    pub fn canonical(&self, off: FileOffset) -> Option<SnesAddress> {
        let off = off.0;
        if off >= self.rom_len {
            return None;
        }
        match self.mode {
            MappingMode::LoRom => {
                let bank = (off / 0x8000) as u8; // 0..=0x7F for images up to 4 MB
                let addr = 0x8000 | (off % 0x8000) as u16;
                // $7E/$7F are WRAM, so those two slots only exist as FastROM banks.
                let fast = self.fast_rom || bank >= 0x7E;
                Some(SnesAddress::new(
                    if fast { bank | 0x80 } else { bank },
                    addr,
                ))
            }
            MappingMode::HiRom => {
                let bank = (off >> 16) as u8; // 0..=0x3F
                let addr = (off & 0xFFFF) as u16;
                let fast = self.fast_rom || bank >= 0x3E;
                Some(SnesAddress::new(
                    if fast { 0xC0 | bank } else { 0x40 | bank },
                    addr,
                ))
            }
            MappingMode::ExHiRom => {
                let addr = (off & 0xFFFF) as u16;
                if off < 0x40_0000 {
                    Some(SnesAddress::new(0xC0 | (off >> 16) as u8, addr))
                } else {
                    let bank = ((off - 0x40_0000) >> 16) as u8; // 0..=0x3F
                    if bank <= 0x3D {
                        Some(SnesAddress::new(0x40 | bank, addr))
                    } else if addr >= 0x8000 {
                        Some(SnesAddress::new(bank, addr))
                    } else {
                        None
                    }
                }
            }
        }
    }

    /// Every CPU address whose mapping lands on this file offset directly,
    /// ascending: the slow and fast banks, and for HiROM the four bank
    /// windows. Image-size folding (a 32 KB image repeating in every bank)
    /// is not listed; it is a property of the size, not of the mapping.
    pub fn mirrors(&self, off: FileOffset) -> Vec<SnesAddress> {
        if off.0 >= self.rom_len {
            return Vec::new();
        }
        let low16 = (off.0 & 0xFFFF) as u16;
        let low15 = (off.0 & 0x7FFF) as u16 | 0x8000;
        let mut out = Vec::new();
        for bank in 0u8..=0xFF {
            for candidate in [low16, low15] {
                let a = SnesAddress::new(bank, candidate);
                if self.raw_offset(a) == Some(off.0) {
                    out.push(a);
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Coarse classification of a CPU address.
    pub fn classify(&self, addr: SnesAddress) -> Region {
        let bank = addr.bank();
        let off = addr.offset();
        if bank == 0x7E || bank == 0x7F {
            return Region::Wram;
        }
        let system_bank = matches!(bank, 0x00..=0x3F | 0x80..=0xBF);
        if system_bank && off < 0x8000 {
            return match off {
                0x0000..=0x1FFF => Region::LowRam,
                0x2000..=0x5FFF => Region::Hardware,
                _ => {
                    let hi_sram_bank = matches!(bank, 0x20..=0x3F | 0xA0..=0xBF);
                    if self.mode != MappingMode::LoRom && hi_sram_bank {
                        Region::Sram
                    } else {
                        Region::OpenBus
                    }
                }
            };
        }
        if self.mode == MappingMode::LoRom
            && matches!(bank, 0x70..=0x7D | 0xF0..=0xFF)
            && off < 0x8000
        {
            return Region::Sram;
        }
        if self.file_offset(addr).is_some() {
            return Region::Rom;
        }
        Region::OpenBus
    }
}
