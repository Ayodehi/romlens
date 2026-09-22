//! Callees that skip inline arguments by adding to their stacked return
//! address, recognised from the first instructions of the routine.
//!
//! The idiom has one shape with many spellings: load the return address
//! from the stack (`LDA $01,S`, or `$03,S` after `PHP; PHB`), possibly park
//! it in X or Y while the arguments are read, add a constant, and store it
//! back to the same slot. The constant is the number of inline bytes.

use crate::cpu65816::{AddressingMode, Instruction, Mnemonic, Operand};

/// Instructions scanned from the callee's entry before giving up.
pub const SCAN_LIMIT: u32 = 40;

/// Follows the return address through A, X and Y from a callee's entry.
#[derive(Debug, Default)]
pub struct ReturnAdjust {
    /// Bytes pushed since the entry: the return address is at `1 + pushed`,S.
    pushed: i32,
    /// The adjustment accumulated on the copy each register holds, if any.
    a: Option<u16>,
    x: Option<u16>,
    y: Option<u16>,
    seen: u32,
}

impl ReturnAdjust {
    /// Feed the next instruction of the callee. `Some(n)` when it wrote the
    /// return address back `n` bytes further on.
    pub fn observe(&mut self, insn: &Instruction) -> Option<u16> {
        use AddressingMode::*;
        use Mnemonic::*;
        self.seen += 1;
        if self.seen > SCAN_LIMIT {
            return None;
        }
        let f = insn.flags_before;
        let m_bytes: i32 = if f.eff_m() { 1 } else { 2 };
        let x_bytes: i32 = if f.eff_x() { 1 } else { 2 };
        let slot = matches!(insn.operand, Operand::Byte(k) if k as i32 == 1 + self.pushed);
        let mut found = None;
        match (insn.mnemonic, insn.mode) {
            (LDA, StackRelative) if slot => self.a = Some(0),
            (STA, StackRelative) if slot => {
                found = self.a.filter(|n| *n > 0);
                self.a = None;
            }
            (ADC, ImmediateM) => {
                self.a = self.a.map(|n| n.wrapping_add(insn.operand.value() as u16));
            }
            (INC, Accumulator) => self.a = self.a.map(|n| n.wrapping_add(1)),
            (INX, _) => self.x = self.x.map(|n| n.wrapping_add(1)),
            (INY, _) => self.y = self.y.map(|n| n.wrapping_add(1)),
            (TAX, _) => self.x = self.a,
            (TAY, _) => self.y = self.a,
            (TXA, _) => self.a = self.x,
            (TYA, _) => self.a = self.y,
            (TXY, _) => self.y = self.x,
            (TYX, _) => self.x = self.y,
            // Anything else that loads a register loses the copy in it.
            (LDA | PLA | TDC | TSC | XBA | ADC | SBC | AND | ORA | EOR, _) => self.a = None,
            (LDX | PLX | TSX | DEX, _) => self.x = None,
            (LDY | PLY | DEY, _) => self.y = None,
            _ => {}
        }
        self.pushed += match insn.mnemonic {
            PHP | PHB | PHK => 1,
            PHA => m_bytes,
            PHX | PHY => x_bytes,
            PHD | PEA | PEI | PER => 2,
            PLP | PLB => -1,
            PLA => -m_bytes,
            PLX | PLY => -x_bytes,
            PLD => -2,
            _ => 0,
        };
        found
    }
}
