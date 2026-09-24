//! An execution log's counts by ROM offset, so every mirror of an
//! instruction adds up to the one the analysis shows.

use std::collections::BTreeMap;

use crate::model::exec_log::{ExecLog, FlowKind, MemKind};

/// How often each instruction ran, and each transfer out of it.
#[derive(Debug, Clone, Default)]
pub struct Counts {
    runs: BTreeMap<u32, u64>,
    /// By the ROM offset of the instruction control left.
    flows: BTreeMap<u32, Vec<FlowCount>>,
}

/// A transfer out of an instruction, as the log saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowCount {
    /// Where control went, as the CPU addressed it.
    pub to: u32,
    /// Its ROM offset, when it ran from ROM.
    pub to_offset: Option<u32>,
    pub kind: FlowKind,
    pub count: u64,
}

impl Counts {
    pub fn new(log: &ExecLog) -> Counts {
        let mut runs: BTreeMap<u32, u64> = BTreeMap::new();
        for i in &log.insns {
            if i.kind == MemKind::PrgRom && i.abs >= 0 {
                *runs.entry(i.abs as u32).or_insert(0) += u64::from(i.count);
            }
        }
        let mut flows: BTreeMap<u32, Vec<FlowCount>> = BTreeMap::new();
        for f in &log.flows {
            let Some(from) = log.rom_offset(f.from) else {
                continue;
            };
            let to_offset = log.rom_offset(f.to);
            let list = flows.entry(from).or_default();
            match list
                .iter_mut()
                .find(|x| x.kind == f.kind && (x.to_offset, x.to) == (to_offset, f.to))
            {
                Some(x) => x.count += u64::from(f.count),
                None => list.push(FlowCount {
                    to: f.to,
                    to_offset,
                    kind: f.kind,
                    count: u64::from(f.count),
                }),
            }
        }
        Counts { runs, flows }
    }

    /// How many times the instruction at `offset` ran: 0 when it never did.
    pub fn runs(&self, offset: u32) -> u64 {
        self.runs.get(&offset).copied().unwrap_or(0)
    }

    /// The transfers out of the instruction at `offset`.
    pub fn flows_from(&self, offset: u32) -> &[FlowCount] {
        self.flows.get(&offset).map(Vec::as_slice).unwrap_or(&[])
    }

    /// How many times control left `offset` in one of `kinds`, to
    /// `to_offset` when given.
    pub fn flow(&self, offset: u32, kinds: &[FlowKind], to_offset: Option<u32>) -> u64 {
        self.flows_from(offset)
            .iter()
            .filter(|f| kinds.contains(&f.kind))
            .filter(|f| to_offset.is_none() || f.to_offset == to_offset)
            .map(|f| f.count)
            .sum()
    }
}
