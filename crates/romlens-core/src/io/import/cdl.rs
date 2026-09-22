//! Mesen2 Code/Data Logger files.
//!
//! Verified against Mesen2's `Core/Debugger/CodeDataLogger.cpp` and
//! `DebugTypes.h`. The payload is one byte per ROM byte **in file order**, so
//! it aligns with our `FileOffset` with no address translation at all — the
//! reason this is the first importer to build.

use crate::error::ProjectError;
use crate::model::coverage::Coverage;

/// `"CDLv2"` then a u32 CRC32 of the ROM the log was taken from.
pub const MAGIC: &[u8; 5] = b"CDLv2";
pub const HEADER_LEN: usize = 9;

pub const CODE: u8 = 0x01;
pub const DATA: u8 = 0x02;
pub const JUMP_TARGET: u8 = 0x04;
pub const SUB_ENTRY_POINT: u8 = 0x08;
/// SNES-specific: the index registers were 8-bit at this opcode.
pub const INDEX_MODE_8: u8 = 0x10;
/// SNES-specific: the accumulator was 8-bit at this opcode.
pub const MEMORY_MODE_8: u8 = 0x20;

/// Whether `bytes` look like a CDL for a ROM of `rom_len` bytes.
pub fn looks_like(bytes: &[u8], rom_len: u32) -> bool {
    bytes.starts_with(MAGIC) || bytes.len() as u32 == rom_len
}

/// The CRC32 the header records, if there is a header.
pub fn recorded_crc32(bytes: &[u8]) -> Option<u32> {
    if !bytes.starts_with(MAGIC) || bytes.len() < HEADER_LEN {
        return None;
    }
    Some(u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]))
}

/// Read a CDL for a ROM of `rom_len` bytes.
///
/// Files written before the `CDLv2` header are accepted too: they are the bare
/// payload, and Mesen wrote plenty of them.
pub fn read(bytes: &[u8], rom_len: u32) -> Result<Coverage, ProjectError> {
    let payload = if bytes.starts_with(MAGIC) {
        if bytes.len() < HEADER_LEN {
            return Err(ProjectError::BadFormat(
                "CDL header is truncated".to_owned(),
            ));
        }
        &bytes[HEADER_LEN..]
    } else {
        bytes
    };
    if payload.len() as u32 != rom_len {
        return Err(ProjectError::BadFormat(format!(
            "CDL covers {} bytes but the ROM is {rom_len}; it was recorded from a different image",
            payload.len()
        )));
    }
    let mut coverage = Coverage::new(rom_len);
    // Mesen records the widths with the opcode fetch, so they are meaningful
    // only where CODE is set.
    coverage.flags.recorded = true;
    for (i, flags) in payload.iter().enumerate() {
        let i = i as u32;
        if flags & CODE != 0 {
            coverage.executed.insert(i);
            // Mesen sets CODE on every byte of the instruction and marks a
            // jump target or subroutine entry only on the opcode, so those are
            // what identify an instruction start.
            if flags & (JUMP_TARGET | SUB_ENTRY_POINT) != 0 {
                coverage.mark_opcode(i, flags & SUB_ENTRY_POINT != 0);
            }
            if flags & MEMORY_MODE_8 != 0 {
                coverage.flags.m8.insert(i);
            }
            if flags & INDEX_MODE_8 != 0 {
                coverage.flags.x8.insert(i);
            }
        }
        if flags & DATA != 0 {
            coverage.read.insert(i);
        }
    }
    Ok(coverage)
}

/// Write a CDL, for tests and for `romlens truth from-cdl`'s round trip.
pub fn write(coverage: &Coverage, crc32: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + coverage.len as usize);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&crc32.to_le_bytes());
    for i in 0..coverage.len {
        let mut flags = 0u8;
        if coverage.executed.get(i) {
            flags |= CODE;
            if coverage.entry.get(i) {
                flags |= SUB_ENTRY_POINT;
            } else if coverage.opcode_start.get(i) {
                flags |= JUMP_TARGET;
            }
            if coverage.flags.m8.get(i) {
                flags |= MEMORY_MODE_8;
            }
            if coverage.flags.x8.get(i) {
                flags |= INDEX_MODE_8;
            }
        }
        if coverage.read.get(i) {
            flags |= DATA;
        }
        out.push(flags);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_documented_flags() {
        let mut payload = vec![0u8; 16];
        payload[0] = CODE | SUB_ENTRY_POINT | MEMORY_MODE_8;
        payload[1] = CODE;
        payload[2] = CODE | JUMP_TARGET | INDEX_MODE_8;
        payload[8] = DATA;
        payload[9] = CODE | DATA; // a byte both executed and read
        let mut file = MAGIC.to_vec();
        file.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        file.extend_from_slice(&payload);

        assert_eq!(recorded_crc32(&file), Some(0x1234_5678));
        let c = read(&file, 16).unwrap();
        assert_eq!(c.executed.iter().collect::<Vec<_>>(), vec![0, 1, 2, 9]);
        assert_eq!(c.opcode_start.iter().collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(c.entry.iter().collect::<Vec<_>>(), vec![0]);
        assert_eq!(c.read.iter().collect::<Vec<_>>(), vec![8, 9]);
        assert_eq!(c.flags.m8.iter().collect::<Vec<_>>(), vec![0]);
        assert_eq!(c.flags.x8.iter().collect::<Vec<_>>(), vec![2]);
        assert!(c.flags.recorded);
    }

    #[test]
    fn accepts_a_header_less_file_and_refuses_a_mismatch() {
        let payload = vec![CODE; 16];
        assert_eq!(read(&payload, 16).unwrap().executed.count(), 16);
        assert!(looks_like(&payload, 16));
        assert_eq!(recorded_crc32(&payload), None);
        let err = read(&payload, 32).unwrap_err();
        assert!(
            format!("{err}").contains("recorded from a different image"),
            "{err}"
        );
        assert!(read(&MAGIC[..3], 16).is_err());
    }

    #[test]
    fn round_trips() {
        let mut c = Coverage::new(32);
        c.mark_opcode(0, true);
        c.mark_opcode(4, false);
        c.read.insert(20);
        c.flags.m8.insert(0);
        c.flags.recorded = true;
        assert_eq!(read(&write(&c, 7), 32).unwrap(), c);
    }
}
