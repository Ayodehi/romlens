//! The SPC700's execution log (docs/23, A6): the main CPU's format
//! (`exec_log`) with CPU 1 in its header, audio RAM's addresses, and two
//! more access kinds for the DSP's own reads and writes. The format is
//! defined in the Mesen fork's `Core/SNES/Debugger/SpcExecutionLogger.cpp`.

use crate::error::ProjectError;
use crate::model::spc_log::{SpcAccess, SpcAccessRun, SpcFlow, SpcInsn, SpcLog};

use super::exec_log::MAGIC;

const HEADER_LEN: usize = 20;
/// The log's kind for the boot ROM (`SpcExecutionLogger::MemKind`).
const KIND_BOOT_ROM: u8 = 7;

fn bad(msg: impl Into<String>) -> ProjectError {
    ProjectError::BadFormat(format!("SPC700 execution log: {}", msg.into()))
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Whether `bytes` is an execution log of the SPC700.
pub fn is_spc_log(bytes: &[u8]) -> bool {
    bytes.len() >= HEADER_LEN && bytes.starts_with(MAGIC) && bytes[6] == 1
}

/// Read a log, checking it was recorded from `rom` when given (by its CRC32
/// and size, as the main CPU's log is).
pub fn read(bytes: &[u8], rom: Option<&[u8]>) -> Result<SpcLog, ProjectError> {
    if !bytes.starts_with(MAGIC) || bytes.len() < HEADER_LEN {
        return Err(bad("not an execution log (no MXLG header)"));
    }
    if u16_at(bytes, 4) != 1 {
        return Err(bad(format!(
            "version {} is not one this reads",
            u16_at(bytes, 4)
        )));
    }
    if bytes[6] != 1 {
        return Err(bad(format!(
            "recorded for CPU {}, not the SPC700",
            bytes[6]
        )));
    }
    let crc = u32_at(bytes, 8);
    let size = u32_at(bytes, 12);
    if let Some(rom) = rom
        && (size as usize != rom.len() || crc != crate::io::crc32::crc32(rom))
    {
        return Err(bad("recorded from another ROM"));
    }
    let mut log = SpcLog {
        rom_crc32: crc,
        rom_size: size,
        ..SpcLog::default()
    };
    let mut at = HEADER_LEN;
    for _ in 0..u32_at(bytes, 16) {
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
            .ok_or_else(|| bad("a section claims more bytes than the file has"))?;
        let want = match &tag {
            b"INST" | b"FLOW" => 16,
            b"ACCS" => 24,
            _ => 0,
        };
        if rec < want {
            return Err(bad("a section's records are too short"));
        }
        for r in bytes[at..at + body].chunks_exact(rec.max(1)) {
            match &tag {
                b"INST" => log.insns.push(SpcInsn {
                    pc: u32_at(r, 0) as u16,
                    boot_rom: r[8] == KIND_BOOT_ROM,
                    count: u32_at(r, 12),
                }),
                b"ACCS" => {
                    if let Some(access) = SpcAccess::from_u8(r[16]) {
                        log.accesses.push(SpcAccessRun {
                            pc: u32_at(r, 0) as u16,
                            addr: u32_at(r, 4) as u16,
                            len: u32_at(r, 12),
                            access,
                            count: u32_at(r, 20),
                        });
                    }
                }
                b"FLOW" => log.flows.push(SpcFlow {
                    from: u32_at(r, 0) as u16,
                    to: u32_at(r, 4) as u16,
                    kind: r[8],
                    count: u32_at(r, 12),
                }),
                _ => {}
            }
        }
        at += body;
    }
    Ok(log)
}

/// Write a log in the same format, for tests and fixtures.
pub fn write(log: &SpcLog) -> Vec<u8> {
    let mut b = MAGIC.to_vec();
    b.extend(1u16.to_le_bytes());
    b.extend([1, 0]);
    b.extend(log.rom_crc32.to_le_bytes());
    b.extend(log.rom_size.to_le_bytes());
    b.extend(3u32.to_le_bytes());
    let section = |b: &mut Vec<u8>, tag: &[u8; 4], rec: u32, n: usize| {
        b.extend(tag);
        b.extend(rec.to_le_bytes());
        b.extend((n as u32).to_le_bytes());
    };
    section(&mut b, b"INST", 16, log.insns.len());
    for i in &log.insns {
        b.extend((i.pc as u32).to_le_bytes());
        b.extend((i.pc as u32).to_le_bytes());
        b.extend([if i.boot_rom { KIND_BOOT_ROM } else { 6 }, 0, 0, 0]);
        b.extend(i.count.to_le_bytes());
    }
    section(&mut b, b"ACCS", 24, log.accesses.len());
    for a in &log.accesses {
        b.extend((a.pc as u32).to_le_bytes());
        b.extend((a.addr as u32).to_le_bytes());
        b.extend((a.addr as u32).to_le_bytes());
        b.extend(a.len.to_le_bytes());
        b.extend([a.access.code(), 6, 0, 0]);
        b.extend(a.count.to_le_bytes());
    }
    section(&mut b, b"FLOW", 16, log.flows.len());
    for f in &log.flows {
        b.extend((f.from as u32).to_le_bytes());
        b.extend((f.to as u32).to_le_bytes());
        b.extend([f.kind, 0, 0, 0]);
        b.extend(f.count.to_le_bytes());
    }
    b
}
