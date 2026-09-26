//! The SPC700, the sound CPU (docs/23): its instruction set, decoded and
//! formatted in the Sony syntax, encoded and assembled for tests and
//! fixtures, and its I/O registers named.

pub mod aram;
pub mod decode;
pub mod encode;
pub mod format;
pub mod names;
pub mod opcodes;

pub use decode::{Flow, Instruction, Value, decode, decode_at};
pub use encode::{AsmError, Program, assemble, encode};
pub use format::{AramSymbols, Formatted, NoAramSymbols, format_instruction};
pub use names::{IO_REGISTERS, IoRegister, io_named, io_register};
pub use opcodes::{Arg, Mnemonic, OPCODES, OpcodeInfo, opcode_for};
