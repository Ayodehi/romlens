//! What the CPU did, as relationships rather than per-byte flags.
//!
//! A code/data log says each ROM byte was executed or read. An execution log
//! says *which* instruction read it, where each jump and call actually went,
//! and what each DMA transfer moved where: the edges a disassembler's
//! cross-references and data typing are made of. Each relationship is kept
//! once with a count, so a log grows with how much of the game was seen, not
//! with how long it ran (`docs/17-execution-log.md`).
//!
//! Addresses are the 24-bit CPU addresses the game used. `abs` is the offset
//! in the memory `kind` names, as the emulator mapped it at the time, which
//! for PRG ROM is a Romlens `FileOffset`.

use std::collections::BTreeMap;

use crate::cpu65816::{FlagState, OPCODES};
use crate::model::coverage::Coverage;

/// Which memory an address was mapped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MemKind {
    Unmapped,
    PrgRom,
    WorkRam,
    SaveRam,
    Register,
    Other,
}

impl MemKind {
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => MemKind::PrgRom,
            2 => MemKind::WorkRam,
            3 => MemKind::SaveRam,
            4 => MemKind::Register,
            0 => MemKind::Unmapped,
            _ => MemKind::Other,
        }
    }

    pub const fn to_u8(self) -> u8 {
        match self {
            MemKind::Unmapped => 0,
            MemKind::PrgRom => 1,
            MemKind::WorkRam => 2,
            MemKind::SaveRam => 3,
            MemKind::Register => 4,
            MemKind::Other => 5,
        }
    }
}

/// How control moved from one instruction to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FlowKind {
    Branch,
    BranchNotTaken,
    Jump,
    IndirectJump,
    Call,
    IndirectCall,
    Return,
    ReturnFromInterrupt,
    Nmi,
    Irq,
    SoftwareInterrupt,
}

impl FlowKind {
    pub const ALL: [FlowKind; 11] = [
        FlowKind::Branch,
        FlowKind::BranchNotTaken,
        FlowKind::Jump,
        FlowKind::IndirectJump,
        FlowKind::Call,
        FlowKind::IndirectCall,
        FlowKind::Return,
        FlowKind::ReturnFromInterrupt,
        FlowKind::Nmi,
        FlowKind::Irq,
        FlowKind::SoftwareInterrupt,
    ];

    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }

    pub fn to_u8(self) -> u8 {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0) as u8
    }

    /// Control reached the target as the start of a routine.
    pub const fn enters_routine(self) -> bool {
        matches!(
            self,
            FlowKind::Call
                | FlowKind::IndirectCall
                | FlowKind::Nmi
                | FlowKind::Irq
                | FlowKind::SoftwareInterrupt
        )
    }

    pub const fn is_indirect(self) -> bool {
        matches!(self, FlowKind::IndirectJump | FlowKind::IndirectCall)
    }
}

/// An instruction that ran, and every width state it ran in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecInsn {
    pub pc: u32,
    pub abs: i32,
    pub kind: MemKind,
    /// Bit `M + 2*X + 4*E` for each state, with M/X set for 8-bit.
    pub states: u8,
    pub count: u32,
}

impl ExecInsn {
    /// Each state it ran in, as flags. Emulation mode forces 8-bit widths.
    pub fn flag_states(&self) -> impl Iterator<Item = FlagState> + '_ {
        (0..8u8).filter(|s| self.states & (1 << s) != 0).map(|s| {
            let e = s & 4 != 0;
            FlagState {
                m: s & 1 != 0 || e,
                x: s & 2 != 0 || e,
                e,
                ..FlagState::NATIVE_VECTOR
            }
        })
    }

    /// Ran with more than one accumulator or index width.
    pub fn mixed_widths(&self) -> bool {
        let mut widths = self.flag_states().map(|f| (f.m, f.x));
        let first = widths.next();
        widths.any(|w| Some(w) != first)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Access {
    Read,
    Write,
}

/// A run of consecutive addresses one instruction read or wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessRun {
    pub pc: u32,
    pub addr: u32,
    pub abs: i32,
    pub len: u32,
    pub access: Access,
    pub kind: MemKind,
    /// Summed over the run.
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flow {
    pub from: u32,
    pub to: u32,
    pub kind: FlowKind,
    pub count: u32,
}

/// A run of A-bus addresses a DMA channel moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaRun {
    /// The instruction that wrote `$420B`; 0 for HDMA.
    pub pc: u32,
    pub addr: u32,
    pub abs: i32,
    pub len: u32,
    /// The channel's B-bus address, `$21xx`'s low byte.
    pub bbus: u8,
    pub to_a_bus: bool,
    pub hdma: bool,
    pub mode: u8,
    pub kind: MemKind,
    pub channel: u8,
    pub count: u32,
}

