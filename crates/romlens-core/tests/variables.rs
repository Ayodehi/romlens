//! Variables: a named, typed address, and how the listing names what uses it.

use std::collections::BTreeMap;

use romlens_core::cpu65816::format::SymbolLookup;
use romlens_core::fixtures;
use romlens_core::io::{VARIABLES_FILE, from_files, to_files};
use romlens_core::model::symbols::Symbols;
use romlens_core::model::{Command, Origin, Project, UndoStack, VarType, VarWidth};
use romlens_core::{RomImage, SnesAddress};

fn rom() -> RomImage {
    RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap()
}

fn a(v: u32) -> SnesAddress {
    SnesAddress::from_u24(v)
}

fn define(p: &mut Project, rom: &RomImage, at: u32, name: &str, ty: VarType) -> String {
    let entry = p
        .apply_batch(
            rom,
            vec![
                Command::SetLabel {
                    address: a(at),
                    name: Some(name.into()),
                },
                Command::SetVariable {
                    address: a(at),
                    ty: Some(ty),
                },
            ],
            Origin::User,
        )
        .unwrap();
    entry.title
}

fn name(p: &Project, rom: &RomImage, at: u32) -> Option<String> {
    let auto = BTreeMap::new();
    Symbols::new(rom, p, &auto).name_for(a(at)).map(|s| s.name)
}

#[test]
fn a_variable_names_every_byte_it_spans() {
    let rom = rom();
    let mut p = Project::new(&rom);
    let title = define(
        &mut p,
        &rom,
        0x7F_8000,
        "Buffer",
        VarType::scalar(VarWidth::Word),
    );
    assert_eq!(
        title, "Define Variable",
        "one undo step, named for what it is"
    );
    assert_eq!(name(&p, &rom, 0x7F_8000).as_deref(), Some("Buffer"));
    assert_eq!(name(&p, &rom, 0x7F_8001).as_deref(), Some("Buffer+1"));
    assert_eq!(name(&p, &rom, 0x7F_8002), None, "a word is two bytes");

    define(
        &mut p,
        &rom,
        0x7E_0100,
        "Table",
        VarType {
            width: VarWidth::Long,
            count: 4,
        },
    );
    assert_eq!(name(&p, &rom, 0x7E_010B).as_deref(), Some("Table+11"));
    assert_eq!(name(&p, &rom, 0x7E_010C), None);
}

#[test]
fn low_ram_mirrors_name_the_same_variable() {
    let rom = rom();
    let mut p = Project::new(&rom);
    define(
        &mut p,
        &rom,
        0x00_0094,
        "PlayerX",
        VarType::scalar(VarWidth::Word),
    );
    // Stored at WRAM's own address, whichever mirror named it.
    assert!(p.variables.contains_key(&a(0x7E_0094)));
    assert!(p.labels.contains_key(&a(0x7E_0094)));
    // `STA $0094` from bank $80 and `STA $7E0095` both find it.
    assert_eq!(name(&p, &rom, 0x80_0094).as_deref(), Some("PlayerX"));
    assert_eq!(name(&p, &rom, 0x7E_0095).as_deref(), Some("PlayerX+1"));
    // Above the mirror, bank $80 is not WRAM.
    assert_eq!(
        Project::canonical(&rom, a(0x80_2100)),
        a(0x00_2100),
        "a register"
    );
    assert_eq!(Project::canonical(&rom, a(0x7E_2000)), a(0x7E_2000));
}

#[test]
fn undo_removes_it_and_a_project_keeps_it() {
    let rom = rom();
    let mut p = Project::new(&rom);
    let mut undo = UndoStack::default();
    let entry = p
        .apply_batch(
            &rom,
            vec![
                Command::SetLabel {
                    address: a(0x7E_1000),
                    name: Some("Timer".into()),
                },
                Command::SetVariable {
                    address: a(0x7E_1000),
                    ty: Some(VarType::scalar(VarWidth::Byte)),
                },
            ],
            Origin::User,
        )
        .unwrap();
    undo.push(entry);

    let files = to_files(&rom, &p);
    assert!(files.contains_key(VARIABLES_FILE));
    let back = from_files(&rom, &files).unwrap();
    assert_eq!(back.variables, p.variables);
    assert_eq!(back.labels, p.labels);

    assert!(undo.undo(&mut p, &rom).unwrap());
    assert!(p.variables.is_empty() && p.labels.is_empty());
    assert!(
        !to_files(&rom, &p).contains_key(VARIABLES_FILE),
        "a project with no variables keeps its usual files"
    );
}

#[test]
fn a_variable_must_fit_its_bank() {
    let rom = rom();
    let mut p = Project::new(&rom);
    let err = p
        .apply(
            &rom,
            Command::SetVariable {
                address: a(0x7F_FFFF),
                ty: Some(VarType::scalar(VarWidth::Word)),
            },
        )
        .unwrap_err();
    assert!(
        format!("{err}").contains("past the end of the bank"),
        "{err}"
    );
}
