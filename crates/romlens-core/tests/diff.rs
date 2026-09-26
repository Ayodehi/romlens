//! Two versions of a ROM compared (docs/22, D1), on the fixture pair: a
//! patched routine and a block inserted in a table.

use romlens_core::RomImage;
use romlens_core::analysis::snapshot::AnalysisSnapshot;
use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::diff::{LineOp, Pairing, RunKind, Side, compare};
use romlens_core::fixtures;
use romlens_core::model::project::Project;

fn open(bytes: Vec<u8>) -> (RomImage, Project, AnalysisSnapshot) {
    let rom = RomImage::from_bytes(bytes, "d.sfc").unwrap();
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    (rom, project, snap)
}

#[test]
fn a_patch_and_an_insertion() {
    let (a, b) = fixtures::diff_pair();
    let (ra, pa, sa) = open(a);
    let (rb, pb, sb) = open(b);
    let c = compare(
        Side {
            rom: &ra,
            project: &pa,
            snap: &sa,
        },
        Side {
            rom: &rb,
            project: &pb,
            snap: &sb,
        },
    );
    let changes: Vec<_> = c
        .bytes
        .runs
        .iter()
        .filter(|r| r.kind != RunKind::Same)
        .map(|r| (r.kind, r.a.clone(), r.b.clone()))
        .collect();
    assert_eq!(
        changes,
        vec![
            (RunKind::Changed, 0x21..0x22, 0x21..0x22),
            (RunKind::Inserted, 0x1200..0x1200, 0x1200..0x1240),
            // Filler is all one byte, so which 64 of it went is anyone's
            // guess: the alignment says the last, before the header.
            (RunKind::Deleted, 0x7F80..0x7FC0, 0x7FC0..0x7FC0),
        ]
    );
    // The table after the insertion is the same bytes, 64 on.
    assert_eq!(c.bytes.map(0x1300), Some(0x1340));

    let changed: Vec<_> = c
        .routines
        .iter()
        .filter(|r| r.pairing != Pairing::Same)
        .collect();
    assert_eq!(changed.len(), 1, "{changed:#?}");
    let r = changed[0];
    assert_eq!(r.pairing, Pairing::Changed);
    assert_eq!(r.a.as_ref().unwrap().name, "SUB_008020");
    let diff: Vec<_> = r.lines.iter().filter(|l| l.op != LineOp::Same).collect();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].a.as_ref().unwrap().1, "LDX #$0F");
    assert_eq!(diff[0].b.as_ref().unwrap().1, "LDX #$1F");
    assert!(
        c.routines
            .iter()
            .filter(|r| r.pairing == Pairing::Same)
            .count()
            >= 4
    );
}
