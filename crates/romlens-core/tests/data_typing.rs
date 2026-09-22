//! Typed data: a table of addresses reads as the labels it names, and the
//! parameters survive a save.

use romlens_core::analysis::{AnalysisControl, AnalysisSnapshot, analyze};
use romlens_core::fixtures;
use romlens_core::io::{AsarOptions, export_asar, from_files, to_files};
use romlens_core::model::{
    BankRule, Command, DataKind, OverrideKind, Project, RegionKind, TableElem,
};
use romlens_core::viewmodel::asm_lines::{LineIndex, TextOptions, format_lines_text};
use romlens_core::{AddressStyle, FileOffset, RomImage, SnesAddress};

/// A LoROM with a four-entry table at `0x20` naming the boot routine and the
/// catch-all, and a three-byte table at `0x40` naming them with their banks.
fn rom() -> RomImage {
    let mut bytes = fixtures::minimal_lorom();
    for (i, target) in [0x8000u16, 0x800E, 0x8000, 0x800E].iter().enumerate() {
        bytes[0x20 + i * 2..0x22 + i * 2].copy_from_slice(&target.to_le_bytes());
    }
    for (i, target) in [0x00_8000u32, 0x00_800E].iter().enumerate() {
        bytes[0x40 + i * 3..0x43 + i * 3].copy_from_slice(&target.to_le_bytes()[..3]);
    }
    RomImage::from_bytes(bytes, "tables.sfc").unwrap()
}

fn mark(project: &mut Project, rom: &RomImage, start: u32, len: u32, kind: DataKind) {
    project
        .apply(
            rom,
            Command::MarkRegion {
                start: FileOffset(start),
                len,
                kind: OverrideKind::Data(kind),
            },
        )
        .unwrap();
}

fn listing(rom: &RomImage, project: &Project, snap: &AnalysisSnapshot, at: u32) -> String {
    let index = LineIndex::build(rom, snap, project);
    let first = index.line_for_offset(at).unwrap_or(0) as u32;
    format_lines_text(
        rom,
        snap,
        project,
        &index,
        first,
        4,
        TextOptions {
            style: AddressStyle::File,
            verbose: false,
        },
    )
}

/// The most legible single result of the phase: a dispatch table reads as the
/// routines it dispatches to.
#[test]
fn a_code_table_renders_as_its_targets() {
    let rom = rom();
    let mut project = Project::new(&rom);
    mark(
        &mut project,
        &rom,
        0x20,
        8,
        DataKind::Table {
            stride: 2,
            elem: TableElem::Code(BankRule::SameBank),
        },
    );
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let text = listing(&rom, &project, &snap, 0x20);
    assert!(
        text.contains("dw RESET_008000,NMI_00800E,RESET_008000,NMI_00800E"),
        "{text}"
    );

    // Marking it code makes its targets code, which is the correction path a
    // table the resolver could not read points at.
    assert_eq!(
        snap.region_at(FileOffset(0x0E)).unwrap().kind,
        RegionKind::Code
    );
    // One uncertain xref per entry: the user asserted the table's shape, not
    // that any one entry is ever taken.
    let to_catch_all: Vec<_> = snap
        .xrefs_to(SnesAddress::new(0x00, 0x800E))
        .iter()
        .filter(|x| x.from.0 >= 0x20 && x.from.0 < 0x28)
        .collect();
    assert_eq!(to_catch_all.len(), 2);
    assert!(to_catch_all.iter().all(|x| !x.certain));
}

/// A three-byte entry carries its own bank, and the row width follows.
#[test]
fn a_long_pointer_table_uses_its_own_banks() {
    let rom = rom();
    let mut project = Project::new(&rom);
    mark(
        &mut project,
        &rom,
        0x40,
        6,
        DataKind::Pointer {
            bank: BankRule::FromEntry,
        },
    );
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let text = listing(&rom, &project, &snap, 0x40);
    assert!(text.contains("dl RESET_008000,NMI_00800E"), "{text}");
}

