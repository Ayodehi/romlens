//! A pixel's whole chain, as data (docs/22, P5): what drew it, the writes
//! that put each of its bytes in the PPU's memories, and the hop before
//! them, for the CLI to print and the inspector to show.

use crate::graphics::compose::{Lines, Winner, compose, compose_lines};
use crate::graphics::ppu_state::bg_format;
use crate::memory::address::{FileOffset, SnesAddress};
use crate::memory::map::MemoryClass;
use crate::model::exec_log::ExecLog;
use crate::provenance::source::{CodeWrite, RomSource, Written, code_writing, rom_source};
use crate::provenance::{
    Confidence, Target, Writer, last_write, mdmaen_store, pixel_parts, write_run,
};
use crate::recording::lines::{Memory, frame_replay};
use crate::recording::{MachineState, MachineStateSource, RecordingError};
use crate::rom::image::RomImage;

/// How a group of a part's bytes got where they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// No logged write reached them between the two frames.
    NotFound {
        from: u64,
        to: u64,
    },
    Dma {
        frame: u64,
        channel: u8,
        /// The instruction that wrote `MDMAEN`.
        started_at: Option<SnesAddress>,
        /// The transfer: its first source byte, its length, its B-bus
        /// register.
        source: SnesAddress,
        bytes: u32,
        b_bus: u8,
        confidence: Confidence,
    },
    Hblank {
        frame: u64,
        line: i16,
    },
    Cpu {
        frame: u64,
        line: i16,
    },
}

/// One byte of a part, and where a DMA read it from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteLink {
    pub target: Target,
    pub source: Option<SnesAddress>,
}

/// Bytes that got there the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub link: Link,
    pub bytes: Vec<ByteLink>,
}

/// Where a run of the part's bytes is in the ROM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// "the 973 bytes written with it", "the tile", "the palette row of
    /// 16 colours".
    pub what: String,
    pub source: RomSource,
    /// The part's first byte within it: in the ROM for a run as it is, in
    /// the decompressed output for a stream.
    pub focus: u32,
}

/// The hop before the PPU: the code that filled the WRAM buffer a DMA
/// read, or wrote the port itself, and where the bytes are in the ROM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub written: Written,
    /// `None` without an execution log.
    pub code: Option<Vec<CodeWrite>>,
    pub placed: Option<Placed>,
    /// What was searched for when nothing was placed.
    pub searched: Option<String>,
}

/// One part of what a pixel was drawn from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartChain {
    pub what: &'static str,
    pub groups: Vec<Group>,
    pub hop: Option<Hop>,
    /// A DMA read these bytes straight from the ROM.
    pub from_rom: Option<SnesAddress>,
}

/// A pixel, and every part's chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    pub frame: u64,
    pub x: u32,
    pub y: u32,
    pub winner: Winner,
    pub parts: Vec<PartChain>,
}

