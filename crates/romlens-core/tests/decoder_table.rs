//! Every opcode against the transcribed oracle: mnemonic, mode, length under
//! four flag states (from a test-side length table), and encode∘decode.

use romlens_core::cpu65816::{
    AddressingMode, FlagState, Mnemonic, OPCODES, Operand, decode, encode, opcode_for,
};
use romlens_core::{FileOffset, SnesAddress};

fn oracle() -> Vec<(u8, String, String)> {
    let text = include_str!("data/opcodes_65816.txt");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let mut it = l.split_whitespace();
            let op = u8::from_str_radix(it.next().unwrap(), 16).unwrap();
            (
                op,
                it.next().unwrap().to_owned(),
                it.next().unwrap().to_owned(),
            )
        })
        .collect()
}

/// Operand length by oracle mode name under (eff_m, eff_x); independent of
/// the crate's `operand_len`.
fn oracle_len(mode: &str, m8: bool, x8: bool) -> u8 {
    match mode {
        "imp" | "acc" => 0,
        "imm_m" => {
            if m8 {
                1
            } else {
                2
            }
        }
        "imm_x" => {
            if x8 {
                1
            } else {
                2
            }
        }
        "imm8" | "rel8" | "dp" | "dp_x" | "dp_y" | "dp_ind" | "dp_x_ind" | "dp_ind_y"
        | "dp_indl" | "dp_indl_y" | "sr" | "sr_ind_y" => 1,
        "rel16" | "abs" | "abs_x" | "abs_y" | "abs_ind" | "abs_x_ind" | "abs_indl" | "move" => 2,
        "long" | "long_x" => 3,
        other => panic!("unknown oracle mode {other}"),
    }
}

#[test]
fn table_matches_the_oracle() {
    let oracle = oracle();
    assert_eq!(oracle.len(), 256);
    for (op, mnemonic, mode) in &oracle {
        let info = OPCODES[*op as usize];
        assert_eq!(
            info.mnemonic.as_str(),
            mnemonic,
            "opcode ${op:02X} mnemonic"
        );
        assert_eq!(info.mode.oracle_name(), mode, "opcode ${op:02X} mode");
    }
    // Every mnemonic and mode appears at least once.
    for m in Mnemonic::all() {
        assert!(OPCODES.iter().any(|o| o.mnemonic == m), "{m} unused");
    }
    for mode in AddressingMode::all() {
        assert!(OPCODES.iter().any(|o| o.mode == mode), "{mode:?} unused");
    }
}

#[test]
fn lengths_under_every_width_state() {
    let states = [
        (
            "m0x0",
            FlagState {
                m: false,
                x: false,
                e: false,
                ..FlagState::NATIVE_VECTOR
            },
        ),
        (
            "m1x0",
            FlagState {
                m: true,
                x: false,
                e: false,
                ..FlagState::NATIVE_VECTOR
            },
        ),
        (
            "m0x1",
            FlagState {
                m: false,
                x: true,
                e: false,
                ..FlagState::NATIVE_VECTOR
            },
        ),
        (
            "e1",
            FlagState {
                m: false,
                x: false,
                e: true,
                ..FlagState::NATIVE_VECTOR
            },
        ),
    ];
    let bytes = [0u8, 0x12, 0x34, 0x56];
    for (op, _, mode) in oracle() {
        for (name, flags) in states {
            let mut b = bytes;
            b[0] = op;
            let insn = decode(&b, SnesAddress::new(0x80, 0x8000), FileOffset(0), flags).unwrap();
            let expected = 1 + oracle_len(&mode, flags.eff_m(), flags.eff_x());
            assert_eq!(insn.len, expected, "opcode ${op:02X} under {name}");
            assert_eq!(insn.bytes(), &b[..expected as usize]);
        }
    }
}

#[test]
fn encode_inverts_decode() {
    let flags = FlagState {
        m: false,
        x: false,
        e: false,
        ..FlagState::NATIVE_VECTOR
    };
    for op in 0..=255u8 {
        let b = [op, 0x12, 0x34, 0x56];
        let insn = decode(&b, SnesAddress::new(0x80, 0x8000), FileOffset(0), flags).unwrap();
        let re = encode(insn.mnemonic, insn.mode, insn.operand).unwrap();
        assert_eq!(re.as_slice(), insn.bytes(), "opcode ${op:02X}");
        assert_eq!(opcode_for(insn.mnemonic, insn.mode), Some(op));
    }
    // Immediate width follows the operand.
    assert_eq!(
        encode(Mnemonic::LDA, AddressingMode::ImmediateM, Operand::Byte(1)).unwrap(),
        vec![0xA9, 0x01]
    );
    assert_eq!(
        encode(
            Mnemonic::LDA,
            AddressingMode::ImmediateM,
            Operand::Word(0x1234)
        )
        .unwrap(),
        vec![0xA9, 0x34, 0x12]
    );
    assert_eq!(
        encode(Mnemonic::LDA, AddressingMode::Relative8, Operand::Byte(1)),
        None
    );
    assert_eq!(
        encode(
            Mnemonic::MVN,
            AddressingMode::BlockMove,
            Operand::Move {
                src: 0x7E,
                dst: 0x80
            }
        )
        .unwrap(),
        vec![0x54, 0x80, 0x7E]
    );
}

#[test]
fn truncated_input_is_none() {
    let flags = FlagState::NATIVE_VECTOR;
    assert!(decode(&[], SnesAddress::new(0, 0x8000), FileOffset(0), flags).is_none());
    assert!(
        decode(
            &[0x5C, 0x23, 0x84],
            SnesAddress::new(0, 0x8000),
            FileOffset(0),
            flags
        )
        .is_none()
    );
    assert!(
        decode(
            &[0x5C, 0x23, 0x84, 0x80],
            SnesAddress::new(0, 0x8000),
            FileOffset(0),
            flags
        )
        .is_some()
    );
}
