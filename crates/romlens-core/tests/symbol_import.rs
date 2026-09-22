//! Symbol import: the round trip against our own exporter, the collision rule
//! and the fact that nothing is ever silently dropped.

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::fixtures;
use romlens_core::io::export_symbols;
use romlens_core::io::import::symbols::{self, SymbolFormat};
use romlens_core::io::{from_files, to_files};
use romlens_core::model::{
    Command, CommentKind, ImportRecord, LabelSource, Origin, Project, UndoStack,
};
use romlens_core::{RomImage, SnesAddress};

fn rom() -> RomImage {
    RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap()
}

fn a(bank: u8, off: u16) -> SnesAddress {
    SnesAddress::new(bank, off)
}

fn annotated(rom: &RomImage) -> Project {
    let mut p = Project::new(rom);
    for (address, name) in [
        (a(0x00, 0x8000), "Boot"),
        (a(0x00, 0x800E), "Idle"),
        (a(0x7E, 0x0A1C), "SamusPose"),
    ] {
        p.apply(
            rom,
            Command::SetLabel {
                address,
                name: Some(name.into()),
            },
        )
        .unwrap();
    }
    p.apply(
        rom,
        Command::SetComment {
            address: a(0x00, 0x8000),
            kind: CommentKind::Line,
            text: Some("disable IRQ".into()),
        },
    )
    .unwrap();
    p
}

/// What `io::symbol_export` writes, `io::import::symbols` reads back. This is
/// why WLA was the format to do first: the correctness test is free.
#[test]
fn our_own_export_round_trips() {
    let rom = rom();
    let project = annotated(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let mut text = String::new();
    export_symbols(&rom, &snap, &project, false, &mut text);

    assert_eq!(symbols::detect(&text), SymbolFormat::Wla);
    let file = symbols::read(&text, None).unwrap();
    assert!(file.rewritten.is_empty(), "our own names needed rewriting");
    assert!(file.skipped.is_empty(), "{:?}", file.skipped);
    for (address, label) in &project.labels {
        assert_eq!(
            file.labels.get(address),
            Some(&label.name),
            "{address} did not survive the round trip"
        );
    }
    assert_eq!(
        file.comments[&(a(0x00, 0x8000), CommentKind::Line)],
        "disable IRQ"
    );

    // Into an empty project, it reproduces the same annotations — as imports.
    let mut fresh = Project::new(&rom);
    let plan = symbols::plan(&rom, &fresh, &file);
    fresh
        .apply_batch(&rom, plan.commands, Origin::Import("export.sym".into()))
        .unwrap();
    assert_eq!(fresh.labels.len(), project.labels.len());
    for label in fresh.labels.values() {
        assert_eq!(label.source, LabelSource::Imported("export.sym".into()));
    }
}

/// A symbol file is somebody else's opinion, and it does not get to rename
/// what this person named.
#[test]
fn the_users_names_are_never_overwritten() {
    let rom = rom();
    let mut project = Project::new(&rom);
    project
        .apply(
            &rom,
            Command::SetLabel {
                address: a(0x00, 0x8000),
                name: Some("MyOwnBoot".into()),
            },
        )
        .unwrap();
    let file = symbols::read("00:8000 TheirBoot\n00:800E TheirIdle\n", None).unwrap();
    let plan = symbols::plan(&rom, &project, &file);
    assert_eq!(plan.kept_user, vec![a(0x00, 0x8000)]);
    assert_eq!(plan.labels_added, 1);
    project
        .apply_batch(&rom, plan.commands, Origin::Import("theirs.sym".into()))
        .unwrap();
    assert_eq!(project.labels[&a(0x00, 0x8000)].name, "MyOwnBoot");
    assert_eq!(
        project.labels[&a(0x00, 0x8000)].source,
        LabelSource::User,
        "a user label was quietly relabelled as imported"
    );
    assert_eq!(project.labels[&a(0x00, 0x800E)].name, "TheirIdle");

    // A second import from another source replaces the first's names and says
    // so, since the user never claimed them.
    let file = symbols::read("00:800E BetterIdle\n", None).unwrap();
    let plan = symbols::plan(&rom, &project, &file);
    assert_eq!(plan.replaced, 1);
    assert!(plan.kept_user.is_empty());
}

/// Twenty thousand labels are one undo entry, and undoing restores each label
/// to exactly the source it had.
#[test]
fn an_import_is_one_undoable_step() {
    let rom = rom();
    let mut project = Project::new(&rom);
    project
        .apply(
            &rom,
            Command::SetLabel {
                address: a(0x00, 0x800E),
                name: Some("Mine".into()),
            },
        )
        .unwrap();
    let before = project.clone();

    let mut text = String::from("[labels]\n");
    for i in 0..500u16 {
        text.push_str(&format!("00:{:04X} Sym{i}\n", 0x8100 + i));
    }
    let file = symbols::read(&text, None).unwrap();
    let plan = symbols::plan(&rom, &project, &file);
    let mut stack = UndoStack::default();
    stack.push(
        project
            .apply_batch(&rom, plan.commands, Origin::Import("big.sym".into()))
            .unwrap(),
    );
    assert_eq!(project.labels.len(), 501);
    assert_eq!(stack.entries().len(), 1, "one entry, not five hundred");
    assert_eq!(stack.undo_title(), Some("Import from big.sym"));

    assert!(stack.undo(&mut project, &rom).unwrap());
    assert_eq!(
        project, before,
        "undoing the import did not restore exactly"
    );
    assert_eq!(project.labels[&a(0x00, 0x800E)].source, LabelSource::User);
    assert!(stack.redo(&mut project, &rom).unwrap());
    assert_eq!(project.labels.len(), 501);
}

/// Names are rewritten and reported, never dropped, and two that rewrite alike
/// stay apart.
#[test]
fn nothing_is_silently_lost() {
    let text = "; Licence: 0BSD\n\n[labels]\n\
00:8000 Player::Draw\n00:8001 Player.Draw\n00:8002 9lives\n\
00:8003 \n\
not a symbol\n";
    let file = symbols::read(text, None).unwrap();
    assert_eq!(file.notice, "Licence: 0BSD");
    assert_eq!(file.rewritten.len(), 3);
    let names: Vec<&str> = file.labels.values().map(String::as_str).collect();
    assert_eq!(names, vec!["Player_Draw", "Player_Draw_2", "_9lives"]);
    assert_eq!(file.skipped.len(), 2, "{:?}", file.skipped);
}

/// The licence notice travels with what it covers, through the package.
#[test]
fn the_notice_survives_a_save() {
    let rom = rom();
    let mut project = Project::new(&rom);
    project.add_import(ImportRecord {
        source: "pjboy.sym".into(),
        format: "wla".into(),
        labels: 3,
        comments: 0,
        notice: "Bank log by PJBoy, used with permission".into(),
    });
    let files = to_files(&rom, &project);
    let back = from_files(&rom, &files).unwrap();
    assert_eq!(back.imports, project.imports);
    assert!(
        String::from_utf8_lossy(&files["project.json"]).contains("PJBoy"),
        "the notice did not reach project.json"
    );
}