/// What a pixel of `frame` was drawn from, and where each byte came from.
/// `earliest` says how far back to search for a byte's last write (from
/// the change index where there is one); `rom` places source addresses and
/// `log` names the code before the DMA. `None` off the screen.
pub fn chain(
    src: &dyn MachineStateSource,
    frame: u64,
    x: u32,
    y: u32,
    rom: Option<&RomImage>,
    log: Option<&ExecLog>,
    earliest: &dyn Fn(Target) -> u64,
) -> Result<Option<Chain>, RecordingError> {
    let state = src.state_at(frame)?;
    let Some(ppu) = state.ppu() else {
        return Ok(None);
    };
    let composed = match frame_replay(src, frame)? {
        Some(mut r) => compose_lines(&mut r),
        None => {
            let (Some(v), Some(c), Some(o)) = (state.vram(), state.cgram(), state.oam()) else {
                return Ok(None);
            };
            compose(v, c, o, Lines::Frame(&ppu))
        }
    };
    let Some(winner) = composed.winner(x, y) else {
        return Ok(None);
    };
    let (bpp, mode7) = match winner {
        Winner::Bg(p) => (
            bg_format(ppu.bg_mode(), p.layer).map_or(4, |f| f.bpp()),
            ppu.bg_mode() == 7,
        ),
        _ => (4, false),
    };
    let mut parts = Vec::new();
    for part in pixel_parts(&winner, bpp, mode7) {
        let mut groups: Vec<Group> = Vec::new();
        let mut first = None;
        for t in &part.targets {
            let from = earliest(*t);
            let found = last_write(src, frame, *t, from)?;
            if first.is_none() {
                first.clone_from(&found);
            }
            let (link, source) = match found {
                None => (Link::NotFound { from, to: frame }, None),
                Some(f) => match f.writer {
                    Writer::Dma {
                        byte, transfer, pc, ..
                    } => (
                        Link::Dma {
                            frame: f.frame,
                            channel: byte.channel,
                            started_at: pc.map(|p| rom.map_or(p, |r| mdmaen_store(r, p))),
                            source: transfer.source,
                            bytes: transfer.bytes,
                            b_bus: transfer.b_bus,
                            confidence: byte.confidence,
                        },
                        Some(byte.source),
                    ),
                    Writer::Hblank => (
                        Link::Hblank {
                            frame: f.frame,
                            line: f.write.line,
                        },
                        None,
                    ),
                    Writer::Cpu => (
                        Link::Cpu {
                            frame: f.frame,
                            line: f.write.line,
                        },
                        None,
                    ),
                },
            };
            let b = ByteLink { target: *t, source };
            match groups.iter_mut().find(|g| g.link == link) {
                Some(g) => g.bytes.push(b),
                None => groups.push(Group {
                    link,
                    bytes: vec![b],
                }),
            }
        }
        let mut from_rom = None;
        let mut hop = None;
        if let (Some(f), Some(rom)) = (&first, rom) {
            let run = write_run(src, f.frame, part.targets[0])?;
            match written_by(rom, f) {
                Ok(written) => {
                    hop = Some(next_hop(
                        rom,
                        log,
                        &state,
                        &winner,
                        part.what,
                        written,
                        bpp,
                        run.as_ref(),
                    ))
                }
                Err(a) => from_rom = a,
            }
        }
        parts.push(PartChain {
            what: part.what,
            groups,
            hop,
            from_rom,
        });
    }
    Ok(Some(Chain {
        frame,
        x,
        y,
        winner,
        parts,
    }))
}

/// What code wrote before the PPU: the DMA's WRAM source, or the port the
/// CPU wrote. `Err` with the ROM address for a DMA straight from the ROM,
/// and `Err(None)` for HDMA's writes, which have no buffer to follow.
fn written_by(
    rom: &RomImage,
    f: &crate::provenance::Found,
) -> Result<Written, Option<SnesAddress>> {
    match &f.writer {
        Writer::Dma { byte, .. } => {
            let a = byte.source;
            match rom.map().classify(a) {
                MemoryClass::Wram => Ok(Written::Wram(
                    (u32::from(a.bank()) - 0x7E) << 16 | u32::from(a.offset()),
                )),
                MemoryClass::LowRam => Ok(Written::Wram(u32::from(a.offset()))),
                MemoryClass::Rom => Err(Some(a)),
                _ => Err(None),
            }
        }
        Writer::Cpu => Ok(Written::Port(0x2100 + u16::from(f.write.reg))),
        Writer::Hblank => Err(None),
    }
}

