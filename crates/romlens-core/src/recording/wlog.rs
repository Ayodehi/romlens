//! The `WLOG` layer: DMA transfers started in a frame (docs/13, "`WLOG`").
//!
//! Records are 104 bytes each, so a reader skips a kind it does not know.
//! Kind 1 is a write to `MDMAEN` with the eight channels' registers. Kind 3,
//! from recorder stream version 2, follows its kind 1 and says where the
//! bytes went (`VMADD`, `CGADD`, the OAM and WRAM port addresses) and which
//! instruction started the transfer: what makes "which DMA filled this VRAM
//! word" exact rather than inferred (docs/22, P3).

use crate::recording::io_state::{DMA_CHANNEL_LEN, IoState};

pub const KIND_DMA: u8 = 1;
pub const KIND_DMA_CONTEXT: u8 = 3;
pub const RECORD_LEN: usize = 8 + 8 * DMA_CHANNEL_LEN;

/// The ports a DMA writes through, as they stood when it started, and the
/// instruction that wrote `MDMAEN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DmaContext {
    /// Master cycles into the scanline.
    pub h_clock: u16,
    /// `VMADD`, the VRAM word address.
    pub vram_address: u16,
    /// `CGADD`.
    pub cgram_address: u8,
    /// The OAM byte address.
    pub oam_address: u16,
    /// `WMADD`, the WRAM port's address (17 bits).
    pub wram_address: u32,
    /// The program bank and counter of the instruction that wrote `$420B`.
    pub k: u8,
    pub pc: u16,
}

/// One general DMA start as the log holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmaRecord {
    /// The value written to `MDMAEN`: the channels started.
    pub value: u8,
    pub scanline: u16,
    /// The eight channels' `$43x0`–`$43xB`, 12 bytes each.
    pub channels: [u8; 8 * DMA_CHANNEL_LEN],
    pub context: Option<DmaContext>,
}

impl DmaRecord {
    /// Channel `ch`'s twelve registers.
    pub fn channel(&self, ch: usize) -> &[u8] {
        &self.channels[ch * DMA_CHANNEL_LEN..(ch + 1) * DMA_CHANNEL_LEN]
    }
}

/// A frame's body: the frame (u64), the record count (u32), the records.
pub fn encode(frame: u64, records: &[DmaRecord]) -> Vec<u8> {
    let count: usize = records
        .iter()
        .map(|r| 1 + r.context.is_some() as usize)
        .sum();
    let mut b = Vec::with_capacity(12 + count * RECORD_LEN);
    b.extend_from_slice(&frame.to_le_bytes());
    b.extend_from_slice(&(count as u32).to_le_bytes());
    for r in records {
        b.extend_from_slice(&[KIND_DMA, r.value]);
        b.extend_from_slice(&r.scanline.to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&r.channels);
        if let Some(c) = r.context {
            let mut rec = [0u8; RECORD_LEN];
            rec[0] = KIND_DMA_CONTEXT;
            rec[2..4].copy_from_slice(&r.scanline.to_le_bytes());
            rec[4..6].copy_from_slice(&c.h_clock.to_le_bytes());
            rec[8..10].copy_from_slice(&c.vram_address.to_le_bytes());
            rec[10] = c.cgram_address;
            rec[11..13].copy_from_slice(&c.oam_address.to_le_bytes());
            rec[13..17].copy_from_slice(&c.wram_address.to_le_bytes());
            rec[17] = c.k;
            rec[18..20].copy_from_slice(&c.pc.to_le_bytes());
            b.extend_from_slice(&rec);
        }
    }
    b
}

/// The channel registers of a stream's `$4300`–`$437F`, as a record keeps
/// them.
pub fn channels_from(registers: &[u8; 128]) -> [u8; 8 * DMA_CHANNEL_LEN] {
    let mut io = IoState::default();
    io.set_dma(registers);
    let mut out = [0u8; 8 * DMA_CHANNEL_LEN];
    out.copy_from_slice(&io.bytes[crate::recording::io_state::DMA..]);
    out
}

/// A body's frame and DMA starts, each with its context when one follows.
/// Kinds it does not know are skipped.
pub fn decode(body: &[u8]) -> Result<(u64, Vec<DmaRecord>), String> {
    if body.len() < 12 {
        return Err("a write log chunk is shorter than its 12-byte head".into());
    }
    let frame = u64::from_le_bytes(body[0..8].try_into().unwrap());
    let count = u32::from_le_bytes(body[8..12].try_into().unwrap()) as usize;
    if body.len() != 12 + count * RECORD_LEN {
        return Err(format!(
            "a write log chunk holds {} bytes for {count} records of {RECORD_LEN}",
            body.len() - 12
        ));
    }
    let mut out: Vec<DmaRecord> = Vec::new();
    for r in body[12..].chunks(RECORD_LEN) {
        match r[0] {
            KIND_DMA => {
                let mut channels = [0u8; 8 * DMA_CHANNEL_LEN];
                channels.copy_from_slice(&r[8..]);
                out.push(DmaRecord {
                    value: r[1],
                    scanline: u16::from_le_bytes([r[2], r[3]]),
                    channels,
                    context: None,
                });
            }
            KIND_DMA_CONTEXT => {
                if let Some(last) = out.last_mut() {
                    last.context = Some(DmaContext {
                        h_clock: u16::from_le_bytes([r[4], r[5]]),
                        vram_address: u16::from_le_bytes([r[8], r[9]]),
                        cgram_address: r[10],
                        oam_address: u16::from_le_bytes([r[11], r[12]]),
                        wram_address: u32::from_le_bytes([r[13], r[14], r[15], r[16]]),
                        k: r[17],
                        pc: u16::from_le_bytes([r[18], r[19]]),
                    });
                }
            }
            _ => {}
        }
    }
    Ok((frame, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_with_and_without_context() {
        let a = DmaRecord {
            value: 1,
            scanline: 230,
            channels: [7; 96],
            context: None,
        };
        let b = DmaRecord {
            value: 2,
            scanline: 240,
            channels: [9; 96],
            context: Some(DmaContext {
                h_clock: 100,
                vram_address: 0x6000,
                cgram_address: 0x80,
                oam_address: 0x200,
                wram_address: 0x1_2345,
                k: 0x80,
                pc: 0x8123,
            }),
        };
        let body = encode(5, &[a.clone(), b.clone()]);
        assert_eq!(body.len(), 12 + 3 * RECORD_LEN);
        assert_eq!(decode(&body).unwrap(), (5, vec![a, b]));
    }
}
