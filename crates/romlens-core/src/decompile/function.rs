//! Which instructions make up a routine, and where each one goes next.
//!
//! A function is found by following the analysis's own instructions from an
//! entry, with the widths the analysis decoded them in. Branches, `BRA`,
//! `BRL` and jumps stay inside it; calls are call sites; returns leave it. A
//! jump to another routine's entry is a tail call rather than more of this
//! one, which is what keeps a routine that ends in `JMP Common` from
//! swallowing `Common` and everything it reaches.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::cpu65816::{AddressingMode, Instruction, Mnemonic};
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::project::Project;
use crate::model::xref::XRefKind;
use crate::rom::image::RomImage;

/// Instructions followed before a function is cut short.
pub const MAX_INSNS: usize = 8192;

/// Where control goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dest {
    /// An instruction of this function.
    Local(FileOffset),
    /// Another routine's entry: a tail call.
    Tail(SnesAddress),
    /// Somewhere this cannot follow, and why.
    Unknown(String),
}

/// What an instruction does to control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transfer {
    /// Falls through.
    Next(Dest),
    /// A call, then on to `next` (past any inline arguments).
    Call {
        callee: Callee,
        inline: u16,
        next: Dest,
    },
    /// A conditional branch.
    Branch { taken: Dest, next: Dest },
    /// `BRA`, `BRL`, `JMP`, `JML`.
    Jump(Dest),
    /// `JMP (abs,X)` through a resolved table, one destination per entry.
    Switch {
        table: SnesAddress,
        cases: Vec<Dest>,
    },
    /// `RTS`, `RTL` or `RTI`.
    Return,
    /// `BRK` or `STP`: control does not come back.
    Halt,
}

/// What a call calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callee {
    Direct(SnesAddress),
    /// `JSR (abs,X)` through a resolved table.
    Table {
        table: SnesAddress,
        targets: Vec<SnesAddress>,
    },
    /// Through a pointer nothing resolved.
    Indirect,
}

impl Transfer {
    /// The destinations inside or outside the function, in order.
    pub fn dests(&self) -> Vec<&Dest> {
        match self {
            Transfer::Next(d) | Transfer::Jump(d) => vec![d],
            Transfer::Call { next, .. } => vec![next],
            Transfer::Branch { taken, next } => vec![taken, next],
            Transfer::Switch { cases, .. } => cases.iter().collect(),
            Transfer::Return | Transfer::Halt => vec![],
        }
    }
}

/// One instruction and where it goes.
#[derive(Debug, Clone)]
pub struct Step {
    pub insn: Instruction,
    pub transfer: Transfer,
}

#[derive(Debug, Clone)]
pub struct Function {
    /// Canonical.
    pub entry: SnesAddress,
    pub entry_offset: FileOffset,
    /// Sorted by file offset.
    pub steps: Vec<Step>,
    /// Stopped at `MAX_INSNS`.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FunctionError {
    /// The address is not in ROM.
    NotInRom(SnesAddress),
    /// No instruction the analysis decoded starts there.
    NotCode(SnesAddress),
}

impl std::fmt::Display for FunctionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FunctionError::NotInRom(a) => write!(f, "{a} is not in ROM"),
            FunctionError::NotCode(a) => {
                write!(f, "no instruction the analysis found starts at {a}")
            }
        }
    }
}

impl std::error::Error for FunctionError {}

impl Function {
    pub fn index_of(&self, off: FileOffset) -> Option<usize> {
        self.steps
            .binary_search_by_key(&off.0, |s| s.insn.file_offset.0)
            .ok()
    }

    /// The step whose bytes include `off`.
    pub fn step_containing(&self, off: FileOffset) -> Option<usize> {
        let i = self
            .steps
            .partition_point(|s| s.insn.file_offset.0 <= off.0)
            .checked_sub(1)?;
        let s = &self.steps[i].insn;
        (off.0 < s.file_offset.0 + s.len as u32).then_some(i)
    }
}

/// The addresses other code enters as a routine: call targets, vectors, and
/// the targets of `JSR (abs,X)` tables. Canonical.
pub fn entries(snap: &AnalysisSnapshot) -> BTreeSet<SnesAddress> {
    let mut out: BTreeSet<SnesAddress> = snap
        .xrefs_by_target
        .iter()
        .filter(|x| matches!(x.kind, XRefKind::Call | XRefKind::Vector))
        .map(|x| x.to)
        .collect();
    for t in snap.jump_tables.iter().filter(|t| t.call) {
        out.extend(t.targets.iter().map(|(a, _)| *a));
    }
    out
}

/// Follow a function from `entry`.
pub fn discover(
    rom: &RomImage,
    snap: &AnalysisSnapshot,
    entries: &BTreeSet<SnesAddress>,
    entry: SnesAddress,
) -> Result<Function, FunctionError> {
    let entry = Project::canonical(rom, entry);
    let entry_offset = rom
        .file_offset_for(entry)
        .ok_or(FunctionError::NotInRom(entry))?;
    if !starts_instruction(snap, entry_offset) {
        return Err(FunctionError::NotCode(entry));
    }
    let tables: BTreeMap<u32, &crate::analysis::jumptable::JumpTable> =
        snap.jump_tables.iter().map(|t| (t.site, t)).collect();

    let mut steps: BTreeMap<u32, Step> = BTreeMap::new();
    let mut work = vec![entry_offset];
    let mut truncated = false;
    while let Some(off) = work.pop() {
        if steps.contains_key(&off.0) {
            continue;
        }
        if steps.len() >= MAX_INSNS {
            truncated = true;
            break;
        }
        let Some(insn) = snap
            .instruction_at(off)
            .filter(|r| r.offset == off.0)
            .and_then(|r| snap.decode_at(rom, r))
        else {
            continue;
        };
        let transfer = transfer(rom, snap, entries, entry, &tables, &insn);
        for d in transfer.dests() {
            if let Dest::Local(o) = d {
                work.push(*o);
            }
        }
        steps.insert(off.0, Step { insn, transfer });
    }
    Ok(Function {
        entry,
        entry_offset,
        steps: steps.into_values().collect(),
        truncated,
    })
}

