//! SPC700 assembly text in the Sony syntax fullsnes uses, with the same
//! typed tokens as the 65816 listing so the shells colour both alike.
//!
//! `MOV A,#$12`, `MOV $F2,#$4C`, `MOV A,!$1234+X`, `MOV A,[$12]+Y`,
//! `MOV (X)+,A`, `BBS $12.3,$0456`, `MOV1 C,$0123.5`, `JMP [!$1234+X]`.
//! A `!` marks an absolute address; a bare one is in the direct page.

use crate::cpu65816::format::{Symbol, Token, TokenKind};
use crate::spc700::decode::{Instruction, Value};
use crate::spc700::names::{IoRegister, io_register};
use crate::spc700::opcodes::Arg;

/// Where the formatter asks for names of audio RAM addresses.
pub trait AramSymbols {
    fn name_for(&self, address: u16) -> Option<Symbol>;
}

/// No labels.
pub struct NoAramSymbols;

impl AramSymbols for NoAramSymbols {
    fn name_for(&self, _: u16) -> Option<Symbol> {
        None
    }
}

/// Text and tokens for one instruction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Formatted {
    pub text: String,
    pub tokens: Vec<Token>,
    /// The I/O register an operand names, the first if two do.
    pub register: Option<&'static IoRegister>,
}

struct Builder<'a> {
    text: String,
    tokens: Vec<Token>,
    symbols: &'a dyn AramSymbols,
    register: Option<&'static IoRegister>,
}

impl Builder<'_> {
    fn push(&mut self, kind: TokenKind, s: &str) {
        let start = self.text.len() as u16;
        self.text.push_str(s);
        self.tokens.push(Token {
            kind,
            start,
            len: s.len() as u16,
        });
    }

    /// An address, by its label, its I/O name or its number.
    fn address(&mut self, address: u16, digits: usize, io: bool) {
        if let Some(sym) = self.symbols.name_for(address) {
            let kind = if sym.user {
                TokenKind::UserLabel
            } else {
                TokenKind::AutoLabel
            };
            self.push(kind, &sym.name);
            return;
        }
        if io && let Some(r) = io_register(address) {
            self.register.get_or_insert(r);
            self.push(TokenKind::HardwareRegister, r.name);
            return;
        }
        self.push(TokenKind::Number, &format!("${address:0digits$X}"));
    }
}

/// Format an instruction. The direct page is taken as page 0 (P clear),
/// as every sound driver runs, so `$F2` reads `DSPADDR`.
pub fn format_instruction(insn: &Instruction, symbols: &dyn AramSymbols) -> Formatted {
    let mut b = Builder {
        text: String::new(),
        tokens: Vec::new(),
        symbols,
        register: None,
    };
    b.push(TokenKind::Mnemonic, insn.mnemonic().as_str());
    for (i, arg) in insn.info.args.iter().enumerate() {
        if *arg == Arg::None {
            break;
        }
        if i == 0 {
            b.text.push(' ');
        } else {
            b.push(TokenKind::Punct, ",");
        }
        operand(&mut b, *arg, insn.values[i]);
    }
    Formatted {
        text: b.text,
        tokens: b.tokens,
        register: b.register,
    }
}

fn operand(b: &mut Builder, arg: Arg, value: Value) {
    use TokenKind::{Immediate, Number, Punct};
    let byte = match value {
        Value::Byte(v) => v as u16,
        _ => 0,
    };
    let word = match value {
        Value::Word(v) | Value::Branch(v) => v,
        _ => 0,
    };
    match arg {
        Arg::None => {}
        Arg::A => b.push(Punct, "A"),
        Arg::X => b.push(Punct, "X"),
        Arg::Y => b.push(Punct, "Y"),
        Arg::Ya => b.push(Punct, "YA"),
        Arg::Sp => b.push(Punct, "SP"),
        Arg::Psw => b.push(Punct, "PSW"),
        Arg::C => b.push(Punct, "C"),
        Arg::Imm => b.push(Immediate, &format!("#${byte:02X}")),
        Arg::Dp => b.address(byte, 2, true),
        Arg::DpX => {
            b.address(byte, 2, true);
            b.push(Punct, "+X");
        }
        Arg::DpY => {
            b.address(byte, 2, true);
            b.push(Punct, "+Y");
        }
        Arg::Abs => {
            b.push(Punct, "!");
            b.address(word, 4, true);
        }
        Arg::AbsX => {
            b.push(Punct, "!");
            b.address(word, 4, true);
            b.push(Punct, "+X");
        }
        Arg::AbsY => {
            b.push(Punct, "!");
            b.address(word, 4, true);
            b.push(Punct, "+Y");
        }
        Arg::IndX => b.push(Punct, "(X)"),
        Arg::IndXInc => b.push(Punct, "(X)+"),
        Arg::IndY => b.push(Punct, "(Y)"),
        Arg::DpXInd => {
            b.push(Punct, "[");
            b.address(byte, 2, false);
            b.push(Punct, "+X]");
        }
        Arg::DpIndY => {
            b.push(Punct, "[");
            b.address(byte, 2, false);
            b.push(Punct, "]+Y");
        }
        Arg::AbsXInd => {
            b.push(Punct, "[!");
            b.address(word, 4, false);
            b.push(Punct, "+X]");
        }
        Arg::Rel => b.address(word, 4, false),
        Arg::DpBit(_) | Arg::MemBit | Arg::NotMemBit => {
            if let Value::Bit { address, bit } = value {
                if arg == Arg::NotMemBit {
                    b.push(Punct, "/");
                }
                let digits = if matches!(arg, Arg::DpBit(_)) { 2 } else { 4 };
                b.address(address, digits, true);
                b.push(Punct, &format!(".{bit}"));
            }
        }
        Arg::Upage => b.address(0xFF00 | byte, 4, false),
        Arg::Table(n) => b.push(Number, &n.to_string()),
    }
}
