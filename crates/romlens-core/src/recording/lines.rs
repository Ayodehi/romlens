//! PPU register writes by scanline, and the registers each line was drawn
//! with (docs/13, "`LINE`"; docs/22, P1).
//!
//! A frame's snapshot holds the registers as they stood when the frame
//! ended. Games change them while the screen is drawn: an IRQ switches the
//! mode below a status bar, HDMA writes a colour gradient or a wave into the
//! scroll one line at a time. The recorder therefore also logs every write
//! to `$2100`–`$2133` with the scanline and dot it happened at, DMA's
//! bytes through the data ports included. [`Replay`] plays them over the
//! previous frame's end: the registers of each visible line, and VRAM,
//! CGRAM and OAM as they stood when it was drawn, since a game may upload
//! palettes or sprites in a forced blank part way down the screen.

use crate::graphics::ppu_state::PpuState;
use crate::recording::format::{COMPRESSION_NONE, COMPRESSION_ZSTD, pack, unpack};

/// One write to a PPU register while a frame was drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegWrite {
    /// The scanline in the frame's own numbering: 0 is the first line of
    /// the frame, the vertical blank before it is negative.
    pub line: i16,
    /// The dot within the line, 0–340.
    pub dot: u16,
    /// `$2100` + `reg`.
    pub reg: u8,
    pub value: u8,
}

/// Bytes per write in a `LINE` chunk.
pub const WRITE_LEN: usize = 6;

/// The first dot of the visible picture: a write before it still reaches
/// the line it is on.
const FIRST_DOT: u16 = 22;

/// A `LINE` chunk's body: the frame (u64), the write count (u32), a
/// compression byte (as the frame payloads: 0 none, 1 zstd), then the writes, 6 bytes each (i16
/// line, u16 dot, u8 register, u8 value), compressed as one block.
pub fn encode(frame: u64, writes: &[RegWrite], zstd: bool) -> Vec<u8> {
    let mut raw = Vec::with_capacity(writes.len() * WRITE_LEN);
    for w in writes {
        raw.extend_from_slice(&w.line.to_le_bytes());
        raw.extend_from_slice(&w.dot.to_le_bytes());
        raw.extend_from_slice(&[w.reg, w.value]);
    }
    let mut b = Vec::with_capacity(13 + raw.len());
    b.extend_from_slice(&frame.to_le_bytes());
    b.extend_from_slice(&(writes.len() as u32).to_le_bytes());
    let compression = if zstd && cfg!(feature = "recording") {
        COMPRESSION_ZSTD
    } else {
        COMPRESSION_NONE
    };
    b.push(compression as u8);
    b.extend_from_slice(&pack(&raw, compression));
    b
}

/// A `LINE` chunk's frame and writes.
pub fn decode(body: &[u8]) -> Result<(u64, Vec<RegWrite>), String> {
    if body.len() < 13 {
        return Err("a line-write chunk is shorter than its 13-byte head".into());
    }
    let frame = u64::from_le_bytes(body[0..8].try_into().unwrap());
    let count = u32::from_le_bytes(body[8..12].try_into().unwrap()) as usize;
    let raw = unpack(&body[13..], count * WRITE_LEN, u32::from(body[12]))
        .map_err(|e| format!("a line-write chunk: {e}"))?;
    if raw.len() != count * WRITE_LEN {
        return Err(format!(
            "a line-write chunk holds {} bytes for {count} writes",
            raw.len()
        ));
    }
    let writes = raw
        .chunks(WRITE_LEN)
        .map(|w| RegWrite {
            line: i16::from_le_bytes([w[0], w[1]]),
            dot: u16::from_le_bytes([w[2], w[3]]),
            reg: w[4],
            value: w[5],
        })
        .collect();
    Ok((frame, writes))
}

/// The two-write registers' latches, as the PPU keeps them.
#[derive(Debug, Clone, Copy, Default)]
struct Latches {
    /// The previous byte written to any BG scroll register, and to any
    /// horizontal one: `BGnHOFS` takes bits 3–7 of the first and bits 0–2
    /// of the second (Mesen, anomie).
    bg_old: u8,
    h_old: u8,
    /// The previous byte written to any Mode 7 register (the scroll ports
    /// share it).
    m7_old: u8,
    /// BG1's scroll and M7HOFS/M7VOFS, which share ports, apart.
    bg1: (u16, u16),
    m7: (u16, u16),
}

