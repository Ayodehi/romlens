//! The inverse of the table: bytes for a mnemonic, mode and operand. Used by
//! the export byte-exactness test and the assembler round trip.

use crate::cpu65816::decode::Operand;
use crate::cpu65816::mnemonic::Mnemonic;
use crate::cpu65816::mode::AddressingMode;
use crate::cpu65816::opcodes::opcode_for;

/// Encode an instruction. Immediate widths follow the operand's own width
/// (`Byte` or `Word`). `None` when the mnemonic has no such mode or the
/// operand does not fit.
pub fn encode(mnemonic: Mnemonic, mode: AddressingMode, operand: Operand) -> Option<Vec<u8>> {
    use AddressingMode::*;
    let opcode = opcode_for(mnemonic, mode)?;
    let mut out = vec![opcode];
    match (mode, operand) {
        (Implied | Accumulator, Operand::None) => {}
        (ImmediateM | ImmediateX, Operand::Byte(b)) => out.push(b),
        (ImmediateM | ImmediateX, Operand::Word(w)) => out.extend_from_slice(&w.to_le_bytes()),
        (Immediate8 | Relative8, Operand::Byte(b)) => out.push(b),
        (m, Operand::Byte(b))
            if m.is_direct() || matches!(m, StackRelative | StackRelativeIndirectIndexed) =>
        {
            out.push(b)
        }
        (Relative16, Operand::Word(w)) => out.extend_from_slice(&w.to_le_bytes()),
        (m, Operand::Word(w)) if m.is_absolute() => out.extend_from_slice(&w.to_le_bytes()),
        (m, Operand::Long(l)) if m.is_long() => {
            out.extend_from_slice(&l.to_le_bytes()[..3]);
        }
        (BlockMove, Operand::Move { src, dst }) => {
            out.push(dst);
            out.push(src);
        }
        _ => return None,
    }
    Some(out)
}