/// A fixed bank rule reads the entries somewhere else entirely.
#[test]
fn a_fixed_bank_rule_changes_where_entries_point() {
    let rom = rom();
    let mut project = Project::new(&rom);
    // Bank $80 mirrors bank $00 in LoROM, so the same bytes resolve to the
    // same ROM offsets and the same labels.
    mark(
        &mut project,
        &rom,
        0x20,
        8,
        DataKind::Table {
            stride: 2,
            elem: TableElem::Code(BankRule::Fixed(0x80)),
        },
    );
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    assert!(listing(&rom, &project, &snap, 0x20).contains("dw RESET_008000"));

    // A bank with no ROM behind it names nothing, and the entries stay
    // numbers rather than becoming wrong labels.
    let mut project = Project::new(&rom);
    mark(
        &mut project,
        &rom,
        0x20,
        8,
        DataKind::Table {
            stride: 2,
            elem: TableElem::Pointer(BankRule::Fixed(0x7E)),
        },
    );
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let text = listing(&rom, &project, &snap, 0x20);
    assert!(text.contains("dw $8000,$800E"), "{text}");
}

/// `Raw` is the default and means "just bytes", so a plain table is unchanged
/// from Phase 1.
#[test]
fn a_raw_table_is_still_numbers() {
    let rom = rom();
    let mut project = Project::new(&rom);
    mark(
        &mut project,
        &rom,
        0x20,
        8,
        DataKind::Table {
            stride: 2,
            elem: TableElem::Raw,
        },
    );
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let text = listing(&rom, &project, &snap, 0x20);
    assert!(text.contains("db $00,$80"), "{text}");
    assert!(!text.contains("RESET_008000"), "{text}");
}

/// The parameters survive a save, and a v1 package — which had neither field —
/// still opens as a raw table in the same bank.
#[test]
fn parameters_round_trip_through_a_package() {
    let rom = rom();
    let mut project = Project::new(&rom);
    for (start, len, kind) in [
        (
            0x20u32,
            8u32,
            DataKind::Table {
                stride: 2,
                elem: TableElem::Code(BankRule::Fixed(0xC0)),
            },
        ),
        (
            0x40,
            6,
            DataKind::Pointer {
                bank: BankRule::FromEntry,
            },
        ),
        (
            0x60,
            4,
            DataKind::Table {
                stride: 4,
                elem: TableElem::Pointer(BankRule::SameBank),
            },
        ),
    ] {
        mark(&mut project, &rom, start, len, kind);
    }
    let files = to_files(&rom, &project);
    assert_eq!(from_files(&rom, &files).unwrap(), project);

    let json = String::from_utf8(files["regions.json"].clone()).unwrap();
    assert!(json.contains("\"elem\": \"code\""), "{json}");
    assert!(json.contains("\"bank\": \"$C0\""), "{json}");
    assert!(json.contains("\"bank\": \"entry\""), "{json}");
    assert!(
        !json.contains("\"bank\": \"same\""),
        "the default is left out so a plain table's JSON is as short as v1's:\n{json}"
    );
}

/// The asar export names the targets too. asar resolves a label there exactly
/// as it resolves a branch target, so the listing reassembles.
#[test]
fn the_asar_export_names_its_targets() {
    let rom = rom();
    let mut project = Project::new(&rom);
    mark(
        &mut project,
        &rom,
        0x20,
        8,
        DataKind::Table {
            stride: 2,
            elem: TableElem::Code(BankRule::SameBank),
        },
    );
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let mut out = String::new();
    export_asar(
        &rom,
        &snap,
        &project,
        AsarOptions {
            range: Some((0x20, 8)),
            ..AsarOptions::default()
        },
        &mut out,
    );
    assert!(
        out.contains("dw RESET_008000,NMI_00800E,RESET_008000,NMI_00800E"),
        "{out}"
    );
}
