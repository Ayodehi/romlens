//! The SPC700's 256 opcodes, row by row as in the fullsnes matrix.
//!
//! Each opcode has at most two operands, written destination first as the
//! assembler reads them (`MOV A,#$12`, `MOV $F2,#$4C`). The bytes follow
//! the opcode in the order the operands are written, except for the two
//! memory-to-memory forms (`dp,dp` and `dp,#imm`), whose source byte comes
//! first. The cycles are the not-taken count; a branch taken adds 2.

use std::fmt;

/// The instruction names, in the Sony syntax fullsnes uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mnemonic {
    Adc,
    Addw,
    And,
    And1,
    Asl,
    Bbc,
    Bbs,
    Bcc,
    Bcs,
    Beq,
    Bmi,
    Bne,
    Bpl,
    Bra,
    Brk,
    Bvc,
    Bvs,
    Call,
    Cbne,
    Clr1,
    Clrc,
    Clrp,
    Clrv,
    Cmp,
    Cmpw,
    Daa,
    Das,
    Dbnz,
    Dec,
    Decw,
    Di,
    Div,
    Ei,
    Eor,
    Eor1,
    Inc,
    Incw,
    Jmp,
    Lsr,
    Mov,
    Mov1,
    Movw,
    Mul,
    Nop,
    Not1,
    Notc,
    Or,
    Or1,
    Pcall,
    Pop,
    Push,
    Ret,
    Reti,
    Rol,
    Ror,
    Sbc,
    Set1,
    Setc,
    Setp,
    Sleep,
    Stop,
    Subw,
    Tcall,
    Tclr1,
    Tset1,
    Xcn,
}

impl Mnemonic {
    pub const ALL: [Mnemonic; 66] = {
        use Mnemonic::*;
        [
            Adc, Addw, And, And1, Asl, Bbc, Bbs, Bcc, Bcs, Beq, Bmi, Bne, Bpl, Bra, Brk, Bvc, Bvs,
            Call, Cbne, Clr1, Clrc, Clrp, Clrv, Cmp, Cmpw, Daa, Das, Dbnz, Dec, Decw, Di, Div, Ei,
            Eor, Eor1, Inc, Incw, Jmp, Lsr, Mov, Mov1, Movw, Mul, Nop, Not1, Notc, Or, Or1, Pcall,
            Pop, Push, Ret, Reti, Rol, Ror, Sbc, Set1, Setc, Setp, Sleep, Stop, Subw, Tcall, Tclr1,
            Tset1, Xcn,
        ]
    };

    pub const fn as_str(self) -> &'static str {
        use Mnemonic::*;
        match self {
            Adc => "ADC",
            Addw => "ADDW",
            And => "AND",
            And1 => "AND1",
            Asl => "ASL",
            Bbc => "BBC",
            Bbs => "BBS",
            Bcc => "BCC",
            Bcs => "BCS",
            Beq => "BEQ",
            Bmi => "BMI",
            Bne => "BNE",
            Bpl => "BPL",
            Bra => "BRA",
            Brk => "BRK",
            Bvc => "BVC",
            Bvs => "BVS",
            Call => "CALL",
            Cbne => "CBNE",
            Clr1 => "CLR1",
            Clrc => "CLRC",
            Clrp => "CLRP",
            Clrv => "CLRV",
            Cmp => "CMP",
            Cmpw => "CMPW",
            Daa => "DAA",
            Das => "DAS",
            Dbnz => "DBNZ",
            Dec => "DEC",
            Decw => "DECW",
            Di => "DI",
            Div => "DIV",
            Ei => "EI",
            Eor => "EOR",
            Eor1 => "EOR1",
            Inc => "INC",
            Incw => "INCW",
            Jmp => "JMP",
            Lsr => "LSR",
            Mov => "MOV",
            Mov1 => "MOV1",
            Movw => "MOVW",
            Mul => "MUL",
            Nop => "NOP",
            Not1 => "NOT1",
            Notc => "NOTC",
            Or => "OR",
            Or1 => "OR1",
            Pcall => "PCALL",
            Pop => "POP",
            Push => "PUSH",
            Ret => "RET",
            Reti => "RETI",
            Rol => "ROL",
            Ror => "ROR",
            Sbc => "SBC",
            Set1 => "SET1",
            Setc => "SETC",
            Setp => "SETP",
            Sleep => "SLEEP",
            Stop => "STOP",
            Subw => "SUBW",
            Tcall => "TCALL",
            Tclr1 => "TCLR1",
            Tset1 => "TSET1",
            Xcn => "XCN",
        }
    }

    pub fn parse(s: &str) -> Option<Mnemonic> {
        Mnemonic::ALL
            .into_iter()
            .find(|m| m.as_str().eq_ignore_ascii_case(s))
    }

    /// A branch that may or may not be taken.
    pub const fn is_conditional(self) -> bool {
        use Mnemonic::*;
        matches!(
            self,
            Bpl | Bmi | Bvc | Bvs | Bcc | Bcs | Bne | Beq | Bbs | Bbc | Cbne | Dbnz
        )
    }
}