impl DmaRun {
    /// What the B-bus end is, in words.
    pub fn destination(&self) -> &'static str {
        match self.bbus {
            0x18 | 0x19 => "VRAM",
            0x22 => "CGRAM",
            0x04 => "OAM",
            0x40..=0x43 => "the APU",
            0x80 => "WRAM",
            _ => "a PPU register",
        }
    }
}

/// One execution log, or several merged.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecLog {
    /// CRC32 of the PRG ROM the log was taken from.
    pub rom_crc32: u32,
    pub rom_size: u32,
    pub insns: Vec<ExecInsn>,
    pub accesses: Vec<AccessRun>,
    pub flows: Vec<Flow>,
    pub dma: Vec<DmaRun>,
}

/// Saturating, as the format's counts are.
fn add(a: u32, b: u32) -> u32 {
    a.saturating_add(b)
}

impl ExecLog {
    /// Put every list in its canonical order and merge what can merge:
    /// duplicate instructions and transfers, and access or DMA runs that
    /// overlap or touch. Readers may rely on the order.
    pub fn normalize(&mut self) {
        let mut insns: BTreeMap<u32, ExecInsn> = BTreeMap::new();
        for i in self.insns.drain(..) {
            insns
                .entry(i.pc)
                .and_modify(|e| {
                    e.states |= i.states;
                    e.count = add(e.count, i.count);
                })
                .or_insert(i);
        }
        self.insns = insns.into_values().collect();

        let mut flows: BTreeMap<(u32, u32, FlowKind), u32> = BTreeMap::new();
        for f in self.flows.drain(..) {
            let c = flows.entry((f.from, f.to, f.kind)).or_insert(0);
            *c = add(*c, f.count);
        }
        self.flows = flows
            .into_iter()
            .map(|((from, to, kind), count)| Flow {
                from,
                to,
                kind,
                count,
            })
            .collect();

        self.accesses
            .sort_by_key(|a| (a.pc, a.access, a.kind, a.addr, a.len));
        self.accesses = coalesce(std::mem::take(&mut self.accesses), |a, b| {
            a.pc == b.pc && a.access == b.access && a.kind == b.kind
        });

        self.dma.sort_by_key(|d| {
            (
                d.pc, d.bbus, d.to_a_bus, d.hdma, d.mode, d.channel, d.kind, d.addr, d.len,
            )
        });
        self.dma = coalesce(std::mem::take(&mut self.dma), |a, b| {
            a.pc == b.pc
                && a.bbus == b.bbus
                && a.to_a_bus == b.to_a_bus
                && a.hdma == b.hdma
                && a.mode == b.mode
                && a.channel == b.channel
                && a.kind == b.kind
        });
    }

    /// Fold another log of the same ROM into this one. Like coverage, what an
    /// emulator saw is never wrong, only incomplete, so this is a union.
    pub fn merge(&mut self, other: &ExecLog) {
        self.insns.extend_from_slice(&other.insns);
        self.accesses.extend_from_slice(&other.accesses);
        self.flows.extend_from_slice(&other.flows);
        self.dma.extend_from_slice(&other.dma);
        self.normalize();
    }

    /// The instruction at `pc`, if it ran.
    pub fn insn(&self, pc: u32) -> Option<&ExecInsn> {
        self.insns
            .binary_search_by_key(&pc, |i| i.pc)
            .ok()
            .map(|i| &self.insns[i])
    }

    /// The ROM offset `pc` was mapped to when it ran.
    pub fn rom_offset(&self, pc: u32) -> Option<u32> {
        self.insn(pc)
            .filter(|i| i.kind == MemKind::PrgRom && i.abs >= 0)
            .map(|i| i.abs as u32)
    }

