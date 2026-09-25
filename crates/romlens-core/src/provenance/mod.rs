//! Where the bytes on screen came from (docs/22, P3 and P4).
//!
//! A pixel is drawn from bytes of VRAM (the tile's bitplanes, the tilemap
//! entry), CGRAM (its colour) and OAM (a sprite's entry). A recording's line
//! log (`recording::lines`) holds every byte written through the PPU's data
//! ports, in order, and its DMA log (`recording::wlog`) every general DMA
//! started, with its channels' registers. Together they say, for any byte,
//! which write put it there and, when a DMA did, which channel and which
//! byte of its source: the source address is exact, since a transfer walks
//! its source a byte at a time.
//!
//! [`last_write`] finds the last write to a byte at or before a frame;
//! [`attribute`] assigns a frame's port writes to the DMA transfers that
//! made them.

use crate::graphics::compose::Winner;
use crate::graphics::tile::byte_index;
use crate::memory::address::SnesAddress;
use crate::recording::lines::{Memory, PortHit, RegWrite, Replay};
use crate::recording::wlog::DmaRecord;
use crate::recording::{MachineStateSource, RecordingError};

/// The instruction that wrote `MDMAEN`, from the program counter Mesen
/// reports while it writes: the address after it. A store to `$420B` is
/// `STA`/`STX`/`STY`/`STZ` absolute (3 bytes), `STA` long (4) or a direct
/// page store (2); the ROM's bytes say which.
pub fn mdmaen_store(rom: &crate::rom::image::RomImage, after: SnesAddress) -> SnesAddress {
    let at = |back: u16| SnesAddress::new(after.bank(), after.offset().wrapping_sub(back));
    let bytes = |a: SnesAddress, n: usize| -> Option<Vec<u8>> {
        let o = rom.file_offset_for(a)?;
        rom.bytes()
            .get(o.0 as usize..o.0 as usize + n)
            .map(|b| b.to_vec())
    };
    if let Some(b) = bytes(at(3), 3)
        && matches!(b[0], 0x8D | 0x8E | 0x8C | 0x9C)
        && b[1] == 0x0B
        && b[2] == 0x42
    {
        return at(3);
    }
    if let Some(b) = bytes(at(4), 4)
        && b[0] == 0x8F
        && b[1] == 0x0B
        && b[2] == 0x42
    {
        return at(4);
    }
    if let Some(b) = bytes(at(2), 2)
        && matches!(b[0], 0x85 | 0x86 | 0x84 | 0x64)
    {
        return at(2);
    }
    after
}

/// A byte of one of the PPU's memories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Target {
    pub memory: Memory,
    pub byte: u32,
}

/// How sure a link is (docs/22, scope decision 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// A record names it: the DMA's start, its registers and the port
    /// writes that followed all agree.
    Exact,
    /// The writes line up with a transfer, but something about it had to be
    /// assumed (no start time or destination was recorded).
    Matched,
}

/// A byte one DMA channel wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaByte {
    /// Which of the frame's DMA starts (in time order).
    pub start: usize,
    pub channel: u8,
    /// The byte's position in the channel's transfer, from 0.
    pub index: u32,
    /// The A-bus address it was read from.
    pub source: SnesAddress,
    pub confidence: Confidence,
}

/// One channel's transfer, as its registers said when it started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transfer {
    pub channel: u8,
    /// `DMAPn`.
    pub mode: u8,
    /// `BBADn`: the B-bus register, `$21xx`.
    pub b_bus: u8,
    /// Where it read from, and how many bytes.
    pub source: SnesAddress,
    pub bytes: u32,
    /// The source address stays put (a fill) or counts down.
    pub fixed: bool,
    pub decrement: bool,
}

