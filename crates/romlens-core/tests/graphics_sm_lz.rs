//! The Super Metroid decompressor against the game's own data. Opt-in through
//! `ROMLENS_ROM_DIR`: the corpus is the development ROM, which never ships.
//!
//! The in-crate tests prove the decoder matches the format as written down;
//! this proves the format as written down matches the game. Both streams
//! below come out at a whole number of kilobytes — 16 KB and 12 KB, whole
//! banks of tiles — which a decoder that misread a command would not manage
//! by accident.

mod common;

use romlens_core::graphics::compress::sm_lz::{SmLzError, decompress};

fn at(rom: &romlens_core::RomImage, expr: &str) -> usize {
    rom.resolve(expr).unwrap().file_offset.0 as usize
}

#[test]
fn the_games_streams_decompress_whole() {
    let Some(rom) = common::dev_rom() else { return };
    for (expr, output, consumed) in [("$95:80D8", 16384, 9481), ("$B9:8000", 12288, 8349)] {
        let d = decompress(&rom.bytes()[at(&rom, expr)..]).unwrap();
        assert_eq!(d.output.len(), output, "{expr}");
        assert_eq!(d.consumed, consumed, "{expr}");
        // Every command but the one reachable only in the long form is used
        // by real data, so none of them is untested against the game.
        assert!(
            d.commands[..7].iter().all(|n| *n > 0),
            "{expr}: {:?}",
            d.commands
        );
    }
}

#[test]
fn the_middle_of_a_stream_is_refused() {
    let Some(rom) = common::dev_rom() else { return };
    // An address the decompressor reads *during* a stream is not a stream
    // start, and must fail rather than produce plausible garbage.
    let r = decompress(&rom.bytes()[at(&rom, "$95:818A")..]);
    assert!(matches!(r, Err(SmLzError::BadReference { .. })), "{r:?}");
}
