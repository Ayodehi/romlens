//! The 32-byte internal header plus the 32-byte vector table that follows.
//!
//! Layout relative to the header offset (`0x7FC0` LoROM, `0xFFC0` HiROM,
//! `0x40FFC0` ExHiROM):
//!
//! ```text
//! +0x00  title, 21 bytes, space padded      +0x24  native COP    +0x34  emulation COP
//! +0x15  map mode ($20 LoROM, $21 HiROM,    +0x26  native BRK    +0x36  (unused)
//!        $25 ExHiROM; bit 4 = FastROM)      +0x28  native ABORT  +0x38  emulation ABORT
//! +0x16  cartridge type                     +0x2A  native NMI    +0x3A  emulation NMI
//! +0x17  ROM size code (1 << (10 + n))      +0x2C  (unused)      +0x3C  emulation RESET
//! +0x18  RAM size code                      +0x2E  native IRQ    +0x3E  emulation IRQ/BRK
//! +0x19  region
//! +0x1A  developer id ($33 = extended header at -0x10)
//! +0x1B  version
//! +0x1C  complement (LE)   +0x1E  checksum (LE)
//! ```

use crate::memory::map::MappingMode;

pub const HEADER_LEN: usize = 0x40;
pub const TITLE_LEN: usize = 21;
pub const EXTENDED_HEADER_LEN: usize = 0x10;

/// The six interrupt vectors of one CPU mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vectors {
    pub cop: u16,
    /// Native BRK; in emulation mode this slot is unused (BRK shares IRQ).
    pub brk: u16,
    pub abort: u16,
    pub nmi: u16,
    /// Emulation RESET; in native mode this slot is unused.
    pub reset: u16,
    pub irq: u16,
}

impl Vectors {
    fn read(bytes: &[u8]) -> Self {
        let w = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        Self {
            cop: w(0),
            brk: w(2),
            abort: w(4),
            nmi: w(6),
            reset: w(8),
            irq: w(10),
        }
    }

    /// `(name, value)` pairs in table order.
    pub fn named(&self, native: bool) -> [(&'static str, u16); 6] {
        if native {
            [
                ("COP", self.cop),
                ("BRK", self.brk),
                ("ABORT", self.abort),
                ("NMI", self.nmi),
                ("RESET (unused)", self.reset),
                ("IRQ", self.irq),
            ]
        } else {
            [
                ("COP", self.cop),
                ("(unused)", self.brk),
                ("ABORT", self.abort),
                ("NMI", self.nmi),
                ("RESET", self.reset),
                ("IRQ/BRK", self.irq),
            ]
        }
    }
}

/// The 16 bytes before the header, present when the developer id is `$33`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedHeader {
    pub maker_code: String,
    pub game_code: String,
    pub expansion_flash_size: u8,
    pub expansion_ram_size: u8,
    pub special_version: u8,
    pub chipset_subtype: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomHeader {
    pub title_raw: [u8; TITLE_LEN],
    /// Title with trailing padding removed, invalid bytes replaced.
    pub title: String,
    pub map_mode: u8,
    pub cartridge_type: u8,
    pub rom_size_code: u8,
    pub ram_size_code: u8,
    pub region: u8,
    pub developer_id: u8,
    pub version: u8,
    pub complement: u16,
    pub checksum: u16,
    pub native: Vectors,
    pub emulation: Vectors,
    pub extended: Option<ExtendedHeader>,
}

fn ascii_lossy(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (0x20..0x7F).contains(&b) {
                b as char
            } else {
                '\u{FFFD}'
            }
        })
        .collect()
}

