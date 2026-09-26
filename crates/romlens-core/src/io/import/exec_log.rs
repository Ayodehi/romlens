//! Execution logs from the Mesen fork's `emu.getExecutionLog()`.
//!
//! The format is defined in the fork's `Core/SNES/Debugger/SnesExecutionLogger.cpp`
//! and restated in `docs/17-execution-log.md`: a 20-byte header naming the
//! PRG ROM by CRC32 and size, then tagged sections of fixed-size records.
//! Unknown sections are skipped by their declared record size, so a newer
//! log with more sections still reads.
//!
//! Romlens writes the same format back for a project's merged log, so a
//! package holds one file in a format other tools can read too.

use crate::error::ProjectError;
use crate::model::exec_log::{
    Access, AccessRun, DmaRun, ExecInsn, ExecLog, Flow, FlowKind, MemKind,
};

pub const MAGIC: &[u8; 4] = b"MXLG";
pub const VERSION: u16 = 1;
const HEADER_LEN: usize = 20;

const INST: &[u8; 4] = b"INST";
const ACCS: &[u8; 4] = b"ACCS";
const FLOW: &[u8; 4] = b"FLOW";
const DMA: &[u8; 4] = b"DMA ";
const INST_LEN: usize = 16;
const ACCS_LEN: usize = 24;
const FLOW_LEN: usize = 16;
const DMA_LEN: usize = 24;

