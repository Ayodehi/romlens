//! The 65816 decoder: opcode table, addressing modes, flag state, decoding
//! with static targets and flag effects, encoding (for round trips) and
//! formatting with typed tokens.

pub mod decode;
pub mod encode;
pub mod flags;
pub mod format;
pub mod mnemonic;
pub mod mode;
pub mod opcodes;

pub use decode::{
    ASSUMED_DBR, ASSUMED_DP, ASSUMED_PLP, ASSUMED_WIDTHS, ASSUMED_XCE_CARRY, BANK_WRAP,
    Instruction, Operand, Target, TargetKind, assumption_names, decode,
};
pub use encode::encode;
pub use flags::FlagState;
pub use format::{
    Formatted, NoSymbols, Symbol, SymbolLookup, Token, TokenKind, format_bytes, format_instruction,
    register_for,
};
pub use mnemonic::Mnemonic;
pub use mode::AddressingMode;
pub use opcodes::{OPCODES, OpcodeInfo, opcode_for};
