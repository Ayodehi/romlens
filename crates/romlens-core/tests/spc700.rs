//! The SPC700 instruction set (docs/23, A1).

use romlens_core::spc700::{
    Arg, Flow, Mnemonic, NoAramSymbols, OPCODES, Value, aram, assemble, decode, format_instruction,
};

fn text(bytes: [u8; 3], at: u16) -> String {
    format_instruction(&decode(bytes, at), &NoAramSymbols).text
}

/// Every opcode's text assembles back to its own bytes.
#[test]
fn every_opcode_round_trips_through_the_assembler() {
    let at = 0x0400;
    for op in 0..=255u8 {
        let bytes = [op, 0x12, 0x34];
        let insn = decode(bytes, at);
        let t = format_instruction(&insn, &NoAramSymbols).text;
        let program = assemble(&format!(".org $0400\n{t}\n"))
            .unwrap_or_else(|e| panic!("${op:02X} `{t}`: {e}"));
        assert_eq!(program.chunks.len(), 1, "${op:02X}");
        assert_eq!(program.chunks[0].1, insn.raw(), "${op:02X} `{t}`");
    }
}

#[test]
fn the_table_is_whole() {
    // Every opcode is a real instruction, lengths 1 to 3.
    for (i, o) in OPCODES.iter().enumerate() {
        assert!((1..=3).contains(&o.len), "${i:02X}");
        assert!(o.cycles >= 2, "${i:02X}");
    }
    // Each mnemonic is used.
    for m in Mnemonic::ALL {
        assert!(OPCODES.iter().any(|o| o.mnemonic == m), "{m}");
    }
    // Sixteen TCALLs and eight of each bit instruction, one per bit.
    assert_eq!(
        OPCODES
            .iter()
            .filter(|o| o.mnemonic == Mnemonic::Tcall)
            .count(),
        16
    );
    for m in [Mnemonic::Set1, Mnemonic::Clr1, Mnemonic::Bbs, Mnemonic::Bbc] {
        let bits: Vec<u8> = OPCODES
            .iter()
            .filter(|o| o.mnemonic == m)
            .filter_map(|o| match o.args[0] {
                Arg::DpBit(b) => Some(b),
                _ => None,
            })
            .collect();
        assert_eq!(bits, (0..8).collect::<Vec<_>>(), "{m}");
    }
    // No two opcodes share a name and operands.
    for (i, a) in OPCODES.iter().enumerate() {
        for b in &OPCODES[i + 1..] {
            assert!(
                !(a.mnemonic == b.mnemonic && a.args == b.args),
                "{} {:?}",
                a.mnemonic,
                a.args
            );
        }
    }
}

/// Instructions checked by hand against fullsnes's opcode list.
#[test]
fn known_instructions_read_as_fullsnes_writes_them() {
    let cases: &[([u8; 3], &str)] = &[
        ([0x8F, 0x4C, 0xF2], "MOV DSPADDR,#$4C"),
        ([0xC4, 0xF3, 0x00], "MOV DSPDATA,A"),
        ([0xE4, 0xF4, 0x00], "MOV A,CPUIO0"),
        ([0xFA, 0xF4, 0x12], "MOV $12,CPUIO0"),
        ([0xE8, 0xAA, 0x00], "MOV A,#$AA"),
        ([0xCD, 0xEF, 0x00], "MOV X,#$EF"),
        ([0xBD, 0x00, 0x00], "MOV SP,X"),
        ([0xC6, 0x00, 0x00], "MOV (X),A"),
        ([0xAF, 0x00, 0x00], "MOV (X)+,A"),
        ([0xF5, 0x00, 0x02], "MOV A,!$0200+X"),
        ([0xF7, 0x14, 0x00], "MOV A,[$14]+Y"),
        ([0xC7, 0x14, 0x00], "MOV [$14+X],A"),
        ([0xBA, 0x14, 0x00], "MOVW YA,$14"),
        ([0xDA, 0x14, 0x00], "MOVW $14,YA"),
        ([0x3F, 0x34, 0x12], "CALL !$1234"),
        ([0x1F, 0x00, 0x05], "JMP [!$0500+X]"),
        ([0x5F, 0xC9, 0xFF], "JMP !$FFC9"),
        ([0x4F, 0xC0, 0x00], "PCALL $FFC0"),
        ([0x61, 0x00, 0x00], "TCALL 6"),
        ([0xE2, 0x12, 0x00], "SET1 $12.7"),
        ([0x13, 0x12, 0xFE], "BBC $12.0,$03FE"),
        ([0x2E, 0xF4, 0xFD], "CBNE CPUIO0,$03FD"),
        ([0xFE, 0xFE, 0x00], "DBNZ Y,$03FE"),
        ([0x6E, 0x10, 0xFD], "DBNZ $10,$03FD"),
        ([0xAA, 0x45, 0xA1], "MOV1 C,$0145.5"),
        ([0x2A, 0x45, 0x01], "OR1 C,/$0145.0"),
        ([0x8D, 0x00, 0x00], "MOV Y,#$00"),
        ([0x9E, 0x00, 0x00], "DIV YA,X"),
        ([0xCF, 0x00, 0x00], "MUL YA"),
        ([0x9F, 0x00, 0x00], "XCN A"),
        ([0x2D, 0x00, 0x00], "PUSH A"),
        ([0x8E, 0x00, 0x00], "POP PSW"),
        ([0x19, 0x00, 0x00], "OR (X),(Y)"),
        ([0x18, 0x0F, 0x12], "OR $12,#$0F"),
        ([0xD8, 0xFA, 0x00], "MOV T0TARGET,X"),
        ([0x0C, 0xF1, 0x00], "ASL !CONTROL"),
    ];
    for (bytes, want) in cases {
        let at = 0x0400u16.wrapping_sub(decode(*bytes, 0).len() as u16);
        assert_eq!(text(*bytes, at), *want, "{bytes:02X?}");
    }
}

