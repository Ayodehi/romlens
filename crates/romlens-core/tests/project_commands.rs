//! Every command's inverse restores equality; mirrors canonicalise; labels
//! validate; region marks split and merge.

use romlens_core::fixtures;
use romlens_core::model::{
    Command, CommentKind, DataKind, FlagOverride, Origin, OverrideKind, Project, RegionOverride,
    RegionParams, TableElem, UndoStack,
};
use romlens_core::{FileOffset, ProjectError, RomImage, SnesAddress};

fn rom() -> RomImage {
    RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap()
}

fn a(bank: u8, off: u16) -> SnesAddress {
    SnesAddress::new(bank, off)
}

fn mark(start: u32, len: u32, kind: OverrideKind) -> Command {
    Command::MarkRegion {
        start: FileOffset(start),
        len,
        kind,
    }
}

fn check_inverse(rom: &RomImage, project: &mut Project, cmd: Command) {
    let before = project.clone();
    let entry = project.apply(rom, cmd.clone()).unwrap();
    assert_eq!(entry.done, vec![cmd.clone()]);
    for inv in &entry.inverse {
        project.apply(rom, inv.clone()).unwrap();
    }
    assert_eq!(
        *project, before,
        "inverse of {cmd:?} did not restore the project"
    );
    // Re-apply so later steps build on it.
    project.apply(rom, cmd).unwrap();
}

#[test]
fn labels_and_comments() {
    let rom = rom();
    let mut p = Project::new(&rom);
    check_inverse(
        &rom,
        &mut p,
        Command::SetLabel {
            address: a(0x80, 0x8000),
            name: Some("Boot".into()),
        },
    );
    // Mirrors collapse onto the canonical (slow, since the fixture is SlowROM) address.
    assert_eq!(p.labels.len(), 1);
    assert_eq!(p.label_at(a(0x00, 0x8000)).unwrap().name, "Boot");
    check_inverse(
        &rom,
        &mut p,
        Command::SetLabel {
            address: a(0x00, 0x8000),
            name: Some("Reset".into()),
        },
    );
    assert_eq!(p.labels.len(), 1);
    assert_eq!(p.label_at(a(0x00, 0x8000)).unwrap().name, "Reset");
    check_inverse(
        &rom,
        &mut p,
        Command::SetLabel {
            address: a(0x00, 0x8000),
            name: None,
        },
    );
    assert!(p.labels.is_empty());
    // RAM labels keep their address as written.
    check_inverse(
        &rom,
        &mut p,
        Command::SetLabel {
            address: a(0x7E, 0x0A1C),
            name: Some("SamusPose".into()),
        },
    );
    assert_eq!(p.label_at(a(0x7E, 0x0A1C)).unwrap().name, "SamusPose");
    // Validation.
    assert!(matches!(
        p.apply(
            &rom,
            Command::SetLabel {
                address: a(0x00, 0x8000),
                name: Some("bad name".into())
            }
        ),
        Err(ProjectError::InvalidLabelName(_))
    ));
    assert!(matches!(
        p.apply(
            &rom,
            Command::SetLabel {
                address: a(0x00, 0x8000),
                name: Some("SUB_00800E".into())
            }
        ),
        Err(ProjectError::InvalidLabelName(_))
    ));
    assert!(
        p.apply(
            &rom,
            Command::SetLabel {
                address: a(0x00, 0x8000),
                name: Some("SUB_008000".into())
            }
        )
        .is_ok()
    );
    // Comments.
    check_inverse(
        &rom,
        &mut p,
        Command::SetComment {
            address: a(0x80, 0x8000),
            kind: CommentKind::Line,
            text: Some("disable IRQ".into()),
        },
    );
    check_inverse(
        &rom,
        &mut p,
        Command::SetComment {
            address: a(0x00, 0x8000),
            kind: CommentKind::Block,
            text: Some("Boot\nsequence".into()),
        },
    );
    assert_eq!(p.comments.len(), 2);
    assert_eq!(
        p.comment_at(a(0x00, 0x8000), CommentKind::Line)
            .unwrap()
            .text,
        "disable IRQ"
    );
    check_inverse(
        &rom,
        &mut p,
        Command::SetComment {
            address: a(0x00, 0x8000),
            kind: CommentKind::Line,
            text: Some("   ".into()),
        },
    );
    assert_eq!(p.comments.len(), 1, "blank text removes");
}