fn apply(s: &mut PpuState, l: &mut Latches, reg: u8, v: u8) {
    let addr = 0x2100 + u16::from(reg);
    match addr {
        0x210D..=0x2114 => {
            let bg = ((addr - 0x210D) / 2 + 1) as u8;
            let vertical = (addr - 0x210D) % 2 == 1;
            let new = if vertical {
                u16::from(v) << 8 | u16::from(l.bg_old)
            } else {
                u16::from(v) << 8 | u16::from(l.bg_old & !7) | u16::from(l.h_old & 7)
            } & 0x3FF;
            l.bg_old = v;
            if !vertical {
                l.h_old = v;
            }
            if bg == 1 {
                let m7 = (u16::from(v) << 8 | u16::from(l.m7_old)) & 0x1FFF;
                l.m7_old = v;
                if vertical {
                    l.bg1.1 = new;
                    l.m7.1 = m7;
                } else {
                    l.bg1.0 = new;
                    l.m7.0 = m7;
                }
            } else {
                s.set_scroll(bg, vertical, new);
            }
        }
        0x211B..=0x2120 => {
            let i = (addr - 0x211B) as usize;
            let word = (u16::from(v) << 8 | u16::from(l.m7_old)) as i16;
            l.m7_old = v;
            // M7X and M7Y are 13-bit signed.
            let value = if i >= 4 { (word << 3) >> 3 } else { word };
            s.set_mode7(i, value);
        }
        0x2132 => {
            let c = s.fixed_colour();
            let part = u16::from(v & 0x1F);
            let mut c2 = c;
            if v & 0x20 != 0 {
                c2 = c2 & !0x001F | part;
            }
            if v & 0x40 != 0 {
                c2 = c2 & !0x03E0 | part << 5;
            }
            if v & 0x80 != 0 {
                c2 = c2 & !0x7C00 | part << 10;
            }
            s.set_fixed_colour(c2);
        }
        _ => s.set_register(addr, v),
    }
}

/// Where the data ports write next, as the PPU keeps it.
#[derive(Debug, Clone, Copy, Default)]
struct Ports {
    /// `VMADD`, a word address.
    vram: u16,
    /// `CGADD`, and the first byte of a colour waiting for its second (with
    /// the write it came from).
    cgram: u8,
    cgram_low: Option<(u8, usize)>,
    /// The OAM byte address, and the even byte waiting for its odd one.
    oam: u16,
    oam_low: (u8, usize),
}

/// One of the PPU's three memories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Memory {
    Vram,
    Cgram,
    Oam,
}

/// A data-port write that reached a byte of memory: which write of the
/// frame's log (its index) put which byte where. A colour's first byte, and
/// OAM's even byte, are latched and land with the second write; they are
/// named by the write that carried them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortHit {
    pub write: usize,
    pub memory: Memory,
    /// The byte address in that memory.
    pub byte: u32,
    pub value: u8,
}

/// `VMAIN`'s address translation (bits 2–3): the low 8, 9 or 10 bits
/// rotated left by 3, for writing tiles a row at a time.
fn remap(addr: u16, vmain: u8) -> u16 {
    let rot = |bits: u16| {
        let mask = (1u16 << bits) - 1;
        let low = addr & mask;
        (addr & !mask) | ((low << 3) & mask) | (low >> (bits - 3))
    };
    match vmain >> 2 & 3 {
        0 => addr,
        1 => rot(8),
        2 => rot(9),
        _ => rot(10),
    }
}

/// A frame's machine replayed line by line: the registers and the PPU's
/// three memories.
pub struct Replay {
    ppu: PpuState,
    latches: Latches,
    ports: Ports,
    pub vram: Vec<u8>,
    pub cgram: Vec<u8>,
    pub oam: Vec<u8>,
    writes: Vec<RegWrite>,
    next: usize,
    /// The last visible scanline, after which the PPU no longer draws.
    last_visible: i16,
    /// Where each data-port write landed, when asked for.
    hits: Option<Vec<PortHit>>,
}