    /// The same observation as a code/data log for a ROM of `rom`'s bytes:
    /// every instruction start and its widths, the bytes it spans, the bytes
    /// read and written as data (DMA sources included), and the entries of
    /// routines control was seen to call.
    ///
    /// Where an instruction ran in more than one width state, the first
    /// (16-bit before 8-bit) is the one recorded, as the coverage model keeps
    /// one width per opcode; it is the order the format lists them in.
    pub fn to_coverage(&self, rom: &[u8]) -> Coverage {
        let len = rom.len() as u32;
        let mut c = Coverage::new(len);
        c.flags.recorded = true;
        for i in &self.insns {
            if i.kind != MemKind::PrgRom || i.abs < 0 || i.abs as u32 >= len {
                continue;
            }
            let off = i.abs as u32;
            let opcode = rom[off as usize];
            let mut first = true;
            for flags in i.flag_states() {
                let n = 1 + OPCODES[opcode as usize].mode.operand_len(flags) as u32;
                for b in off..(off + n).min(len) {
                    c.executed.insert(b);
                }
                if first {
                    c.mark_opcode(off, false);
                    c.flags.m8.set(off, flags.m);
                    c.flags.x8.set(off, flags.x);
                    first = false;
                }
            }
        }
        for f in &self.flows {
            if f.kind.enters_routine()
                && let Some(off) = self.rom_offset(f.to)
            {
                c.mark_opcode(off, true);
            }
        }
        for a in &self.accesses {
            if a.kind != MemKind::PrgRom || a.abs < 0 {
                continue;
            }
            let set = match a.access {
                Access::Read => &mut c.read,
                Access::Write => &mut c.written,
            };
            for b in a.abs as u32..(a.abs as u32).saturating_add(a.len).min(len) {
                set.insert(b);
            }
        }
        for d in &self.dma {
            if d.kind == MemKind::PrgRom && d.abs >= 0 && !d.to_a_bus {
                for b in d.abs as u32..(d.abs as u32).saturating_add(d.len).min(len) {
                    c.read.insert(b);
                }
            }
        }
        c
    }

    pub fn is_empty(&self) -> bool {
        self.insns.is_empty()
            && self.accesses.is_empty()
            && self.flows.is_empty()
            && self.dma.is_empty()
    }
}

/// Merge sorted runs that overlap or touch, when `same` says they belong
/// together and their `abs` offsets line up with their addresses.
fn coalesce<T: Run>(runs: Vec<T>, same: impl Fn(&T, &T) -> bool) -> Vec<T> {
    let mut out: Vec<T> = Vec::with_capacity(runs.len());
    for r in runs {
        if let Some(last) = out.last_mut()
            && same(last, &r)
            && r.addr() >= last.addr()
            && r.addr() <= last.addr() + last.len()
            && (last.abs() < 0 && r.abs() < 0
                || last.abs() >= 0
                    && r.abs() as i64 - last.abs() as i64 == r.addr() as i64 - last.addr() as i64)
        {
            let end = (last.addr() + last.len()).max(r.addr() + r.len());
            last.set_len(end - last.addr());
            last.add_count(r.count());
            continue;
        }
        out.push(r);
    }
    out
}

trait Run {
    fn addr(&self) -> u32;
    fn abs(&self) -> i32;
    fn len(&self) -> u32;
    fn count(&self) -> u32;
    fn set_len(&mut self, len: u32);
    fn add_count(&mut self, n: u32);
}

macro_rules! impl_run {
    ($t:ty) => {
        impl Run for $t {
            fn addr(&self) -> u32 {
                self.addr
            }
            fn abs(&self) -> i32 {
                self.abs
            }
            fn len(&self) -> u32 {
                self.len
            }
            fn count(&self) -> u32 {
                self.count
            }
            fn set_len(&mut self, len: u32) {
                self.len = len;
            }
            fn add_count(&mut self, n: u32) {
                self.count = add(self.count, n);
            }
        }
    };
}
impl_run!(AccessRun);
impl_run!(DmaRun);

#[cfg(test)]
mod tests {
    use super::*;

    fn read(pc: u32, addr: u32, len: u32, count: u32) -> AccessRun {
        AccessRun {
            pc,
            addr,
            abs: (addr & 0x7FFF) as i32,
            len,
            access: Access::Read,
            kind: MemKind::PrgRom,
            count,
        }
    }

