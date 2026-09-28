//! Byte-exactness without asar: re-parse the listing with a tiny grammar
//! (`org`, `NAME:`, `NAME = $v`, `db/dw/dl`, mnemonic with suffix) and
//! re-encode through the decoder's inverse; the bytes must equal the ROM.
//! Plus the `.sym` shape.

use std::collections::HashMap;

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::cpu65816::{AddressingMode, Mnemonic, Operand, encode};
use romlens_core::fixtures;
use romlens_core::io::{AsarOptions, export_asar, export_symbols};
use romlens_core::model::{Command, CommentKind, DataKind, OverrideKind, Project};
use romlens_core::{FileOffset, MappingMode, RomImage, SnesAddress};

fn parse_value(text: &str, labels: &HashMap<String, u32>, strict: bool) -> Option<u32> {
    let t = text.trim();
    if let Some(h) = t.strip_prefix('$') {
        return u32::from_str_radix(h, 16).ok();
    }
    match labels.get(t) {
        Some(v) => Some(*v),
        None if strict => panic!("undefined label {t:?}"),
        None => Some(0),
    }
}

fn operand_for(
    mnemonic: Mnemonic,
    suffix: &str,
    text: &str,
    pc_next: u32,
    labels: &HashMap<String, u32>,
    strict: bool,
) -> (AddressingMode, Operand) {
    use AddressingMode::*;
    let val = |s: &str| parse_value(s, labels, strict).unwrap();
    let sized = |v: u32| match suffix {
        ".b" => Operand::Byte(v as u8),
        ".w" => Operand::Word(v as u16),
        ".l" => Operand::Long(v & 0xFF_FFFF),
        _ => panic!("missing width suffix on {mnemonic} {text}"),
    };
    let t = text.trim();
    if t.is_empty() {
        return (Implied, Operand::None);
    }
    if t == "A" {
        return (Accumulator, Operand::None);
    }
    if let Some(imm) = t.strip_prefix('#') {
        let v = val(imm);
        let mode = match mnemonic {
            Mnemonic::REP | Mnemonic::SEP | Mnemonic::COP | Mnemonic::BRK | Mnemonic::WDM => {
                Immediate8
            }
            Mnemonic::CPX | Mnemonic::CPY | Mnemonic::LDX | Mnemonic::LDY => ImmediateX,
            _ => ImmediateM,
        };
        return (mode, sized(v));
    }
    // As asar does it (measured against asar 1.91): a block move's operands
    // in the order of its bytes, destination bank first; and a number after
    // a branch is the displacement, where a label is the target.
    if matches!(mnemonic, Mnemonic::MVN | Mnemonic::MVP) {
        let (a, b) = t.split_once(',').unwrap();
        return (
            BlockMove,
            Operand::Move {
                dst: val(a) as u8,
                src: val(b) as u8,
            },
        );
    }
    let relative = |pc_next: u32| {
        let v = val(t);
        if t.starts_with('$') {
            v as i32
        } else {
            (v as i32 & 0xFFFF) - (pc_next as i32 & 0xFFFF)
        }
    };
    if mnemonic.is_branch() || mnemonic == Mnemonic::BRA {
        return (Relative8, Operand::Byte(relative(pc_next) as i8 as u8));
    }
    if matches!(mnemonic, Mnemonic::BRL | Mnemonic::PER) {
        return (Relative16, Operand::Word(relative(pc_next) as i16 as u16));
    }
    if let Some(inner) = t.strip_prefix('(').and_then(|s| s.strip_suffix(",S),Y")) {
        return (
            StackRelativeIndirectIndexed,
            Operand::Byte(val(inner) as u8),
        );
    }
    if let Some(inner) = t.strip_suffix(",S") {
        return (StackRelative, Operand::Byte(val(inner) as u8));
    }
    if let Some(inner) = t.strip_prefix('(').and_then(|s| s.strip_suffix(",X)")) {
        let mode = if suffix == ".b" {
            DirectIndexedIndirect
        } else {
            AbsoluteIndexedIndirect
        };
        return (mode, sized(val(inner)));
    }
    if let Some(inner) = t.strip_prefix('(').and_then(|s| s.strip_suffix("),Y")) {
        return (DirectIndirectIndexed, sized(val(inner)));
    }
    if let Some(inner) = t.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        let mode = if suffix == ".b" {
            DirectIndirect
        } else {
            AbsoluteIndirect
        };
        return (mode, sized(val(inner)));
    }
    if let Some(inner) = t.strip_prefix('[').and_then(|s| s.strip_suffix("],Y")) {
        return (DirectIndirectLongIndexed, sized(val(inner)));
    }
    if let Some(inner) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let mode = if suffix == ".b" {
            DirectIndirectLong
        } else {
            AbsoluteIndirectLong
        };
        return (mode, sized(val(inner)));
    }
    if let Some(inner) = t.strip_suffix(",X") {
        let mode = match suffix {
            ".b" => DirectX,
            ".w" => AbsoluteX,
            _ => AbsoluteLongX,
        };
        return (mode, sized(val(inner)));
    }
    if let Some(inner) = t.strip_suffix(",Y") {
        let mode = if suffix == ".b" { DirectY } else { AbsoluteY };
        return (mode, sized(val(inner)));
    }
    let mode = match suffix {
        ".b" => Direct,
        ".w" => Absolute,
        _ => AbsoluteLong,
    };
    (mode, sized(val(t)))
}

