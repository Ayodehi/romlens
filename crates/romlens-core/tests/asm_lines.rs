//! Line index and batch pinned like `hex_rows.rs`.

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::cpu65816::TokenKind;
use romlens_core::fixtures;
use romlens_core::model::{Command, CommentKind, DataKind, OverrideKind, Project};
use romlens_core::viewmodel::asm_lines::NONE_ADDRESS;
use romlens_core::{
    ASM_BATCH_HEADER_LEN, ASM_LINE_STRIDE, ASM_LINE_VERSION, AddressStyle, FileOffset, LineIndex,
    LineKind, RomImage, SnesAddress, TextOptions, encode_lines, format_lines_text,
};

fn setup() -> (RomImage, Project) {
    let rom = RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap();
    let mut project = Project::new(&rom);
    let a = |o: u16| SnesAddress::new(0, o);
    project
        .apply(
            &rom,
            Command::SetComment {
                address: a(0x8007),
                kind: CommentKind::Block,
                text: Some("Force blank\nthen spin".into()),
            },
        )
        .unwrap();
    project
        .apply(
            &rom,
            Command::SetComment {
                address: a(0x8000),
                kind: CommentKind::Line,
                text: Some("disable IRQ".into()),
            },
        )
        .unwrap();
    project
        .apply(
            &rom,
            Command::SetLabel {
                address: a(0x8000),
                name: Some("Boot".into()),
            },
        )
        .unwrap();
    (rom, project)
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes(b[i..i + 2].try_into().unwrap())
}

#[test]
fn line_order_and_lookups() {
    let (rom, project) = setup();
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let idx = LineIndex::build(&rom, &snap, &project);
    let kinds: Vec<(u32, LineKind)> = idx
        .lines
        .iter()
        .take(20)
        .map(|l| (l.offset, l.kind))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (0, LineKind::Section),
            (0, LineKind::Label),
            (0, LineKind::Instruction),
            (1, LineKind::Instruction),
            (2, LineKind::Instruction),
            (3, LineKind::Instruction),
            (5, LineKind::Instruction),
            (7, LineKind::Comment),
            (7, LineKind::Comment),
            (7, LineKind::Instruction),
            (10, LineKind::Label),
            (10, LineKind::Instruction),
            (12, LineKind::Blank),
            (12, LineKind::Section),
            (12, LineKind::Data),
            (14, LineKind::Blank),
            (14, LineKind::Section),
            (14, LineKind::Label),
            (14, LineKind::Instruction),
            (15, LineKind::Blank),
        ]
    );
    // Inside multi-byte instructions and data rows.
    assert_eq!(idx.line_for_offset(0), Some(2));
    assert_eq!(idx.line_for_offset(4), Some(5), "operand byte of SEP");
    assert_eq!(idx.line_for_offset(7), Some(9));
    assert_eq!(idx.line_for_offset(9), Some(9));
    assert_eq!(idx.line_for_offset(13), Some(14));
    assert_eq!(idx.line_for_offset(14), Some(18));
    assert_eq!(idx.line_for_offset(0x8000), None);
    assert_eq!(idx.offset_for_line(9), Some(7));
    assert_eq!(idx.item_range(9), Some((7, 3)));
    assert_eq!(idx.item_range(10), Some((10, 0)));
    assert_eq!(
        idx.line_numbers_for_bytes(0, 15),
        vec![2, 3, 4, 5, 5, 6, 6, 9, 9, 9, 11, 11, 14, 14, 18]
    );
    assert_eq!(
        idx.line_numbers_for_bytes(0x7FFF, 2),
        vec![idx.len() as u32 - 1, NONE_ADDRESS]
    );
    // Data rows split at labelled and referenced addresses and at region ends.
    let mut project = project.clone();
    project
        .apply(
            &rom,
            Command::SetLabel {
                address: SnesAddress::new(0, 0x8025),
                name: Some("Table".into()),
            },
        )
        .unwrap();
    project
        .apply(
            &rom,
            Command::MarkRegion {
                start: FileOffset(0x30),
                len: 6,
                kind: OverrideKind::Data(DataKind::Word),
            },
        )
        .unwrap();
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let idx2 = LineIndex::build(&rom, &snap, &project);
    let rows: Vec<(u32, u8)> = idx2
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Data && l.offset < 0x40)
        .map(|l| (l.offset, l.sub))
        .collect();
    assert_eq!(
        rows,
        vec![
            (12, 2),
            (15, 16),
            (0x1F, 6),
            (0x25, 11),
            (0x30, 6),
            (0x36, 16)
        ]
    );
    assert!(idx2.len() > idx.len(), "labels add lines");
}