    #[test]
    fn merging_unions_states_sums_counts_and_joins_runs() {
        let mut a = ExecLog {
            insns: vec![ExecInsn {
                pc: 0x80_8000,
                abs: 0,
                kind: MemKind::PrgRom,
                states: 0b0001,
                count: 3,
            }],
            accesses: vec![read(0x80_8000, 0x80_9000, 4, 4)],
            flows: vec![Flow {
                from: 0x80_8000,
                to: 0x80_8010,
                kind: FlowKind::Call,
                count: 1,
            }],
            ..Default::default()
        };
        let b = ExecLog {
            insns: vec![ExecInsn {
                pc: 0x80_8000,
                abs: 0,
                kind: MemKind::PrgRom,
                states: 0b0100,
                count: u32::MAX,
            }],
            // Overlapping, touching and far runs from the same instruction.
            accesses: vec![
                read(0x80_8000, 0x80_9002, 4, 4),
                read(0x80_8000, 0x80_9006, 2, 2),
                read(0x80_8000, 0x80_9100, 1, 1),
            ],
            flows: vec![Flow {
                from: 0x80_8000,
                to: 0x80_8010,
                kind: FlowKind::Call,
                count: 2,
            }],
            ..Default::default()
        };
        a.merge(&b);
        assert_eq!(a.insns.len(), 1);
        assert_eq!(a.insns[0].states, 0b0101);
        assert_eq!(a.insns[0].count, u32::MAX, "counts saturate");
        assert_eq!(
            a.accesses
                .iter()
                .map(|r| (r.addr, r.len, r.count))
                .collect::<Vec<_>>(),
            vec![(0x80_9000, 8, 10), (0x80_9100, 1, 1)]
        );
        assert_eq!(a.flows.len(), 1);
        assert_eq!(a.flows[0].count, 3);
    }

    #[test]
    fn runs_that_map_apart_do_not_merge() {
        // Adjacent CPU addresses mapped to unrelated ROM offsets (a bank
        // boundary under some mapper) stay two runs.
        let mut log = ExecLog {
            accesses: vec![read(0x80_FFFE, 0x80_FFFE, 2, 2), {
                let mut r = read(0x80_0000, 0x81_0000, 2, 2);
                r.pc = 0x80_FFFE;
                r.abs = 0x4000;
                r
            }],
            ..Default::default()
        };
        log.normalize();
        assert_eq!(log.accesses.len(), 2);
    }

    #[test]
    fn coverage_marks_every_instruction_with_its_widths() {
        // $80:8000 A9 xx xx  LDA #imm: 3 bytes with 16-bit A, 2 with 8-bit.
        let mut rom = vec![0u8; 0x100];
        rom[0] = 0xA9;
        rom[3] = 0x60; // RTS
        let log = ExecLog {
            insns: vec![
                ExecInsn {
                    pc: 0x80_8000,
                    abs: 0,
                    kind: MemKind::PrgRom,
                    states: 0b0011, // 16-bit A/X, then 8-bit A
                    count: 2,
                },
                ExecInsn {
                    pc: 0x80_8003,
                    abs: 3,
                    kind: MemKind::PrgRom,
                    states: 0b0010, // 8-bit A
                    count: 1,
                },
                ExecInsn {
                    pc: 0x7E_2000,
                    abs: 0x2000,
                    kind: MemKind::WorkRam,
                    states: 1,
                    count: 1,
                },
            ],
            accesses: vec![read(0x80_8000, 0x80_8080, 2, 2)],
            flows: vec![Flow {
                from: 0x80_9000,
                to: 0x80_8000,
                kind: FlowKind::Call,
                count: 1,
            }],
            ..Default::default()
        };
        assert!(log.insns[0].mixed_widths());
        let c = log.to_coverage(&rom);
        assert_eq!(c.opcode_start.iter().collect::<Vec<_>>(), vec![0, 3]);
        assert_eq!(c.executed.iter().collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        assert!(!c.flags.m8.get(0), "the first state, 16-bit A, is kept");
        assert!(c.flags.m8.get(3));
        assert_eq!(c.entry.iter().collect::<Vec<_>>(), vec![0]);
        assert_eq!(c.read.iter().collect::<Vec<_>>(), vec![0x80, 0x81]);
        assert!(c.flags.recorded);
    }
}
