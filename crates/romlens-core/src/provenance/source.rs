//! The hop before the PPU (docs/22, P4): which code filled a WRAM buffer
//! or wrote VRAM through the port, and where in the ROM the bytes are.
//!
//! The execution log (`model::exec_log`, docs/17) is aggregated: for every
//! instruction, the address runs it read and wrote over the whole session,
//! with counts. So it names the code that writes a buffer, not the write of
//! a given frame, which is why each answer here says "writes this" rather
//! than "wrote this byte".
//!
//! Where the bytes are in the ROM: first as they are (a palette or a sprite
//! sheet copied unchanged), then inside a compressed stream. Super
//! Metroid's format is the one Romlens decompresses (`sm_lz`); a stream's
//! start is taken from the log, where the decompressor's reads through its
//! source pointer each form a run from the stream's first byte.

use crate::graphics::compress::sm_lz;
use crate::memory::address::FileOffset;
use crate::model::exec_log::{Access, ExecLog, MemKind};
use crate::rom::image::RomImage;

/// What code writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// A WRAM offset, `$7E:0000` = 0 to `$7F:FFFF` = `$1FFFF`.
    Wram(u32),
    /// A PPU register written by the CPU itself, `$2100`–`$21FF`.
    Port(u16),
}

/// An instruction that writes it, and how often the log saw it do so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeWrite {
    /// The 24-bit address of the instruction.
    pub pc: u32,
    pub count: u32,
    /// The widest run of addresses it writes: a loop clearing all of WRAM
    /// writes every buffer, and says little about any one.
    pub span: u32,
}

impl CodeWrite {
    /// It writes 4 KB or more in one run: clearing or filling memory.
    pub fn clears(&self) -> bool {
        self.span >= 0x1000
    }
}

/// The instructions the log saw write `what`: the ones writing the
/// narrowest range first, so the code that fills this buffer comes before
/// the loops that clear all of memory; then the most often.
pub fn code_writing(log: &ExecLog, what: Written) -> Vec<CodeWrite> {
    let mut out: Vec<CodeWrite> = Vec::new();
    for a in &log.accesses {
        if a.access != Access::Write {
            continue;
        }
        let hit = match what {
            Written::Wram(off) => {
                a.kind == MemKind::WorkRam
                    && a.abs >= 0
                    && (a.abs as u32..a.abs as u32 + a.len.max(1)).contains(&off)
            }
            Written::Port(reg) => {
                let start = a.addr & 0xFFFF;
                a.kind == MemKind::Register
                    && (start..start + a.len.max(1)).contains(&u32::from(reg))
            }
        };
        if hit {
            match out.iter_mut().find(|c| c.pc == a.pc) {
                Some(c) => {
                    c.count += a.count;
                    c.span = c.span.max(a.len);
                }
                None => out.push(CodeWrite {
                    pc: a.pc,
                    count: a.count,
                    span: a.len,
                }),
            }
        }
    }
    // A run's length is how far one instruction's writes reach in a row;
    // the log merges an instruction's neighbouring writes into one run.
    let widest: std::collections::HashMap<u32, u32> = log
        .accesses
        .iter()
        .filter(|a| a.access == Access::Write)
        .fold(Default::default(), |mut m, a| {
            let e = m.entry(a.pc).or_insert(0);
            *e = (*e).max(a.len);
            m
        });
    for c in &mut out {
        c.span = widest.get(&c.pc).copied().unwrap_or(c.span);
    }
    out.sort_by_key(|c| (c.clears(), std::cmp::Reverse(c.count)));
    out
}

/// Where a run of bytes is in the ROM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RomSource {
    /// As it is, at `at`; `copies` places in all hold the same bytes.
    Verbatim { at: FileOffset, copies: usize },
    /// In the Super Metroid LZ stream starting at `stream`, `consumed`
    /// bytes long: its output holds the run at `output_at`, and the run's
    /// byte at the asked-for position came from the stream's byte at
    /// `input`.
    Compressed {
        stream: FileOffset,
        consumed: usize,
        output_at: usize,
        input: FileOffset,
    },
}

