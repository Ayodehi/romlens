//! Decoding one SPC700 instruction, with where it can go next and which
//! memory it names.

use crate::spc700::opcodes::{Arg, Mnemonic, OPCODES, OpcodeInfo};

/// An operand's value, as the bytes give it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    None,
    /// An immediate, a direct-page offset, or `PCALL`'s byte.
    Byte(u8),
    /// An absolute address.
    Word(u16),
    /// A branch's target, worked out from its offset.
    Branch(u16),
    /// A bit of memory: `MemBit`'s 13-bit address and bit, or a `DpBit`'s
    /// direct-page offset and bit.
    Bit {
        address: u16,
        bit: u8,
    },
    /// `TCALL`'s number.
    Table(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub address: u16,
    pub opcode: u8,
    pub info: &'static OpcodeInfo,
    /// Each operand's value, in the order the operands are written.
    pub values: [Value; 2],
    pub bytes: [u8; 3],
}

/// Where control goes after an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// To the next instruction.
    Next,
    /// To the target or the next instruction.
    Branch(u16),
    /// Only to the target; `None` through a table (`JMP [!a+X]`).
    Jump(Option<u16>),
    /// To a subroutine, then back. `None` when the target is in a vector
    /// (`TCALL`, `BRK`): see [`Instruction::vector`].
    Call(Option<u16>),
    /// `RET`, `RETI`.
    Return,
    /// `SLEEP`, `STOP`: the SPC700 stops until reset.
    Halt,
}

impl Instruction {
    pub fn mnemonic(&self) -> Mnemonic {
        self.info.mnemonic
    }

    pub fn len(&self) -> u8 {
        self.info.len
    }

    /// Never empty: an opcode is one byte at least.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The instruction's bytes.
    pub fn raw(&self) -> &[u8] {
        &self.bytes[..self.info.len as usize]
    }

    pub fn next(&self) -> u16 {
        self.address.wrapping_add(self.info.len as u16)
    }

    pub fn flow(&self) -> Flow {
        use Mnemonic::*;
        let target = self.values.iter().find_map(|v| match *v {
            Value::Branch(t) => Some(t),
            _ => None,
        });
        match self.info.mnemonic {
            m if m.is_conditional() => Flow::Branch(target.unwrap_or(0)),
            Bra => Flow::Jump(target),
            Jmp => match self.values[0] {
                Value::Word(a) if self.info.args[0] == Arg::Abs => Flow::Jump(Some(a)),
                _ => Flow::Jump(None),
            },
            Call => match self.values[0] {
                Value::Word(a) => Flow::Call(Some(a)),
                _ => Flow::Call(None),
            },
            Pcall => match self.values[0] {
                Value::Byte(u) => Flow::Call(Some(0xFF00 | u as u16)),
                _ => Flow::Call(None),
            },
            Tcall | Brk => Flow::Call(None),
            Ret | Reti => Flow::Return,
            Sleep | Stop => Flow::Halt,
            _ => Flow::Next,
        }
    }

    /// Where `TCALL n` and `BRK` read their target: the vector at
    /// `$FFDE - 2n` (`BRK` uses `TCALL 0`'s).
    pub fn vector(&self) -> Option<u16> {
        match self.info.mnemonic {
            Mnemonic::Tcall => match self.values[0] {
                Value::Table(n) => Some(0xFFDE - 2 * n as u16),
                _ => None,
            },
            Mnemonic::Brk => Some(0xFFDE),
            _ => None,
        }
    }

    /// The memory each operand names, `p` being the direct-page flag
    /// (page 1 when set). Indexed and indirect operands name their base.
    pub fn address(&self, operand: usize, p: bool) -> Option<u16> {
        let page = if p { 0x100 } else { 0 };
        match (self.info.args[operand], self.values[operand]) {
            (Arg::Dp | Arg::DpX | Arg::DpY | Arg::DpXInd | Arg::DpIndY, Value::Byte(d)) => {
                Some(page | d as u16)
            }
            (Arg::DpBit(_), Value::Bit { address, .. }) => Some(page | address),
            (Arg::Abs | Arg::AbsX | Arg::AbsY | Arg::AbsXInd, Value::Word(a)) => Some(a),
            (Arg::MemBit | Arg::NotMemBit, Value::Bit { address, .. }) => Some(address),
            _ => None,
        }
    }
}

/// Decode the instruction whose bytes are `bytes` (the opcode and the two
/// bytes after it, whether or not it uses them) at `address`.
pub fn decode(bytes: [u8; 3], address: u16) -> Instruction {
    let info = &OPCODES[bytes[0] as usize];
    let next = address.wrapping_add(info.len as u16);
    // The operand bytes, in the order the operands are written.
    let mut at = 1usize;
    let mut take = |n: u8| {
        let v = match n {
            1 => bytes[at] as u16,
            2 => u16::from_le_bytes([bytes[at], bytes[at + 1]]),
            _ => 0,
        };
        at += n as usize;
        v
    };
    let mut raw = [0u16; 2];
    if info.source_first() {
        raw[1] = take(info.args[1].bytes());
        raw[0] = take(info.args[0].bytes());
    } else {
        raw[0] = take(info.args[0].bytes());
        raw[1] = take(info.args[1].bytes());
    }
    let mut values = [Value::None; 2];
    for (i, arg) in info.args.iter().enumerate() {
        let v = raw[i];
        values[i] = match *arg {
            Arg::Imm | Arg::Dp | Arg::DpX | Arg::DpY | Arg::DpXInd | Arg::DpIndY | Arg::Upage => {
                Value::Byte(v as u8)
            }
            Arg::Abs | Arg::AbsX | Arg::AbsY | Arg::AbsXInd => Value::Word(v),
            Arg::Rel => Value::Branch(next.wrapping_add(v as u8 as i8 as u16)),
            Arg::DpBit(b) => Value::Bit { address: v, bit: b },
            Arg::MemBit | Arg::NotMemBit => Value::Bit {
                address: v & 0x1FFF,
                bit: (v >> 13) as u8,
            },
            Arg::Table(n) => Value::Table(n),
            _ => Value::None,
        };
    }
    Instruction {
        address,
        opcode: bytes[0],
        info,
        values,
        bytes,
    }
}

/// Decode at `address` in a 64 KB image, wrapping at its end as the
/// SPC700 does. A shorter image reads as zero past its end.
pub fn decode_at(image: &[u8], address: u16) -> Instruction {
    let byte = |a: u16| image.get(a as usize).copied().unwrap_or(0);
    decode(
        [
            byte(address),
            byte(address.wrapping_add(1)),
            byte(address.wrapping_add(2)),
        ],
        address,
    )
}
