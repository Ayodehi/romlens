//! Assembly text for one instruction with typed tokens, so shells colour by
//! meaning ("semantic, not styled", docs/08).

use std::fmt::Write as _;

use crate::cpu65816::decode::{Instruction, Operand, TargetKind};
use crate::cpu65816::mnemonic::Mnemonic;
use crate::cpu65816::mode::AddressingMode;
use crate::memory::address::SnesAddress;
use crate::model::hardware::{HardwareRegister, hardware_register, is_system_bank};

/// Token kinds; the numbering is the asm-line batch encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TokenKind {
    Mnemonic = 1,
    Punct = 2,
    Immediate = 3,
    Number = 4,
    AutoLabel = 5,
    UserLabel = 6,
    HardwareRegister = 7,
    Comment = 8,
    AutoComment = 9,
    AutoLabelDef = 10,
    UserLabelDef = 11,
    Directive = 12,
    DataValue = 13,
    Section = 14,
    Warning = 15,
}

impl TokenKind {
    pub const fn from_u8(v: u8) -> Option<TokenKind> {
        use TokenKind::*;
        Some(match v {
            1 => Mnemonic,
            2 => Punct,
            3 => Immediate,
            4 => Number,
            5 => AutoLabel,
            6 => UserLabel,
            7 => HardwareRegister,
            8 => Comment,
            9 => AutoComment,
            10 => AutoLabelDef,
            11 => UserLabelDef,
            12 => Directive,
            13 => DataValue,
            14 => Section,
            15 => Warning,
            _ => return None,
        })
    }
}

/// A byte range of the text with a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: u16,
    pub len: u16,
}

/// A name for an address, and whether a person chose it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// User or imported label (vs. an analyzer-generated one).
    pub user: bool,
}

/// Where the formatter asks for names. Implementations canonicalise mirrors.
pub trait SymbolLookup {
    fn name_for(&self, address: SnesAddress) -> Option<Symbol>;
}

/// No labels at all.
pub struct NoSymbols;

impl SymbolLookup for NoSymbols {
    fn name_for(&self, _: SnesAddress) -> Option<Symbol> {
        None
    }
}

/// Text and tokens for one instruction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Formatted {
    /// `LDA #$01`
    pub text: String,
    pub tokens: Vec<Token>,
    /// Operand only (`#$01`).
    pub operand_text: String,
    /// `MEMSEL`: the hardware register the operand names, when it names one.
    pub register: Option<&'static HardwareRegister>,
}

struct Builder {
    text: String,
    tokens: Vec<Token>,
}

impl Builder {
    fn push(&mut self, kind: TokenKind, s: &str) {
        let start = self.text.len() as u16;
        self.text.push_str(s);
        self.tokens.push(Token {
            kind,
            start,
            len: s.len() as u16,
        });
    }
}

/// The hardware register an instruction's operand names, when the effective
/// address is in the register window of a system bank (or of an assumed bank).
pub fn register_for(insn: &Instruction) -> Option<&'static HardwareRegister> {
    if !(insn.mode.is_absolute() || insn.mode.is_long()) || insn.mode.is_indirect() {
        return None;
    }
    if matches!(insn.mnemonic, Mnemonic::PEA | Mnemonic::JSR | Mnemonic::JMP) {
        return None;
    }
    let t = insn.target?;
    if t.kind != TargetKind::Data {
        return None;
    }
    let bank_ok = if insn.mode.is_long() {
        is_system_bank(t.address.bank())
    } else {
        insn.flags_before.dbr.is_none() || is_system_bank(t.address.bank())
    };
    if !bank_ok {
        return None;
    }
    hardware_register(t.address.offset())
}

fn hex(v: u32, digits: usize) -> String {
    format!("${v:0digits$X}")
}

