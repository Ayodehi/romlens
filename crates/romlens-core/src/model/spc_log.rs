//! The SPC700's execution log (docs/23, A6): what the sound CPU ran, what
//! each instruction read and wrote, and what the DSP read and wrote by
//! itself, from the Mesen fork's `emu.getExecutionLog(emu.cpuType.spc)`.
//!
//! Its addresses are audio RAM's, which the driver can rewrite (a new song,
//! new samples), so a log belongs with the recording it was made beside,
//! not with the ROM.

use std::collections::BTreeSet;

/// Where the boot ROM is mapped while it runs.
const BOOT_ROM_START: u16 = 0xFFC0;

/// Who touched the bytes, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SpcAccess {
    Read,
    Write,
    /// The DSP read them by itself: the sample directory, BRR sample data,
    /// the echo buffer.
    DspRead,
    /// The DSP wrote them: the echo buffer.
    DspWrite,
}

impl SpcAccess {
    pub const fn from_u8(v: u8) -> Option<SpcAccess> {
        Some(match v {
            0 => SpcAccess::Read,
            1 => SpcAccess::Write,
            2 => SpcAccess::DspRead,
            3 => SpcAccess::DspWrite,
            _ => return None,
        })
    }

    pub const fn code(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpcInsn {
    pub pc: u16,
    /// In the boot ROM (mapped at `$FFC0`), not RAM.
    pub boot_rom: bool,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpcAccessRun {
    /// The instruction; 0 for the DSP's own accesses.
    pub pc: u16,
    pub addr: u16,
    pub len: u32,
    pub access: SpcAccess,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpcFlow {
    pub from: u16,
    pub to: u16,
    /// The main CPU log's flow kinds (`FlowKind`'s numbers).
    pub kind: u8,
    pub count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpcLog {
    pub rom_crc32: u32,
    pub rom_size: u32,
    pub insns: Vec<SpcInsn>,
    pub accesses: Vec<SpcAccessRun>,
    pub flows: Vec<SpcFlow>,
}

impl SpcLog {
    /// Where an instruction in RAM started.
    pub fn code_starts(&self) -> BTreeSet<u16> {
        self.insns
            .iter()
            .filter(|i| !i.boot_rom)
            .map(|i| i.pc)
            .collect()
    }

    /// Every byte `access` touched.
    pub fn touched(&self, access: SpcAccess) -> BTreeSet<u16> {
        self.accesses
            .iter()
            .filter(|a| a.access == access)
            .flat_map(|a| (0..a.len).map(move |i| a.addr.wrapping_add(i as u16)))
            .collect()
    }

    /// Every byte the driver's code (not the boot ROM) read or wrote. The
    /// boot ROM's upload loop reads and writes every byte it uploads, code
    /// and samples alike, so its accesses say nothing of what the bytes are.
    pub fn touched_by_driver(&self) -> BTreeSet<u16> {
        self.accesses
            .iter()
            .filter(|a| {
                matches!(a.access, SpcAccess::Read | SpcAccess::Write) && a.pc < BOOT_ROM_START
            })
            .flat_map(|a| (0..a.len).map(move |i| a.addr.wrapping_add(i as u16)))
            .collect()
    }

    /// Add another log (a delta, or a later session) to this one: counts
    /// add, and each relationship stays once.
    pub fn merge(&mut self, other: &SpcLog) {
        use std::collections::BTreeMap;
        let mut insns: BTreeMap<(u16, bool), u32> = BTreeMap::new();
        for i in self.insns.iter().chain(&other.insns) {
            let c = insns.entry((i.pc, i.boot_rom)).or_default();
            *c = c.saturating_add(i.count);
        }
        self.insns = insns
            .into_iter()
            .map(|((pc, boot_rom), count)| SpcInsn {
                pc,
                boot_rom,
                count,
            })
            .collect();
        // Accesses by byte, then back into runs.
        let mut bytes: BTreeMap<(u16, SpcAccess, u16), u32> = BTreeMap::new();
        for a in self.accesses.iter().chain(&other.accesses) {
            for i in 0..a.len {
                let c = bytes
                    .entry((a.pc, a.access, a.addr.wrapping_add(i as u16)))
                    .or_default();
                // A run's count is its bytes' sum; spread it evenly.
                *c = c.saturating_add((a.count / a.len.max(1)).max(1));
            }
        }
        let mut runs: Vec<SpcAccessRun> = Vec::new();
        for ((pc, access, addr), count) in bytes {
            match runs.last_mut() {
                Some(r)
                    if r.pc == pc && r.access == access && r.addr as u32 + r.len == addr as u32 =>
                {
                    r.len += 1;
                    r.count = r.count.saturating_add(count);
                }
                _ => runs.push(SpcAccessRun {
                    pc,
                    addr,
                    len: 1,
                    access,
                    count,
                }),
            }
        }
        self.accesses = runs;
        let mut flows: BTreeMap<(u16, u16, u8), u32> = BTreeMap::new();
        for f in self.flows.iter().chain(&other.flows) {
            let c = flows.entry((f.from, f.to, f.kind)).or_default();
            *c = c.saturating_add(f.count);
        }
        self.flows = flows
            .into_iter()
            .map(|((from, to, kind), count)| SpcFlow {
                from,
                to,
                kind,
                count,
            })
            .collect();
    }
}
