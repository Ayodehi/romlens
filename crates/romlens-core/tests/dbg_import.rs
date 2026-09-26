//! ca65 debug information (`io::import::dbg`) on the committed fixture,
//! `tests/data/ca65`, which `build.sh` there assembles with cc65.

use std::path::Path;

use romlens_core::io::import::dbg;
use romlens_core::io::import::symbols;
use romlens_core::model::source_map::LineKind;
use romlens_core::model::{Origin, Project};
use romlens_core::{FileOffset, RomImage, SnesAddress};

fn fixture() -> (RomImage, String) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/ca65");
    let rom = RomImage::load(dir.join("fixture.sfc")).unwrap();
    let text = std::fs::read_to_string(dir.join("fixture.dbg")).unwrap();
    (rom, text)
}

fn read() -> (RomImage, dbg::DbgFile) {
    let (rom, text) = fixture();
    let file = dbg::read(&text, &rom, "fixture.dbg", "tests/data/ca65").unwrap();
    (rom, file)
}

#[test]
fn symbols_become_labels_with_their_scopes() {
    let (rom, file) = read();
    let at = |a: &str| {
        let r = rom.resolve(a).unwrap();
        rom.snes_address_for(r.file_offset).unwrap()
    };
    let labels = &file.symbols.labels;
    assert_eq!(labels[&at("$00:8000")], "Reset");
    assert_eq!(
        labels[&at("$00:801B")],
        "Reset_wait",
        "a cheap local by its routine"
    );
    assert_eq!(labels[&at("$00:8020")], "ClearBuffer");
    assert_eq!(labels[&at("$00:8023")], "ClearBuffer_loop");
    assert_eq!(labels[&at("$00:802A")], "Nmi");
    assert_eq!(
        labels[&at("$00:802B")],
        "ClearPalette",
        "from the second module"
    );
    assert_eq!(labels[&at("$00:803E")], "Palette", "in RODATA, after CODE");
    // RAM, where the program uses it.
    assert_eq!(labels[&SnesAddress::new(0x00, 0x0200)], "buffer");
    assert_eq!(labels[&SnesAddress::new(0x00, 0x0000)], "frame");
    // The equate and the import are not labels.
    assert_eq!(file.not_labels, 2);
    assert_eq!(labels.len(), 10);
    assert_eq!(file.output, "fixture.sfc");
    assert_eq!(file.spans_unplaced, 0);
    assert!(
        file.symbols.skipped.is_empty(),
        "{:?}",
        file.symbols.skipped
    );
}

#[test]
fn lines_map_to_the_bytes_they_made() {
    let (_, file) = read();
    let map = &file.map;
    let names: Vec<&str> = map.files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["main.s", "macros.inc", "palette.s"]);
    let main = 0;
    let macros = 1;
    let palette = 2;
    // `txs`, one byte.
    let txs = map.line(main, 23).unwrap();
    assert_eq!(txs.ranges, vec![(FileOffset(10), 1)]);
    // `brightness $80` is the whole expansion; the macro's `lda #level`
    // is two bytes of each expansion.
    let invoke = map.line(main, 24).unwrap();
    assert_eq!(invoke.ranges, vec![(FileOffset(11), 5)]);
    let lda = map.line(macros, 5).unwrap();
    assert_eq!(lda.kind, LineKind::Macro);
    assert_eq!(lda.ranges, vec![(FileOffset(11), 2), (FileOffset(22), 2)]);
    // A byte of an expansion: the program's line first, then the macro's.
    let at: Vec<(u32, u32)> = map
        .lines_at(FileOffset(12))
        .iter()
        .map(|l| (l.file, l.line))
        .collect();
    assert_eq!(at, vec![(main, 24), (macros, 5)]);
    // The second module, and the data.
    assert_eq!(
        map.line(palette, 15).unwrap().ranges,
        vec![(FileOffset(0x31), 3)]
    );
    assert_eq!(
        map.line(palette, 25).unwrap().ranges,
        vec![(FileOffset(0x3E), 8)]
    );
    // The header, 64 bytes at $FFC0.
    assert_eq!(
        map.line(main, 47).unwrap().ranges,
        vec![(FileOffset(0x7FC0), 21)]
    );
    // Lines that made nothing (`.proc`, blank, comments) are not kept.
    assert!(map.line(main, 15).is_none());
    // CODE, RODATA and the header.
    assert_eq!(map.bytes_covered(), 0x3E + 8 + 64);
}

#[test]
fn the_import_names_what_it_found() {
    let (rom, file) = read();
    let mut project = Project::new(&rom);
    let plan = symbols::plan(&rom, &project, &file.symbols);
    assert_eq!(plan.labels_added, 10);
    project
        .apply_batch(&rom, plan.commands, Origin::Import("fixture.dbg".into()))
        .unwrap();
    // RAM labels land where every other RAM label does.
    let buffer = project.labels.get(&SnesAddress::new(0x7E, 0x0200)).unwrap();
    assert_eq!(buffer.name, "buffer");
}

#[test]
fn a_dbg_from_another_output_places_nothing() {
    let (rom, text) = fixture();
    let other = text.replace("oname=\"fixture.sfc\"", "oname=\"other.sfc\"");
    // Another output name alone still places: the only output is used.
    assert!(dbg::read(&other, &rom, "x.dbg", ".").is_ok());
    let none = text
        .lines()
        .filter(|l| !l.contains("ooffs="))
        .filter(|l| !l.starts_with("sym"))
        .collect::<Vec<_>>()
        .join("\n");
    let err = dbg::read(&none, &rom, "x.dbg", ".").unwrap_err();
    assert!(format!("{err}").contains("places nothing"), "{err}");
    assert!(dbg::read("version\tmajor=3,minor=0\n", &rom, "x.dbg", ".").is_err());
    assert!(dbg::read("hello\n", &rom, "x.dbg", ".").is_err());
}

#[test]
fn a_project_keeps_its_source_map() {
    let (rom, file) = read();
    let mut project = Project::new(&rom);
    project.add_source_map(file.map.clone());
    let files = romlens_core::io::to_files(&rom, &project);
    assert!(files.contains_key("imports/sources.json"));
    let back = romlens_core::io::from_files(&rom, &files).unwrap();
    assert_eq!(back.source_maps, vec![file.map.clone()]);
    // Importing the same file again replaces it.
    project.add_source_map(file.map);
    assert_eq!(project.source_maps.len(), 1);
}
