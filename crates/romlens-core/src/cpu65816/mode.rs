//! Addressing modes. The WDC list plus three immediate classes, because the
//! immediate width is the one thing that depends on the flag state.

use crate::cpu65816::flags::FlagState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressingMode {
    Implied,
    Accumulator,
    /// `#imm`: 1 byte when the effective M flag is set, else 2 (ADC AND BIT CMP EOR LDA ORA SBC).
    ImmediateM,
    /// `#imm`: 1 byte when the effective X flag is set, else 2 (CPX CPY LDX LDY).
    ImmediateX,
    /// `#imm8` always (REP SEP COP BRK WDM).
    Immediate8,
    /// `rel8` branches.
    Relative8,
    /// `rel16` (BRL, PER).
    Relative16,
    /// `dp`
    Direct,
    /// `dp,X`
    DirectX,
    /// `dp,Y`
    DirectY,
    /// `(dp)`
    DirectIndirect,
    /// `(dp,X)`
    DirectIndexedIndirect,
    /// `(dp),Y`
    DirectIndirectIndexed,
    /// `[dp]`
    DirectIndirectLong,
    /// `[dp],Y`
    DirectIndirectLongIndexed,
    /// `abs`
    Absolute,
    /// `abs,X`
    AbsoluteX,
    /// `abs,Y`
    AbsoluteY,
    /// `long`
    AbsoluteLong,
    /// `long,X`
    AbsoluteLongX,
    /// `(abs)` — `JMP ($6C)`, pointer read from bank 0.
    AbsoluteIndirect,
    /// `(abs,X)` — `JMP ($7C)`, `JSR ($FC)`, pointer read from the program bank.
    AbsoluteIndexedIndirect,
    /// `[abs]` — `JML ($DC)`, 24-bit pointer read from bank 0.
    AbsoluteIndirectLong,
    /// `sr,S`
    StackRelative,
    /// `(sr,S),Y`
    StackRelativeIndirectIndexed,
    /// `MVN`/`MVP`: two bank bytes, destination first in the byte stream.
    BlockMove,
}

impl AddressingMode {
    /// Operand bytes under `flags` (the opcode byte is not counted).
    pub const fn operand_len(self, flags: FlagState) -> u8 {
        use AddressingMode::*;
        match self {
            Implied | Accumulator => 0,
            ImmediateM => {
                if flags.eff_m() {
                    1
                } else {
                    2
                }
            }
            ImmediateX => {
                if flags.eff_x() {
                    1
                } else {
                    2
                }
            }
            Immediate8 | Relative8 => 1,
            Direct
            | DirectX
            | DirectY
            | DirectIndirect
            | DirectIndexedIndirect
            | DirectIndirectIndexed
            | DirectIndirectLong
            | DirectIndirectLongIndexed
            | StackRelative
            | StackRelativeIndirectIndexed => 1,
            Relative16
            | Absolute
            | AbsoluteX
            | AbsoluteY
            | AbsoluteIndirect
            | AbsoluteIndexedIndirect
            | AbsoluteIndirectLong
            | BlockMove => 2,
            AbsoluteLong | AbsoluteLongX => 3,
        }
    }

    pub const fn is_immediate(self) -> bool {
        matches!(
            self,
            AddressingMode::ImmediateM | AddressingMode::ImmediateX | AddressingMode::Immediate8
        )
    }

    /// The operand names a direct-page address.
    pub const fn is_direct(self) -> bool {
        use AddressingMode::*;
        matches!(
            self,
            Direct
                | DirectX
                | DirectY
                | DirectIndirect
                | DirectIndexedIndirect
                | DirectIndirectIndexed
                | DirectIndirectLong
                | DirectIndirectLongIndexed
        )
    }

    /// The operand is a 16-bit address in some bank.
    pub const fn is_absolute(self) -> bool {
        use AddressingMode::*;
        matches!(
            self,
            Absolute
                | AbsoluteX
                | AbsoluteY
                | AbsoluteIndirect
                | AbsoluteIndexedIndirect
                | AbsoluteIndirectLong
        )
    }

    pub const fn is_long(self) -> bool {
        matches!(
            self,
            AddressingMode::AbsoluteLong | AddressingMode::AbsoluteLongX
        )
    }