/// Assemble the listing. Returns `(file offset of the first byte, bytes)`.
fn assemble(rom: &RomImage, listing: &str) -> (u32, Vec<u8>) {
    let mut labels: HashMap<String, u32> = HashMap::new();
    let mut result = (0u32, Vec::new());
    for pass in 0..2 {
        let strict = pass == 1;
        let mut pc: Option<u32> = None; // SNES address
        let mut first: Option<u32> = None;
        let mut out: Vec<u8> = Vec::new();
        for raw in listing.lines() {
            let line = raw.split(';').next().unwrap().trim();
            if line.is_empty() || matches!(line, "lorom" | "hirom" | "exhirom") {
                continue;
            }
            if let Some(rest) = line.strip_prefix("org ") {
                let a = parse_value(rest, &labels, true).unwrap();
                if let Some(p) = pc {
                    // Contiguous ranges only: the export writes one org per bank
                    // start, and banks are contiguous in file order.
                    let prev_off = rom.file_offset_for(SnesAddress::from_u24(p)).map(|o| o.0);
                    let new_off = rom.file_offset_for(SnesAddress::from_u24(a)).map(|o| o.0);
                    assert_eq!(prev_off, new_off, "org {rest} is not where the bytes ended");
                }
                pc = Some(a);
                if first.is_none() {
                    first = rom.file_offset_for(SnesAddress::from_u24(a)).map(|o| o.0);
                }
                continue;
            }
            if let Some((name, value)) = line.split_once(" = ") {
                labels.insert(
                    name.trim().to_owned(),
                    parse_value(value, &labels, true).unwrap(),
                );
                continue;
            }
            if let Some(name) = line.strip_suffix(':') {
                labels.insert(name.to_owned(), pc.expect("label before org"));
                continue;
            }
            let pc_now = pc.expect("code before org");
            let (head, rest) = match line.split_once(' ') {
                Some((h, r)) => (h, r.trim()),
                None => (line, ""),
            };
            let bytes: Vec<u8> = match head {
                "db" | "dw" | "dl" => {
                    let width = match head {
                        "db" => 1,
                        "dw" => 2,
                        _ => 3,
                    };
                    let mut b = Vec::new();
                    for v in rest.split(',') {
                        let v = parse_value(v, &labels, strict).unwrap();
                        b.extend_from_slice(&v.to_le_bytes()[..width]);
                    }
                    b
                }
                _ => {
                    let (mn, suffix) = match head.split_once('.') {
                        Some((m, s)) => (m, format!(".{s}")),
                        None => (head, String::new()),
                    };
                    let mnemonic = Mnemonic::parse(mn).unwrap_or_else(|| panic!("mnemonic {mn}"));
                    // Length is fixed by the suffix, so pc_next is known before resolving.
                    let probe = operand_for(mnemonic, &suffix, rest, 0, &labels, false);
                    let len = encode(mnemonic, probe.0, probe.1)
                        .map(|b| b.len())
                        .unwrap_or(0) as u32;
                    let (mode, operand) =
                        operand_for(mnemonic, &suffix, rest, pc_now + len, &labels, strict);
                    encode(mnemonic, mode, operand)
                        .unwrap_or_else(|| panic!("cannot encode {line}"))
                }
            };
            pc = Some(pc_now + bytes.len() as u32);
            out.extend_from_slice(&bytes);
        }
        result = (first.unwrap_or(0), out);
    }
    result
}