/// A run too plain to place: one repeated byte matches everywhere.
fn plain(run: &[u8]) -> bool {
    run.len() < 8 || run.iter().all(|&b| b == run[0])
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Where `run` is in the ROM: as it is, or in a compressed stream the log
/// saw read, with the stream byte behind `run[focus]`. `None` when it is too
/// plain to place, or not found.
pub fn rom_source(
    rom: &RomImage,
    log: Option<&ExecLog>,
    run: &[u8],
    focus: usize,
) -> Option<RomSource> {
    if plain(run) {
        return None;
    }
    let bytes = rom.bytes();
    let mut copies = 0;
    let mut first = None;
    let mut at = 0;
    while let Some(i) = find(&bytes[at..], run) {
        first.get_or_insert(at + i);
        copies += 1;
        at += i + 1;
    }
    if let Some(i) = first {
        return Some(RomSource::Verbatim {
            at: FileOffset(i as u32),
            copies,
        });
    }
    let log = log?;
    // The ROM the code read, as intervals: a decompressor reads a stream's
    // headers with one instruction and its literals with another, and the
    // log keeps each instruction's runs apart, so only their union starts
    // where the stream does. Streams stored back to back read as one
    // interval: the next starts where the last one ended.
    let mut runs: Vec<(u32, u32)> = log
        .accesses
        .iter()
        .filter(|a| a.access == Access::Read && a.kind == MemKind::PrgRom && a.abs >= 0)
        .map(|a| (a.abs as u32, a.abs as u32 + a.len.max(1)))
        .collect();
    runs.sort_unstable();
    let mut intervals: Vec<(u32, u32)> = Vec::new();
    for (s, e) in runs {
        match intervals.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => intervals.push((s, e)),
        }
    }
    let mut tried = std::collections::HashSet::new();
    for (start, end) in intervals.into_iter().filter(|(s, e)| e - s >= 4) {
        let mut at = start;
        while at < end && tried.insert(at) {
            let Some(tail) = bytes.get(at as usize..) else {
                break;
            };
            let Ok((d, origin)) = sm_lz::decompress_traced(tail) else {
                break;
            };
            if let Some(output_at) = find(&d.output, run) {
                return Some(RomSource::Compressed {
                    stream: FileOffset(at),
                    consumed: d.consumed,
                    output_at,
                    input: FileOffset(at + origin[output_at + focus.min(run.len() - 1)]),
                });
            }
            at += d.consumed as u32;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::exec_log::AccessRun;

    fn log(accesses: Vec<AccessRun>) -> ExecLog {
        ExecLog {
            rom_crc32: 0,
            rom_size: 0,
            insns: Vec::new(),
            accesses,
            flows: Vec::new(),
            dma: Vec::new(),
        }
    }

    fn run(
        pc: u32,
        addr: u32,
        abs: i32,
        len: u32,
        access: Access,
        kind: MemKind,
        count: u32,
    ) -> AccessRun {
        AccessRun {
            pc,
            addr,
            abs,
            len,
            access,
            kind,
            count,
        }
    }

    #[test]
    fn the_code_that_writes_a_buffer_or_the_port() {
        let l = log(vec![
            run(
                0x80_9000,
                0x7E_C000,
                0xC000,
                0x200,
                Access::Write,
                MemKind::WorkRam,
                60,
            ),
            run(
                0x80_9100,
                0x7E_C100,
                0xC100,
                0x20,
                Access::Write,
                MemKind::WorkRam,
                5,
            ),
            run(
                0x80_B2A0,
                0x00_2118,
                0x2118,
                1,
                Access::Write,
                MemKind::Register,
                900,
            ),
            run(
                0x80_9000,
                0x7E_C000,
                0xC000,
                0x200,
                Access::Read,
                MemKind::WorkRam,
                60,
            ),
        ]);
        let w = code_writing(&l, Written::Wram(0xC110));
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].pc, w[0].count), (0x80_9000, 60));
        let p = code_writing(&l, Written::Port(0x2118));
        assert_eq!((p.len(), p[0].pc), (1, 0x80_B2A0));
    }

    #[test]
    fn a_run_is_found_as_it_is_or_decompressed() {
        // A tile the compressor turns into an incrementing fill and a copy,
        // so its bytes are not in the stream as they are.
        let tile: Vec<u8> = (0..32).map(|i| (i % 16) as u8).collect();
        let stream = sm_lz::tests::compress(&[vec![0u8; 40], tile.clone()].concat());
        let mut code = vec![0u8; 0x100];
        code[0x40..0x40 + stream.len()].copy_from_slice(&stream);
        code[0xC0..0xC8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let rom = RomImage::from_bytes(
            crate::fixtures::build_with_code(
                crate::MappingMode::LoRom,
                0x8000,
                false,
                &code,
                "SOURCES",
            ),
            "s.sfc",
        )
        .unwrap();
        let base = rom
            .file_offset_for(crate::SnesAddress::new(0, 0x8000))
            .unwrap()
            .0;
        // As it is.
        assert_eq!(
            rom_source(&rom, None, &[1, 2, 3, 4, 5, 6, 7, 8], 0),
            Some(RomSource::Verbatim {
                at: FileOffset(base + 0xC0),
                copies: 1
            })
        );
        // Compressed, found from the stream start the log saw read.
        assert_eq!(rom_source(&rom, None, &tile, 0), None, "no log, no stream");
        let l = log(vec![run(
            0x80_B119,
            0x80_8040,
            (base + 0x40) as i32,
            stream.len() as u32,
            Access::Read,
            MemKind::PrgRom,
            1,
        )]);
        let Some(RomSource::Compressed {
            stream: at,
            output_at,
            ..
        }) = rom_source(&rom, Some(&l), &tile, 0)
        else {
            panic!("not found")
        };
        assert_eq!((at.0, output_at), (base + 0x40, 40));
        // Too plain to place.
        assert_eq!(rom_source(&rom, None, &[0; 32], 0), None);
    }
}
