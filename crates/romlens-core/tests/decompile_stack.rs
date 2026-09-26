//! The stack by offset in the C (docs/22, L1): `LDA $03,S` reading what
//! the routine pushed is that push's variable, and `LDA ($01,S),Y` reads
//! through a pushed pointer.

use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::decompile::{self, DecompileOptions, Level};
use romlens_core::fixtures;
use romlens_core::model::Project;
use romlens_core::{RomImage, SnesAddress};

fn rom() -> RomImage {
    RomImage::from_bytes(fixtures::stack_lorom(), "s.sfc").unwrap()
}

fn c(level: Level, at: u16) -> String {
    let rom = rom();
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let opts = DecompileOptions {
        level,
        ..DecompileOptions::default()
    };
    decompile::decompile(&rom, &project, &snap, SnesAddress::new(0, at), &opts)
        .unwrap()
        .text
}

#[test]
fn a_pushed_word_updated_in_place_is_a_variable() {
    for level in [Level::Clean, Level::Full] {
        let text = c(level, 0x8020);
        assert!(!text.contains("STACK"), "{text}");
        assert!(!text.contains("push16"), "{text}");
        assert!(!text.contains("note: the stack"), "{text}");
    }
    // At lift the stack stays as the CPU sees it.
    let lift = c(Level::Lift, 0x8020);
    assert!(lift.contains("STACK8(S + 2)"), "{lift}");
}

#[test]
fn a_pushed_pointer_is_read_through() {
    let text = c(Level::Clean, 0x8040);
    assert!(!text.contains("STACK"), "{text}");
    // The pushed pointer plus Y, folded: the word at $8072.
    assert!(text.contains("MEM16(0x8072)"), "{text}");
}

#[test]
fn an_argument_the_caller_pushed_is_named_and_read_at_entry() {
    // At clean the routine reads it as it starts: a call in C leaves no
    // return address, so the caller's word is just above S.
    let text = c(Level::Clean, 0x8060);
    assert!(text.contains("arg3 = STACK16(S + 1);"), "{text}");
    assert!(text.contains("ADDR_7E0014 = arg3"), "{text}");
    assert!(!text.contains("note: the stack"), "{text}");
    // And the caller's push stays a push, for the callee to read.
    let caller = c(Level::Clean, 0x8000);
    assert!(caller.contains("push16(0x1234)"), "{caller}");
}

#[test]
fn at_full_the_argument_is_passed_in_the_call() {
    let text = c(Level::Full, 0x8060);
    assert!(text.contains("void SUB_008060(u16 arg3)"), "{text}");
    assert!(text.contains("ADDR_7E0014 = arg3"), "{text}");
    assert!(!text.contains("STACK"), "{text}");
    let caller = c(Level::Full, 0x8000);
    assert!(caller.contains("SUB_008060(0x1234);"), "{caller}");
    assert!(!caller.contains("push16"), "{caller}");
}

/// The fixture's C, for a look: `cargo test --test decompile_stack show -- --ignored --nocapture`.
#[test]
#[ignore]
fn show_the_c() {
    for at in [0x8000, 0x8020, 0x8060] {
        eprintln!("{}", c(Level::Full, at));
    }
}