impl Replay {
    /// From the previous frame's end: its registers and memories, then this
    /// frame's writes in the order they happened.
    pub fn new(
        start: &PpuState,
        vram: &[u8],
        cgram: &[u8],
        oam: &[u8],
        writes: Vec<RegWrite>,
    ) -> Self {
        let (h, v) = (start.scroll(1, false), start.scroll(1, true));
        Replay {
            ppu: start.clone(),
            latches: Latches {
                bg1: (h, v),
                m7: (h, v),
                ..Latches::default()
            },
            ports: Ports {
                vram: start.vram_address(),
                cgram: start.cgram_address(),
                oam: (start.oam_address() & 0x1FF) << 1,
                ..Ports::default()
            },
            vram: vram.to_vec(),
            cgram: cgram.to_vec(),
            oam: oam.to_vec(),
            writes,
            next: 0,
            last_visible: if start.register(0x2133) & 0x04 != 0 {
                239
            } else {
                224
            },
            hits: None,
        }
    }

    /// Replay the whole frame, returning where every data-port write went,
    /// with the frame's writes (which the hits index).
    pub fn port_hits(mut self) -> (Vec<PortHit>, Vec<RegWrite>) {
        self.hits = Some(Vec::new());
        self.advance((i16::MAX, u16::MAX));
        (self.hits.take().unwrap_or_default(), self.writes)
    }

    /// Apply every write before `(line, dot)`.
    fn advance(&mut self, until: (i16, u16)) {
        while self.next < self.writes.len() {
            let w = self.writes[self.next];
            if (w.line, w.dot) >= until {
                break;
            }
            self.write(w, self.next);
            self.next += 1;
        }
    }

    /// Everything the frame wrote: its end state, to check against the
    /// frame's own snapshot.
    pub fn finish(mut self) -> (PpuState, Vec<u8>, Vec<u8>, Vec<u8>) {
        self.advance((i16::MAX, u16::MAX));
        settle(&mut self.ppu, &self.latches);
        (self.ppu, self.vram, self.cgram, self.oam)
    }

    fn write(&mut self, w: RegWrite, index: usize) {
        let v = w.value;
        let mut hit = |memory: Memory, byte: u32, value: u8, write: usize| {
            if let Some(h) = &mut self.hits {
                h.push(PortHit {
                    write,
                    memory,
                    byte,
                    value,
                });
            }
        };
        // VRAM and OAM take writes only in a blank: the vertical one, or a
        // forced blank.
        let blank =
            self.ppu.register(0x2100) & 0x80 != 0 || !(1..=self.last_visible).contains(&w.line);
        let vmain = self.ppu.register(0x2115);
        let step = match vmain & 3 {
            0 => 1,
            1 => 32,
            _ => 128,
        };
        let p = &mut self.ports;
        match 0x2100 + u16::from(w.reg) {
            0x2102 => {
                p.oam = (p.oam & 0x200) | u16::from(v) << 1;
                self.ppu.set_register(0x2102, v);
            }
            0x2103 => {
                p.oam = (u16::from(v & 1) << 9) | (p.oam & 0x1FE);
                self.ppu.set_register(0x2103, v);
            }
            0x2104 => {
                let at = p.oam as usize;
                if blank {
                    if at < 0x200 {
                        if at & 1 == 0 {
                            p.oam_low = (v, index);
                        } else {
                            let (low, from) = p.oam_low;
                            if at < self.oam.len() {
                                self.oam[at - 1] = low;
                                self.oam[at] = v;
                            }
                            hit(Memory::Oam, at as u32 - 1, low, from);
                            hit(Memory::Oam, at as u32, v, index);
                        }
                    } else {
                        let at = 0x200 + (at & 0x1F);
                        if let Some(b) = self.oam.get_mut(at) {
                            *b = v;
                        }
                        hit(Memory::Oam, at as u32, v, index);
                    }
                }
                p.oam = (p.oam + 1) & 0x3FF;
            }
            0x2116 => p.vram = (p.vram & 0xFF00) | u16::from(v),
            0x2117 => p.vram = (p.vram & 0x00FF) | u16::from(v) << 8,
            0x2118 | 0x2119 => {
                let high = w.reg == 0x19;
                if blank {
                    let at = (remap(p.vram, vmain) as usize & 0x7FFF) * 2 + high as usize;
                    if let Some(b) = self.vram.get_mut(at) {
                        *b = v;
                    }
                    hit(Memory::Vram, at as u32, v, index);
                }
                if high == (vmain & 0x80 != 0) {
                    p.vram = p.vram.wrapping_add(step);
                }
            }
            0x2121 => {
                p.cgram = v;
                p.cgram_low = None;
            }
            0x2122 => match p.cgram_low.take() {
                None => p.cgram_low = Some((v, index)),
                Some((low, from)) => {
                    let at = usize::from(p.cgram) * 2;
                    if at + 1 < self.cgram.len() {
                        self.cgram[at] = low;
                        self.cgram[at + 1] = v & 0x7F;
                    }
                    hit(Memory::Cgram, at as u32, low, from);
                    hit(Memory::Cgram, at as u32 + 1, v & 0x7F, index);
                    p.cgram = p.cgram.wrapping_add(1);
                }
            },
            _ => apply(&mut self.ppu, &mut self.latches, w.reg, v),
        }
    }
}