fn check_exact(rom: &RomImage, project: &Project, range: Option<(u32, u32)>) -> String {
    let snap = analyze(rom, project, &AnalysisControl::silent()).unwrap();
    let mut listing = String::new();
    export_asar(
        rom,
        &snap,
        project,
        AsarOptions {
            range,
            comments: true,
        },
        &mut listing,
    );
    let (start, bytes) = assemble(rom, &listing);
    let (expect_start, expect_len) = range.unwrap_or((0, rom.len() as u32));
    assert_eq!(start, expect_start);
    let expected = &rom.bytes()[expect_start as usize..(expect_start + expect_len) as usize];
    assert_eq!(bytes.len(), expected.len(), "length differs");
    if let Some(i) = bytes.iter().zip(expected).position(|(a, b)| a != b) {
        panic!(
            "byte {} differs: got {:02X}, ROM has {:02X}\n{listing}",
            FileOffset(expect_start + i as u32),
            bytes[i],
            expected[i]
        );
    }
    listing
}

#[test]
fn fixtures_reassemble_byte_exact() {
    for mode in MappingMode::all() {
        let rom = RomImage::from_bytes(fixtures::for_mapping(mode), "t.sfc").unwrap();
        // ExHiROM is 4 MB + 64 KB: export only the boot bank and the header bank.
        let range = match mode {
            MappingMode::ExHiRom => Some((0x40_8000, 0x8000)),
            _ => None,
        };
        let listing = check_exact(&rom, &Project::new(&rom), range);
        assert!(listing.contains("  SEI\n"), "{mode}");
        assert!(
            listing.contains("  STA.w $2100  ; INIDISP"),
            "{mode}\n{listing}"
        );
        assert!(listing.contains("  BRA CODE_"), "{mode}");
        assert!(listing.contains("RESET_"), "{mode}");
    }
}

#[test]
fn every_opcode_reassembles_through_asar_syntax() {
    use romlens_core::cpu65816::{FlagState, decode};
    use romlens_core::model::Symbols;
    let rom = RomImage::from_bytes(fixtures::all_opcodes_lorom(), "ops.sfc").unwrap();
    let project = Project::new(&rom);
    let auto = std::collections::BTreeMap::new();
    let symbols = Symbols::new(&rom, &project, &auto);
    let flags = FlagState {
        m: false,
        x: false,
        e: false,
        ..FlagState::NATIVE_VECTOR
    };
    let mut listing = String::from("lorom\norg $008000\n");
    let mut pos = 0u32;
    while pos < 571 {
        let insn = decode(
            &rom.bytes()[pos as usize..],
            SnesAddress::new(0, 0x8000 + pos as u16),
            FileOffset(pos),
            flags,
        )
        .unwrap();
        listing.push_str("  ");
        listing.push_str(&romlens_core::io::asar_instruction(&insn, &symbols));
        listing.push('\n');
        pos += insn.len as u32;
    }
    assert!(listing.contains("  LDA.w #$3412\n"), "{listing}");
    assert!(
        listing.contains("  MVN $12,$34  ; from bank $34 to bank $12\n"),
        "{listing}"
    );
    assert!(listing.contains("  ASL A\n"));
    assert!(listing.contains("  JML.l $563412\n"));
    assert!(listing.contains("  LDA.b ($12,S),Y\n"));
    let (start, bytes) = assemble(&rom, &listing);
    assert_eq!(start, 0);
    assert_eq!(bytes, &rom.bytes()[..571]);
}