impl Transfer {
    fn of(r: &DmaRecord, channel: u8) -> Transfer {
        let c = r.channel(channel as usize);
        let count = u16::from_le_bytes([c[5], c[6]]);
        Transfer {
            channel,
            mode: c[0],
            b_bus: c[1],
            source: SnesAddress::new(c[4], u16::from_le_bytes([c[2], c[3]])),
            bytes: if count == 0 {
                0x10000
            } else {
                u32::from(count)
            },
            fixed: c[0] & 0x08 != 0,
            decrement: c[0] & 0x10 != 0,
        }
    }

    /// The B-bus register byte `j` goes to, as `DMAPn`'s transfer pattern
    /// cycles through `BBADn` and the registers after it.
    fn register(&self, j: u32) -> u8 {
        const PATTERNS: [&[u8]; 8] = [
            &[0],
            &[0, 1],
            &[0, 0],
            &[0, 0, 1, 1],
            &[0, 1, 2, 3],
            &[0, 1, 0, 1],
            &[0, 0],
            &[0, 0, 1, 1],
        ];
        let p = PATTERNS[(self.mode & 7) as usize];
        self.b_bus.wrapping_add(p[j as usize % p.len()])
    }

    /// The A-bus address byte `j` came from.
    pub fn source_of(&self, j: u32) -> SnesAddress {
        let step = if self.fixed {
            0
        } else if self.decrement {
            (j as u16).wrapping_neg()
        } else {
            j as u16
        };
        SnesAddress::new(self.source.bank(), self.source.offset().wrapping_add(step))
    }
}

/// A DMA start placed on the frame's line numbering: before line 0 in the
/// vertical blank that opens the frame.
fn start_time(r: &DmaRecord, total: i16, last_visible: i16) -> (i16, u16) {
    let s = r.scanline as i16;
    let line = if s > last_visible { s - total } else { s };
    (line, r.context.map_or(0, |c| c.h_clock / 4))
}

/// Which DMA channel wrote each of a frame's logged port writes: `None` for
/// a write no DMA made (the CPU's, or HDMA's). Channels run in order from
/// the lowest, each writing its count of bytes to its B-bus registers in
/// its pattern; a byte out of pattern ends the match.
pub fn attribute(writes: &[RegWrite], dma: &[DmaRecord]) -> Vec<Option<DmaByte>> {
    let mut out = vec![None; writes.len()];
    let total = if dma.iter().any(|r| r.scanline > 261) {
        312
    } else {
        262
    };
    let mut starts: Vec<(usize, &DmaRecord)> = dma.iter().enumerate().collect();
    starts.sort_by_key(|(_, r)| start_time(r, total, 224));
    for (n, r) in starts {
        let t0 = start_time(r, total, 224);
        let mut k = writes.partition_point(|w| (w.line, w.dot) < t0);
        for ch in 0..8u8 {
            if r.value & (1 << ch) == 0 {
                continue;
            }
            let t = Transfer::of(r, ch);
            // B-bus to A-bus: nothing is written to the PPU's ports.
            if t.mode & 0x80 != 0 {
                continue;
            }
            // Without the start's position within its line, skip the CPU's
            // writes earlier on it to the first of this transfer's.
            if r.context.is_none() {
                while k < writes.len() && writes[k].line == t0.0 && writes[k].reg != t.register(0) {
                    k += 1;
                }
            }
            let confidence = if r.context.is_some() {
                Confidence::Exact
            } else {
                Confidence::Matched
            };
            for j in 0..t.bytes {
                let Some(w) = writes.get(k) else { break };
                if w.reg != t.register(j) || out[k].is_some() {
                    break;
                }
                out[k] = Some(DmaByte {
                    start: n,
                    channel: ch,
                    index: j,
                    source: t.source_of(j),
                    confidence,
                });
                k += 1;
            }
        }
    }
    out
}

/// Who made the write that last put a byte where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Writer {
    Dma {
        byte: DmaByte,
        transfer: Transfer,
        /// When the DMA started, and the instruction that started it.
        started: (i16, u16),
        pc: Option<SnesAddress>,
    },
    /// A write during a visible line's horizontal blank that no general
    /// DMA made: HDMA, or code run from an H-IRQ.
    Hblank,
    /// The CPU, writing the port itself.
    Cpu,
}

