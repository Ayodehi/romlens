//! Built-in names for the hardware registers in the `$2100–$43FF` window of
//! the system banks, so unlabeled code reads as `STA $420D ; MEMSEL`
//! (docs/04). Beyond the docs/04 list this also names the WRAM port
//! (`$2180–$2183`) and the joypad serial ports (`$4016/$4017`).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    Read,
    Write,
    ReadWrite,
}

impl Access {
    pub const fn as_str(self) -> &'static str {
        match self {
            Access::Read => "R",
            Access::Write => "W",
            Access::ReadWrite => "RW",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardwareRegister {
    /// Offset within a system bank (`$2100`).
    pub address: u16,
    pub name: &'static str,
    pub access: Access,
    pub description: &'static str,
}

const fn reg(
    address: u16,
    name: &'static str,
    access: Access,
    description: &'static str,
) -> HardwareRegister {
    HardwareRegister {
        address,
        name,
        access,
        description,
    }
}

use Access::{Read as R, ReadWrite as RW, Write as W};

macro_rules! dma_channel {
    ($n:literal) => {
        [
            reg(
                0x4300 + $n * 0x10,
                concat!("DMAP", $n),
                RW,
                concat!("DMA channel ", $n, " parameters"),
            ),
            reg(
                0x4301 + $n * 0x10,
                concat!("BBAD", $n),
                RW,
                concat!("DMA channel ", $n, " B-bus address ($21xx)"),
            ),
            reg(
                0x4302 + $n * 0x10,
                concat!("A1T", $n, "L"),
                RW,
                concat!("DMA channel ", $n, " A-bus address, low"),
            ),
            reg(
                0x4303 + $n * 0x10,
                concat!("A1T", $n, "H"),
                RW,
                concat!("DMA channel ", $n, " A-bus address, high"),
            ),
            reg(
                0x4304 + $n * 0x10,
                concat!("A1B", $n),
                RW,
                concat!("DMA channel ", $n, " A-bus bank"),
            ),
            reg(
                0x4305 + $n * 0x10,
                concat!("DAS", $n, "L"),
                RW,
                concat!(
                    "DMA channel ",
                    $n,
                    " byte count / HDMA indirect address, low"
                ),
            ),
            reg(
                0x4306 + $n * 0x10,
                concat!("DAS", $n, "H"),
                RW,
                concat!(
                    "DMA channel ",
                    $n,
                    " byte count / HDMA indirect address, high"
                ),
            ),
            reg(
                0x4307 + $n * 0x10,
                concat!("DASB", $n),
                RW,
                concat!("DMA channel ", $n, " HDMA indirect bank"),
            ),
            reg(
                0x4308 + $n * 0x10,
                concat!("A2A", $n, "L"),
                RW,
                concat!("DMA channel ", $n, " HDMA table address, low"),
            ),
            reg(
                0x4309 + $n * 0x10,
                concat!("A2A", $n, "H"),
                RW,
                concat!("DMA channel ", $n, " HDMA table address, high"),
            ),
            reg(
                0x430A + $n * 0x10,
                concat!("NTRL", $n),
                RW,
                concat!("DMA channel ", $n, " HDMA line counter"),
            ),
            reg(
                0x430B + $n * 0x10,
                concat!("UNUSED", $n),
                RW,
                concat!("DMA channel ", $n, " unused byte"),
            ),
        ]
    };
}

static PPU: [HardwareRegister; 64] = [
    reg(
        0x2100,
        "INIDISP",
        W,
        "Screen display: force blank and brightness",
    ),
    reg(0x2101, "OBSEL", W, "Object size and tile base"),
    reg(0x2102, "OAMADDL", W, "OAM address, low"),
    reg(
        0x2103,
        "OAMADDH",
        W,
        "OAM address, high and priority rotation",
    ),
    reg(0x2104, "OAMDATA", W, "OAM data write"),
    reg(0x2105, "BGMODE", W, "Background mode and tile sizes"),
    reg(0x2106, "MOSAIC", W, "Mosaic size and enable"),
    reg(0x2107, "BG1SC", W, "BG1 tilemap address and size"),
    reg(0x2108, "BG2SC", W, "BG2 tilemap address and size"),
    reg(0x2109, "BG3SC", W, "BG3 tilemap address and size"),
    reg(0x210A, "BG4SC", W, "BG4 tilemap address and size"),
    reg(0x210B, "BG12NBA", W, "BG1 and BG2 character base"),
    reg(0x210C, "BG34NBA", W, "BG3 and BG4 character base"),
    reg(0x210D, "BG1HOFS", W, "BG1 horizontal scroll (write twice)"),
    reg(0x210E, "BG1VOFS", W, "BG1 vertical scroll (write twice)"),
    reg(0x210F, "BG2HOFS", W, "BG2 horizontal scroll"),
    reg(0x2110, "BG2VOFS", W, "BG2 vertical scroll"),
    reg(0x2111, "BG3HOFS", W, "BG3 horizontal scroll"),
    reg(0x2112, "BG3VOFS", W, "BG3 vertical scroll"),
    reg(0x2113, "BG4HOFS", W, "BG4 horizontal scroll"),
    reg(0x2114, "BG4VOFS", W, "BG4 vertical scroll"),
    reg(0x2115, "VMAIN", W, "VRAM address increment mode"),
    reg(0x2116, "VMADDL", W, "VRAM address, low"),
    reg(0x2117, "VMADDH", W, "VRAM address, high"),
    reg(0x2118, "VMDATAL", W, "VRAM data write, low"),
    reg(0x2119, "VMDATAH", W, "VRAM data write, high"),
    reg(0x211A, "M7SEL", W, "Mode 7 settings"),
    reg(0x211B, "M7A", W, "Mode 7 matrix A (also multiplicand)"),
    reg(0x211C, "M7B", W, "Mode 7 matrix B (also multiplier)"),
    reg(0x211D, "M7C", W, "Mode 7 matrix C"),
    reg(0x211E, "M7D", W, "Mode 7 matrix D"),
    reg(0x211F, "M7X", W, "Mode 7 centre X"),
    reg(0x2120, "M7Y", W, "Mode 7 centre Y"),
    reg(0x2121, "CGADD", W, "CGRAM (palette) address"),
    reg(0x2122, "CGDATA", W, "CGRAM data write"),
    reg(0x2123, "W12SEL", W, "Window mask settings for BG1 and BG2"),
    reg(0x2124, "W34SEL", W, "Window mask settings for BG3 and BG4"),
    reg(
        0x2125,
        "WOBJSEL",
        W,
        "Window mask settings for OBJ and colour",
    ),
    reg(0x2126, "WH0", W, "Window 1 left position"),
    reg(0x2127, "WH1", W, "Window 1 right position"),
    reg(0x2128, "WH2", W, "Window 2 left position"),
    reg(0x2129, "WH3", W, "Window 2 right position"),
    reg(0x212A, "WBGLOG", W, "Window mask logic for backgrounds"),
    reg(0x212B, "WOBJLOG", W, "Window mask logic for OBJ and colour"),
    reg(0x212C, "TM", W, "Main screen layer enable"),
    reg(0x212D, "TS", W, "Sub screen layer enable"),
    reg(0x212E, "TMW", W, "Window mask enable, main screen"),
    reg(0x212F, "TSW", W, "Window mask enable, sub screen"),
    reg(0x2130, "CGWSEL", W, "Colour math control A"),
    reg(0x2131, "CGADSUB", W, "Colour math control B"),
    reg(0x2132, "COLDATA", W, "Fixed colour data"),
    reg(
        0x2133,
        "SETINI",
        W,
        "Screen mode select (interlace, overscan, pseudo-hires)",
    ),
    reg(0x2134, "MPYL", R, "Multiplication result, low"),
    reg(0x2135, "MPYM", R, "Multiplication result, middle"),
    reg(0x2136, "MPYH", R, "Multiplication result, high"),
    reg(0x2137, "SLHV", R, "Latch H/V counters"),
    reg(0x2138, "RDOAM", R, "OAM data read"),
    reg(0x2139, "RDVRAML", R, "VRAM data read, low"),
    reg(0x213A, "RDVRAMH", R, "VRAM data read, high"),
    reg(0x213B, "RDCGRAM", R, "CGRAM data read"),
    reg(0x213C, "OPHCT", R, "Horizontal counter latch"),
    reg(0x213D, "OPVCT", R, "Vertical counter latch"),
    reg(0x213E, "STAT77", R, "PPU1 status and version"),
    reg(0x213F, "STAT78", R, "PPU2 status and version"),
];

static APU: [HardwareRegister; 4] = [
    reg(0x2140, "APUIO0", RW, "APU I/O port 0"),
    reg(0x2141, "APUIO1", RW, "APU I/O port 1"),
    reg(0x2142, "APUIO2", RW, "APU I/O port 2"),
    reg(0x2143, "APUIO3", RW, "APU I/O port 3"),
];

static WRAM: [HardwareRegister; 4] = [
    reg(0x2180, "WMDATA", RW, "WRAM data port"),
    reg(0x2181, "WMADDL", W, "WRAM address, low"),
    reg(0x2182, "WMADDM", W, "WRAM address, middle"),
    reg(0x2183, "WMADDH", W, "WRAM address, high"),
];

static JOY: [HardwareRegister; 2] = [
    reg(0x4016, "JOYSER0", RW, "Joypad serial port 0 (write: latch)"),
    reg(0x4017, "JOYSER1", R, "Joypad serial port 1"),
];

static CPU: [HardwareRegister; 30] = [
    reg(
        0x4200,
        "NMITIMEN",
        W,
        "Interrupt enable: NMI, IRQ timers, auto joypad read",
    ),
    reg(0x4201, "WRIO", W, "Programmable I/O port, write"),
    reg(0x4202, "WRMPYA", W, "Multiplicand A"),
    reg(0x4203, "WRMPYB", W, "Multiplier B (starts the multiply)"),
    reg(0x4204, "WRDIVL", W, "Dividend, low"),
    reg(0x4205, "WRDIVH", W, "Dividend, high"),
    reg(0x4206, "WRDIVB", W, "Divisor (starts the divide)"),
    reg(0x4207, "HTIMEL", W, "H-count timer, low"),
    reg(0x4208, "HTIMEH", W, "H-count timer, high"),
    reg(0x4209, "VTIMEL", W, "V-count timer, low"),
    reg(0x420A, "VTIMEH", W, "V-count timer, high"),
    reg(0x420B, "MDMAEN", W, "General DMA enable (channel bits)"),
    reg(0x420C, "HDMAEN", W, "HDMA enable (channel bits)"),
    reg(0x420D, "MEMSEL", W, "FastROM enable"),
    reg(0x4210, "RDNMI", R, "NMI flag and CPU version"),
    reg(0x4211, "TIMEUP", R, "IRQ flag"),
    reg(0x4212, "HVBJOY", R, "H/V blank flags and joypad status"),
    reg(0x4213, "RDIO", R, "Programmable I/O port, read"),
    reg(0x4214, "RDDIVL", R, "Quotient, low"),
    reg(0x4215, "RDDIVH", R, "Quotient, high"),
    reg(0x4216, "RDMPYL", R, "Product or remainder, low"),
    reg(0x4217, "RDMPYH", R, "Product or remainder, high"),
    reg(0x4218, "JOY1L", R, "Joypad 1, low"),
    reg(0x4219, "JOY1H", R, "Joypad 1, high"),
    reg(0x421A, "JOY2L", R, "Joypad 2, low"),
    reg(0x421B, "JOY2H", R, "Joypad 2, high"),
    reg(0x421C, "JOY3L", R, "Joypad 3, low"),
    reg(0x421D, "JOY3H", R, "Joypad 3, high"),
    reg(0x421E, "JOY4L", R, "Joypad 4, low"),
    reg(0x421F, "JOY4H", R, "Joypad 4, high"),
];

static DMA: [[HardwareRegister; 12]; 8] = [
    dma_channel!(0),
    dma_channel!(1),
    dma_channel!(2),
    dma_channel!(3),
    dma_channel!(4),
    dma_channel!(5),
    dma_channel!(6),
    dma_channel!(7),
];

/// The register at a bank offset, if any.
pub fn hardware_register(address: u16) -> Option<&'static HardwareRegister> {
    match address {
        0x2100..=0x213F => Some(&PPU[(address - 0x2100) as usize]),
        0x2140..=0x2143 => Some(&APU[(address - 0x2140) as usize]),
        0x2180..=0x2183 => Some(&WRAM[(address - 0x2180) as usize]),
        0x4016..=0x4017 => Some(&JOY[(address - 0x4016) as usize]),
        0x4200..=0x420D => Some(&CPU[(address - 0x4200) as usize]),
        0x4210..=0x421F => Some(&CPU[(address - 0x4210 + 14) as usize]),
        0x4300..=0x437F => {
            let ch = ((address - 0x4300) >> 4) as usize;
            let i = (address & 0x0F) as usize;
            (i < 12).then(|| &DMA[ch][i])
        }
        _ => None,
    }
}

/// Every named register, ascending by address.
pub fn all_hardware_registers() -> Vec<&'static HardwareRegister> {
    let mut out: Vec<&'static HardwareRegister> = PPU
        .iter()
        .chain(APU.iter())
        .chain(WRAM.iter())
        .chain(JOY.iter())
        .chain(CPU.iter())
        .chain(DMA.iter().flatten())
        .collect();
    out.sort_by_key(|r| r.address);
    out
}

/// Banks whose low half holds the hardware window (`$00–$3F`, `$80–$BF`).
pub const fn is_system_bank(bank: u8) -> bool {
    matches!(bank, 0x00..=0x3F | 0x80..=0xBF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        assert_eq!(hardware_register(0x420D).unwrap().name, "MEMSEL");
        assert_eq!(hardware_register(0x2100).unwrap().name, "INIDISP");
        assert_eq!(hardware_register(0x4212).unwrap().name, "HVBJOY");
        assert_eq!(hardware_register(0x4375).unwrap().name, "DAS7L");
        assert_eq!(hardware_register(0x430C), None);
        assert_eq!(hardware_register(0x420E), None);
        assert_eq!(hardware_register(0x2144), None);
        assert_eq!(all_hardware_registers().len(), 64 + 4 + 4 + 2 + 30 + 96);
        for r in all_hardware_registers() {
            assert_eq!(hardware_register(r.address).unwrap().name, r.name);
        }
    }
}