impl fmt::Display for Mnemonic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One operand's form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arg {
    None,
    A,
    X,
    Y,
    /// Y and A as one 16-bit value, Y high.
    Ya,
    Sp,
    Psw,
    /// The carry flag, for the bit instructions.
    C,
    /// `#$12`
    Imm,
    /// `$12`, in the direct page (P selects page 0 or 1).
    Dp,
    /// `$12+X`
    DpX,
    /// `$12+Y`
    DpY,
    /// `!$1234`
    Abs,
    /// `!$1234+X`
    AbsX,
    /// `!$1234+Y`
    AbsY,
    /// `(X)`: the direct-page byte X points to.
    IndX,
    /// `(X)+`: the same, then X goes up by one.
    IndXInc,
    /// `(Y)`
    IndY,
    /// `[$12+X]`: the pointer at `$12+X`.
    DpXInd,
    /// `[$12]+Y`: the pointer at `$12`, plus Y.
    DpIndY,
    /// `[!$1234+X]`: `JMP` through a table.
    AbsXInd,
    /// A branch's signed offset.
    Rel,
    /// `$12.3`: a bit of a direct-page byte; the bit is in the opcode.
    DpBit(u8),
    /// `$0123.5`: a bit anywhere in the first 8 KB, 13 bits of address and 3 of bit.
    MemBit,
    /// `/$0123.5`: the same bit, inverted.
    NotMemBit,
    /// `PCALL`'s byte: a call to `$FF00` plus it.
    Upage,
    /// `TCALL`'s number, in the opcode.
    Table(u8),
}