impl RomHeader {
    /// Parse the header at `offset`; `None` when the payload is too short.
    pub fn parse(payload: &[u8], offset: usize) -> Option<Self> {
        let h = payload.get(offset..offset + HEADER_LEN)?;
        let mut title_raw = [0u8; TITLE_LEN];
        title_raw.copy_from_slice(&h[..TITLE_LEN]);
        let title = ascii_lossy(&title_raw).trim_end().to_owned();
        let developer_id = h[0x1A];
        let extended = if developer_id == 0x33 && offset >= EXTENDED_HEADER_LEN {
            let e = &payload[offset - EXTENDED_HEADER_LEN..offset];
            Some(ExtendedHeader {
                maker_code: ascii_lossy(&e[0..2]),
                game_code: ascii_lossy(&e[2..6]),
                expansion_flash_size: e[0x0C],
                expansion_ram_size: e[0x0D],
                special_version: e[0x0E],
                chipset_subtype: e[0x0F],
            })
        } else {
            None
        };
        Some(Self {
            title_raw,
            title,
            map_mode: h[0x15],
            cartridge_type: h[0x16],
            rom_size_code: h[0x17],
            ram_size_code: h[0x18],
            region: h[0x19],
            developer_id,
            version: h[0x1B],
            complement: u16::from_le_bytes([h[0x1C], h[0x1D]]),
            checksum: u16::from_le_bytes([h[0x1E], h[0x1F]]),
            native: Vectors::read(&h[0x24..0x30]),
            emulation: Vectors::read(&h[0x34..0x40]),
            extended,
        })
    }

    /// Bit 4 of the map-mode byte.
    pub const fn is_fast_rom(&self) -> bool {
        self.map_mode & 0x10 != 0
    }

    /// Mapping named by the low nibble of the map-mode byte, if it is one
    /// Phase 0 models.
    pub const fn mapping(&self) -> Option<MappingMode> {
        MappingMode::from_map_mode_nibble(self.map_mode)
    }

    pub const fn complement_valid(&self) -> bool {
        self.checksum ^ self.complement == 0xFFFF
    }

    /// Bytes the size code declares (`1 << (10 + code)`), saturating for
    /// nonsense codes.
    pub const fn declared_rom_size(&self) -> u64 {
        Self::size_from_code(self.rom_size_code)
    }

    pub const fn declared_ram_size(&self) -> u64 {
        if self.ram_size_code == 0 {
            0
        } else {
            Self::size_from_code(self.ram_size_code)
        }
    }

    const fn size_from_code(code: u8) -> u64 {
        if code > 40 {
            u64::MAX
        } else {
            1u64 << (10 + code)
        }
    }

    /// All 21 title bytes are printable ASCII.
    pub fn title_printable(&self) -> bool {
        self.title_raw.iter().all(|b| (0x20..0x7F).contains(b))
    }

    pub fn region_name(&self) -> &'static str {
        match self.region {
            0x00 => "Japan",
            0x01 => "USA / Canada",
            0x02 => "Europe",
            0x03 => "Sweden / Scandinavia",
            0x04 => "Finland",
            0x05 => "Denmark",
            0x06 => "France",
            0x07 => "Netherlands",
            0x08 => "Spain",
            0x09 => "Germany",
            0x0A => "Italy",
            0x0B => "China / Hong Kong",
            0x0C => "Indonesia",
            0x0D => "South Korea",
            0x0E => "Common / International",
            0x0F => "Canada",
            0x10 => "Brazil",
            0x11 => "Australia",
            _ => "unknown",
        }
    }

    pub fn cartridge_type_name(&self) -> &'static str {
        match self.cartridge_type {
            0x00 => "ROM only",
            0x01 => "ROM + RAM",
            0x02 => "ROM + RAM + battery",
            0x03 => "ROM + DSP",
            0x04 => "ROM + DSP + RAM",
            0x05 => "ROM + DSP + RAM + battery",
            0x13 => "ROM + Super FX",
            0x14 => "ROM + Super FX + RAM",
            0x15 => "ROM + Super FX + RAM + battery",
            0x1A => "ROM + Super FX + RAM + battery (GSU-1)",
            0x25 => "ROM + OBC1 + RAM + battery",
            0x32 => "ROM + SA-1 + RAM",
            0x33 => "ROM + SA-1",
            0x34 => "ROM + SA-1 + RAM",
            0x35 => "ROM + SA-1 + RAM + battery",
            0x43 => "ROM + S-DD1",
            0x45 => "ROM + S-DD1 + RAM + battery",
            0x55 => "ROM + S-RTC + RAM + battery",
            0xE3 => "ROM + Game Boy (Super Game Boy)",
            0xF3 => "ROM + Cx4",
            0xF5 => "ROM + ST018 + RAM + battery",
            0xF6 => "ROM + ST010/ST011",
            0xF9 => "ROM + SPC7110 + RAM + battery",
            _ => "unknown",
        }
    }
}