#[test]
fn region_marks_split_and_merge() {
    let rom = rom();
    let mut p = Project::new(&rom);
    check_inverse(&rom, &mut p, mark(0x100, 0x100, OverrideKind::Code));
    check_inverse(
        &rom,
        &mut p,
        mark(0x300, 0x100, OverrideKind::Data(DataKind::Word)),
    );
    // A mark spanning both existing ones and the gap replaces them.
    check_inverse(
        &rom,
        &mut p,
        mark(
            0x180,
            0x200,
            OverrideKind::Data(DataKind::Table {
                stride: 4,
                elem: TableElem::Raw,
            }),
        ),
    );
    assert_eq!(
        p.region_overrides,
        vec![
            RegionOverride {
                start: FileOffset(0x100),
                len: 0x80,
                kind: OverrideKind::Code,
                params: RegionParams::default(),
            },
            RegionOverride {
                start: FileOffset(0x180),
                len: 0x200,
                kind: OverrideKind::Data(DataKind::Table {
                    stride: 4,
                    elem: TableElem::Raw,
                }),
                params: RegionParams::default(),
            },
            RegionOverride {
                start: FileOffset(0x380),
                len: 0x80,
                kind: OverrideKind::Data(DataKind::Word),
                params: RegionParams::default(),
            },
        ]
    );
    // A mark inside one override splits it.
    check_inverse(&rom, &mut p, mark(0x200, 0x10, OverrideKind::Unknown));
    assert_eq!(p.region_overrides.len(), 5);
    assert_eq!(p.override_kind_at(0x205), Some(OverrideKind::Unknown));
    assert_eq!(
        p.override_kind_at(0x210),
        Some(OverrideKind::Data(DataKind::Table {
            stride: 4,
            elem: TableElem::Raw,
        }))
    );
    // Clearing part of the list.
    check_inverse(
        &rom,
        &mut p,
        Command::ClearRegionOverride {
            start: FileOffset(0x000),
            len: 0x210,
        },
    );
    assert_eq!(p.override_kind_at(0x100), None);
    assert_eq!(
        p.override_kind_at(0x210),
        Some(OverrideKind::Data(DataKind::Table {
            stride: 4,
            elem: TableElem::Raw,
        }))
    );
    // Ranges are checked.
    assert!(matches!(
        p.apply(&rom, mark(0x7FFF, 2, OverrideKind::Code)),
        Err(ProjectError::BadRange(_))
    ));
    assert!(matches!(
        p.apply(&rom, mark(0, 0, OverrideKind::Code)),
        Err(ProjectError::BadRange(_))
    ));
}

#[test]
fn flag_overrides() {
    let rom = rom();
    let mut p = Project::new(&rom);
    let f = FlagOverride {
        m: Some(true),
        dbr: Some(0x80),
        ..Default::default()
    };
    check_inverse(
        &rom,
        &mut p,
        Command::SetFlagOverride {
            offset: FileOffset(0x41C),
            flags: Some(f),
        },
    );
    assert_eq!(p.flag_overrides[&FileOffset(0x41C)], f);
    check_inverse(
        &rom,
        &mut p,
        Command::SetFlagOverride {
            offset: FileOffset(0x41C),
            flags: Some(FlagOverride::default()),
        },
    );
    assert!(p.flag_overrides.is_empty(), "an empty override removes");
    assert!(
        p.apply(
            &rom,
            Command::SetFlagOverride {
                offset: FileOffset(0x8000),
                flags: Some(f)
            }
        )
        .is_err()
    );
}

