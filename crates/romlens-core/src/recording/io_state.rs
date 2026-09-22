//! The `io` region: the CPU-side I/O registers at a frame boundary
//! (`13-recording-format.md`, "I/O state block"). 128 bytes, little-endian.
//!
//! | Offset | Size | Contents |
//! |---|---|---|
//! | `0x00` | 1 | `NMITIMEN` (`$4200`) |
//! | `0x01` | 1 | `HDMAEN` (`$420C`) |
//! | `0x02` | 1 | `MEMSEL` (`$420D`) |
//! | `0x03` | 1 | `WRIO` (`$4201`) |
//! | `0x04` | 2 | `HTIME` (`$4207`/8) |
//! | `0x06` | 2 | `VTIME` (`$4209`/A) |
//! | `0x08` | 1 | `WRMPYA` (`$4202`) |
//! | `0x09` | 1 | `WRMPYB` (`$4203`) |
//! | `0x0A` | 2 | `WRDIVL`/`WRDIVH` (`$4204`/5) |
//! | `0x0C` | 1 | `WRDIVB` (`$4206`) |
//! | `0x0E` | 2 | the H counter |
//! | `0x10` | 2 | the V counter |
//! | `0x12` | 8 | `JOY1`–`JOY4` (`$4218`–`$421F`) |
//! | `0x20` | 96 | the eight DMA channels, 12 bytes each: `$43x0`–`$43xB` |
//!
//! Everything else is reserved and zero. A 1.0 file written before this
//! layout was defined carries zeroes throughout.

pub const IO_STATE_LEN: usize = 128;

pub const NMITIMEN: usize = 0x00;
pub const HDMAEN: usize = 0x01;
pub const MEMSEL: usize = 0x02;
pub const WRIO: usize = 0x03;
pub const HTIME: usize = 0x04;
pub const VTIME: usize = 0x06;
pub const WRMPYA: usize = 0x08;
pub const WRMPYB: usize = 0x09;
pub const WRDIV: usize = 0x0A;
pub const WRDIVB: usize = 0x0C;
pub const H_COUNTER: usize = 0x0E;
pub const V_COUNTER: usize = 0x10;
pub const JOY: usize = 0x12;
pub const DMA: usize = 0x20;
/// Bytes of each DMA channel kept: `$43x0`–`$43xB`.
pub const DMA_CHANNEL_LEN: usize = 12;

const _: () = assert!(JOY + 8 <= DMA && DMA + 8 * DMA_CHANNEL_LEN == IO_STATE_LEN);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoState {
    pub bytes: [u8; IO_STATE_LEN],
}

impl Default for IoState {
    fn default() -> Self {
        IoState {
            bytes: [0; IO_STATE_LEN],
        }
    }
}

impl IoState {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut s = IoState::default();
        let n = bytes.len().min(IO_STATE_LEN);
        s.bytes[..n].copy_from_slice(&bytes[..n]);
        s
    }

    pub fn set_u8(&mut self, at: usize, v: u8) {
        self.bytes[at] = v;
    }

    pub fn set_u16(&mut self, at: usize, v: u16) {
        self.bytes[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }

    pub fn u16_at(&self, at: usize) -> u16 {
        u16::from_le_bytes([self.bytes[at], self.bytes[at + 1]])
    }

    /// Channel `ch`'s registers, `$43c0`–`$43cB`.
    pub fn dma_channel(&self, ch: usize) -> &[u8] {
        let at = DMA + ch * DMA_CHANNEL_LEN;
        &self.bytes[at..at + DMA_CHANNEL_LEN]
    }

    /// Fill the channels from the 128 bytes `$4300`–`$437F`.
    pub fn set_dma(&mut self, registers: &[u8; 128]) {
        for ch in 0..8 {
            let at = DMA + ch * DMA_CHANNEL_LEN;
            self.bytes[at..at + DMA_CHANNEL_LEN]
                .copy_from_slice(&registers[ch * 16..ch * 16 + DMA_CHANNEL_LEN]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_land_in_order() {
        let mut regs = [0u8; 128];
        for (i, b) in regs.iter_mut().enumerate() {
            *b = i as u8;
        }
        let mut io = IoState::default();
        io.set_dma(&regs);
        assert_eq!(io.dma_channel(0), &regs[0..12]);
        assert_eq!(io.dma_channel(7), &regs[0x70..0x7C]);
    }
}