impl Arg {
    /// The operand bytes this form takes.
    pub const fn bytes(self) -> u8 {
        use Arg::*;
        match self {
            Imm | Dp | DpX | DpY | DpXInd | DpIndY | Rel | DpBit(_) | Upage => 1,
            Abs | AbsX | AbsY | AbsXInd | MemBit | NotMemBit => 2,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpcodeInfo {
    pub mnemonic: Mnemonic,
    pub args: [Arg; 2],
    /// Length with the opcode.
    pub len: u8,
    /// Cycles, a branch not taken.
    pub cycles: u8,
}

impl OpcodeInfo {
    /// Both operands take bytes and neither is a branch offset: the source's
    /// byte is first (`MOV dp,dp`, `OR dp,#imm`).
    pub const fn source_first(&self) -> bool {
        self.args[0].bytes() > 0 && self.args[1].bytes() > 0 && !matches!(self.args[1], Arg::Rel)
    }
}

const fn op(mnemonic: Mnemonic, a: Arg, b: Arg, cycles: u8) -> OpcodeInfo {
    OpcodeInfo {
        mnemonic,
        args: [a, b],
        len: 1 + a.bytes() + b.bytes(),
        cycles,
    }
}

use Arg::*;
use Mnemonic as M;

/// Opcode → name, operands, length and cycles.
pub static OPCODES: [OpcodeInfo; 256] = [
    // $00
    op(M::Nop, None, None, 2),
    op(M::Tcall, Table(0), None, 8),
    op(M::Set1, DpBit(0), None, 4),
    op(M::Bbs, DpBit(0), Rel, 5),
    op(M::Or, A, Dp, 3),
    op(M::Or, A, Abs, 4),
    op(M::Or, A, IndX, 3),
    op(M::Or, A, DpXInd, 6),
    op(M::Or, A, Imm, 2),
    op(M::Or, Dp, Dp, 6),
    op(M::Or1, C, MemBit, 5),
    op(M::Asl, Dp, None, 4),
    op(M::Asl, Abs, None, 5),
    op(M::Push, Psw, None, 4),
    op(M::Tset1, Abs, None, 6),
    op(M::Brk, None, None, 8),
    // $10
    op(M::Bpl, Rel, None, 2),
    op(M::Tcall, Table(1), None, 8),
    op(M::Clr1, DpBit(0), None, 4),
    op(M::Bbc, DpBit(0), Rel, 5),
    op(M::Or, A, DpX, 4),
    op(M::Or, A, AbsX, 5),
    op(M::Or, A, AbsY, 5),
    op(M::Or, A, DpIndY, 6),
    op(M::Or, Dp, Imm, 5),
    op(M::Or, IndX, IndY, 5),
    op(M::Decw, Dp, None, 6),
    op(M::Asl, DpX, None, 5),
    op(M::Asl, A, None, 2),
    op(M::Dec, X, None, 2),
    op(M::Cmp, X, Abs, 4),
    op(M::Jmp, AbsXInd, None, 6),
    // $20
    op(M::Clrp, None, None, 2),
    op(M::Tcall, Table(2), None, 8),
    op(M::Set1, DpBit(1), None, 4),
    op(M::Bbs, DpBit(1), Rel, 5),
    op(M::And, A, Dp, 3),
    op(M::And, A, Abs, 4),
    op(M::And, A, IndX, 3),
    op(M::And, A, DpXInd, 6),
    op(M::And, A, Imm, 2),
    op(M::And, Dp, Dp, 6),
    op(M::Or1, C, NotMemBit, 5),
    op(M::Rol, Dp, None, 4),
    op(M::Rol, Abs, None, 5),
    op(M::Push, A, None, 4),
    op(M::Cbne, Dp, Rel, 5),
    op(M::Bra, Rel, None, 4),
    // $30
    op(M::Bmi, Rel, None, 2),
    op(M::Tcall, Table(3), None, 8),
    op(M::Clr1, DpBit(1), None, 4),
    op(M::Bbc, DpBit(1), Rel, 5),
    op(M::And, A, DpX, 4),
    op(M::And, A, AbsX, 5),
    op(M::And, A, AbsY, 5),
    op(M::And, A, DpIndY, 6),
    op(M::And, Dp, Imm, 5),
    op(M::And, IndX, IndY, 5),
    op(M::Incw, Dp, None, 6),
    op(M::Rol, DpX, None, 5),
    op(M::Rol, A, None, 2),
    op(M::Inc, X, None, 2),
    op(M::Cmp, X, Dp, 3),
    op(M::Call, Abs, None, 8),
    // $40
    op(M::Setp, None, None, 2),
    op(M::Tcall, Table(4), None, 8),
    op(M::Set1, DpBit(2), None, 4),
    op(M::Bbs, DpBit(2), Rel, 5),
    op(M::Eor, A, Dp, 3),
    op(M::Eor, A, Abs, 4),
    op(M::Eor, A, IndX, 3),
    op(M::Eor, A, DpXInd, 6),
    op(M::Eor, A, Imm, 2),
    op(M::Eor, Dp, Dp, 6),
    op(M::And1, C, MemBit, 4),
    op(M::Lsr, Dp, None, 4),
    op(M::Lsr, Abs, None, 5),
    op(M::Push, X, None, 4),
    op(M::Tclr1, Abs, None, 6),
    op(M::Pcall, Upage, None, 6),
    // $50
    op(M::Bvc, Rel, None, 2),
    op(M::Tcall, Table(5), None, 8),
    op(M::Clr1, DpBit(2), None, 4),
    op(M::Bbc, DpBit(2), Rel, 5),
    op(M::Eor, A, DpX, 4),
    op(M::Eor, A, AbsX, 5),
    op(M::Eor, A, AbsY, 5),
    op(M::Eor, A, DpIndY, 6),
    op(M::Eor, Dp, Imm, 5),
    op(M::Eor, IndX, IndY, 5),
    op(M::Cmpw, Ya, Dp, 4),
    op(M::Lsr, DpX, None, 5),
    op(M::Lsr, A, None, 2),
    op(M::Mov, X, A, 2),
    op(M::Cmp, Y, Abs, 4),
    op(M::Jmp, Abs, None, 3),
    // $60
    op(M::Clrc, None, None, 2),
    op(M::Tcall, Table(6), None, 8),
    op(M::Set1, DpBit(3), None, 4),
    op(M::Bbs, DpBit(3), Rel, 5),
    op(M::Cmp, A, Dp, 3),
    op(M::Cmp, A, Abs, 4),
    op(M::Cmp, A, IndX, 3),
    op(M::Cmp, A, DpXInd, 6),
    op(M::Cmp, A, Imm, 2),
    op(M::Cmp, Dp, Dp, 6),
    op(M::And1, C, NotMemBit, 4),
    op(M::Ror, Dp, None, 4),
    op(M::Ror, Abs, None, 5),
    op(M::Push, Y, None, 4),
    op(M::Dbnz, Dp, Rel, 5),
    op(M::Ret, None, None, 5),
    // $70
    op(M::Bvs, Rel, None, 2),
    op(M::Tcall, Table(7), None, 8),
    op(M::Clr1, DpBit(3), None, 4),
    op(M::Bbc, DpBit(3), Rel, 5),
    op(M::Cmp, A, DpX, 4),
    op(M::Cmp, A, AbsX, 5),
    op(M::Cmp, A, AbsY, 5),
    op(M::Cmp, A, DpIndY, 6),
    op(M::Cmp, Dp, Imm, 5),
    op(M::Cmp, IndX, IndY, 5),
    op(M::Addw, Ya, Dp, 5),
    op(M::Ror, DpX, None, 5),
    op(M::Ror, A, None, 2),
    op(M::Mov, A, X, 2),
    op(M::Cmp, Y, Dp, 3),
    op(M::Reti, None, None, 6),
    // $80
    op(M::Setc, None, None, 2),
    op(M::Tcall, Table(8), None, 8),
    op(M::Set1, DpBit(4), None, 4),
    op(M::Bbs, DpBit(4), Rel, 5),
    op(M::Adc, A, Dp, 3),
    op(M::Adc, A, Abs, 4),
    op(M::Adc, A, IndX, 3),
    op(M::Adc, A, DpXInd, 6),
    op(M::Adc, A, Imm, 2),
    op(M::Adc, Dp, Dp, 6),
    op(M::Eor1, C, MemBit, 5),
    op(M::Dec, Dp, None, 4),
    op(M::Dec, Abs, None, 5),
    op(M::Mov, Y, Imm, 2),
    op(M::Pop, Psw, None, 4),
    op(M::Mov, Dp, Imm, 5),
    // $90
    op(M::Bcc, Rel, None, 2),
    op(M::Tcall, Table(9), None, 8),
    op(M::Clr1, DpBit(4), None, 4),
    op(M::Bbc, DpBit(4), Rel, 5),
    op(M::Adc, A, DpX, 4),
    op(M::Adc, A, AbsX, 5),
    op(M::Adc, A, AbsY, 5),
    op(M::Adc, A, DpIndY, 6),
    op(M::Adc, Dp, Imm, 5),
    op(M::Adc, IndX, IndY, 5),
    op(M::Subw, Ya, Dp, 5),
    op(M::Dec, DpX, None, 5),
    op(M::Dec, A, None, 2),
    op(M::Mov, X, Sp, 2),
    op(M::Div, Ya, X, 12),
    op(M::Xcn, A, None, 5),
    // $A0
    op(M::Ei, None, None, 3),
    op(M::Tcall, Table(10), None, 8),
    op(M::Set1, DpBit(5), None, 4),
    op(M::Bbs, DpBit(5), Rel, 5),
    op(M::Sbc, A, Dp, 3),
    op(M::Sbc, A, Abs, 4),
    op(M::Sbc, A, IndX, 3),
    op(M::Sbc, A, DpXInd, 6),
    op(M::Sbc, A, Imm, 2),
    op(M::Sbc, Dp, Dp, 6),
    op(M::Mov1, C, MemBit, 4),
    op(M::Inc, Dp, None, 4),
    op(M::Inc, Abs, None, 5),
    op(M::Cmp, Y, Imm, 2),
    op(M::Pop, A, None, 4),
    op(M::Mov, IndXInc, A, 4),
    // $B0
    op(M::Bcs, Rel, None, 2),
    op(M::Tcall, Table(11), None, 8),
    op(M::Clr1, DpBit(5), None, 4),
    op(M::Bbc, DpBit(5), Rel, 5),
    op(M::Sbc, A, DpX, 4),
    op(M::Sbc, A, AbsX, 5),
    op(M::Sbc, A, AbsY, 5),
    op(M::Sbc, A, DpIndY, 6),
    op(M::Sbc, Dp, Imm, 5),
    op(M::Sbc, IndX, IndY, 5),
    op(M::Movw, Ya, Dp, 5),
    op(M::Inc, DpX, None, 5),
    op(M::Inc, A, None, 2),
    op(M::Mov, Sp, X, 2),
    op(M::Das, A, None, 3),
    op(M::Mov, A, IndXInc, 4),
    // $C0
    op(M::Di, None, None, 3),
    op(M::Tcall, Table(12), None, 8),
    op(M::Set1, DpBit(6), None, 4),
    op(M::Bbs, DpBit(6), Rel, 5),
    op(M::Mov, Dp, A, 4),
    op(M::Mov, Abs, A, 5),
    op(M::Mov, IndX, A, 4),
    op(M::Mov, DpXInd, A, 7),
    op(M::Cmp, X, Imm, 2),
    op(M::Mov, Abs, X, 5),
    op(M::Mov1, MemBit, C, 6),
    op(M::Mov, Dp, Y, 4),
    op(M::Mov, Abs, Y, 5),
    op(M::Mov, X, Imm, 2),
    op(M::Pop, X, None, 4),
    op(M::Mul, Ya, None, 9),
    // $D0
    op(M::Bne, Rel, None, 2),
    op(M::Tcall, Table(13), None, 8),
    op(M::Clr1, DpBit(6), None, 4),
    op(M::Bbc, DpBit(6), Rel, 5),
    op(M::Mov, DpX, A, 5),
    op(M::Mov, AbsX, A, 6),
    op(M::Mov, AbsY, A, 6),
    op(M::Mov, DpIndY, A, 7),
    op(M::Mov, Dp, X, 4),
    op(M::Mov, DpY, X, 5),
    op(M::Movw, Dp, Ya, 5),
    op(M::Mov, DpX, Y, 5),
    op(M::Dec, Y, None, 2),
    op(M::Mov, A, Y, 2),
    op(M::Cbne, DpX, Rel, 6),
    op(M::Daa, A, None, 3),
    // $E0
    op(M::Clrv, None, None, 2),
    op(M::Tcall, Table(14), None, 8),
    op(M::Set1, DpBit(7), None, 4),
    op(M::Bbs, DpBit(7), Rel, 5),
    op(M::Mov, A, Dp, 3),
    op(M::Mov, A, Abs, 4),
    op(M::Mov, A, IndX, 3),
    op(M::Mov, A, DpXInd, 6),
    op(M::Mov, A, Imm, 2),
    op(M::Mov, X, Abs, 4),
    op(M::Not1, MemBit, None, 5),
    op(M::Mov, Y, Dp, 3),
    op(M::Mov, Y, Abs, 4),
    op(M::Notc, None, None, 3),
    op(M::Pop, Y, None, 4),
    op(M::Sleep, None, None, 3),
    // $F0
    op(M::Beq, Rel, None, 2),
    op(M::Tcall, Table(15), None, 8),
    op(M::Clr1, DpBit(7), None, 4),
    op(M::Bbc, DpBit(7), Rel, 5),
    op(M::Mov, A, DpX, 4),
    op(M::Mov, A, AbsX, 5),
    op(M::Mov, A, AbsY, 5),
    op(M::Mov, A, DpIndY, 6),
    op(M::Mov, X, Dp, 3),
    op(M::Mov, X, DpY, 4),
    op(M::Mov, Dp, Dp, 5),
    op(M::Mov, Y, DpX, 4),
    op(M::Inc, Y, None, 2),
    op(M::Mov, Y, A, 2),
    op(M::Dbnz, Y, Rel, 4),
    op(M::Stop, None, None, 3),
];

/// The opcode for a name and operand forms, if the SPC700 has one.
pub fn opcode_for(mnemonic: Mnemonic, args: [Arg; 2]) -> Option<u8> {
    OPCODES
        .iter()
        .position(|o| o.mnemonic == mnemonic && o.args == args)
        .map(|i| i as u8)
}
