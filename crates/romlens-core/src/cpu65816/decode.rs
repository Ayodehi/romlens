//! Decode one instruction under a flag state: bytes, operand, effective
//! target and the flag state after it.

use crate::cpu65816::flags::FlagState;
use crate::cpu65816::mnemonic::Mnemonic;
use crate::cpu65816::mode::AddressingMode;
use crate::cpu65816::opcodes::OPCODES;
use crate::memory::address::{FileOffset, SnesAddress};

/// The raw operand as read from the byte stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    None,
    Byte(u8),
    Word(u16),
    Long(u32),
    /// `MVN`/`MVP`: source and destination banks (destination is first in the bytes).
    Move {
        src: u8,
        dst: u8,
    },
}

impl Operand {
    pub const fn value(self) -> u32 {
        match self {
            Operand::None => 0,
            Operand::Byte(b) => b as u32,
            Operand::Word(w) => w as u32,
            Operand::Long(l) => l,
            Operand::Move { src, dst } => ((src as u32) << 8) | dst as u32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetKind {
    /// The instruction transfers control there.
    Code,
    /// The instruction reads or writes there.
    Data,
    /// The instruction reads a pointer from there.
    Pointer,
}

/// Where an instruction's operand points, when that is statically known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub address: SnesAddress,
    pub kind: TargetKind,
    /// `false` when an index register or an assumption is involved.
    pub certain: bool,
}

/// Assumptions the decoder made (`Instruction::assumptions` bits).
pub const ASSUMED_XCE_CARRY: u8 = 0b0000_0001;
pub const ASSUMED_PLP: u8 = 0b0000_0010;
pub const ASSUMED_DBR: u8 = 0b0000_0100;
pub const ASSUMED_DP: u8 = 0b0000_1000;
pub const BANK_WRAP: u8 = 0b0001_0000;
/// Set by the analyzer, not the decoder: this instruction's operand width
/// (or the alignment of the stream it sits in) rests on M/X values assumed
/// after a `PLP` or an `XCE` with unknown carry.
pub const ASSUMED_WIDTHS: u8 = 0b0010_0000;
/// Set by the analyzer, not the decoder: this `PLP` pulls the status a
/// `PHP` earlier on the same path pushed, so M and X after it are that
/// `PHP`'s (the next instruction's record carries them), not assumed.
pub const RESTORED_PLP: u8 = 0b0100_0000;

pub fn assumption_names(bits: u8) -> Vec<&'static str> {
    let mut out = Vec::new();
    if bits & ASSUMED_XCE_CARRY != 0 {
        out.push("carry unknown at XCE; assumed switching to native mode");
    }
    if bits & ASSUMED_PLP != 0 {
        out.push("PLP/RTI: M and X assumed unchanged");
    }
    if bits & ASSUMED_DBR != 0 {
        out.push("data bank unknown; assumed the program bank");
    }
    if bits & ASSUMED_DP != 0 {
        out.push("direct page unknown");
    }
    if bits & BANK_WRAP != 0 {
        out.push("instruction crosses the end of the bank");
    }
    if bits & ASSUMED_WIDTHS != 0 {
        out.push("operand widths rest on M/X assumed after an earlier PLP or XCE");
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub address: SnesAddress,
    pub file_offset: FileOffset,
    pub opcode: u8,
    pub mnemonic: Mnemonic,
    pub mode: AddressingMode,
    pub len: u8,
    pub bytes: [u8; 4],
    pub operand: Operand,
    pub flags_before: FlagState,
    pub flags_after: FlagState,
    pub target: Option<Target>,
    pub assumptions: u8,
}

impl Instruction {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    /// Address of the next instruction in the same bank (wraps at `$FFFF`).
    pub fn next_address(&self) -> SnesAddress {
        SnesAddress::new(
            self.address.bank(),
            self.address.offset().wrapping_add(self.len as u16),
        )
    }

    /// The operand as a signed displacement for the relative modes.
    pub fn displacement(&self) -> i32 {
        match (self.mode, self.operand) {
            (AddressingMode::Relative8, Operand::Byte(b)) => b as i8 as i32,
            (AddressingMode::Relative16, Operand::Word(w)) => w as i16 as i32,
            _ => 0,
        }
    }
}

/// Decode the instruction at the start of `bytes`. `None` when the slice is
/// shorter than the instruction needs under `flags`.
pub fn decode(
    bytes: &[u8],
    at: SnesAddress,
    file_offset: FileOffset,
    flags: FlagState,
) -> Option<Instruction> {
    let opcode = *bytes.first()?;
    let info = OPCODES[opcode as usize];
    let olen = info.mode.operand_len(flags) as usize;
    let len = 1 + olen;
    if bytes.len() < len {
        return None;
    }
    let mut raw = [0u8; 4];
    raw[..len].copy_from_slice(&bytes[..len]);
    let operand = match (info.mode, olen) {
        (AddressingMode::BlockMove, _) => Operand::Move {
            dst: raw[1],
            src: raw[2],
        },
        (_, 0) => Operand::None,
        (_, 1) => Operand::Byte(raw[1]),
        (_, 2) => Operand::Word(u16::from_le_bytes([raw[1], raw[2]])),
        _ => Operand::Long(u32::from_le_bytes([raw[1], raw[2], raw[3], 0])),
    };
    let mut assumptions = 0u8;
    if (at.offset() as usize) + len > 0x1_0000 {
        assumptions |= BANK_WRAP;
    }
    let mut insn = Instruction {
        address: at,
        file_offset,
        opcode,
        mnemonic: info.mnemonic,
        mode: info.mode,
        len: len as u8,
        bytes: raw,
        operand,
        flags_before: flags,
        flags_after: flags,
        target: None,
        assumptions,
    };
    let mut assumptions = insn.assumptions;
    insn.target = compute_target(&insn, &mut assumptions);
    insn.flags_after = apply_flag_effects(&insn, &mut assumptions);
    insn.assumptions = assumptions;
    Some(insn)
}

fn compute_target(insn: &Instruction, assumptions: &mut u8) -> Option<Target> {
    use AddressingMode::*;
    use Mnemonic::*;
    let pb = insn.address.bank();
    let next = insn.next_address();
    let flags = insn.flags_before;
    let m = insn.mnemonic;
    let val = insn.operand.value();
    // PEA pushes a constant, not an address it accesses.
    if m == PEA {
        return None;
    }
    let code = matches!(m, JMP | JML | JSR | JSL | BRA | BRL) || m.is_branch();
    match insn.mode {
        Implied
        | Accumulator
        | ImmediateM
        | ImmediateX
        | Immediate8
        | BlockMove
        | StackRelative
        | StackRelativeIndirectIndexed => None,
        Relative8 | Relative16 => {
            let off = (next.offset() as i32 + insn.displacement()) as u16;
            Some(Target {
                address: SnesAddress::new(pb, off),
                // PER pushes the address it names; everything else jumps there.
                kind: if m == PER {
                    TargetKind::Data
                } else {
                    TargetKind::Code
                },
                certain: true,
            })
        }
        Absolute | AbsoluteX | AbsoluteY => {
            let (bank, certain) = if code {
                (pb, true)
            } else {
                match flags.dbr {
                    Some(b) => (b, true),
                    None => {
                        *assumptions |= ASSUMED_DBR;
                        (pb, false)
                    }
                }
            };
            Some(Target {
                address: SnesAddress::new(bank, val as u16),
                kind: if code {
                    TargetKind::Code
                } else {
                    TargetKind::Data
                },
                certain: certain && !insn.mode.is_indexed(),
            })
        }
        AbsoluteLong | AbsoluteLongX => Some(Target {
            address: SnesAddress::from_u24(val),
            kind: if code {
                TargetKind::Code
            } else {
                TargetKind::Data
            },
            certain: !insn.mode.is_indexed(),
        }),
        AbsoluteIndirect | AbsoluteIndirectLong => Some(Target {
            address: SnesAddress::new(0, val as u16),
            kind: TargetKind::Pointer,
            certain: true,
        }),
        AbsoluteIndexedIndirect => Some(Target {
            address: SnesAddress::new(pb, val as u16),
            kind: TargetKind::Pointer,
            certain: false,
        }),
        mode if mode.is_direct() => {
            let Some(dp) = flags.dp else {
                *assumptions |= ASSUMED_DP;
                return None;
            };
            let addr = SnesAddress::new(0, dp.wrapping_add(val as u16));
            let kind = if mode.is_indirect() {
                TargetKind::Pointer
            } else {
                TargetKind::Data
            };
            // (dp),Y and [dp],Y index after the pointer read, so the slot is exact.
            let certain = matches!(
                mode,
                Direct
                    | DirectIndirect
                    | DirectIndirectLong
                    | DirectIndirectIndexed
                    | DirectIndirectLongIndexed
            );
            Some(Target {
                address: addr,
                kind,
                certain,
            })
        }
        _ => None,
    }
}

fn apply_flag_effects(insn: &Instruction, assumptions: &mut u8) -> FlagState {
    use Mnemonic::*;
    let mut f = insn.flags_before;
    let imm = insn.operand.value() as u8;
    match insn.mnemonic {
        SEP => {
            if imm & 0x20 != 0 {
                f.m = true;
            }
            if imm & 0x10 != 0 {
                f.x = true;
            }
            if imm & 0x01 != 0 {
                f.c = Some(true);
            }
        }
        REP => {
            // In emulation mode M and X are forced to 1; REP cannot clear them.
            if !f.e {
                if imm & 0x20 != 0 {
                    f.m = false;
                }
                if imm & 0x10 != 0 {
                    f.x = false;
                }
            }
            if imm & 0x01 != 0 {
                f.c = Some(false);
            }
        }
        CLC => f.c = Some(false),
        SEC => f.c = Some(true),
        XCE => {
            let old_e = f.e;
            let new_e = match f.c {
                Some(c) => c,
                None => {
                    *assumptions |= ASSUMED_XCE_CARRY;
                    false
                }
            };
            f.e = new_e;
            f.c = Some(old_e);
            if new_e {
                f.m = true;
                f.x = true;
            }
        }
        PLP | RTI => {
            *assumptions |= ASSUMED_PLP;
            f.c = None;
        }
        PLB => f.dbr = None,
        PLD | TCD => f.dp = None,
        ADC | SBC | CMP | CPX | CPY | ASL | LSR | ROL | ROR => f.c = None,
        JSR | JSL => f.c = None,
        _ => {}
    }
    f
}