#[test]
fn the_memory_to_memory_forms_take_the_source_byte_first() {
    // FA ss dd: MOV dd,ss
    let insn = decode([0xFA, 0x20, 0x30], 0);
    assert_eq!(insn.values, [Value::Byte(0x30), Value::Byte(0x20)]);
    assert_eq!(insn.address(0, false), Some(0x30));
    assert_eq!(insn.address(1, true), Some(0x120), "P set: page 1");
    // 8F ii dd: MOV dd,#ii
    let insn = decode([0x8F, 0x7F, 0x0C], 0);
    assert_eq!(insn.values, [Value::Byte(0x0C), Value::Byte(0x7F)]);
}

#[test]
fn flow_names_where_control_goes() {
    assert_eq!(decode([0xF0, 0x02, 0], 0x100).flow(), Flow::Branch(0x104));
    assert_eq!(
        decode([0x2F, 0xFE, 0], 0x100).flow(),
        Flow::Jump(Some(0x100))
    );
    assert_eq!(
        decode([0x5F, 0x00, 0x08], 0).flow(),
        Flow::Jump(Some(0x0800))
    );
    assert_eq!(decode([0x1F, 0x00, 0x08], 0).flow(), Flow::Jump(None));
    assert_eq!(
        decode([0x3F, 0x00, 0x08], 0).flow(),
        Flow::Call(Some(0x0800))
    );
    assert_eq!(decode([0x4F, 0x20, 0], 0).flow(), Flow::Call(Some(0xFF20)));
    let t = decode([0xF1, 0, 0], 0);
    assert_eq!((t.flow(), t.vector()), (Flow::Call(None), Some(0xFFC0)));
    assert_eq!(decode([0x0F, 0, 0], 0).vector(), Some(0xFFDE));
    assert_eq!(decode([0x6F, 0, 0], 0).flow(), Flow::Return);
    assert_eq!(decode([0xFF, 0, 0], 0).flow(), Flow::Halt);
}

#[test]
fn the_walk_follows_code_and_leaves_data() {
    let program = assemble(
        "
        .org $0200
        start:  MOV X,#$EF
                MOV SP,X
                CALL !init
        main:   MOV A,CPUIO0
                CBNE CPUIO0,main   ; wait for a change
                MOV CPUIO0,A
                TCALL 15
                BRA main
        init:   MOV DSPADDR,#$6C
                MOV DSPDATA,#$20
                RET
        table:  .db $01, $02, $03
                .dw init
        tc:     RET
                .org $FFC0
                .dw tc              ; TCALL 15's vector
        ",
    )
    .unwrap();
    let image = program.image();
    let start = program.label("start").unwrap();
    let w = aram::walk(&image, &[start]);
    for name in ["start", "main", "init"] {
        assert!(w.starts.contains(&program.label(name).unwrap()), "{name}");
    }
    let table = program.label("table").unwrap();
    assert!(!w.code.contains(&table), "the table is data");
    assert!(w.routines.contains(&program.label("init").unwrap()));
    assert!(
        w.routines.contains(&program.label("tc").unwrap()),
        "TCALL through its vector"
    );
    let listing = aram::disassemble(&image, start, 3, &NoAramSymbols);
    let lines: Vec<_> = listing.iter().map(|(_, f)| f.text.as_str()).collect();
    assert_eq!(lines, ["MOV X,#$EF", "MOV SP,X", "CALL !$0210"]);
}

#[test]
fn the_assembler_says_what_is_wrong() {
    let e = assemble(".org $0200\nBRA far\n.org $0300\nfar: NOP\n").unwrap_err();
    assert_eq!(e.line, 2);
    assert!(e.message.contains("too far"), "{e}");
    let e = assemble("MOV A,$1234\n").unwrap_err();
    assert!(e.message.contains("fit in a byte"), "{e}");
    let e = assemble("MOV (X)+,Y\n").unwrap_err();
    assert!(e.message.contains("no form"), "{e}");
    let e = assemble("LDA #$12\n").unwrap_err();
    assert!(e.message.contains("not an SPC700 instruction"), "{e}");
}
