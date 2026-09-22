//! `PpuState`: the write-only PPU registers as the emulator holds them.
//!
//! The CPU cannot read these back, so a recording has to carry them, and the
//! layout is ours (`13-recording-format.md`, "PPU state block"). It is 256
//! bytes, little-endian:
//!
//! | Offset | Size | Contents |
//! |---|---|---|
//! | `0x00`–`0x33` | 1 each | `$2100`–`$2133` at offset `addr − $2100`: the last byte written. The one-write registers (`INIDISP`, `OBSEL`, `BGMODE`, `BGnSC`, `BGnNBA`, `VMAIN`, `M7SEL`, windows, `TM`/`TS`, colour math, `SETINI`) are read from here |
//! | `0x40`–`0x4F` | 2 each | The eight scroll registers, `BG1HOFS` … `BG4VOFS`, as the whole value both writes built |
//! | `0x50`–`0x5B` | 2 each | `M7A`, `M7B`, `M7C`, `M7D`, `M7X`, `M7Y`, signed |
//! | `0x5C` | 2 | `OAMADDL` \| `OAMADDH` << 8, as written |
//! | `0x5E` | 2 | `VMADDL` \| `VMADDH` << 8 |
//! | `0x60` | 1 | `CGADD` |
//! | `0x62` | 2 | The fixed colour `COLDATA` built, BGR15 |
//! | the rest | | Reserved, zero; readers ignore it |
//!
//! The names come from [`crate::model::hardware`]'s register table, so there
//! is one list of PPU registers in the core, not two.

use crate::graphics::oam::ObjSelect;
use crate::graphics::tile::TileFormat;
use crate::graphics::tilemap::{ScreenSize, char_base_word, sc_base_word};
use crate::model::hardware::hardware_register;

pub const PPU_STATE_LEN: usize = 256;

const SCROLL: usize = 0x40;
const MODE7: usize = 0x50;
const OAMADD: usize = 0x5C;
const VMADD: usize = 0x5E;
const CGADD: usize = 0x60;
const FIXED_COLOUR: usize = 0x62;

/// One named field of the layout, for `rec info` and the register panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpuField {
    pub offset: usize,
    pub len: usize,
    pub name: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuState {
    pub bytes: [u8; PPU_STATE_LEN],
}

impl Default for PpuState {
    fn default() -> Self {
        PpuState {
            bytes: [0; PPU_STATE_LEN],
        }
    }
}

impl PpuState {
    /// Read a block; a short one is zero-filled.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut s = PpuState::default();
        let n = bytes.len().min(PPU_STATE_LEN);
        s.bytes[..n].copy_from_slice(&bytes[..n]);
        s
    }

    fn u16_at(&self, at: usize) -> u16 {
        u16::from_le_bytes([self.bytes[at], self.bytes[at + 1]])
    }

    fn set_u16(&mut self, at: usize, v: u16) {
        self.bytes[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }

    /// The last byte written to `$2100`–`$2133`; zero outside that window.
    pub fn register(&self, address: u16) -> u8 {
        match address {
            0x2100..=0x2133 => self.bytes[(address - 0x2100) as usize],
            _ => 0,
        }
    }

    pub fn set_register(&mut self, address: u16, value: u8) {
        if let 0x2100..=0x2133 = address {
            self.bytes[(address - 0x2100) as usize] = value;
        }
    }

    /// `bg` 1–4; `vertical` picks `VOFS` over `HOFS`.
    pub fn scroll(&self, bg: u8, vertical: bool) -> u16 {
        self.u16_at(SCROLL + scroll_slot(bg, vertical))
    }

    pub fn set_scroll(&mut self, bg: u8, vertical: bool, value: u16) {
        self.set_u16(SCROLL + scroll_slot(bg, vertical), value);
    }

    /// `M7A` … `M7Y` as 0–5.
    pub fn mode7(&self, index: usize) -> i16 {
        self.u16_at(MODE7 + index.min(5) * 2) as i16
    }

    pub fn set_mode7(&mut self, index: usize, value: i16) {
        self.set_u16(MODE7 + index.min(5) * 2, value as u16);
    }

    pub fn oam_address(&self) -> u16 {
        self.u16_at(OAMADD)
    }
    pub fn vram_address(&self) -> u16 {
        self.u16_at(VMADD)
    }
    pub fn cgram_address(&self) -> u8 {
        self.bytes[CGADD]
    }
    pub fn fixed_colour(&self) -> u16 {
        self.u16_at(FIXED_COLOUR)
    }

    pub fn set_oam_address(&mut self, v: u16) {
        self.set_u16(OAMADD, v);
    }
    pub fn set_vram_address(&mut self, v: u16) {
        self.set_u16(VMADD, v);
    }
    pub fn set_cgram_address(&mut self, v: u8) {
        self.bytes[CGADD] = v;
    }
    pub fn set_fixed_colour(&mut self, v: u16) {
        self.set_u16(FIXED_COLOUR, v);
    }

    /// `BGMODE` bits 0–2.
    pub fn bg_mode(&self) -> u8 {
        self.register(0x2105) & 7
    }

    /// Whether BG `bg` (1–4) uses 16×16 tiles: `BGMODE` bits 4–7.
    pub fn tile16(&self, bg: u8) -> bool {
        (1..=4).contains(&bg) && self.register(0x2105) >> (3 + bg) & 1 != 0
    }

    /// `BGnSC` for BG `bg` (1–4).
    pub fn bg_sc(&self, bg: u8) -> u8 {
        self.register(0x2106 + bg.clamp(1, 4) as u16)
    }

    pub fn screen_size(&self, bg: u8) -> ScreenSize {
        ScreenSize::from_bits(self.bg_sc(bg))
    }

    pub fn tilemap_word(&self, bg: u8) -> u16 {
        sc_base_word(self.bg_sc(bg))
    }

    /// The character base for BG `bg`, from `BG12NBA` / `BG34NBA`.
    pub fn char_word(&self, bg: u8) -> u16 {
        let bg = bg.clamp(1, 4);
        let reg = self.register(if bg <= 2 { 0x210B } else { 0x210C });
        let nibble = if bg % 2 == 1 { reg & 0x0F } else { reg >> 4 };
        char_base_word(nibble)
    }

    pub fn obj_select(&self) -> ObjSelect {
        ObjSelect::from_register(self.register(0x2101))
    }

    /// The layout's named fields, in offset order.
    pub fn fields() -> Vec<PpuField> {
        let mut out = Vec::new();
        for addr in 0x2100u16..=0x2133 {
            if let Some(r) = hardware_register(addr) {
                out.push(PpuField {
                    offset: (addr - 0x2100) as usize,
                    len: 1,
                    name: r.name,
                });
            }
        }
        for (i, addr) in (0x210Du16..=0x2114).enumerate() {
            out.push(PpuField {
                offset: SCROLL + i * 2,
                len: 2,
                name: hardware_register(addr).map(|r| r.name).unwrap_or("?"),
            });
        }
        for (i, addr) in (0x211Bu16..=0x2120).enumerate() {
            out.push(PpuField {
                offset: MODE7 + i * 2,
                len: 2,
                name: hardware_register(addr).map(|r| r.name).unwrap_or("?"),
            });
        }
        out.extend([
            PpuField {
                offset: OAMADD,
                len: 2,
                name: "OAMADD",
            },
            PpuField {
                offset: VMADD,
                len: 2,
                name: "VMADD",
            },
            PpuField {
                offset: CGADD,
                len: 1,
                name: "CGADD",
            },
            PpuField {
                offset: FIXED_COLOUR,
                len: 2,
                name: "COLDATA",
            },
        ]);
        out
    }
}

