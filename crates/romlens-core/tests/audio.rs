//! A recording's sound side read for a student (docs/23, A5), on the
//! fixture recording: the sound fixture's driver, directory and sample, a
//! note keyed on in frame 1 and off in frame 3.

use romlens_core::audio::{
    NoteKind, PartKind, aram_map, directory, note_name, port_messages, sample_tuning, timeline,
    voices,
};
use romlens_core::recording::mesen::stream::encode;
use romlens_core::recording::mesen::{PackOptions, pack};
use romlens_core::recording::{MachineStateSource, RomrecSource, SpcState, StateRegion};
use romlens_core::rom::image::RomImage;

fn recording() -> RomrecSource {
    let rom = RomImage::from_bytes(romlens_core::fixtures::minimal_lorom(), "f.sfc").unwrap();
    let stream = encode::fixture_with_audio(rom.bytes(), 5);
    let mut out = std::io::Cursor::new(Vec::new());
    pack(stream.as_slice(), &rom, &mut out, PackOptions::default()).unwrap();
    RomrecSource::from_bytes(out.into_inner(), false).unwrap()
}

#[test]
fn the_voices_read_their_registers() {
    let rec = recording();
    let s = rec.state_at(2).unwrap();
    let v = voices(
        s.region(StateRegion::DspRegisters).unwrap(),
        s.region(StateRegion::Aram),
    );
    assert_eq!(v.len(), 8);
    let v0 = &v[0];
    assert_eq!((v0.volume, v0.pitch, v0.source), ((127, 127), 0x1000, 0));
    assert!(v0.sounding());
    assert_eq!(v0.sample, Some((0x4000, 0x4012)));
    assert_eq!(
        v0.envelope(),
        "ADSR: decay 0: halves in 326 ms, attack 15: full at once, sustain at 8/8, sustain rate 0: never steps"
    );
    assert!(v0.pitch_words().starts_with("32000 Hz"));
    assert!(!v[1].sounding());
}

#[test]
fn the_map_finds_the_driver_directory_and_sample() {
    let rec = recording();
    let s = rec.state_at(2).unwrap();
    let aram = s.region(StateRegion::Aram).unwrap();
    let dsp = s.region(StateRegion::DspRegisters).unwrap();
    let spc = SpcState::decode(s.region(StateRegion::SpcState).unwrap());
    let d = directory(aram, 0x3C, &[0]);
    assert_eq!(d.len(), 1);
    assert_eq!(
        (d[0].start, d[0].loop_at, d[0].blocks, d[0].loops),
        (0x4000, 0x4012, 4, true)
    );
    let map = aram_map(aram, dsp, &spc, &[0], &[], None);
    let at = |a: u16| {
        map.iter()
            .find(|p| p.start <= a && (a as u32) < p.start as u32 + p.len)
            .unwrap()
    };
    assert_eq!(at(0x0010).kind, PartKind::DirectPage);
    assert_eq!(at(0x00F4).kind, PartKind::Io);
    assert_eq!(at(0x0209).kind, PartKind::Code);
    assert_eq!(
        at(0x020F).kind,
        PartKind::Code,
        "the timer loop, reached by the walk"
    );
    assert_eq!(
        at(0x0200).kind,
        PartKind::Other,
        "the start-up code is not reachable from main"
    );
    assert_eq!(at(0x3C00).kind, PartKind::Directory);
    assert_eq!(at(0x3C00).len, 4);
    let sample = at(0x4000);
    assert_eq!((sample.kind, sample.len), (PartKind::Sample(0), 36));
    assert_eq!(sample.label, "sample 0: 4 blocks, loops at $4012");
    // The parts cover audio RAM once.
    assert_eq!(map.iter().map(|p| p.len).sum::<u32>(), 0x10000);
}

#[test]
fn notes_start_and_stop_with_key_on_and_off() {
    let rec = recording();
    let t = timeline(&rec, 0, 4).unwrap();
    assert_eq!(t.len(), 2);
    assert_eq!(
        (t[0].voice, t[0].frame, t[0].kind, t[0].pitch),
        (0, 1, NoteKind::On, 0x1000)
    );
    assert_eq!((t[1].frame, t[1].kind), (3, NoteKind::Off));
    assert!(t[0].spc_cycle > 17_066 && t[0].spc_cycle < 2 * 17_066);
}

#[test]
fn a_samples_loop_gives_its_tuning() {
    let rec = recording();
    let aram = rec.region_at(2, StateRegion::Aram).unwrap();
    // The loop is two blocks of a square wave 16 samples long: 2000 Hz at
    // pitch $1000, just above B6.
    assert_eq!(sample_tuning(&aram, 0x4000, 0x4012), Some(2000.0));
    assert_eq!(note_name(2000.0), "B6 +21");
    assert_eq!(note_name(440.0), "A4");
    assert_eq!(note_name(261.63), "C4");
}