#[test]
fn batch_layout_is_pinned() {
    let (rom, project) = setup();
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let idx = LineIndex::build(&rom, &snap, &project);
    let batch = encode_lines(&rom, &snap, &project, &idx, 0, 12);
    assert_eq!(u16_at(&batch, 0), ASM_LINE_VERSION);
    assert_eq!(u16_at(&batch, 2), ASM_LINE_STRIDE);
    assert_eq!(u32_at(&batch, 4), 12);
    let text_off = u32_at(&batch, 8) as usize;
    assert_eq!(text_off, ASM_BATCH_HEADER_LEN + 12 * 96);
    let text_len = u32_at(&batch, 12) as usize;
    assert_eq!(batch.len(), text_off + text_len);
    let rec = |i: usize| &batch[ASM_BATCH_HEADER_LEN + i * 96..ASM_BATCH_HEADER_LEN + (i + 1) * 96];
    let text = |r: &[u8]| {
        let off = u32_at(r, 20) as usize;
        let len = u16_at(r, 18) as usize;
        String::from_utf8(batch[off..off + len].to_vec()).unwrap()
    };
    // Section, label, SEI.
    assert_eq!(rec(0)[8], LineKind::Section as u8);
    assert_eq!(text(rec(0)), "; ==== bank $00 ====");
    assert_eq!(rec(1)[8], LineKind::Label as u8);
    assert_eq!(text(rec(1)), "Boot:");
    assert_eq!(rec(1)[52], TokenKind::UserLabelDef as u8);
    let sei = rec(2);
    assert_eq!(u32_at(sei, 0), 0);
    assert_eq!(u32_at(sei, 4), 0x8000);
    assert_eq!(sei[8], LineKind::Instruction as u8);
    assert_eq!(sei[9], 1);
    assert_eq!(sei[10], 1, "code");
    assert_eq!(sei[11], 90);
    assert_eq!(
        sei[12] & 0x1F,
        0b11111,
        "m x e set, dbr and dp known at reset"
    );
    assert_eq!(sei[13] & 0x01, 0x01, "line comment");
    assert_eq!(sei[13] & 0x04, 0x04, "labelled");
    assert_eq!(sei[13] & 0x10, 0x10, "vector xref in");
    assert_eq!(u16_at(sei, 32), 1);
    assert_eq!(text(sei), "SEI  ; disable IRQ");
    assert_eq!(sei[14], 2);
    assert_eq!(
        (sei[52], u16_at(sei, 54), u16_at(sei, 56)),
        (TokenKind::Mnemonic as u8, 0, 3)
    );
    assert_eq!(
        (sei[58], u16_at(sei, 60), u16_at(sei, 62)),
        (TokenKind::Comment as u8, 5, 13)
    );
    assert_eq!(sei[36], 0x78);
    assert_eq!(u32_at(sei, 24), 0xFFFF_FFFF);
    // STA $2100: hardware register token and auto comment, target with no file offset.
    let sta = rec(9);
    assert_eq!(text(sta), "STA $2100  ; INIDISP");
    assert_eq!(sta[13] & 0x02, 0x02, "block comment above");
    assert_eq!(&sta[36..39], &[0x8D, 0x00, 0x21]);
    assert_eq!(u32_at(sta, 24), 0x2100);
    assert_eq!(u32_at(sta, 28), 0xFFFF_FFFF);
    let kinds: Vec<u8> = (0..sta[14] as usize).map(|t| sta[52 + t * 6]).collect();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Mnemonic as u8,
            TokenKind::HardwareRegister as u8,
            TokenKind::AutoComment as u8
        ]
    );
    // BRA to the auto label: label token, target with a file offset, block end.
    let bra = rec(11);
    assert_eq!(text(bra), "BRA CODE_00800A");
    assert_eq!(bra[13] & 0x08, 0x08);
    assert_eq!(u32_at(bra, 24), 0x800A);
    assert_eq!(u32_at(bra, 28), 10);
    assert_eq!(bra[58], TokenKind::AutoLabel as u8);
    // Clipping.
    let tail = encode_lines(&rom, &snap, &project, &idx, idx.len() as u32 - 2, 10);
    assert_eq!(u32_at(&tail, 4), 2);
}