fn bad(msg: impl Into<String>) -> ProjectError {
    ProjectError::BadFormat(format!("execution log: {}", msg.into()))
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Read a log. `rom` is checked against the log's CRC32 and size, so a log
/// from another ROM, or another revision of this one, is refused rather than
/// painting its addresses over the wrong bytes.
pub fn read(bytes: &[u8], rom: &[u8]) -> Result<ExecLog, ProjectError> {
    if !bytes.starts_with(MAGIC) {
        return Err(bad("not an execution log (no MXLG header)"));
    }
    if bytes.len() < HEADER_LEN {
        return Err(bad("the header is truncated"));
    }
    let version = u16_at(bytes, 4);
    if version != VERSION {
        return Err(bad(format!(
            "version {version} is not one this reads (expected {VERSION})"
        )));
    }
    if bytes[6] == 1 {
        return Err(bad(
            "this is the sound CPU's (SPC700) log, which goes with its recording: romlens apu map --rec R --log FILE",
        ));
    }
    if bytes[6] != 0 {
        return Err(bad(format!(
            "recorded for CPU {}, not the SNES main CPU",
            bytes[6]
        )));
    }
    let crc = u32_at(bytes, 8);
    let size = u32_at(bytes, 12);
    if size as usize != rom.len() {
        return Err(bad(format!(
            "recorded from a {size}-byte ROM, but this one is {}",
            rom.len()
        )));
    }
    let ours = crate::io::crc32::crc32(rom);
    if crc != ours {
        return Err(bad(format!(
            "recorded from a ROM with CRC32 {crc:08X}, but this one is {ours:08X}"
        )));
    }
    let sections = u32_at(bytes, 16);
    let mut log = ExecLog {
        rom_crc32: crc,
        rom_size: size,
        ..Default::default()
    };
    let mut at = HEADER_LEN;
    for _ in 0..sections {
        if bytes.len() < at + 12 {
            return Err(bad("a section header is truncated"));
        }
        let tag: [u8; 4] = bytes[at..at + 4].try_into().unwrap();
        let rec = u32_at(bytes, at + 4) as usize;
        let n = u32_at(bytes, at + 8) as usize;
        at += 12;
        let body = rec
            .checked_mul(n)
            .filter(|len| at + len <= bytes.len())
            .ok_or_else(|| {
                bad(format!(
                    "section {} claims more bytes than the file has",
                    String::from_utf8_lossy(&tag)
                ))
            })?;
        let known = match &tag {
            INST => Some(INST_LEN),
            ACCS => Some(ACCS_LEN),
            FLOW => Some(FLOW_LEN),
            DMA => Some(DMA_LEN),
            _ => None,
        };
        if let Some(want) = known
            && rec < want
        {
            return Err(bad(format!(
                "{} records are {rec} bytes, fewer than the {want} this reads",
                String::from_utf8_lossy(&tag)
            )));
        }
        for r in bytes[at..at + body].chunks_exact(rec.max(1)) {
            match &tag {
                INST => log.insns.push(ExecInsn {
                    pc: u32_at(r, 0) & 0xFF_FFFF,
                    abs: u32_at(r, 4) as i32,
                    kind: MemKind::from_u8(r[8]),
                    states: r[9],
                    count: u32_at(r, 12),
                }),
                ACCS => log.accesses.push(AccessRun {
                    pc: u32_at(r, 0) & 0xFF_FFFF,
                    addr: u32_at(r, 4) & 0xFF_FFFF,
                    abs: u32_at(r, 8) as i32,
                    len: u32_at(r, 12),
                    access: if r[16] == 0 {
                        Access::Read
                    } else {
                        Access::Write
                    },
                    kind: MemKind::from_u8(r[17]),
                    count: u32_at(r, 20),
                }),
                FLOW => {
                    // A transfer kind this version does not know is dropped,
                    // not guessed at.
                    if let Some(kind) = FlowKind::from_u8(r[8]) {
                        log.flows.push(Flow {
                            from: u32_at(r, 0) & 0xFF_FFFF,
                            to: u32_at(r, 4) & 0xFF_FFFF,
                            kind,
                            count: u32_at(r, 12),
                        });
                    }
                }
                DMA => log.dma.push(DmaRun {
                    pc: u32_at(r, 0) & 0xFF_FFFF,
                    addr: u32_at(r, 4) & 0xFF_FFFF,
                    abs: u32_at(r, 8) as i32,
                    len: u32_at(r, 12),
                    bbus: r[16],
                    to_a_bus: r[17] & 1 != 0,
                    hdma: r[17] & 2 != 0,
                    mode: (r[17] >> 2) & 7,
                    kind: MemKind::from_u8(r[18]),
                    channel: r[19] & 7,
                    count: u32_at(r, 20),
                }),
                _ => {}
            }
        }
        at += body;
    }
    log.normalize();
    Ok(log)
}

/// Write a log in the same format, sections in the order `read` expects.
pub fn write(log: &ExecLog) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        HEADER_LEN
            + 48
            + log.insns.len() * INST_LEN
            + log.accesses.len() * ACCS_LEN
            + log.flows.len() * FLOW_LEN
            + log.dma.len() * DMA_LEN,
    );
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&log.rom_crc32.to_le_bytes());
    out.extend_from_slice(&log.rom_size.to_le_bytes());
    out.extend_from_slice(&4u32.to_le_bytes());
    let section = |out: &mut Vec<u8>, tag: &[u8; 4], rec: usize, n: usize| {
        out.extend_from_slice(tag);
        out.extend_from_slice(&(rec as u32).to_le_bytes());
        out.extend_from_slice(&(n as u32).to_le_bytes());
    };
    section(&mut out, INST, INST_LEN, log.insns.len());
    for i in &log.insns {
        out.extend_from_slice(&i.pc.to_le_bytes());
        out.extend_from_slice(&i.abs.to_le_bytes());
        out.extend_from_slice(&[i.kind.to_u8(), i.states, 0, 0]);
        out.extend_from_slice(&i.count.to_le_bytes());
    }
    section(&mut out, ACCS, ACCS_LEN, log.accesses.len());
    for a in &log.accesses {
        out.extend_from_slice(&a.pc.to_le_bytes());
        out.extend_from_slice(&a.addr.to_le_bytes());
        out.extend_from_slice(&a.abs.to_le_bytes());
        out.extend_from_slice(&a.len.to_le_bytes());
        let access = match a.access {
            Access::Read => 0,
            Access::Write => 1,
        };
        out.extend_from_slice(&[access, a.kind.to_u8(), 0, 0]);
        out.extend_from_slice(&a.count.to_le_bytes());
    }
    section(&mut out, FLOW, FLOW_LEN, log.flows.len());
    for f in &log.flows {
        out.extend_from_slice(&f.from.to_le_bytes());
        out.extend_from_slice(&f.to.to_le_bytes());
        out.extend_from_slice(&[f.kind.to_u8(), 0, 0, 0]);
        out.extend_from_slice(&f.count.to_le_bytes());
    }
    section(&mut out, DMA, DMA_LEN, log.dma.len());
    for d in &log.dma {
        out.extend_from_slice(&d.pc.to_le_bytes());
        out.extend_from_slice(&d.addr.to_le_bytes());
        out.extend_from_slice(&d.abs.to_le_bytes());
        out.extend_from_slice(&d.len.to_le_bytes());
        let flags = u8::from(d.to_a_bus) | u8::from(d.hdma) << 1 | (d.mode & 7) << 2;
        out.extend_from_slice(&[d.bbus, flags, d.kind.to_u8(), d.channel]);
        out.extend_from_slice(&d.count.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom() -> Vec<u8> {
        (0..0x8000u32).map(|i| (i * 7) as u8).collect()
    }

    fn sample(rom: &[u8]) -> ExecLog {
        let mut log = ExecLog {
            rom_crc32: crate::io::crc32::crc32(rom),
            rom_size: rom.len() as u32,
            insns: vec![ExecInsn {
                pc: 0x80_8000,
                abs: 0,
                kind: MemKind::PrgRom,
                states: 0b1001,
                count: 7,
            }],
            accesses: vec![AccessRun {
                pc: 0x80_8000,
                addr: 0x7E_0100,
                abs: 0x100,
                len: 2,
                access: Access::Write,
                kind: MemKind::WorkRam,
                count: 2,
            }],
            flows: vec![Flow {
                from: 0x80_8000,
                to: 0x80_8123,
                kind: FlowKind::IndirectJump,
                count: 1,
            }],
            dma: vec![DmaRun {
                pc: 0x80_8000,
                addr: 0x7F_0000,
                abs: 0x1_0000,
                len: 0x800,
                bbus: 0x18,
                to_a_bus: false,
                hdma: false,
                mode: 1,
                kind: MemKind::WorkRam,
                channel: 1,
                count: 0x800,
            }],
        };
        log.normalize();
        log
    }

    #[test]
    fn round_trips() {
        let rom = rom();
        let log = sample(&rom);
        assert_eq!(read(&write(&log), &rom).unwrap(), log);
    }

    #[test]
    fn refuses_another_rom_and_skips_unknown_sections() {
        let rom = rom();
        let bytes = write(&sample(&rom));
        let mut other = rom.clone();
        other[5] ^= 1;
        let err = read(&bytes, &other).unwrap_err();
        assert!(format!("{err}").contains("CRC32"), "{err}");
        assert!(format!("{}", read(&bytes, &rom[..0x4000]).unwrap_err()).contains("16384"));

        // A future section, then the four known ones.
        let mut future = bytes[..HEADER_LEN].to_vec();
        future[16..20].copy_from_slice(&5u32.to_le_bytes());
        future.extend_from_slice(b"XTRA");
        future.extend_from_slice(&3u32.to_le_bytes());
        future.extend_from_slice(&2u32.to_le_bytes());
        future.extend_from_slice(&[9; 6]);
        future.extend_from_slice(&bytes[HEADER_LEN..]);
        assert_eq!(read(&future, &rom).unwrap(), sample(&rom));

        assert!(read(b"MXLG", &rom).is_err());
        assert!(
            read(&bytes[..bytes.len() - 1], &rom).is_err(),
            "a short section"
        );
    }
}