    /// Indexed by X or Y: the effective address is only a base.
    pub const fn is_indexed(self) -> bool {
        use AddressingMode::*;
        matches!(
            self,
            DirectX
                | DirectY
                | DirectIndexedIndirect
                | DirectIndirectIndexed
                | DirectIndirectLongIndexed
                | AbsoluteX
                | AbsoluteY
                | AbsoluteLongX
                | AbsoluteIndexedIndirect
                | StackRelativeIndirectIndexed
        )
    }

    /// Reads a pointer from the operand address before accessing memory.
    pub const fn is_indirect(self) -> bool {
        use AddressingMode::*;
        matches!(
            self,
            DirectIndirect
                | DirectIndexedIndirect
                | DirectIndirectIndexed
                | DirectIndirectLong
                | DirectIndirectLongIndexed
                | AbsoluteIndirect
                | AbsoluteIndexedIndirect
                | AbsoluteIndirectLong
                | StackRelativeIndirectIndexed
        )
    }

    /// Short name as used in the opcode oracle (`tests/data/opcodes_65816.txt`).
    pub const fn oracle_name(self) -> &'static str {
        use AddressingMode::*;
        match self {
            Implied => "imp",
            Accumulator => "acc",
            ImmediateM => "imm_m",
            ImmediateX => "imm_x",
            Immediate8 => "imm8",
            Relative8 => "rel8",
            Relative16 => "rel16",
            Direct => "dp",
            DirectX => "dp_x",
            DirectY => "dp_y",
            DirectIndirect => "dp_ind",
            DirectIndexedIndirect => "dp_x_ind",
            DirectIndirectIndexed => "dp_ind_y",
            DirectIndirectLong => "dp_indl",
            DirectIndirectLongIndexed => "dp_indl_y",
            Absolute => "abs",
            AbsoluteX => "abs_x",
            AbsoluteY => "abs_y",
            AbsoluteLong => "long",
            AbsoluteLongX => "long_x",
            AbsoluteIndirect => "abs_ind",
            AbsoluteIndexedIndirect => "abs_x_ind",
            AbsoluteIndirectLong => "abs_indl",
            StackRelative => "sr",
            StackRelativeIndirectIndexed => "sr_ind_y",
            BlockMove => "move",
        }
    }

    /// Human name for the inspector.
    pub const fn describe(self) -> &'static str {
        use AddressingMode::*;
        match self {
            Implied => "implied",
            Accumulator => "accumulator",
            ImmediateM => "immediate (width from M)",
            ImmediateX => "immediate (width from X)",
            Immediate8 => "immediate, 8-bit",
            Relative8 => "program counter relative",
            Relative16 => "program counter relative long",
            Direct => "direct page",
            DirectX => "direct page indexed, X",
            DirectY => "direct page indexed, Y",
            DirectIndirect => "direct page indirect",
            DirectIndexedIndirect => "direct page indexed indirect, X",
            DirectIndirectIndexed => "direct page indirect indexed, Y",
            DirectIndirectLong => "direct page indirect long",
            DirectIndirectLongIndexed => "direct page indirect long indexed, Y",
            Absolute => "absolute",
            AbsoluteX => "absolute indexed, X",
            AbsoluteY => "absolute indexed, Y",
            AbsoluteLong => "absolute long",
            AbsoluteLongX => "absolute long indexed, X",
            AbsoluteIndirect => "absolute indirect",
            AbsoluteIndexedIndirect => "absolute indexed indirect, X",
            AbsoluteIndirectLong => "absolute indirect long",
            StackRelative => "stack relative",
            StackRelativeIndirectIndexed => "stack relative indirect indexed, Y",
            BlockMove => "block move",
        }
    }

    pub const fn all() -> [AddressingMode; 26] {
        use AddressingMode::*;
        [
            Implied,
            Accumulator,
            ImmediateM,
            ImmediateX,
            Immediate8,
            Relative8,
            Relative16,
            Direct,
            DirectX,
            DirectY,
            DirectIndirect,
            DirectIndexedIndirect,
            DirectIndirectIndexed,
            DirectIndirectLong,
            DirectIndirectLongIndexed,
            Absolute,
            AbsoluteX,
            AbsoluteY,
            AbsoluteLong,
            AbsoluteLongX,
            AbsoluteIndirect,
            AbsoluteIndexedIndirect,
            AbsoluteIndirectLong,
            StackRelative,
            StackRelativeIndirectIndexed,
            BlockMove,
        ]
    }
}
