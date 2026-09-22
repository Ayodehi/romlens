//! Peepholes that recover the data bank and direct page from the idioms
//! every SNES program uses to set them.

use crate::cpu65816::{AddressingMode, Instruction, Mnemonic, Operand};

/// Refine `cur.flags_after` from the instructions decoded just before it
/// (`history` is oldest first; only the last two matter).
///
/// * `PHK; PLB` → DBR = program bank
/// * `LDA #imm8; PHA; PLB` → DBR = imm8
/// * `PEA #imm16; PLB` → DBR = low byte; a second `PLB` → high byte
/// * `LDA #imm16; TCD` → D = imm16
/// * `PEA #imm16; PLD` → D = imm16
pub fn refine(history: &[Instruction], cur: &mut Instruction) {
    let prev = history.last();
    let prev2 = history.len().checked_sub(2).map(|i| &history[i]);
    match cur.mnemonic {
        Mnemonic::PLB => {
            if let Some(p) = prev {
                match (p.mnemonic, p.operand) {
                    (Mnemonic::PHK, _) => cur.flags_after.dbr = Some(cur.address.bank()),
                    (Mnemonic::PEA, Operand::Word(w)) => cur.flags_after.dbr = Some(w as u8),
                    (Mnemonic::PHA, _) => {
                        if let Some(p2) = prev2
                            && p2.mnemonic == Mnemonic::LDA
                            && p2.mode == AddressingMode::ImmediateM
                            && let Operand::Byte(b) = p2.operand
                        {
                            cur.flags_after.dbr = Some(b);
                        }
                    }
                    (Mnemonic::PLB, _) => {
                        if let Some(p2) = prev2
                            && p2.mnemonic == Mnemonic::PEA
                            && let Operand::Word(w) = p2.operand
                        {
                            cur.flags_after.dbr = Some((w >> 8) as u8);
                        }
                    }
                    _ => {}
                }
            }
        }
        Mnemonic::TCD => {
            if let Some(p) = prev
                && p.mnemonic == Mnemonic::LDA
                && p.mode == AddressingMode::ImmediateM
                && let Operand::Word(w) = p.operand
            {
                cur.flags_after.dp = Some(w);
            }
        }
        Mnemonic::PLD => {
            if let Some(p) = prev
                && p.mnemonic == Mnemonic::PEA
                && let Operand::Word(w) = p.operand
            {
                cur.flags_after.dp = Some(w);
            }
        }
        _ => {}
    }
}