#[allow(clippy::too_many_arguments)]
fn next_hop(
    rom: &RomImage,
    log: Option<&ExecLog>,
    state: &MachineState,
    w: &Winner,
    what: &str,
    written: Written,
    bpp: u8,
    run: Option<&crate::provenance::WriteRun>,
) -> Hop {
    let code = log.map(|l| code_writing(l, written));
    // The whole tile, or the whole palette row.
    let block: Option<(String, Vec<u8>)> = match (what, w) {
        ("its tile's bytes for this row", Winner::Bg(p)) => state
            .vram()
            .map(|v| ("the tile".to_owned(), tile_bytes(v, p.tile_word, bpp))),
        ("its tile's bytes for this row", Winner::Sprite(p)) => state
            .vram()
            .map(|v| ("the tile".to_owned(), tile_bytes(v, p.tile_word, 4))),
        ("its colour", _) => state.cgram().and_then(|c| {
            let colour = w.colour()?;
            let n = match w {
                Winner::Bg(_) => 1usize << bpp.min(8),
                _ => 16,
            };
            let start = (usize::from(colour) / n) * n * 2;
            c.get(start..start + n * 2)
                .map(|b| (format!("the palette row of {n} colours"), b.to_vec()))
        }),
        _ => None,
    };
    let Some((label, bytes)) = block else {
        return Hop {
            written,
            code,
            placed: None,
            searched: None,
        };
    };
    // The whole run of writes first, which a single tile often is not
    // enough to place; then the tile or palette row alone.
    if let Some(r) = run.filter(|r| r.bytes.len() > bytes.len())
        && let Some(source) = rom_source(rom, log, &r.bytes, r.at)
            .filter(|s| !matches!(s, RomSource::Verbatim { copies, .. } if *copies > 1))
    {
        return Hop {
            written,
            code,
            placed: Some(Placed {
                what: format!("the {} bytes written with it", r.bytes.len()),
                source,
                focus: r.at as u32,
            }),
            searched: None,
        };
    }
    let placed = rom_source(rom, log, &bytes, 0).map(|source| Placed {
        what: label.clone(),
        source,
        focus: 0,
    });
    Hop {
        written,
        code,
        searched: placed.is_none().then_some(label),
        placed,
    }
}

/// A tile's bytes in VRAM from its first word, `bpp` × 8 bytes.
fn tile_bytes(vram: &[u8], word: u16, bpp: u8) -> Vec<u8> {
    let start = usize::from(word) * 2;
    (0..usize::from(bpp) * 8)
        .map(|i| {
            vram.get((start + i) % vram.len().max(1))
                .copied()
                .unwrap_or(0)
        })
        .collect()
}

impl Placed {
    /// The ROM byte behind the part's first byte: in the ROM as it is, or
    /// the stream byte its decompressed byte came from.
    pub fn focus_offset(&self) -> FileOffset {
        match self.source {
            RomSource::Verbatim { at, .. } => FileOffset(at.0 + self.focus),
            RomSource::Compressed { input, .. } => input,
        }
    }
}

/// The memory a target is in, as the recording names regions.
pub fn region_of(m: Memory) -> crate::recording::StateRegion {
    match m {
        Memory::Vram => crate::recording::StateRegion::Vram,
        Memory::Cgram => crate::recording::StateRegion::Cgram,
        Memory::Oam => crate::recording::StateRegion::Oam,
    }
}

impl Link {
    /// One line: how these bytes got where they are.
    pub fn describe(&self) -> String {
        match self {
            Link::NotFound { from, to } => {
                format!("no write logged from frame {from} to {to}: already there")
            }
            Link::Dma {
                frame,
                channel,
                started_at,
                source,
                bytes,
                b_bus,
                confidence,
            } => format!(
                "frame {frame}: DMA channel {channel}{}, ${bytes:04X} bytes from {source} to $21{b_bus:02X} [{}]",
                started_at
                    .map(|p| format!(" started at {p}"))
                    .unwrap_or_default(),
                match confidence {
                    Confidence::Exact => "exact",
                    Confidence::Matched => "matched",
                }
            ),
            Link::Hblank { frame, line } => format!(
                "frame {frame} line {line}: written in a horizontal blank, by HDMA or an H-IRQ"
            ),
            Link::Cpu { frame, line } => {
                format!("frame {frame} line {line}: written by the CPU through the port")
            }
        }
    }
}

impl Hop {
    /// Where the code wrote: `WRAM $7E:C000`, or `$2118`.
    pub fn written_name(&self) -> String {
        match self.written {
            Written::Wram(o) => format!("WRAM ${:02X}:{:04X}", 0x7E + (o >> 16), o & 0xFFFF),
            Written::Port(r) => format!("${r:04X}"),
        }
    }