#[test]
fn text_formatter_shape() {
    let (rom, project) = setup();
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let idx = LineIndex::build(&rom, &snap, &project);
    let text = format_lines_text(
        &rom,
        &snap,
        &project,
        &idx,
        0,
        15,
        TextOptions {
            style: AddressStyle::Both,
            verbose: true,
        },
    );
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "; ==== bank $00 ====");
    assert_eq!(lines[1], "Boot:");
    assert_eq!(
        lines[2],
        "0x000000  $00:8000  78           m1x1e1 $00:$0000  SEI                                         ; disable IRQ"
    );
    assert_eq!(
        lines[5],
        "0x000003  $00:8003  E2 30        m1x1e0 $00:$0000  SEP #$30"
    );
    assert_eq!(
        lines[7],
        "                                                   ; Force blank"
    );
    assert_eq!(
        lines[9],
        "0x000007  $00:8007  8D 00 21     m1x1e0 $00:$0000  STA $2100                                   ; INIDISP"
    );
    assert_eq!(
        lines[11],
        "0x00000A  $00:800A  80 FE        m1x1e0 $00:$0000  BRA CODE_00800A"
    );
    assert_eq!(lines[12], "");
    // Since Phase 2 the entropy heuristic claims the unreached filler.
    assert_eq!(lines[13], "; ---- byte (50%) ----");
    assert_eq!(
        lines[14],
        "0x00000C  $00:800C  EA EA                          db $EA,$EA"
    );
    let snes = format_lines_text(
        &rom,
        &snap,
        &project,
        &idx,
        2,
        1,
        TextOptions {
            style: AddressStyle::Snes,
            verbose: false,
        },
    );
    assert_eq!(
        snes,
        "$00:8000  78           SEI                                         ; disable IRQ\n"
    );
    let file = format_lines_text(
        &rom,
        &snap,
        &project,
        &idx,
        3,
        1,
        TextOptions {
            style: AddressStyle::File,
            verbose: false,
        },
    );
    assert_eq!(file, "0x000001  18           CLC\n");
}

/// A table of `JMP`s padded to four bytes with `NOP`s: each entry is one
/// line, its padding in its bytes, when code follows or the entry before
/// was padded the same; not when the NOP is labelled or stands alone.
#[test]
fn nop_padding_joins_the_jump_before_it() {
    let mut code = vec![0u8; 0x40];
    let mut put = |at: usize, bytes: &[u8]| code[at..at + bytes.len()].copy_from_slice(bytes);
    for (i, to) in [0x8010u16, 0x8014, 0x8020, 0x8024].iter().enumerate() {
        let [lo, hi] = to.to_le_bytes();
        put(i * 3, &[0x20, lo, hi]); // JSR
    }
    put(0x0C, &[0x80, 0xFE]); // BRA $800C
    put(0x10, &[0x4C, 0x30, 0x80, 0xEA]); // JMP $8030 and padding, code after
    put(0x14, &[0x4C, 0x30, 0x80, 0xEA]); // the table's last entry, bytes after
    put(0x20, &[0x4C, 0x30, 0x80, 0xEA]); // its NOP labelled
    put(0x24, &[0x4C, 0x30, 0x80, 0xEA]); // alone, bytes after
    put(0x30, &[0x60]); // RTS
    let bytes = fixtures::build_with_code(
        romlens_core::MappingMode::LoRom,
        0x8000,
        false,
        &code,
        "PADDING",
    );
    let rom = RomImage::from_bytes(bytes, "p.sfc").unwrap();
    let mut project = Project::new(&rom);
    project
        .apply(
            &rom,
            Command::SetLabel {
                address: SnesAddress::new(0, 0x8023),
                name: Some("Spare".into()),
            },
        )
        .unwrap();
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let idx = LineIndex::build(&rom, &snap, &project);
    let line = |off: u32| idx.lines[idx.line_for_offset(off).unwrap()];
    assert_eq!(line(0x10).kind, LineKind::Instruction);
    assert_eq!(line(0x10).sub, 4, "the NOP is in the JMP's line");
    assert_eq!(line(0x13).offset, 0x10, "and selecting it selects the JMP");
    assert_eq!(line(0x14).sub, 4, "the next entry is padded the same");
    assert_eq!(line(0x20).sub, 3, "a labelled NOP stays its own line");
    assert_eq!(line(0x23).kind, LineKind::Data);
    assert_eq!(
        line(0x24).sub,
        3,
        "alone and with no code after, not padding"
    );
    let text = format_lines_text(
        &rom,
        &snap,
        &project,
        &idx,
        0,
        idx.len() as u32,
        TextOptions {
            style: AddressStyle::Snes,
            verbose: false,
        },
    );
    let jmp = text
        .lines()
        .find(|l| l.contains("$00:8010"))
        .unwrap_or_default();
    assert!(jmp.contains("4C 30 80 EA"), "{jmp}");
    assert!(jmp.ends_with("; padded with a NOP"), "{jmp}");
    let from_jmp: Vec<&str> = text
        .lines()
        .skip_while(|l| !l.contains("$00:8010"))
        .take(4)
        .collect();
    assert!(
        from_jmp.iter().all(|l| !l.contains("----")),
        "no section for the padding: {from_jmp:?}"
    );
}