/// The last write to a byte found at or before a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub frame: u64,
    pub write: RegWrite,
    pub value: u8,
    pub writer: Writer,
}

/// The data-port registers that write each memory, less `$2100`.
fn port_regs(m: Memory) -> &'static [u8] {
    match m {
        Memory::Vram => &[0x18, 0x19],
        Memory::Cgram => &[0x22],
        Memory::Oam => &[0x04],
    }
}

/// The last write to `target` in frames `earliest..=frame`, searching back.
/// `None` when no logged write reached it there: it was already in memory
/// before, or the recording has no line log.
pub fn last_write(
    src: &dyn MachineStateSource,
    frame: u64,
    target: Target,
    earliest: u64,
) -> Result<Option<Found>, RecordingError> {
    let regs = port_regs(target.memory);
    for f in (earliest.max(1)..=frame).rev() {
        let Some(writes) = src.line_writes(f)? else {
            continue;
        };
        if !writes.iter().any(|w| regs.contains(&w.reg)) {
            continue;
        }
        let Ok(before) = src.state_at(f - 1) else {
            break;
        };
        let Some(ppu) = before.ppu() else {
            break;
        };
        let (hits, writes) = Replay::new(&ppu, &[], &[], &[], writes).port_hits();
        let Some(hit) = hits
            .iter()
            .rev()
            .find(|h: &&PortHit| h.memory == target.memory && h.byte == target.byte)
        else {
            continue;
        };
        let dma = src.dma_records(f)?;
        let by = attribute(&writes, &dma);
        let w = writes[hit.write];
        let writer = match by[hit.write] {
            Some(b) => {
                let r = &dma[b.start];
                Writer::Dma {
                    byte: b,
                    transfer: Transfer::of(r, b.channel),
                    started: start_time(r, 262, 224),
                    pc: r.context.map(|c| SnesAddress::new(c.k, c.pc)),
                }
            }
            None if (1..=224).contains(&w.line) && w.dot >= 256 => Writer::Hblank,
            None => Writer::Cpu,
        };
        return Ok(Some(Found {
            frame: f,
            write: w,
            value: hit.value,
            writer,
        }));
    }
    Ok(None)
}

/// A named part of what a pixel was drawn from, with its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// "its tile's bytes for this row", "its tilemap entry", …
    pub what: &'static str,
    pub targets: Vec<Target>,
}