/// The scroll registers are `$210D`–`$2114` in the order BG1 H, BG1 V, BG2 H…
fn scroll_slot(bg: u8, vertical: bool) -> usize {
    (bg.clamp(1, 4) as usize - 1) * 4 + if vertical { 2 } else { 0 }
}

/// The depth of each background in each mode, `None` where the mode has no
/// such layer. Mode 7's BG1 is its own linear format; its BG2 (EXTBG) is out
/// of scope for Phase 2.
pub const fn bg_format(mode: u8, bg: u8) -> Option<TileFormat> {
    use TileFormat::*;
    match (mode, bg) {
        (0, 1..=4) => Some(Bpp2),
        (1, 1 | 2) => Some(Bpp4),
        (1, 3) => Some(Bpp2),
        (2, 1 | 2) => Some(Bpp4),
        (3, 1) => Some(Bpp8),
        (3, 2) => Some(Bpp4),
        (4, 1) => Some(Bpp8),
        (4, 2) => Some(Bpp2),
        (5, 1) => Some(Bpp4),
        (5, 2) => Some(Bpp2),
        (6, 1) => Some(Bpp4),
        (7, 1) => Some(Mode7),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_does_not_overlap() {
        let fields = PpuState::fields();
        let mut used = [false; PPU_STATE_LEN];
        for f in &fields {
            for (b, slot) in used.iter_mut().enumerate().skip(f.offset).take(f.len) {
                assert!(!*slot, "{} overlaps at {b:#x}", f.name);
                *slot = true;
            }
        }
        // Every PPU write register in the table has a byte.
        assert!(fields.iter().any(|f| f.name == "BGMODE" && f.offset == 5));
        assert!(
            fields
                .iter()
                .any(|f| f.name == "BG4VOFS" && f.offset == 0x4E)
        );
        assert!(fields.iter().any(|f| f.name == "M7Y" && f.offset == 0x5A));
    }

    #[test]
    fn registers_decode_to_what_the_views_need() {
        let mut s = PpuState::default();
        s.set_register(0x2105, 0x11); // mode 1, BG1 16×16
        s.set_register(0x2107, 0x51); // BG1SC: word $5000, 64×32
        s.set_register(0x210B, 0x42); // BG1 at $2000, BG2 at $4000
        s.set_register(0x210C, 0x06); // BG3 at $6000
        s.set_scroll(2, true, 0x0123);
        s.set_mode7(3, -2);
        assert_eq!(s.bg_mode(), 1);
        assert!(s.tile16(1) && !s.tile16(2));
        assert_eq!(s.tilemap_word(1), 0x5000);
        assert_eq!(s.screen_size(1), ScreenSize::S64x32);
        assert_eq!(s.char_word(1), 0x2000);
        assert_eq!(s.char_word(2), 0x4000);
        assert_eq!(s.char_word(3), 0x6000);
        assert_eq!(s.scroll(2, true), 0x0123);
        assert_eq!(s.bytes[0x46], 0x23);
        assert_eq!(s.mode7(3), -2);
        assert_eq!(PpuState::from_bytes(&s.bytes), s);
    }

    #[test]
    fn modes_name_their_depths() {
        assert_eq!(bg_format(1, 3), Some(TileFormat::Bpp2));
        assert_eq!(bg_format(1, 4), None);
        assert_eq!(bg_format(3, 1), Some(TileFormat::Bpp8));
        assert_eq!(bg_format(7, 1), Some(TileFormat::Mode7));
    }
}
