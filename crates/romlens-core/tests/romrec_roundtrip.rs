//! `.romrec` written, read back and held to the same contract as the
//! in-memory source (`16-phase2-plan.md` 2C.7).

use std::io::Cursor;

use romlens_core::recording::format::{FOOTER_LEN, FRAME_COUNT_AT};
use romlens_core::recording::writer::WriterOptions;
use romlens_core::recording::{
    MachineStateSource, MemorySource, RecordingError, RomrecSource, RomrecWriter, StateRegion,
    conformance, fixtures,
};

fn write(frames: &[romlens_core::recording::MachineState], options: WriterOptions) -> Vec<u8> {
    let regions: Vec<StateRegion> = frames[0].regions.keys().copied().collect();
    let mut w = RomrecWriter::new(
        Cursor::new(Vec::new()),
        &fixtures::identity(),
        &regions,
        options,
        0,
    )
    .unwrap();
    for f in frames {
        w.write_frame(f).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn options(interval: u16) -> WriterOptions {
    WriterOptions {
        keyframe_interval: interval,
        ..WriterOptions::default()
    }
}

#[test]
fn both_sources_meet_the_contract() {
    let frames = fixtures::frames(70);
    let memory = MemorySource::new(fixtures::identity(), frames.clone());
    conformance::check(&memory, &frames).unwrap();
    for interval in [1, 16, 60] {
        let bytes = write(&frames, options(interval));
        let rec = RomrecSource::from_bytes(bytes, false).unwrap();
        conformance::check(&rec, &frames).unwrap_or_else(|e| panic!("interval {interval}: {e}"));
        assert!(!rec.recovered());
    }
    let raw = write(
        &frames,
        WriterOptions {
            compress: false,
            ..options(16)
        },
    );
    conformance::check(&RomrecSource::from_bytes(raw, false).unwrap(), &frames).unwrap();
}

#[test]
fn deltas_are_small_and_changes_are_tight() {
    let frames = fixtures::frames(40);
    let bytes = write(&frames, options(60));
    let rec = RomrecSource::from_bytes(bytes.clone(), false).unwrap();
    // One keyframe, then deltas of a few bytes each: the whole file is far
    // smaller than forty 197 KB frames.
    assert!(bytes.len() < 40_000, "{} bytes", bytes.len());
    let delta = rec.index()[5].len;
    assert!(delta < 1500, "a delta frame is {delta} bytes");
    // VRAM changes once, at frame 30, and only in tile 5.
    assert!(rec.changes(0, 29, StateRegion::Vram).unwrap().is_empty());
    let runs = rec.changes(29, 30, StateRegion::Vram).unwrap();
    assert_eq!(runs.len(), 1);
    assert!(runs[0].offset >= 0xA0 && runs[0].end() <= 0xC0, "{runs:?}");
    // OAM changes every frame; x of sprite 0 is byte 0.
    assert_eq!(rec.changes(3, 4, StateRegion::Oam).unwrap()[0].offset, 0);
}

#[test]
fn a_file_without_a_footer_is_in_progress_and_recoverable() {
    let frames = fixtures::frames(12);
    let bytes = write(&frames, options(4));
    // Cut the index and footer off, and half of the last frame too.
    let rec = RomrecSource::from_bytes(bytes.clone(), false).unwrap();
    let last = rec.index()[11];
    let cut = bytes[..last.offset as usize + last.len as usize / 2].to_vec();
    match RomrecSource::from_bytes(cut.clone(), false) {
        Err(RecordingError::Corrupt(m)) => assert!(m.contains("no footer"), "{m}"),
        other => panic!("{other:?}"),
    }
    let recovered = RomrecSource::from_bytes(cut, true).unwrap();
    assert!(recovered.recovered());
    assert_eq!(recovered.frame_count(), Some(11));
    conformance::check(&recovered, &frames[..11]).unwrap();
    // The header still says "in progress".
    assert_eq!(
        u64::from_le_bytes(bytes[FRAME_COUNT_AT as usize..][..8].try_into().unwrap()),
        12,
        "a finished file has its count patched"
    );
}

#[test]
fn damage_is_detected() {
    let frames = fixtures::frames(3);
    let bytes = write(&frames, options(60));
    let mut header = bytes.clone();
    header[20] ^= 1; // inside the ROM hash
    assert!(matches!(
        RomrecSource::from_bytes(header, false),
        Err(RecordingError::Corrupt(m)) if m.contains("header")
    ));
    let mut index = bytes.clone();
    let at = bytes.len() - FOOTER_LEN - 5;
    index[at] ^= 1;
    assert!(matches!(
        RomrecSource::from_bytes(index, false),
        Err(RecordingError::Corrupt(m)) if m.contains("index")
    ));
    assert!(matches!(
        RomrecSource::from_bytes(b"ROMLENS!".to_vec(), false),
        Err(RecordingError::BadFormat(_))
    ));
}

#[test]
fn wram_between_keyframes_is_absent_not_stale() {
    let frames = fixtures::frames(10);
    let bytes = write(
        &frames,
        WriterOptions {
            wram_keyframe_only: true,
            ..options(4)
        },
    );
    let rec = RomrecSource::from_bytes(bytes, false).unwrap();
    assert!(rec.state_at(4).unwrap().region(StateRegion::Wram).is_some());
    assert!(rec.state_at(5).unwrap().region(StateRegion::Wram).is_none());
    conformance::check(&rec, &frames).unwrap();
}

#[test]
fn a_recording_of_another_rom_is_refused() {
    let rec = RomrecSource::from_bytes(write(&fixtures::frames(1), options(60)), false).unwrap();
    rec.check_rom(&fixtures::identity().rom_sha256).unwrap();
    assert!(matches!(
        rec.check_rom(&[0; 32]),
        Err(RecordingError::RomMismatch { .. })
    ));
}