    /// Who writes it, from the execution log, in a line.
    pub fn describe_code(&self) -> String {
        let w = self.written_name();
        match &self.code {
            None => {
                format!("an execution log from the session would name the code that writes {w}")
            }
            Some(code) if code.is_empty() => {
                format!("{w}: the execution log saw no code write it")
            }
            Some(code) => {
                let list: Vec<String> = code
                    .iter()
                    .take(4)
                    .map(|c| {
                        format!(
                            "{} ({} times{})",
                            SnesAddress::from_u24(c.pc),
                            c.count,
                            if c.clears() { ", clearing memory" } else { "" }
                        )
                    })
                    .collect();
                format!("{w} is written by the code at {}", list.join(", "))
            }
        }
    }

    /// Where the bytes are in the ROM, in a line; with `searched`, what was
    /// not found.
    pub fn describe_placed(&self, rom: &RomImage, have_log: bool) -> Option<String> {
        let snes = |o: FileOffset| {
            rom.snes_address_for(o)
                .map(|a| a.to_string())
                .unwrap_or_default()
        };
        let Some(p) = &self.placed else {
            return self.searched.as_ref().map(|label| {
                format!(
                    "{label} is not in the ROM as it is{}",
                    if have_log {
                        ", nor in a stream the log saw read"
                    } else {
                        "; with an execution log, the compressed streams the code read are searched too"
                    }
                )
            });
        };
        let one = p.what.starts_with("the tile") || p.what.starts_with("the palette");
        let focus = p.focus_offset();
        Some(match p.source {
            RomSource::Verbatim { at, copies } => format!(
                "{} {} in the ROM as {} at {} (file offset 0x{:06X}){}; this byte at {} (0x{:06X})",
                p.what,
                if one { "is" } else { "are" },
                if one { "it is" } else { "they are" },
                snes(at),
                at.0,
                if copies > 1 {
                    format!(", one of {copies} places with the same bytes")
                } else {
                    String::new()
                },
                snes(focus),
                focus.0
            ),
            RomSource::Compressed {
                stream,
                consumed,
                output_at,
                ..
            } => format!(
                "{} {} decompressed from the Super Metroid LZ stream at {} (file offset 0x{:06X}, 0x{consumed:X} bytes), from output byte 0x{output_at:X}; this byte is output byte 0x{:X}, made from the stream's byte at {} (0x{:06X})",
                p.what,
                if one { "is" } else { "are" },
                snes(stream),
                stream.0,
                output_at as u32 + p.focus,
                snes(focus),
                focus.0
            ),
        })
    }
}

impl Chain {
    /// The pixel in a line: what drew it.
    pub fn describe(&self) -> String {
        let (x, y, frame) = (self.x, self.y, self.frame);
        match self.winner {
            Winner::Blank => {
                format!("({x}, {y}) at frame {frame}: the screen is off (forced blank)")
            }
            Winner::Backdrop => {
                format!("({x}, {y}) at frame {frame}: the backdrop, CGRAM colour 0")
            }
            Winner::Bg(p) => format!(
                "({x}, {y}) at frame {frame}: BG{}, tile ${:03X}, pixel ({}, {}), colour {}",
                p.layer, p.tile, p.x, p.y, p.colour
            ),
            Winner::Sprite(p) => format!(
                "({x}, {y}) at frame {frame}: sprite {}, tile ${:03X}, pixel ({}, {}), colour {}",
                p.sprite, p.tile, p.x, p.y, p.colour
            ),
        }
    }
}

/// A byte of the PPU's memories in words: `VRAM $C024`.
pub fn target_name(t: Target) -> String {
    let m = match t.memory {
        Memory::Vram => "VRAM",
        Memory::Cgram => "CGRAM",
        Memory::Oam => "OAM",
    };
    format!("{m} ${:04X}", t.byte)
}