impl crate::graphics::compose::LineSource for Replay {
    fn line(&mut self, y: u32) -> crate::graphics::compose::LineView<'_> {
        self.advance((y as i16 + 1, FIRST_DOT));
        settle(&mut self.ppu, &self.latches);
        crate::graphics::compose::LineView {
            ppu: &self.ppu,
            vram: &self.vram,
            cgram: &self.cgram,
            oam: &self.oam,
        }
    }
}

/// Fill in BG1's scroll slot from the latches for the line's mode: the
/// Mode 7 scroll in Mode 7, BG1's otherwise.
fn settle(s: &mut PpuState, l: &Latches) {
    let (h, v) = if s.bg_mode() == 7 { l.m7 } else { l.bg1 };
    s.set_scroll(1, false, h);
    s.set_scroll(1, true, v);
}

/// The registers of each of `height` visible lines, from the state the
/// previous frame ended with and this frame's writes in the order they
/// happened. Screen row `y` is scanline `y + 1`; a write counts for it when
/// it came before that scanline's first visible dot.
pub fn line_states(start: &PpuState, writes: &[RegWrite], height: u32) -> Vec<PpuState> {
    use crate::graphics::compose::LineSource;
    let mut r = Replay::new(start, &[], &[], &[], writes.to_vec());
    (0..height).map(|y| r.line(y).ppu.clone()).collect()
}

/// A recording's `frame`, ready to replay: the previous frame's end with
/// this frame's writes. `None` for the first frame, where the previous
/// frame is gone (a live session's window), and where the recording has no
/// line writes or lacks a memory.
pub fn frame_replay(
    src: &dyn crate::recording::MachineStateSource,
    frame: u64,
) -> Result<Option<Replay>, crate::recording::RecordingError> {
    if frame == 0 {
        return Ok(None);
    }
    let Some(writes) = src.line_writes(frame)? else {
        return Ok(None);
    };
    // A live session may no longer hold, or never have had, the frame
    // before: then there is nothing to replay over.
    let Ok(before) = src.state_at(frame - 1) else {
        return Ok(None);
    };
    let (Some(ppu), Some(vram), Some(cgram), Some(oam)) =
        (before.ppu(), before.vram(), before.cgram(), before.oam())
    else {
        return Ok(None);
    };
    Ok(Some(Replay::new(&ppu, vram, cgram, oam, writes)))
}