#[test]
fn undo_stack_round_trips() {
    let rom = rom();
    let mut p = Project::new(&rom);
    let mut stack = UndoStack::default();
    let start = p.clone();
    let cmds = [
        Command::SetLabel {
            address: a(0x00, 0x8000),
            name: Some("Boot".into()),
        },
        Command::MarkRegion {
            start: FileOffset(0x100),
            len: 0x100,
            kind: OverrideKind::Code,
        },
        Command::MarkRegion {
            start: FileOffset(0x180),
            len: 0x10,
            kind: OverrideKind::Unknown,
        },
        Command::SetComment {
            address: a(0x00, 0x8007),
            kind: CommentKind::Line,
            text: Some("force blank".into()),
        },
    ];
    let mut states = vec![start.clone()];
    for c in cmds {
        let entry = p.apply(&rom, c).unwrap();
        stack.push(entry);
        states.push(p.clone());
    }
    assert_eq!(stack.undo_title(), Some("Set Comment"));
    assert!(!stack.can_redo());
    for i in (0..4).rev() {
        assert!(stack.undo(&mut p, &rom).unwrap());
        assert_eq!(p, states[i], "after undo to state {i}");
    }
    assert!(!stack.undo(&mut p, &rom).unwrap());
    assert!(!stack.can_undo());
    assert_eq!(stack.redo_title(), Some("Rename Label"));
    for (i, state) in states.iter().enumerate().skip(1) {
        assert!(stack.redo(&mut p, &rom).unwrap());
        assert_eq!(p, *state, "after redo to state {i}");
    }
    assert!(!stack.redo(&mut p, &rom).unwrap());
    // A new command after an undo drops the redo branch.
    stack.undo(&mut p, &rom).unwrap();
    stack.push(
        p.apply(
            &rom,
            Command::SetLabel {
                address: a(0x00, 0x800E),
                name: Some("Idle".into()),
            },
        )
        .unwrap(),
    );
    assert!(!stack.can_redo());
    assert!(
        !Command::SetLabel {
            address: a(0, 0),
            name: None
        }
        .affects_analysis()
    );
    assert!(
        Command::ClearRegionOverride {
            start: FileOffset(0),
            len: 1
        }
        .affects_analysis()
    );
}

/// A batch is one undoable step: one title, one inverse, and an inverse that
/// unwinds in the reverse of the order the commands were applied.
#[test]
fn batches_are_one_undo_step() {
    let rom = rom();
    let mut p = Project::new(&rom);
    let before = p.clone();
    let commands = vec![
        Command::SetLabel {
            address: a(0x00, 0x8000),
            name: Some("Boot".into()),
        },
        Command::SetLabel {
            address: a(0x00, 0x800E),
            name: Some("Idle".into()),
        },
        mark(0x20, 8, OverrideKind::Data(DataKind::Word)),
        // Overlaps the mark above, so unwinding in the wrong order would leave
        // the first mark's split halves behind.
        mark(0x22, 2, OverrideKind::Code),
    ];
    let entry = p
        .apply_batch(&rom, commands.clone(), Origin::Import("boot.sym".into()))
        .unwrap();
    assert_eq!(entry.done, commands);
    assert_eq!(entry.title, "Import from boot.sym");
    assert_eq!(entry.origin, Origin::Import("boot.sym".into()));
    assert!(entry.affects_analysis());
    assert_eq!(p.labels.len(), 2);

    for inv in &entry.inverse {
        p.apply(&rom, inv.clone()).unwrap();
    }
    assert_eq!(p, before, "the batch's inverse did not restore the project");
}

/// All or nothing: one refused command leaves no trace of the ones before it.
#[test]
fn a_refused_command_rolls_the_whole_batch_back() {
    let rom = rom();
    let mut p = Project::new(&rom);
    p.apply(
        &rom,
        Command::SetLabel {
            address: a(0x00, 0x8000),
            name: Some("Existing".into()),
        },
    )
    .unwrap();
    let before = p.clone();

    let err = p
        .apply_batch(
            &rom,
            vec![
                Command::SetLabel {
                    address: a(0x00, 0x8000),
                    name: Some("Renamed".into()),
                },
                mark(0x20, 4, OverrideKind::Data(DataKind::Word)),
                // Past the end of the fixture.
                mark(0x7FFF, 2, OverrideKind::Code),
            ],
            Origin::Import("bad.sym".into()),
        )
        .unwrap_err();
    assert!(matches!(err, ProjectError::BadRange(_)));
    assert_eq!(p, before, "a failed batch left the project half-edited");
}

