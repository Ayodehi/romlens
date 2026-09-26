//! `ChangeIndex` against brute force (`16-phase2-plan.md` 2C: "pin that
//! with a brute-force equivalence test"): every answer equals a scan of
//! every frame's change runs, and never misses a byte that really changed.

use std::io::Cursor;

use romlens_core::recording::change_index::ChangeIndex;
use romlens_core::recording::delta::Run;
use romlens_core::recording::writer::WriterOptions;
use romlens_core::recording::{
    MachineStateSource, RomrecSource, RomrecWriter, StateRegion, fixtures,
};

fn recording(options: WriterOptions) -> RomrecSource {
    let frames = fixtures::frames(70);
    let mut w = RomrecWriter::new(
        Cursor::new(Vec::new()),
        &fixtures::identity(),
        &StateRegion::MAIN,
        options,
        0,
    )
    .unwrap();
    for f in &frames {
        w.write_frame(f).unwrap();
    }
    RomrecSource::from_bytes(w.finish().unwrap().into_inner(), false).unwrap()
}

fn touches(runs: &[Run], offset: u32, len: u32) -> bool {
    runs.iter()
        .any(|r| r.offset < offset + len && offset < r.end())
}

/// The answer with no index: walk every frame's own runs.
fn brute(
    src: &RomrecSource,
    region: StateRegion,
    offset: u32,
    len: u32,
    after: u64,
    backward: bool,
) -> Option<u64> {
    let hit = |f: u64| {
        src.frame_head(f).unwrap().dir.iter().any(|d| {
            d.region == region && touches(if f == 0 { &d.runs } else { &d.changes }, offset, len)
        })
    };
    let n = src.index().len() as u64;
    if backward {
        (0..=after.min(n - 1)).rev().find(|&f| hit(f))
    } else {
        (after + 1..n).find(|&f| hit(f))
    }
}

#[test]
fn every_answer_matches_brute_force() {
    let src = recording(WriterOptions {
        keyframe_interval: 16,
        ..WriterOptions::default()
    });
    let index = ChangeIndex::build(&src).unwrap();
    let mut seed = 7u32;
    let mut rand = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed >> 8
    };
    let mut asked = 0;
    for region in [
        StateRegion::Vram,
        StateRegion::Cgram,
        StateRegion::Oam,
        StateRegion::PpuState,
        StateRegion::Wram,
    ] {
        let size = region.size() as u32;
        // The offsets the fixture changes, and random ones.
        let mut offsets = vec![0u32, 5 * 32, 17 * 2, 0x40, 0x200];
        offsets.extend((0..40).map(|_| rand() % size));
        for &offset in offsets.iter().filter(|o| **o < size) {
            let len = 1 + rand() % 4;
            let len = len.min(size - offset);
            for after in [0u64, 1, 15, 16, 29, 30, 31, 47, 68, 69] {
                for backward in [false, true] {
                    let want = brute(&src, region, offset, len, after, backward);
                    let got = index
                        .when(&src, region, offset, len, after, backward)
                        .unwrap();
                    assert_eq!(
                        got,
                        want,
                        "{} {offset:#x}+{len} after {after} backward {backward}",
                        region.name()
                    );
                    asked += 1;
                }
            }
        }
    }
    assert!(asked > 1000, "{asked}");
}

#[test]
fn no_real_change_is_ever_missed() {
    let src = recording(WriterOptions::default());
    let index = ChangeIndex::build(&src).unwrap();
    for region in [StateRegion::Vram, StateRegion::Cgram, StateRegion::Oam] {
        for f in 1..70u64 {
            let (a, b) = (src.state_at(f - 1).unwrap(), src.state_at(f).unwrap());
            let (x, y) = (a.region(region).unwrap(), b.region(region).unwrap());
            if let Some(at) = (0..x.len()).find(|&i| x[i] != y[i]) {
                assert_eq!(
                    index
                        .when(&src, region, at as u32, 1, f - 1, false)
                        .unwrap(),
                    Some(f),
                    "{} {at:#x} changed at frame {f}",
                    region.name()
                );
            }
        }
    }
}

#[test]
fn the_sidecar_round_trips_and_wram_is_left_out_when_sparse() {
    let src = recording(WriterOptions {
        keyframe_interval: 16,
        wram_keyframe_only: true,
        ..WriterOptions::default()
    });
    let index = ChangeIndex::build(&src).unwrap();
    assert!(index.covers(StateRegion::Vram) && !index.covers(StateRegion::Wram));
    assert_eq!(ChangeIndex::decode(&index.encode()), Some(index.clone()));
    let mut damaged = index.encode();
    damaged.truncate(damaged.len() - 1);
    assert_eq!(ChangeIndex::decode(&damaged), None);
    // Frame 0 is where every byte's history starts.
    assert_eq!(
        index
            .when(&src, StateRegion::Vram, 0x8000, 1, 69, true)
            .unwrap(),
        Some(0)
    );
}