#[test]
fn port_messages_run_both_ways() {
    let rec = recording();
    let e = rec.apu_events(1).unwrap().unwrap();
    let m = port_messages(&e);
    assert_eq!(m.len(), 2);
    assert!(m[0].from_cpu && !m[1].from_cpu);
    assert_eq!((m[0].port, m[0].value, m[1].value), (0, 0x01, 0x01));
}

mod with_a_log {
    use super::*;
    use romlens_core::io::import::spc_log;
    use romlens_core::model::spc_log::{SpcAccess, SpcAccessRun, SpcInsn, SpcLog};

    /// A log of the fixture driver: the boot ROM uploaded `$3000`, the
    /// driver's start-up code ran (which a walk from `main` cannot reach),
    /// it read song bytes at `$2000` and `$2008`, and the DSP read the
    /// directory entry, the sample's first block and, through an entry not
    /// yet set up, the direct page.
    fn log() -> SpcLog {
        SpcLog {
            rom_crc32: 0,
            rom_size: 0,
            insns: [0x0200u16, 0x0202, 0x0203, 0x0206, 0x0209]
                .map(|pc| SpcInsn {
                    pc,
                    boot_rom: false,
                    count: 1,
                })
                .to_vec(),
            accesses: vec![
                SpcAccessRun {
                    pc: 0x0209,
                    addr: 0x2000,
                    len: 2,
                    access: SpcAccess::Read,
                    count: 4,
                },
                SpcAccessRun {
                    pc: 0x0209,
                    addr: 0x2008,
                    len: 1,
                    access: SpcAccess::Read,
                    count: 1,
                },
                SpcAccessRun {
                    pc: 0xFFE2,
                    addr: 0x3000,
                    len: 16,
                    access: SpcAccess::Write,
                    count: 16,
                },
                SpcAccessRun {
                    pc: 0,
                    addr: 0x0000,
                    len: 9,
                    access: SpcAccess::DspRead,
                    count: 9,
                },
                SpcAccessRun {
                    pc: 0,
                    addr: 0x3C00,
                    len: 4,
                    access: SpcAccess::DspRead,
                    count: 1,
                },
                SpcAccessRun {
                    pc: 0,
                    addr: 0x4000,
                    len: 9,
                    access: SpcAccess::DspRead,
                    count: 9,
                },
            ],
            flows: Vec::new(),
        }
    }

    #[test]
    fn the_format_round_trips_and_is_told_apart() {
        let l = log();
        let bytes = spc_log::write(&l);
        assert!(spc_log::is_spc_log(&bytes));
        assert_eq!(spc_log::read(&bytes, None).unwrap(), l);
        // The main CPU's reader sends it where it belongs.
        let err = romlens_core::io::import::exec_log::read(&bytes, &[]).unwrap_err();
        assert!(err.to_string().contains("sound CPU"), "{err}");
        // A ROM that is not the one it was recorded from is refused.
        assert!(spc_log::read(&bytes, Some(&[1, 2, 3])).is_err());
    }

    #[test]
    fn merging_adds_counts_and_keeps_each_relationship_once() {
        let mut a = log();
        a.merge(&log());
        assert_eq!(a.insns.len(), 5);
        assert!(a.insns.iter().all(|i| i.count == 2));
        let song = a.accesses.iter().find(|r| r.addr == 0x2000).unwrap();
        assert_eq!((song.len, song.count), (2, 8));
        assert_eq!(a.touched(SpcAccess::DspRead).len(), 22);
        assert_eq!(a.touched_by_driver().len(), 3, "not the boot ROM's upload");
    }

    #[test]
    fn the_map_takes_code_data_and_samples_from_it() {
        let rec = recording();
        let s = rec.state_at(2).unwrap();
        let spc = SpcState::decode(s.region(StateRegion::SpcState).unwrap());
        let map = aram_map(
            s.region(StateRegion::Aram).unwrap(),
            s.region(StateRegion::DspRegisters).unwrap(),
            &spc,
            &[0],
            &[],
            Some(&log()),
        );
        let at = |a: u16| {
            map.iter()
                .find(|p| p.start <= a && (a as u32) < p.start as u32 + p.len)
                .unwrap()
        };
        assert_eq!(
            at(0x0200).kind,
            PartKind::Code,
            "the start-up code the log saw run"
        );
        assert_eq!(at(0x0200).label, "driver code");
        assert_eq!(at(0x2000).kind, PartKind::DriverData);
        assert_eq!(at(0x2001).kind, PartKind::DriverData);
        assert_eq!(
            at(0x2004).kind,
            PartKind::DriverData,
            "a short gap in a table"
        );
        assert_ne!(
            at(0x3000).kind,
            PartKind::DriverData,
            "the boot ROM uploaded it"
        );
        assert_eq!(at(0x0008).kind, PartKind::DirectPage);
        assert_eq!(
            at(0x4000).label,
            "sample 0: 4 blocks, loops at $4012, played"
        );
    }
}