/// Format the operand, naming labelled targets through `symbols`.
pub fn format_instruction(insn: &Instruction, symbols: &dyn SymbolLookup) -> Formatted {
    use AddressingMode::*;
    let mut b = Builder {
        text: String::new(),
        tokens: Vec::new(),
    };
    b.push(TokenKind::Mnemonic, insn.mnemonic.as_str());
    let register = register_for(insn);
    let operand_start = if insn.mode == Implied || insn.mode == Accumulator {
        b.text.len()
    } else {
        b.text.push(' ');
        b.text.len()
    };
    let val = insn.operand.value();
    // The name to print for the operand's address part, if any.
    let label = |b: &mut Builder, fallback: String| {
        if let Some(t) = insn.target
            && !matches!(insn.mnemonic, Mnemonic::PEA)
            && let Some(sym) = symbols.name_for(t.address)
        {
            b.push(
                if sym.user {
                    TokenKind::UserLabel
                } else {
                    TokenKind::AutoLabel
                },
                &sym.name,
            );
            return;
        }
        let kind = if register.is_some() {
            TokenKind::HardwareRegister
        } else {
            TokenKind::Number
        };
        b.push(kind, &fallback);
    };
    match insn.mode {
        Implied | Accumulator => {}
        ImmediateM | ImmediateX | Immediate8 => {
            let s = match insn.operand {
                Operand::Byte(v) => format!("#{}", hex(v as u32, 2)),
                Operand::Word(v) => format!("#{}", hex(v as u32, 4)),
                _ => "#?".to_owned(),
            };
            b.push(TokenKind::Immediate, &s);
        }
        Relative8 | Relative16 => {
            let t = insn.target.map(|t| t.address.offset()).unwrap_or(0);
            label(&mut b, hex(t as u32, 4));
        }
        Direct => label(&mut b, hex(val, 2)),
        DirectX => {
            label(&mut b, hex(val, 2));
            b.push(TokenKind::Punct, ",X");
        }
        DirectY => {
            label(&mut b, hex(val, 2));
            b.push(TokenKind::Punct, ",Y");
        }
        DirectIndirect => {
            b.push(TokenKind::Punct, "(");
            label(&mut b, hex(val, 2));
            b.push(TokenKind::Punct, ")");
        }
        DirectIndexedIndirect => {
            b.push(TokenKind::Punct, "(");
            label(&mut b, hex(val, 2));
            b.push(TokenKind::Punct, ",X)");
        }
        DirectIndirectIndexed => {
            b.push(TokenKind::Punct, "(");
            label(&mut b, hex(val, 2));
            b.push(TokenKind::Punct, "),Y");
        }
        DirectIndirectLong => {
            b.push(TokenKind::Punct, "[");
            label(&mut b, hex(val, 2));
            b.push(TokenKind::Punct, "]");
        }
        DirectIndirectLongIndexed => {
            b.push(TokenKind::Punct, "[");
            label(&mut b, hex(val, 2));
            b.push(TokenKind::Punct, "],Y");
        }
        Absolute => label(&mut b, hex(val, 4)),
        AbsoluteX => {
            label(&mut b, hex(val, 4));
            b.push(TokenKind::Punct, ",X");
        }
        AbsoluteY => {
            label(&mut b, hex(val, 4));
            b.push(TokenKind::Punct, ",Y");
        }
        AbsoluteLong => label(&mut b, hex(val, 6)),
        AbsoluteLongX => {
            label(&mut b, hex(val, 6));
            b.push(TokenKind::Punct, ",X");
        }
        AbsoluteIndirect => {
            b.push(TokenKind::Punct, "(");
            label(&mut b, hex(val, 4));
            b.push(TokenKind::Punct, ")");
        }
        AbsoluteIndexedIndirect => {
            b.push(TokenKind::Punct, "(");
            label(&mut b, hex(val, 4));
            b.push(TokenKind::Punct, ",X)");
        }
        AbsoluteIndirectLong => {
            b.push(TokenKind::Punct, "[");
            label(&mut b, hex(val, 4));
            b.push(TokenKind::Punct, "]");
        }
        StackRelative => {
            b.push(TokenKind::Number, &hex(val, 2));
            b.push(TokenKind::Punct, ",S");
        }
        StackRelativeIndirectIndexed => {
            b.push(TokenKind::Punct, "(");
            b.push(TokenKind::Number, &hex(val, 2));
            b.push(TokenKind::Punct, ",S),Y");
        }
        BlockMove => {
            if let Operand::Move { src, dst } = insn.operand {
                b.push(TokenKind::Number, &hex(src as u32, 2));
                b.push(TokenKind::Punct, ",");
                b.push(TokenKind::Number, &hex(dst as u32, 2));
            }
        }
    }
    let operand_text = b.text[operand_start..].to_owned();
    Formatted {
        text: b.text,
        tokens: b.tokens,
        operand_text,
        register,
    }
}

/// `78 18 FB` style byte column.
pub fn format_bytes(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{b:02X}");
    }
    s
}