/// The registers of each line of `frame` in a recording, from its replay.
pub fn frame_line_states(
    src: &dyn crate::recording::MachineStateSource,
    frame: u64,
    height: u32,
) -> Result<Option<Vec<PpuState>>, crate::recording::RecordingError> {
    use crate::graphics::compose::LineSource;
    Ok(frame_replay(src, frame)?.map(|mut r| (0..height).map(|y| r.line(y).ppu.clone()).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(line: i16, dot: u16, reg: u8, value: u8) -> RegWrite {
        RegWrite {
            line,
            dot,
            reg,
            value,
        }
    }

    #[test]
    fn data_port_writes_reach_memory_in_a_blank_only() {
        use crate::graphics::compose::LineSource;
        let mut start = PpuState::default();
        start.set_register(0x2115, 0x80); // increment after the high byte
        let writes = vec![
            // In the vertical blank: VRAM word $0010 = $BEEF, colour 3.
            w(-5, 0, 0x16, 0x10),
            w(-5, 1, 0x17, 0x00),
            w(-5, 2, 0x18, 0xEF),
            w(-5, 3, 0x19, 0xBE),
            w(-5, 4, 0x21, 0x03),
            w(-5, 5, 0x22, 0x1F),
            w(-5, 6, 0x22, 0x7C),
            // Mid-screen with the display on: ignored by VRAM.
            w(50, 0, 0x18, 0x11),
            // A forced blank at line 200, then a palette upload.
            w(200, 0, 0x00, 0x80),
            w(200, 1, 0x21, 0x00),
            w(200, 2, 0x22, 0xFF),
            w(200, 3, 0x22, 0x7F),
        ];
        let mut r = Replay::new(&start, &[0; 0x10000], &[0; 512], &[0; 544], writes);
        let l = r.line(10);
        assert_eq!(&l.vram[0x20..0x22], &[0xEF, 0xBE]);
        assert_eq!(&l.cgram[6..8], &[0x1F, 0x7C]);
        assert_eq!(&l.cgram[0..2], &[0, 0], "not yet at line 10");
        let l = r.line(210);
        assert_eq!(&l.cgram[0..2], &[0xFF, 0x7F]);
        let (_, vram, ..) = r.finish();
        assert_eq!(&vram[0x22..0x24], &[0, 0], "the mid-screen write missed");
    }

    #[test]
    fn vram_remapping_rotates_the_low_bits() {
        // Mode 1: aaaaaaaaBBBccccc to aaaaaaaacccccBBB.
        assert_eq!(remap(0b1010_0000_0110_0001, 0x04), 0b1010_0000_0000_1011);
        assert_eq!(remap(0x1234, 0x00), 0x1234);
    }

    #[test]
    fn a_chunk_round_trips_compressed_or_not() {
        let writes = vec![w(-10, 5, 0x05, 0x01), w(40, 300, 0x32, 0xE0)];
        for z in [false, true] {
            let body = encode(7, &writes, z);
            assert_eq!(decode(&body).unwrap(), (7, writes.clone()));
        }
    }

    #[test]
    fn a_mode_change_below_a_status_bar() {
        let mut start = PpuState::default();
        start.set_register(0x2105, 0x01);
        // Mode 1 until an IRQ at the end of scanline 32 sets Mode 7.
        let writes = [w(32, 280, 0x05, 0x07)];
        let lines = line_states(&start, &writes, 224);
        assert_eq!(lines[30].bg_mode(), 1);
        // Row 31 is scanline 32, drawn before the write at its end.
        assert_eq!(lines[31].bg_mode(), 1);
        assert_eq!(lines[32].bg_mode(), 7);
        assert_eq!(lines[223].bg_mode(), 7);
    }

    #[test]
    fn scroll_and_colour_writes_build_their_values() {
        let start = PpuState::default();
        let writes = [
            // BG2HOFS = $0123: low byte then high.
            w(-5, 0, 0x0F, 0x23),
            w(-5, 1, 0x0F, 0x01),
            // COLDATA: red 31, then blue 4.
            w(-4, 0, 0x32, 0x3F),
            w(-4, 1, 0x32, 0x84),
            // M7A = $0100 through the Mode 7 latch.
            w(-3, 0, 0x1B, 0x00),
            w(-3, 1, 0x1B, 0x01),
        ];
        let l = &line_states(&start, &writes, 1)[0];
        assert_eq!(l.scroll(2, false), 0x123);
        assert_eq!(l.fixed_colour(), 0x001F | 4 << 10);
        assert_eq!(l.mode7(0), 0x100);
    }
}