/// The bytes a pixel's winner was drawn from: the tile's bitplane bytes for
/// the pixel's row, the tilemap entry or the OAM entry, and the colour.
/// `bpp` is the layer's depth (8 for Mode 7).
pub fn pixel_parts(winner: &Winner, bpp: u8, mode7: bool) -> Vec<Part> {
    let vram = |byte: u32| Target {
        memory: Memory::Vram,
        byte: byte & 0xFFFF,
    };
    let colour = |c: u8| Part {
        what: "its colour",
        targets: (0..2)
            .map(|k| Target {
                memory: Memory::Cgram,
                byte: u32::from(c) * 2 + k,
            })
            .collect(),
    };
    match winner {
        Winner::Blank => Vec::new(),
        Winner::Backdrop => vec![colour(0)],
        Winner::Bg(p) if mode7 => vec![
            Part {
                what: "its pixel in the tile",
                targets: vec![vram(
                    (u32::from(p.tile) * 64 + u32::from(p.y) * 8 + u32::from(p.x)) * 2 + 1,
                )],
            },
            Part {
                what: "its map entry",
                targets: vec![vram(u32::from(p.map_word) * 2)],
            },
            colour(p.colour),
        ],
        Winner::Bg(p) => vec![
            Part {
                what: "its tile's bytes for this row",
                targets: (0..bpp)
                    .map(|plane| vram(u32::from(p.tile_word) * 2 + byte_index(plane, p.y) as u32))
                    .collect(),
            },
            Part {
                what: "its tilemap entry",
                targets: vec![
                    vram(u32::from(p.map_word) * 2),
                    vram(u32::from(p.map_word) * 2 + 1),
                ],
            },
            colour(p.colour),
        ],
        Winner::Sprite(p) => {
            let i = u32::from(p.sprite);
            vec![
                Part {
                    what: "its tile's bytes for this row",
                    targets: (0..4)
                        .map(|plane| {
                            vram(u32::from(p.tile_word) * 2 + byte_index(plane, p.y) as u32)
                        })
                        .collect(),
                },
                Part {
                    what: "its OAM entry",
                    targets: (0..4)
                        .map(|k| Target {
                            memory: Memory::Oam,
                            byte: i * 4 + k,
                        })
                        .chain(std::iter::once(Target {
                            memory: Memory::Oam,
                            byte: 0x200 + i / 4,
                        }))
                        .collect(),
                },
                colour(p.colour),
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::wlog::DmaContext;

    fn w(line: i16, dot: u16, reg: u8, value: u8) -> RegWrite {
        RegWrite {
            line,
            dot,
            reg,
            value,
        }
    }

    /// Channel `ch` of a record: mode 1 to VMDATA, `count` bytes from
    /// `$7E:C000`.
    fn record(value: u8, channels: &[(usize, u8, u8, u16)], scanline: u16, h: u16) -> DmaRecord {
        let mut c = [0u8; 96];
        for &(ch, mode, bbad, count) in channels {
            let at = ch * 12;
            c[at] = mode;
            c[at + 1] = bbad;
            c[at + 2..at + 4].copy_from_slice(&0xC000u16.to_le_bytes());
            c[at + 4] = 0x7E;
            c[at + 5..at + 7].copy_from_slice(&count.to_le_bytes());
        }
        DmaRecord {
            value,
            scanline,
            channels: c,
            context: Some(DmaContext {
                h_clock: h,
                ..DmaContext::default()
            }),
        }
    }

    #[test]
    fn each_byte_of_a_transfer_names_its_source() {
        // The CPU sets VMADD, then channel 0 sends 4 bytes to $2118/$2119
        // and channel 1 two colours' bytes to CGDATA.
        let writes = vec![
            w(-30, 10, 0x16, 0x00),
            w(-30, 12, 0x17, 0x60),
            w(-30, 40, 0x18, 0xAA),
            w(-30, 42, 0x19, 0xBB),
            w(-30, 44, 0x18, 0xCC),
            w(-30, 46, 0x19, 0xDD),
            w(-30, 60, 0x22, 0x1F),
            w(-30, 62, 0x22, 0x00),
            w(100, 280, 0x32, 0xE0),
        ];
        let dma = [record(0x03, &[(0, 1, 0x18, 4), (1, 0, 0x22, 2)], 232, 120)];
        let by = attribute(&writes, &dma);
        assert_eq!(by[0], None, "the CPU's VMADD");
        let b = by[4].unwrap();
        assert_eq!((b.channel, b.index), (0, 2));
        assert_eq!(b.source, SnesAddress::new(0x7E, 0xC002));
        assert_eq!(b.confidence, Confidence::Exact);
        let c = by[7].unwrap();
        assert_eq!((c.channel, c.index, c.source.offset()), (1, 1, 0xC001));
        assert_eq!(by[8], None, "HDMA's, not a general DMA's");
    }

    #[test]
    fn a_byte_out_of_pattern_ends_the_match() {
        let writes = vec![
            w(-30, 40, 0x18, 1),
            w(-30, 42, 0x22, 2),
            w(-30, 44, 0x19, 3),
        ];
        let dma = [record(0x01, &[(0, 1, 0x18, 4)], 232, 120)];
        let by = attribute(&writes, &dma);
        assert!(by[0].is_some());
        assert_eq!(by[1], None);
        assert_eq!(by[2], None);
    }
}
