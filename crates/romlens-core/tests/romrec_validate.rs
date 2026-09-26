//! The validator against one good recording damaged in specific ways, each
//! asserting the exact diagnostic codes it must produce
//! (`16-phase2-plan.md` 2C: "the heart of 2C's test value").

use std::io::Cursor;

use romlens_core::recording::format::*;
use romlens_core::recording::validate::{Severity, ValidateOptions, ValidateReport, validate};
use romlens_core::recording::writer::WriterOptions;
use romlens_core::recording::{RomrecSource, RomrecWriter, StateRegion, fixtures};

/// 70 frames, keyframes every 16, compressed: the recording every test
/// damages.
fn good() -> Vec<u8> {
    good_with(WriterOptions {
        keyframe_interval: 16,
        ..WriterOptions::default()
    })
}

fn good_with(options: WriterOptions) -> Vec<u8> {
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
    w.finish().unwrap().into_inner()
}

fn check(bytes: Vec<u8>, options: ValidateOptions) -> ValidateReport {
    validate(Box::new(Cursor::new(bytes)), options)
}

fn errors(bytes: Vec<u8>) -> Vec<&'static str> {
    let r = check(bytes, ValidateOptions::default());
    r.diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.code)
        .collect()
}

/// Where frame `n`'s chunk starts, and its parsed head.
fn frame(bytes: &[u8], n: usize) -> (usize, FrameHead) {
    let src = RomrecSource::from_bytes(bytes.to_vec(), false).unwrap();
    let e = src.index()[n];
    let at = e.offset as usize;
    (
        at,
        FrameHead::decode(&bytes[at..at + e.len as usize]).unwrap(),
    )
}

fn header_len(bytes: &[u8]) -> usize {
    Header::declared_len(bytes).unwrap()
}

/// Re-sign the header after editing it, so a test of one field does not
/// also trip the header CRC.
fn resign_header(bytes: &mut [u8]) {
    let hl = header_len(bytes);
    let crc = romlens_core::io::crc32::crc32(&bytes[..hl]);
    let f = bytes.len() - FOOTER_LEN;
    bytes[f + 16..f + 20].copy_from_slice(&crc.to_le_bytes());
}

#[test]
fn a_good_recording_is_clean_and_its_samples_check_out() {
    let r = check(
        good(),
        ValidateOptions {
            rom_sha256: Some(fixtures::identity().rom_sha256),
            sample: 8,
            recover: false,
        },
    );
    assert_eq!(r.diagnostics, vec![], "{:?}", r.diagnostics);
    assert_eq!((r.frames, r.sampled), (70, 8));
}

#[test]
fn another_rom_is_m1() {
    let r = check(
        good(),
        ValidateOptions {
            rom_sha256: Some([0; 32]),
            ..ValidateOptions::default()
        },
    );
    assert_eq!(r.codes(), vec!["M1"]);
}

#[test]
fn header_damage() {
    let mut b = good();
    b[0] = b'X';
    assert_eq!(errors(b), vec!["H1"]);

    let mut b = good();
    b[8] = 9; // major version 9
    assert_eq!(errors(b), vec!["H2"]);

    let mut b = good();
    b[50..52].copy_from_slice(&0u16.to_le_bytes()); // keyframe interval 0
    resign_header(&mut b);
    assert!(errors(b).contains(&"H6"));

    let mut b = good();
    b[76..80].copy_from_slice(&7u32.to_le_bytes()); // compression 7
    resign_header(&mut b);
    let e = errors(b);
    assert!(e.contains(&"H7") && e.contains(&"P2"), "{e:?}");

    let mut b = good();
    b[49] |= 0x80; // an undefined flag bit
    resign_header(&mut b);
    let r = check(b, ValidateOptions::default());
    assert_eq!(r.codes(), vec!["H8"]);
    assert_eq!(r.errors(), 0);
}

#[test]
fn footer_and_crc_damage() {
    let mut b = good();
    let hl = header_len(&b);
    b[hl - 1] ^= 1; // a byte of the string area
    assert_eq!(errors(b), vec!["F4"]);

    let mut b = good();
    let n = b.len();
    b.truncate(n - FOOTER_LEN);
    assert_eq!(errors(b.clone()), vec!["F1"]);
    // With --recover a missing footer is expected, not wrong.
    let r = check(
        b,
        ValidateOptions {
            recover: true,
            ..ValidateOptions::default()
        },
    );
    assert_eq!((r.errors(), r.codes()[0]), (0, "F1"), "{:?}", r.diagnostics);

    let mut b = good();
    let f = b.len() - FOOTER_LEN;
    let index = u64::from_le_bytes(b[f..f + 8].try_into().unwrap()) as usize;
    b[index + 8 + 8] ^= 0x10; // frame 0's offset in the index
    let e = errors(b);
    assert!(e.contains(&"F5") && e.contains(&"I2"), "{e:?}");

    let mut b = good();
    b[FRAME_COUNT_AT as usize] = 69;
    resign_header(&mut b);
    assert_eq!(errors(b), vec!["F3"]);
}