#[test]
fn annotated_project_reassembles_and_exports_symbols() {
    let rom = RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap();
    let mut project = Project::new(&rom);
    let a = |o: u16| SnesAddress::new(0, o);
    project
        .apply(
            &rom,
            Command::SetLabel {
                address: a(0x8000),
                name: Some("Boot".into()),
            },
        )
        .unwrap();
    project
        .apply(
            &rom,
            Command::SetLabel {
                address: SnesAddress::new(0x00, 0x2100),
                name: Some("INIDISP_W".into()),
            },
        )
        .unwrap();
    project
        .apply(
            &rom,
            Command::SetLabel {
                address: SnesAddress::new(0x7E, 0x0A1C),
                name: Some("SamusPose".into()),
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
            Command::MarkRegion {
                start: FileOffset(0x20),
                len: 6,
                kind: OverrideKind::Data(DataKind::Word),
            },
        )
        .unwrap();
    project
        .apply(
            &rom,
            Command::MarkRegion {
                start: FileOffset(0x30),
                len: 7,
                kind: OverrideKind::Data(DataKind::Long),
            },
        )
        .unwrap();
    project
        .apply(
            &rom,
            Command::SetLabel {
                address: a(0x8033),
                name: Some("Mid".into()),
            },
        )
        .unwrap();
    let listing = check_exact(&rom, &project, Some((0, 0x80)));
    assert!(
        listing.contains("Boot:  ; flags: M=1 X=1 E=1 DBR=$00 DP=$0000\n  SEI  ; disable IRQ\n"),
        "{listing}"
    );
    assert!(
        listing.contains("; Force blank\n; then spin\n  STA.w INIDISP_W  ; INIDISP\n"),
        "{listing}"
    );
    assert!(listing.contains("SamusPose = $7E0A1C"));
    assert!(listing.contains("  dw $0000,$0000,$0000\n"));
    assert!(listing.contains("Mid:\n"));
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let mut sym = String::new();
    export_symbols(&rom, &snap, &project, true, &mut sym);
    let expected = "; Romlens symbols for \"ROMLENS TEST\" (SHA-256 a2462f1d74f993d0b3e10ae9425cd9b48253ac82c99913f07ef704d9c85dc6ee)\n\
[labels]\n\
00:2100 INIDISP_W\n\
00:8000 Boot\n\
00:800A CODE_00800A\n\
00:800E NMI_00800E\n\
00:8033 Mid\n\
7E:0A1C SamusPose\n\
\n\
[comments]\n\
00:8000 disable IRQ\n\
00:8007 Force blank | then spin\n";
    assert_eq!(sym, expected);
    let mut share = String::new();
    export_symbols(&rom, &snap, &project, false, &mut share);
    assert!(!share.contains("CODE_00800A") && !share.contains("NMI_00800E"));
    assert!(share.contains("00:8000 Boot\n"));
}

#[test]
fn ranges_start_and_end_anywhere() {
    let rom = RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap();
    let project = Project::new(&rom);
    // Starts inside the boot code and ends inside the header.
    let listing = check_exact(&rom, &project, Some((3, 0x7FF0)));
    assert!(listing.starts_with("; Romlens export"));
    assert!(listing.contains("org $008003"));
}

/// A table read through an index that is never 0 can have its base inside
/// the instruction before it, as Super Mario World's `LDA $9450,X` does
/// (`$9450` is the last byte of a `JMP`). No line starts there, so the
/// label is defined by value, and the operand keeps its name.
///
/// ```text
/// $8020  LDA $8025,X / JMP $8030     ; $8025 is the JMP's high byte
/// $8030  BRA $8030
/// ```
#[test]
fn a_label_inside_an_instruction_is_defined_by_value() {
    let mut code = vec![0u8; 0x40];
    code[..fixtures::BOOT_CODE.len()].copy_from_slice(&fixtures::BOOT_CODE);
    code[0x20..0x26].copy_from_slice(&[0xBD, 0x25, 0x80, 0x4C, 0x30, 0x80]);
    code[0x30..0x32].copy_from_slice(&[0x80, 0xFE]);
    let mut vectors = fixtures::DEFAULT_VECTORS;
    vectors[3] = 0x8020; // native NMI
    let rom = RomImage::from_bytes(
        fixtures::build_custom(MappingMode::LoRom, 0x8000, false, &code, "INSIDE", vectors),
        "i.sfc",
    )
    .unwrap();
    let mut project = Project::new(&rom);
    project
        .apply(
            &rom,
            Command::SetLabel {
                address: SnesAddress::new(0, 0x8025),
                name: Some("TableBase".into()),
            },
        )
        .unwrap();
    let listing = check_exact(&rom, &project, None);
    assert!(listing.contains("LDA.w TableBase,X"), "{listing}");
    assert!(listing.contains("TableBase = $008025"), "{listing}");
}
