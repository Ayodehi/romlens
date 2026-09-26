//! The sound side of a frame (docs/23, A4): every byte the S-CPU wrote to
//! the four ports, and every write the SPC700 made to its I/O registers,
//! the DSP's among them, each on the SPC700's own clock. Kept as the `APUL`
//! layer chunk after its frame.
//!
//! ```text
//! u64 frame, u8 DSPADDR when the frame began, u32 count, u8 compression
//! (0 none, 1 zstd), then the events, each: u8 kind (0 the S-CPU wrote a
//! port, 1 the SPC700 wrote $F0+address), u8 address, u8 value, u64 SPC700
//! cycle, u64 master clock (kind 0)
//! ```
//!
//! A DSP write is two I/O writes: `DSPADDR` picks the register and
//! `DSPDATA` writes it. [`ApuEvents::dsp_writes`] pairs them.

use crate::recording::RecordingError;

pub const APUL_MAGIC: &[u8; 4] = b"APUL";
pub const EVENT_LEN: usize = 19;
/// More events than a frame can hold: the SPC700 runs about 17,000
/// instructions a frame.
pub const MAX_EVENTS: usize = 1 << 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApuEventKind {
    /// The S-CPU wrote `$2140 + address`.
    CpuPort,
    /// The SPC700 wrote `$F0 + address`.
    SpcIo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApuEvent {
    pub kind: ApuEventKind,
    pub address: u8,
    pub value: u8,
    /// The SPC700's cycle count at the write.
    pub spc_cycle: u64,
    /// The S-CPU side's master clock, for its writes; 0 for the SPC700's.
    pub master_clock: u64,
}

impl ApuEvent {
    /// The SPC700 address of an I/O write (`$F0-$FF`).
    pub fn io_address(&self) -> Option<u16> {
        (self.kind == ApuEventKind::SpcIo).then_some(0xF0 | self.address as u16)
    }
}

/// A frame's events.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApuEvents {
    pub frame: u64,
    /// What `DSPADDR` held when the frame began, so its first `DSPDATA`
    /// writes can be named.
    pub dspaddr: u8,
    pub events: Vec<ApuEvent>,
}

/// A write to a DSP register, paired from its two I/O writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DspWrite {
    pub register: u8,
    pub value: u8,
    pub spc_cycle: u64,
    /// The `DSPDATA` write's place in [`ApuEvents::events`].
    pub event: usize,
}

impl ApuEvents {
    /// The DSP writes, in order. A `DSPDATA` write while `DSPADDR` holds
    /// `$80` or more is not one: those addresses only mirror the registers
    /// for reading (fullsnes).
    pub fn dsp_writes(&self) -> Vec<DspWrite> {
        let mut latch = self.dspaddr;
        let mut out = Vec::new();
        for (i, e) in self.events.iter().enumerate() {
            match e.io_address() {
                Some(0xF2) => latch = e.value,
                Some(0xF3) if latch < 0x80 => out.push(DspWrite {
                    register: latch,
                    value: e.value,
                    spc_cycle: e.spc_cycle,
                    event: i,
                }),
                _ => {}
            }
        }
        out
    }

    /// `DSPADDR` after the frame's writes.
    pub fn dspaddr_after(&self) -> u8 {
        self.events
            .iter()
            .rev()
            .find(|e| e.io_address() == Some(0xF2))
            .map_or(self.dspaddr, |e| e.value)
    }

    /// The events as the stream and the chunk lay them out, 19 bytes each.
    pub fn raw_events(events: &[ApuEvent]) -> Vec<u8> {
        let mut b = Vec::with_capacity(events.len() * EVENT_LEN);
        for e in events {
            b.push(match e.kind {
                ApuEventKind::CpuPort => 0,
                ApuEventKind::SpcIo => 1,
            });
            b.push(e.address);
            b.push(e.value);
            b.extend_from_slice(&e.spc_cycle.to_le_bytes());
            b.extend_from_slice(&e.master_clock.to_le_bytes());
        }
        b
    }

