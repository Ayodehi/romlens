//! N-SPC songs read (docs/23, A14), on audio RAM laid out by hand the way
//! the Super Famicom Development Wiki's two format pages describe.

use romlens_core::audio::nspc::*;

fn aram() -> Vec<u8> {
    romlens_core::fixtures::sound::nspc_aram()
}

#[test]
fn the_driver_is_known_by_its_command_lengths() {
    let a = aram();
    let d = recognise(&a).expect("the length table");
    assert_eq!((d.dialect, d.lengths_at), (Dialect::Old, 0x0FC2));
    assert_eq!(d.song_table, Some(0x1360));
    assert_eq!(d.songs, [0x1400, 0x1420]);
    // Found again without a search while it is where it was.
    assert_eq!(refresh(&a, &d), Some(d.clone()));
    // Nothing like it, no driver.
    assert_eq!(recognise(&vec![0u8; 0x10000]), None);
}

#[test]
fn a_song_is_a_list_of_blocks_repeats_and_a_jump() {
    let a = aram();
    let l = song_list(&a, Dialect::Old, 0x1400).unwrap();
    assert_eq!(
        l,
        [
            ListEntry::Block {
                at: 0x1400,
                block: 0x1500
            },
            ListEntry::Block {
                at: 0x1402,
                block: 0x1510
            },
            ListEntry::Repeat {
                at: 0x1404,
                count: 2,
                to: 0x1402
            },
            ListEntry::Jump {
                at: 0x1408,
                to: 0x1400
            },
        ]
    );
    assert_eq!(block_tracks(&a, 0x1500)[7], 0x2100);
    // A jump out of the list is not a song.
    let mut bad = a.clone();
    bad[0x140A..0x140C].copy_from_slice(&0x3000u16.to_le_bytes());
    assert_eq!(song_list(&bad, Dialect::Old, 0x1400), None);
}

#[test]
fn a_track_reads_as_lengths_notes_and_commands() {
    let a = aram();
    let t = track(&a, Dialect::Old, 0x2000);
    assert!(t.ends());
    let text: Vec<String> = t.events.iter().map(|e| e.text()).collect();
    assert_eq!(
        text,
        [
            "instrument $04: the instrument (patch) the voice plays",
            "length 24 ticks (an eighth), quantize 7, velocity 15",
            "note C3",
            "tie: the note before goes on",
            "length 48 ticks (a quarter note)",
            "rest",
            "call $2300, 2 times",
            "percussion 2",
            "end",
        ]
    );
    assert_eq!(t.events[1].bytes, [0x18, 0x7F]);
    assert_eq!(t.event_at(0x2009), Some(6), "inside the call's parameters");
    // The standard version names $80 as C1 and has its tie at $C8.
    assert_eq!(note_name(Dialect::Standard, 0), "C1");
    let std = track(&[0xC8, 0x00].repeat(0x8000), Dialect::Standard, 0);
    assert_eq!(std.events[0].kind, EventKind::Tie);
}

#[test]
fn the_direct_page_says_what_is_playing() {
    let a = aram();
    let d = recognise(&a).unwrap();
    let p = playing(&a, &d).unwrap();
    assert_eq!(p.song, Some(1));
    assert_eq!((p.entry, p.block), (0x1402, 0x1510));
    assert_eq!(p.tracks[0], 0x2200);
    assert_eq!(p.positions[0], Some(0x2201));
    assert_eq!(p.positions[1], None);
}