#[test]
fn frame_damage() {
    // K3: a frame numbered out of order.
    let mut b = good();
    let (at, _) = frame(&b, 5);
    b[at + 8] = 50;
    assert_eq!(errors(b), vec!["K3"]);

    // K1 and I4: frame 0 marked as a delta.
    let mut b = good();
    let (at, _) = frame(&b, 0);
    b[at + 16] = KIND_DELTA;
    let e = errors(b);
    assert!(e.contains(&"K1") && e.contains(&"I4"), "{e:?}");

    // P2: a payload byte flipped.
    let mut b = good();
    let (at, head) = frame(&b, 3);
    b[at + head.payload_at + 2] ^= 0xFF;
    assert!(errors(b).contains(&"P2"));

    // R1: a run pushed past its region.
    let mut b = good();
    let (at, head) = frame(&b, 3);
    let runs_at = at + FRAME_HEADER_LEN + head.dir.len() * DIR_ENTRY_LEN;
    b[runs_at..runs_at + 4].copy_from_slice(&0x0010_0000u32.to_le_bytes());
    assert!(errors(b).contains(&"R1"));
}

#[test]
fn a_change_run_that_misses_a_change_is_caught_by_sampling() {
    // Frame 30 rewrites tile 5 in VRAM (fixtures::frames). Shorten that
    // frame's VRAM change run to one byte, leaving its data run alone: the
    // file still reads, but `changes` now under-reports.
    let mut b = good();
    let (at, head) = frame(&b, 30);
    let mut cursor = at + FRAME_HEADER_LEN + head.dir.len() * DIR_ENTRY_LEN;
    let mut patched = false;
    for d in &head.dir {
        cursor += d.runs.len() * 8;
        if d.region == StateRegion::Vram {
            b[cursor + 4..cursor + 8].copy_from_slice(&1u32.to_le_bytes());
            patched = true;
            break;
        }
        cursor += d.changes.len() * 8;
    }
    assert!(patched, "frame 30 changes VRAM");
    let r = check(
        b,
        ValidateOptions {
            sample: 69,
            ..ValidateOptions::default()
        },
    );
    let codes = r.codes();
    assert!(codes.contains(&"S2"), "{:?}", r.diagnostics);
    assert!(codes.contains(&"R4"), "{:?}", r.diagnostics);
}

#[test]
fn layer_and_wram_flags() {
    // L1: a write log declared with no chunks is a warning.
    let mut b = good();
    b[72] |= 2;
    resign_header(&mut b);
    let r = check(b, ValidateOptions::default());
    assert_eq!((r.codes(), r.errors()), (vec!["L1"], 0));

    // W1: the keyframe-only flag set on a file whose deltas carry WRAM.
    let mut b = good();
    b[49] |= FLAG_WRAM_KEYFRAME_ONLY;
    resign_header(&mut b);
    assert!(errors(b).contains(&"W1"));

    // And a file written keyframe-only is clean.
    let b = good_with(WriterOptions {
        keyframe_interval: 16,
        wram_keyframe_only: true,
        ..WriterOptions::default()
    });
    assert_eq!(check(b, ValidateOptions::default()).diagnostics, vec![]);
}

#[test]
fn a_flood_of_one_problem_is_capped() {
    // Every frame renumbered: one K3 each, capped at five plus a count.
    let mut b = good();
    let offsets: Vec<usize> = RomrecSource::from_bytes(b.clone(), false)
        .unwrap()
        .index()
        .iter()
        .map(|e| e.offset as usize)
        .collect();
    for at in &offsets[1..] {
        b[at + 8] = 0xEE;
    }
    let r = check(b, ValidateOptions::default());
    let k3: Vec<_> = r.diagnostics.iter().filter(|d| d.code == "K3").collect();
    assert_eq!(k3.len(), 6);
    assert_eq!(k3[5].message, "64 more like this");
}
