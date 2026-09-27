//! The sound upload read from the ROM (docs/23, A9), on a fixture of our
//! own: a 65816 upload routine written from the protocol, a reset that
//! points it at a block list, and a driver, directory and sample in it.

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::apu::{Apu, boot_upload};
use romlens_core::audio::upload::{from_ports, trace, trace_uploads};
use romlens_core::fixtures::sound::{UPLOAD_LIST, brr_sample};
use romlens_core::fixtures::sound_upload_lorom;
use romlens_core::model::project::Project;
use romlens_core::model::region::{DataKind, Evidence, RegionKind};
use romlens_core::recording::apu::{ApuEvent, ApuEventKind};
use romlens_core::rom::image::RomImage;
use romlens_core::{FileOffset, SnesAddress};

fn rom() -> RomImage {
    RomImage::from_bytes(sound_upload_lorom(), "upload.sfc").unwrap()
}

#[test]
fn the_routine_the_pointer_and_the_list_are_traced() {
    let rom = rom();
    let snap = analyze(&rom, &Project::new(&rom), &AnalysisControl::silent()).unwrap();
    let r = trace(&rom, &snap);
    assert_eq!(r.routines.len(), 1);
    assert_eq!(r.routines[0].entry, SnesAddress::new(0, 0x8020));
    assert_eq!(r.routines[0].pointer, 0x00);
    assert_eq!(r.uploads.len(), 1);
    let u = &r.uploads[0];
    assert_eq!(u.list, SnesAddress::new(0, 0xD000));
    assert_eq!(u.set_in, SnesAddress::new(0, 0x8000));
    assert_eq!(u.entry, 0x0200);
    let blocks: Vec<(u16, u16)> = u.blocks.iter().map(|b| (b.aram, b.len)).collect();
    assert_eq!(
        blocks[1..],
        [(0x3C00, 4), (0x4000, brr_sample().len() as u16)]
    );
    assert_eq!(blocks[0].0, 0x0200);
    // Each block's bytes sit after its four-byte header.
    assert_eq!(u.blocks[0].rom, FileOffset(UPLOAD_LIST as u32 + 4));
    // The fast path the analysis uses finds the same.
    let (_, fast) = trace_uploads(&rom, &snap);
    assert_eq!(fast, r.uploads);
}

#[test]
fn the_analysis_calls_the_uploaded_sample_a_sample() {
    let rom = rom();
    let snap = analyze(&rom, &Project::new(&rom), &AnalysisControl::silent()).unwrap();
    let (_, uploads) = trace_uploads(&rom, &snap);
    let sample = uploads[0].blocks[2].rom;
    let r = snap.region_at(sample).unwrap();
    assert_eq!(r.kind, RegionKind::Data(DataKind::Sample));
    assert!(
        matches!(&r.evidence[0], Evidence::Uploaded(w) if w.contains("audio RAM $4000")),
        "{:?}",
        r.evidence
    );
    let dir = snap.region_at(uploads[0].blocks[1].rom).unwrap();
    assert!(
        matches!(&dir.evidence[0], Evidence::Uploaded(w) if w.starts_with("the sample directory"))
    );
    // The code that sends it is still code.
    assert_eq!(
        snap.region_at(FileOffset(0x20)).unwrap().kind,
        RegionKind::Code
    );
}

#[test]
fn the_upload_boots_and_plays() {
    let rom = rom();
    let snap = analyze(&rom, &Project::new(&rom), &AnalysisControl::silent()).unwrap();
    let (_, uploads) = trace_uploads(&rom, &snap);
    let mut apu = Apu::new();
    boot_upload(&mut apu, &uploads[0].apu_blocks(&rom), uploads[0].entry);
    apu.run_until(20_000);
    assert_eq!(apu.bus.dsp[0x5D], 0x3C, "the driver set DIR");
    apu.write_port(0, 0x01);
    apu.bus.output = Some(Vec::new());
    apu.run_until(60_000);
    let out = apu.bus.output.take().unwrap();
    assert!(out.iter().any(|f| f.voices[0] != 0), "voice 0 sounds");
}

/// The S-CPU's writes for an upload: each byte as one 16-bit store (the
/// index to port 0, the byte to port 1 a cycle later), as Super Mario
/// World sends them, or with `byte_first` as the fixture's routine does
/// (the byte, then the index, two stores).
fn sent(blocks: &[(u16, Vec<u8>)], entry: u16, byte_first: bool) -> Vec<ApuEvent> {
    let mut out = Vec::new();
    let mut cycle = 0u64;
    let mut w = |port: u8, value: u8, gap: u64| {
        cycle += gap;
        out.push(ApuEvent {
            kind: ApuEventKind::CpuPort,
            address: port,
            value,
            spc_cycle: cycle,
            master_clock: 0,
        });
    };
    let mut command = 0xCCu8;
    for (aram, bytes) in blocks {
        let [lo, hi] = aram.to_le_bytes();
        w(2, lo, 40);
        w(3, hi, 4);
        w(1, 1, 20);
        w(0, command, 20);
        for (i, b) in bytes.iter().enumerate() {
            if byte_first {
                w(1, *b, 30);
                w(0, i as u8, 6);
            } else {
                w(0, i as u8, 30);
                w(1, *b, 1);
            }
        }
        command = (bytes.len() as u8).wrapping_add(1).max(1);
    }
    let [lo, hi] = entry.to_le_bytes();
    w(2, lo, 40);
    w(3, hi, 4);
    w(1, 0, 20);
    w(0, command, 20);
    out
}

#[test]
fn an_upload_reads_back_from_the_port_writes() {
    let blocks = vec![
        (0x0400u16, vec![1, 2, 3]),
        (0x5000, (0..300).map(|i| i as u8).collect()),
    ];
    for byte_first in [false, true] {
        let got = from_ports(&sent(&blocks, 0x0400, byte_first));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].blocks, blocks, "byte first: {byte_first}");
        assert_eq!(got[0].entry, Some(0x0400));
    }
}
