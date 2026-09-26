//! Routines paired across two versions, and the data that changed
//! (docs/22, D1).
//!
//! Each routine is fingerprinted by its instructions with the addresses
//! that point into the ROM masked, so a routine that only moved, or whose
//! callees moved, still reads as the same. Pairs are found in this order:
//! the same fingerprint (same, or moved when the entry differs), then the
//! same entry (changed), then the entry the byte alignment carries it to
//! (changed, and moved). What is left was removed from the first version
//! or added in the second.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::Side;
use super::bytes::{ByteDiff, RunKind};
use crate::cpu65816::decode::Instruction;
use crate::cpu65816::format::format_instruction;
use crate::decompile::{discover, entries};
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::region::RegionKind;
use crate::model::symbols::Symbols;

/// A routine of one version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineInfo {
    pub entry: SnesAddress,
    pub offset: u32,
    pub name: String,
    pub instructions: usize,
    /// Its instructions' bytes, not counting gaps between them.
    pub bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pairing {
    /// The same instructions at the same place.
    Same,
    /// The same instructions somewhere else.
    Moved,
    /// Paired, with different instructions.
    Changed,
    /// Only in the second version.
    Added,
    /// Only in the first.
    Removed,
}

impl Pairing {
    pub const fn name(self) -> &'static str {
        match self {
            Pairing::Same => "same",
            Pairing::Moved => "moved",
            Pairing::Changed => "changed",
            Pairing::Added => "added",
            Pairing::Removed => "removed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOp {
    Same,
    /// The instruction differs; both sides are present.
    Changed,
    Added,
    Removed,
}

/// One line of a changed routine's instruction diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub op: LineOp,
    /// `(file offset, text)` in each version.
    pub a: Option<(u32, String)>,
    pub b: Option<(u32, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutinePair {
    pub pairing: Pairing,
    pub a: Option<RoutineInfo>,
    pub b: Option<RoutineInfo>,
    /// For a changed pair, the instructions side by side.
    pub lines: Vec<DiffLine>,
}

/// A data region with changed bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct DataChange {
    /// The region in the first version, or in the second for bytes only it
    /// has.
    pub in_b: bool,
    pub start: u32,
    pub len: u32,
    pub kind: RegionKind,
    /// The label at its start, if it has one.
    pub name: Option<String>,
    pub changed: u32,
}

struct Routine {
    info: RoutineInfo,
    insns: Vec<Instruction>,
    /// Per instruction, its masked shape.
    tokens: Vec<u64>,
    print: u64,
}

fn mix(h: u64, v: u64) -> u64 {
    (h ^ v).wrapping_mul(0x100_0000_01B3)
}

/// An instruction's shape: its opcode and operand, with an operand that
/// names a ROM address masked.
fn token(side: Side<'_>, i: &Instruction) -> u64 {
    let mut h = mix(0xCBF2_9CE4_8422_2325, i.opcode as u64);
    let operand = &i.bytes()[1..];
    let rom_address = if i.mode.is_long() {
        let a = SnesAddress::from_u24(
            operand[0] as u32 | (operand[1] as u32) << 8 | (operand[2] as u32) << 16,
        );
        side.rom.file_offset_for(a).is_some()
    } else if i.mode.is_absolute() {
        // A 16-bit address from $8000 up is ROM in every bank that maps
        // it; below is RAM or a register, which a changed routine touches
        // differently.
        (operand[0] as u16 | (operand[1] as u16) << 8) >= 0x8000
    } else {
        false
    };
    if !rom_address {
        for &b in operand {
            h = mix(h, b as u64 + 1);
        }
    }
    h
}

fn routines(side: Side<'_>) -> Vec<Routine> {
    let symbols = Symbols::new(side.rom, side.project, &side.snap.auto_labels);
    let all = entries(side.snap);
    let mut out = Vec::new();
    for &entry in &all {
        let Ok(f) = discover(side.rom, side.snap, &all, entry) else {
            continue;
        };
        let insns: Vec<Instruction> = f.steps.into_iter().map(|s| s.insn).collect();
        let tokens: Vec<u64> = insns.iter().map(|i| token(side, i)).collect();
        let print = tokens.iter().fold(insns.len() as u64, |h, &t| mix(h, t));
        let name = symbols
            .label_at(f.entry)
            .map(|l| l.name.clone())
            .unwrap_or_else(|| f.entry.to_string());
        out.push(Routine {
            info: RoutineInfo {
                entry: f.entry,
                offset: f.entry_offset.0,
                name,
                instructions: insns.len(),
                bytes: insns.iter().map(|i| i.len as u32).sum(),
            },
            insns,
            tokens,
            print,
        });
    }
    out
}

fn text(side: Side<'_>, i: &Instruction) -> String {
    let symbols = Symbols::new(side.rom, side.project, &side.snap.auto_labels);
    format_instruction(i, &symbols).text
}

/// The two routines' instructions side by side: the longest run of shapes
/// they share, with what is between as changed, added or removed.
fn lines(a: Side<'_>, ra: &Routine, b: Side<'_>, rb: &Routine) -> Vec<DiffLine> {
    let (n, m) = (ra.tokens.len(), rb.tokens.len());
    let line = |op, i: Option<usize>, j: Option<usize>| DiffLine {
        op,
        a: i.map(|i| (ra.insns[i].file_offset.0, text(a, &ra.insns[i]))),
        b: j.map(|j| (rb.insns[j].file_offset.0, text(b, &rb.insns[j]))),
    };
    let mut out = Vec::new();
    // Longest common subsequence; routines are at most a few thousand
    // instructions, and a pair too big for the table is compared in place.
    if n * m > 4_000_000 {
        for k in 0..n.max(m) {
            let (i, j) = ((k < n).then_some(k), (k < m).then_some(k));
            let op = match (i, j) {
                (Some(i), Some(j)) if ra.tokens[i] == rb.tokens[j] => LineOp::Same,
                (Some(_), Some(_)) => LineOp::Changed,
                (Some(_), None) => LineOp::Removed,
                _ => LineOp::Added,
            };
            out.push(line(op, i, j));
        }
        return out;
    }
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[at(i, j)] = if ra.tokens[i] == rb.tokens[j] {
                table[at(i + 1, j + 1)] + 1
            } else {
                table[at(i + 1, j)].max(table[at(i, j + 1)])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut removed: Vec<usize> = Vec::new();
    let mut added: Vec<usize> = Vec::new();
    // A removal and an addition next to each other are one changed line.
    let flush = |out: &mut Vec<DiffLine>, removed: &mut Vec<usize>, added: &mut Vec<usize>| {
        for k in 0..removed.len().max(added.len()) {
            let (x, y) = (removed.get(k).copied(), added.get(k).copied());
            let op = match (x, y) {
                (Some(_), Some(_)) => LineOp::Changed,
                (Some(_), None) => LineOp::Removed,
                _ => LineOp::Added,
            };
            out.push(line(op, x, y));
        }
        removed.clear();
        added.clear();
    };
    while i < n || j < m {
        if i < n && j < m && ra.tokens[i] == rb.tokens[j] {
            flush(&mut out, &mut removed, &mut added);
            out.push(line(LineOp::Same, Some(i), Some(j)));
            i += 1;
            j += 1;
        } else if j == m || i < n && table[at(i + 1, j)] >= table[at(i, j + 1)] {
            removed.push(i);
            i += 1;
        } else {
            added.push(j);
            j += 1;
        }
    }
    flush(&mut out, &mut removed, &mut added);
    out
}

/// Pair the two versions' routines.
pub fn pair(a: Side<'_>, b: Side<'_>, bytes: &ByteDiff) -> Vec<RoutinePair> {
    let ra = routines(a);
    let rb = routines(b);
    let mut partner: Vec<Option<usize>> = vec![None; ra.len()];
    let mut taken: HashSet<usize> = HashSet::new();
    // 1. The same fingerprint, the nearest entry first where several share
    //    one.
    let mut by_print: HashMap<u64, Vec<usize>> = HashMap::new();
    for (j, r) in rb.iter().enumerate() {
        by_print.entry(r.print).or_default().push(j);
    }
    for (i, r) in ra.iter().enumerate() {
        let target = bytes.map(r.info.offset).unwrap_or(r.info.offset);
        if let Some(js) = by_print.get(&r.print)
            && let Some(&j) = js
                .iter()
                .filter(|j| !taken.contains(j))
                .min_by_key(|&&j| rb[j].info.offset.abs_diff(target))
        {
            partner[i] = Some(j);
            taken.insert(j);
        }
    }
    // 2. The same entry; 3. where the alignment carries the entry.
    let by_offset: BTreeMap<u32, usize> = rb
        .iter()
        .enumerate()
        .map(|(j, r)| (r.info.offset, j))
        .collect();
    for pass in 0..2 {
        for (i, r) in ra.iter().enumerate() {
            if partner[i].is_some() {
                continue;
            }
            let at = if pass == 0 {
                Some(r.info.offset)
            } else {
                bytes.map(r.info.offset)
            };
            if let Some(&j) = at.and_then(|o| by_offset.get(&o))
                && !taken.contains(&j)
            {
                partner[i] = Some(j);
                taken.insert(j);
            }
        }
    }
    let mut out = Vec::new();
    for (i, r) in ra.iter().enumerate() {
        let Some(j) = partner[i] else {
            out.push(RoutinePair {
                pairing: Pairing::Removed,
                a: Some(r.info.clone()),
                b: None,
                lines: Vec::new(),
            });
            continue;
        };
        let s = &rb[j];
        let pairing = if r.print != s.print {
            Pairing::Changed
        } else if r.info.offset == s.info.offset {
            Pairing::Same
        } else {
            Pairing::Moved
        };
        out.push(RoutinePair {
            pairing,
            a: Some(r.info.clone()),
            b: Some(s.info.clone()),
            lines: if pairing == Pairing::Changed {
                lines(a, r, b, s)
            } else {
                Vec::new()
            },
        });
    }
    for (j, s) in rb.iter().enumerate() {
        if !taken.contains(&j) {
            out.push(RoutinePair {
                pairing: Pairing::Added,
                a: None,
                b: Some(s.info.clone()),
                lines: Vec::new(),
            });
        }
    }
    out
}

/// The regions holding changed bytes, other than code (the routines say
/// what changed there), with how many.
pub fn data_changes(a: Side<'_>, b: Side<'_>, bytes: &ByteDiff) -> Vec<DataChange> {
    let mut out: BTreeMap<(bool, u32), DataChange> = BTreeMap::new();
    let mut add = |side: Side<'_>, in_b: bool, range: std::ops::Range<u32>| {
        let regions = &side.snap.regions;
        let mut k = regions.partition_point(|r| r.end() <= range.start);
        while let Some(r) = regions.get(k) {
            if r.start.0 >= range.end {
                break;
            }
            let n = r.end().min(range.end) - r.start.0.max(range.start);
            if r.kind != RegionKind::Code && n > 0 {
                let symbols = Symbols::new(side.rom, side.project, &side.snap.auto_labels);
                let name = side
                    .rom
                    .snes_address_for(FileOffset(r.start.0))
                    .and_then(|at| symbols.label_at(at))
                    .map(|l| l.name.clone());
                out.entry((in_b, r.start.0))
                    .or_insert(DataChange {
                        in_b,
                        start: r.start.0,
                        len: r.len,
                        kind: r.kind,
                        name,
                        changed: 0,
                    })
                    .changed += n;
            }
            k += 1;
        }
    };
    for run in &bytes.runs {
        match run.kind {
            RunKind::Same => {}
            RunKind::Changed | RunKind::Deleted => add(a, false, run.a.clone()),
            RunKind::Inserted => add(b, true, run.b.clone()),
        }
    }
    out.into_values().collect()
}