/// The function containing `off`: the nearest entry at or before it whose
/// function includes it. `None` when no entry within `window` bytes does.
pub fn containing(
    rom: &RomImage,
    snap: &AnalysisSnapshot,
    entries: &BTreeSet<SnesAddress>,
    off: FileOffset,
) -> Option<Function> {
    const TRIES: usize = 32;
    let mut candidates: Vec<(u32, SnesAddress)> = entries
        .iter()
        .filter_map(|a| rom.file_offset_for(*a).map(|o| (o.0, *a)))
        .filter(|(o, _)| *o <= off.0)
        .collect();
    candidates.sort_unstable();
    for (_, a) in candidates.iter().rev().take(TRIES) {
        if let Ok(f) = discover(rom, snap, entries, *a)
            && f.step_containing(off).is_some()
        {
            return Some(f);
        }
    }
    None
}

fn starts_instruction(snap: &AnalysisSnapshot, off: FileOffset) -> bool {
    snap.instruction_at(off).is_some_and(|r| r.offset == off.0)
}

fn transfer(
    rom: &RomImage,
    snap: &AnalysisSnapshot,
    entries: &BTreeSet<SnesAddress>,
    entry: SnesAddress,
    tables: &BTreeMap<u32, &crate::analysis::jumptable::JumpTable>,
    insn: &Instruction,
) -> Transfer {
    use Mnemonic::*;
    let m = insn.mnemonic;
    let fall = || {
        local(
            rom,
            snap,
            insn.next_address(),
            "falls into bytes that were not decoded",
        )
    };
    match m {
        RTS | RTL | RTI => Transfer::Return,
        BRK | STP => Transfer::Halt,
        _ if m.is_branch() => Transfer::Branch {
            taken: jump_dest(rom, snap, entries, entry, insn),
            next: fall(),
        },
        JSR | JSL => {
            let site = insn.file_offset.0;
            let callee = match insn.mode {
                AddressingMode::AbsoluteIndexedIndirect => match tables.get(&site) {
                    Some(t) => Callee::Table {
                        table: t.base_address,
                        targets: t.targets.iter().map(|(a, _)| *a).collect(),
                    },
                    None => Callee::Indirect,
                },
                _ => match insn.target {
                    Some(t) => Callee::Direct(Project::canonical(rom, t.address)),
                    None => Callee::Indirect,
                },
            };
            let inline = match &callee {
                Callee::Direct(a) => rom
                    .file_offset_for(*a)
                    .and_then(|o| snap.inline_args.get(&o.0))
                    .copied()
                    .unwrap_or(0),
                _ => 0,
            };
            let after = SnesAddress::new(
                insn.address.bank(),
                insn.next_address().offset().wrapping_add(inline),
            );
            Transfer::Call {
                callee,
                inline,
                next: local(rom, snap, after, "the call does not return to decoded code"),
            }
        }
        JMP | JML | BRA | BRL => match insn.mode {
            AddressingMode::AbsoluteIndexedIndirect => match tables.get(&insn.file_offset.0) {
                Some(t) => Transfer::Switch {
                    table: t.base_address,
                    cases: t
                        .targets
                        .iter()
                        .map(|(a, _)| dest_for(rom, snap, entries, entry, *a))
                        .collect(),
                },
                None => Transfer::Jump(Dest::Unknown(
                    "jump through a table the analysis did not resolve".into(),
                )),
            },
            AddressingMode::AbsoluteIndirect | AddressingMode::AbsoluteIndirectLong => {
                Transfer::Jump(Dest::Unknown("jump through a pointer".into()))
            }
            _ => Transfer::Jump(jump_dest(rom, snap, entries, entry, insn)),
        },
        _ => Transfer::Next(fall()),
    }
}

fn jump_dest(
    rom: &RomImage,
    snap: &AnalysisSnapshot,
    entries: &BTreeSet<SnesAddress>,
    entry: SnesAddress,
    insn: &Instruction,
) -> Dest {
    match insn.target {
        Some(t) => dest_for(rom, snap, entries, entry, t.address),
        None => Dest::Unknown("no static target".into()),
    }
}

/// A transfer to `to`: a tail call when it is another routine's entry, else
/// more of this function.
fn dest_for(
    rom: &RomImage,
    snap: &AnalysisSnapshot,
    entries: &BTreeSet<SnesAddress>,
    entry: SnesAddress,
    to: SnesAddress,
) -> Dest {
    let canon = Project::canonical(rom, to);
    if canon != entry && entries.contains(&canon) {
        return Dest::Tail(canon);
    }
    local(rom, snap, to, "jumps to bytes that were not decoded")
}

fn local(rom: &RomImage, snap: &AnalysisSnapshot, to: SnesAddress, why: &str) -> Dest {
    match rom.file_offset_for(to) {
        Some(o) if starts_instruction(snap, o) => Dest::Local(o),
        Some(_) => Dest::Unknown(format!("{why} ({to})")),
        None => Dest::Unknown(format!("leaves ROM ({to})")),
    }
}
