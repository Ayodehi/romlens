//! The student's and the tutor's words in the C (docs/24, U4): local names,
//! a routine's note, C comments and C versions; their commands undo, the
//! package keeps them, and the C they shape still compiles.

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::decompile::{self, CTokenKind, DecompileOptions};
use romlens_core::fixtures;
use romlens_core::io::{from_files, to_files};
use romlens_core::model::c_notes::{Anchor, Author, CVersion};
use romlens_core::model::{Command, Origin, Project};
use romlens_core::{RomImage, SnesAddress};

fn at(a: u32) -> SnesAddress {
    SnesAddress::new((a >> 16) as u8, a as u16)
}

fn edits() -> Vec<Command> {
    vec![
        Command::SetLocalName {
            routine: at(0x00_8000),
            local: "x_out".into(),
            name: Some("slots_left".into()),
        },
        Command::SetLocalName {
            routine: at(0x00_8000),
            local: "c".into(),
            name: Some("carry".into()),
        },
        Command::SetRoutineNote {
            routine: at(0x00_8000),
            text: Some("Starts the game: native mode, the screen off, one DMA.".into()),
        },
        Command::SetCComment {
            address: at(0x00_8009),
            text: Some("Screen off while VRAM is loaded".into()),
        },
    ]
}

#[test]
fn names_notes_and_comments_shape_the_c() {
    let rom = RomImage::from_bytes(fixtures::explain_lorom(), "e.sfc").unwrap();
    let mut project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let plain = decompile::decompile(
        &rom,
        &project,
        &snap,
        at(0x00_8000),
        &DecompileOptions::default(),
    )
    .unwrap();
    assert!(plain.text.contains("u16 *x_out"), "{}", plain.text);

    let undo = project
        .apply_batch(
            &rom,
            edits(),
            Origin::Tutor {
                conversation: "c1".into(),
                turn: 3,
            },
        )
        .unwrap();
    assert_eq!(undo.title, "Tutor: 4 Changes");
    let d = decompile::decompile(
        &rom,
        &project,
        &snap,
        at(0x00_8000),
        &DecompileOptions::default(),
    )
    .unwrap();
    let t = &d.text;
    assert!(t.contains("u16 *slots_left"), "{t}");
    assert!(!t.contains("x_out"), "{t}");
    assert!(t.contains("carry = 0;"), "{t}");
    // Not inside words, nor in comments.
    assert!(t.contains("bcd_add"), "{t}");
    assert!(
        t.contains(
            "/* Starts the game: native mode, the screen off, one DMA. */\n/* Reads nothing"
        ),
        "{t}"
    );
    let c = t
        .find("    /* Screen off while VRAM is loaded */\n    INIDISP = 0x8F;")
        .expect(t);
    assert!(c > 0);

    // The tokens still fall on their words, and the lines on their code.
    for k in &d.tokens {
        let w = &t[k.start as usize..(k.start + k.len) as usize];
        assert!(!w.is_empty() && !w.starts_with(' '), "{k:?}");
        if k.kind == CTokenKind::Function && k.address == Some(d.entry) {
            assert_eq!(w, d.name);
        }
    }
    assert_eq!(d.lines.len(), t.lines().count());
    let comment_line = t
        .lines()
        .position(|l| l.contains("Screen off while"))
        .unwrap();
    let inidisp = t
        .lines()
        .position(|l| l.contains("INIDISP = 0x8F"))
        .unwrap();
    // The comment selects its instruction, part of the statement below.
    assert_eq!(d.lines[comment_line].len(), 1);
    assert!(d.lines[inidisp].contains(&d.lines[comment_line][0]));

    // Undone, the C is as it was.
    for cmd in undo.inverse {
        project.apply(&rom, cmd).unwrap();
    }
    let back = decompile::decompile(
        &rom,
        &project,
        &snap,
        at(0x00_8000),
        &DecompileOptions::default(),
    )
    .unwrap();
    assert_eq!(back.text, plain.text);
    assert_eq!(back.tokens, plain.tokens);
    assert_eq!(back.lines, plain.lines);

    // Still C, where a compiler is at hand.
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let dir = std::env::temp_dir().join(format!("romlens-c-notes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("snes.h"), decompile::snes_h()).unwrap();
    std::fs::write(dir.join("r.c"), &d.text).unwrap();
    if let Ok(o) = std::process::Command::new(&cc)
        .args([
            "-std=c11",
            "-fsyntax-only",
            "-Wall",
            "-Wno-unused-parameter",
            "-I",
        ])
        .arg(&dir)
        .arg(dir.join("r.c"))
        .output()
    {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_names_are_refused_and_nothing_changes() {
    let rom = RomImage::from_bytes(fixtures::explain_lorom(), "e.sfc").unwrap();
    let mut project = Project::new(&rom);
    let e = project
        .apply_batch(
            &rom,
            vec![
                Command::SetCComment {
                    address: at(0x00_8009),
                    text: Some("fine".into()),
                },
                Command::SetLocalName {
                    routine: at(0x00_8000),
                    local: "a".into(),
                    name: Some("while".into()),
                },
            ],
            Origin::User,
        )
        .unwrap_err();
    assert!(e.to_string().contains("keyword"), "{e}");
    assert!(project.c_comments.is_empty());
    assert!(
        project
            .apply(
                &rom,
                Command::SetCComment {
                    address: at(0x00_8009),
                    text: Some("a */ b".into())
                }
            )
            .is_err()
    );
}

#[test]
fn the_package_keeps_them() {
    let rom = RomImage::from_bytes(fixtures::explain_lorom(), "e.sfc").unwrap();
    let mut project = Project::new(&rom);
    project.apply_batch(&rom, edits(), Origin::User).unwrap();
    let v = CVersion {
        text: "void Reset(void)\n{\n    screen_off();\n}\n".into(),
        author: Author::Tutor,
        anchors: vec![Anchor {
            first: 3,
            last: 3,
            start: at(0x00_8007),
            end: at(0x00_800B),
        }],
    };
    project
        .apply(
            &rom,
            Command::SetCVersion {
                routine: at(0x00_8000),
                name: "Plain words".into(),
                version: Some(v.clone()),
            },
        )
        .unwrap();
    let files = to_files(&rom, &project);
    assert!(files.contains_key("c_notes.json"));
    assert!(files.contains_key("c_versions.json"));
    let back = from_files(&rom, &files).unwrap();
    assert_eq!(back.local_names, project.local_names);
    assert_eq!(back.routine_notes, project.routine_notes);
    assert_eq!(back.c_comments, project.c_comments);
    assert_eq!(
        back.c_versions.get(&(at(0x00_8000), "Plain words".into())),
        Some(&v)
    );
    // A project without them keeps the files it always had.
    let files = to_files(&rom, &Project::new(&rom));
    assert!(!files.contains_key("c_notes.json") && !files.contains_key("c_versions.json"));
}
