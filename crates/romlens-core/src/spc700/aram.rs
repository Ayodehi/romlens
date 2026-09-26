//! Reading an audio RAM image as a program: which bytes the code reaches
//! from its entry points, and the listing of a range.

use std::collections::{BTreeSet, VecDeque};

use crate::spc700::decode::{Flow, Instruction, decode_at};
use crate::spc700::format::{AramSymbols, Formatted, format_instruction};

/// The boot ROM's page; a driver's code never lives there.
pub const IPL_START: u16 = 0xFFC0;

/// What a walk from the entry points found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Walk {
    /// The first byte of every instruction reached.
    pub starts: BTreeSet<u16>,
    /// Every byte of those instructions.
    pub code: BTreeSet<u16>,
    /// Subroutines called, entries included.
    pub routines: BTreeSet<u16>,
    /// Jumps through a table, which the walk cannot follow.
    pub unresolved: Vec<u16>,
}

/// Follow the code from `entries` through branches, jumps and calls.
/// `TCALL` and `BRK` go through their vectors as the image holds them.
/// The walk stops at the boot ROM's page, at a return and at a halt.
pub fn walk(image: &[u8], entries: &[u16]) -> Walk {
    let mut w = Walk::default();
    let mut queue: VecDeque<u16> = entries.iter().copied().collect();
    w.routines.extend(entries.iter().copied());
    let word = |a: u16| {
        u16::from_le_bytes([
            image.get(a as usize).copied().unwrap_or(0),
            image.get(a.wrapping_add(1) as usize).copied().unwrap_or(0),
        ])
    };
    while let Some(mut at) = queue.pop_front() {
        loop {
            if at >= IPL_START || w.starts.contains(&at) {
                break;
            }
            let insn = decode_at(image, at);
            w.starts.insert(at);
            for i in 0..insn.len() as u16 {
                w.code.insert(at.wrapping_add(i));
            }
            let next = insn.next();
            match insn.flow() {
                Flow::Next => {}
                Flow::Branch(t) => queue.push_back(t),
                Flow::Jump(Some(t)) => {
                    queue.push_back(t);
                    break;
                }
                Flow::Jump(None) => {
                    w.unresolved.push(at);
                    break;
                }
                Flow::Call(t) => {
                    let t = t.or_else(|| insn.vector().map(word));
                    if let Some(t) = t {
                        w.routines.insert(t);
                        queue.push_back(t);
                    }
                }
                Flow::Return | Flow::Halt => break,
            }
            if next < at {
                break;
            }
            at = next;
        }
    }
    w
}

/// Instructions one after another from `from`, `count` of them.
pub fn disassemble(
    image: &[u8],
    from: u16,
    count: usize,
    symbols: &dyn AramSymbols,
) -> Vec<(Instruction, Formatted)> {
    let mut out = Vec::with_capacity(count);
    let mut at = from;
    for _ in 0..count {
        let insn = decode_at(image, at);
        let f = format_instruction(&insn, symbols);
        let next = insn.next();
        out.push((insn, f));
        if next < at {
            break;
        }
        at = next;
    }
    out
}