    /// The inverse of [`raw_events`](Self::raw_events).
    pub fn parse_events(raw: &[u8]) -> Result<Vec<ApuEvent>, String> {
        if !raw.len().is_multiple_of(EVENT_LEN) {
            return Err(format!(
                "{} bytes of sound events is not whole events",
                raw.len()
            ));
        }
        raw.chunks(EVENT_LEN)
            .map(|e| {
                let kind = match e[0] {
                    0 => ApuEventKind::CpuPort,
                    1 => ApuEventKind::SpcIo,
                    k => return Err(format!("a sound event of kind {k}")),
                };
                Ok(ApuEvent {
                    kind,
                    address: e[1],
                    value: e[2],
                    spc_cycle: u64::from_le_bytes(e[3..11].try_into().unwrap()),
                    master_clock: u64::from_le_bytes(e[11..19].try_into().unwrap()),
                })
            })
            .collect()
    }

    /// The `APUL` chunk's body: u64 frame, u8 DSPADDR, u32 count, u8
    /// compression, then the events, zstd-compressed with `zstd`.
    pub fn encode(&self, zstd: bool) -> Vec<u8> {
        use crate::recording::format::{COMPRESSION_NONE, COMPRESSION_ZSTD, pack};
        let raw = Self::raw_events(&self.events);
        let compression = if zstd && cfg!(feature = "recording") {
            COMPRESSION_ZSTD
        } else {
            COMPRESSION_NONE
        };
        let mut b = Vec::with_capacity(14 + raw.len());
        b.extend_from_slice(&self.frame.to_le_bytes());
        b.push(self.dspaddr);
        b.extend_from_slice(&(self.events.len() as u32).to_le_bytes());
        b.push(compression as u8);
        b.extend_from_slice(&pack(&raw, compression));
        b
    }

    pub fn decode(b: &[u8]) -> Result<ApuEvents, RecordingError> {
        let bad = RecordingError::Corrupt;
        if b.len() < 14 {
            return Err(bad("an APUL chunk is too short".to_owned()));
        }
        let frame = u64::from_le_bytes(b[0..8].try_into().unwrap());
        let dspaddr = b[8];
        let count = u32::from_le_bytes(b[9..13].try_into().unwrap()) as usize;
        if count > MAX_EVENTS {
            return Err(bad(format!(
                "an APUL chunk for frame {frame} claims {count} events"
            )));
        }
        let raw = crate::recording::format::unpack(&b[14..], count * EVENT_LEN, u32::from(b[13]))?;
        let events = Self::parse_events(&raw).map_err(bad)?;
        Ok(ApuEvents {
            frame,
            dspaddr,
            events,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io(address: u8, value: u8, cycle: u64) -> ApuEvent {
        ApuEvent {
            kind: ApuEventKind::SpcIo,
            address,
            value,
            spc_cycle: cycle,
            master_clock: 0,
        }
    }

    #[test]
    fn dsp_writes_pair_the_register_with_its_value() {
        let e = ApuEvents {
            frame: 3,
            dspaddr: 0x6C,
            events: vec![
                io(3, 0x20, 10), // FLG, from the frame before's DSPADDR
                ApuEvent {
                    kind: ApuEventKind::CpuPort,
                    address: 0,
                    value: 0x11,
                    spc_cycle: 12,
                    master_clock: 900,
                },
                io(2, 0x4C, 20),
                io(3, 0x01, 22), // KON
                io(2, 0xCC, 30),
                io(3, 0xFF, 31), // a mirror: not a write
                io(4, 0x11, 40), // CPUIO0
            ],
        };
        let w = e.dsp_writes();
        assert_eq!(w.len(), 2);
        assert_eq!(
            (w[0].register, w[0].value, w[0].spc_cycle),
            (0x6C, 0x20, 10)
        );
        assert_eq!((w[1].register, w[1].value, w[1].event), (0x4C, 0x01, 3));
        assert_eq!(e.dspaddr_after(), 0xCC);
        for zstd in [false, true] {
            let back = ApuEvents::decode(&e.encode(zstd)).unwrap();
            assert_eq!(back, e);
        }
        assert!(ApuEvents::decode(&e.encode(false)[..20]).is_err());
    }
}