/// Undo and redo treat a batch as a unit, and redo replays it under the same
/// origin so the Edit menu keeps its wording.
#[test]
fn undo_and_redo_a_batch() {
    let rom = rom();
    let mut p = Project::new(&rom);
    let empty = p.clone();
    let mut stack = UndoStack::default();
    stack.push(
        p.apply_batch(
            &rom,
            vec![
                mark(0x20, 4, OverrideKind::Data(DataKind::Word)),
                mark(0x30, 4, OverrideKind::Code),
            ],
            Origin::User,
        )
        .unwrap(),
    );
    assert_eq!(stack.undo_title(), Some("2 Changes"));
    assert!(stack.undo(&mut p, &rom).unwrap());
    assert_eq!(p, empty);
    assert_eq!(stack.redo_title(), Some("2 Changes"));
    assert!(stack.redo(&mut p, &rom).unwrap());
    assert_eq!(p.region_overrides.len(), 2);
    assert_eq!(stack.undo_title(), Some("2 Changes"));
}

/// A one-command batch keeps the Phase 1 wording, so nothing in a shell's
/// Edit menu reads differently now that every edit goes through a batch.
#[test]
fn origin_titles() {
    let one = vec![Command::ClearRegionOverride {
        start: FileOffset(0),
        len: 1,
    }];
    assert_eq!(Origin::User.title(&one), "Clear Mark");
    assert_eq!(Origin::User.title(&[]), "0 Changes");
    assert_eq!(
        Origin::Import("m.cdl".into()).title(&one),
        "Import from m.cdl"
    );
    assert_eq!(
        Origin::Accepted("the tutor".into()).title(&one),
        "Accept the tutor"
    );
}

#[test]
fn preview_options_live_on_the_override_and_survive_undo() {
    use romlens_core::graphics::tilemap::ScreenSize;
    let rom = rom();
    let mut p = Project::new(&rom);
    let gfx = OverrideKind::Data(DataKind::Graphics { bpp: 4 });
    check_inverse(&rom, &mut p, mark(0x1000, 0x400, gfx));
    let params = RegionParams {
        palette: Some(a(0x00, 0x9820)),
        columns: Some(8),
        screen_size: Some(ScreenSize::S64x32),
        tiles: None,
    };
    let set = Command::SetRegionParams {
        start: FileOffset(0x1000),
        params,
    };
    check_inverse(&rom, &mut p, set);
    assert_eq!(p.region_overrides[0].params, params);
    // Setting options for bytes nobody marked is refused, not lost.
    assert!(matches!(
        p.apply(
            &rom,
            Command::SetRegionParams {
                start: FileOffset(0x1001),
                params,
            }
        ),
        Err(ProjectError::NotMarked(_))
    ));
    // A mark that cuts the range keeps the options on both halves, and its
    // undo puts them back exactly.
    check_inverse(&rom, &mut p, mark(0x1100, 0x20, OverrideKind::Code));
    assert_eq!(p.region_overrides[0].params, params);
    assert_eq!(p.region_overrides[2].params, params);
    // Overwriting the whole range drops them, and undo restores them.
    check_inverse(&rom, &mut p, mark(0x1000, 0x400, gfx));
    assert!(p.region_overrides[0].params.is_default());
    assert!(
        !Command::SetRegionParams {
            start: FileOffset(0),
            params
        }
        .affects_analysis()
    );
    // They round-trip through the package.
    let mut p = Project::new(&rom);
    p.apply(&rom, mark(0x1000, 0x400, gfx)).unwrap();
    p.apply(
        &rom,
        Command::SetRegionParams {
            start: FileOffset(0x1000),
            params,
        },
    )
    .unwrap();
    let files = romlens_core::io::to_files(&rom, &p);
    let back = romlens_core::io::from_files(&rom, &files).unwrap();
    assert_eq!(back.region_overrides, p.region_overrides);
    let json = String::from_utf8(files["regions.json"].clone()).unwrap();
    assert!(json.contains("\"screenSize\": \"64x32\""), "{json}");
}
