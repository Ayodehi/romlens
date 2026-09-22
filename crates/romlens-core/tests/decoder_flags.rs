//! Flag effects, peepholes, target rules and bank wrap.

use romlens_core::analysis::flow::refine;
use romlens_core::cpu65816::{
    ASSUMED_DBR, ASSUMED_DP, ASSUMED_PLP, ASSUMED_XCE_CARRY, BANK_WRAP, FlagState, Instruction,
    Mnemonic, TargetKind, decode,
};
use romlens_core::fixtures;
use romlens_core::{FileOffset, SnesAddress};

fn native() -> FlagState {
    FlagState {
        m: false,
        x: false,
        e: false,
        dbr: Some(0x80),
        dp: Some(0),
        c: None,
    }
}

fn dec(bytes: &[u8], at: SnesAddress, flags: FlagState) -> Instruction {
    decode(bytes, at, FileOffset(0), flags).unwrap()
}

/// Decode a straight-line sequence, threading flags and peepholes.
fn run(bytes: &[u8], start: SnesAddress, flags: FlagState) -> Vec<Instruction> {
    let mut out: Vec<Instruction> = Vec::new();
    let mut pos = 0;
    let mut addr = start;
    let mut flags = flags;
    while pos < bytes.len() {
        let mut insn = decode(&bytes[pos..], addr, FileOffset(pos as u32), flags).unwrap();
        refine(&out, &mut insn);
        flags = insn.flags_after;
        pos += insn.len as usize;
        addr = insn.next_address();
        out.push(insn);
    }
    out
}

#[test]
fn sep_rep_change_widths() {
    let a = SnesAddress::new(0x80, 0x8000);
    let sep = dec(&[0xE2, 0x20], a, native());
    assert!(sep.flags_after.m && !sep.flags_after.x);
    let rep = dec(
        &[0xC2, 0x30],
        a,
        FlagState {
            m: true,
            x: true,
            ..native()
        },
    );
    assert!(!rep.flags_after.m && !rep.flags_after.x);
    // REP in emulation mode cannot clear M/X.
    let e = FlagState::RESET;
    let rep_e = dec(&[0xC2, 0x30], a, e);
    assert!(rep_e.flags_after.m && rep_e.flags_after.x && rep_e.flags_after.e);
    // Widths follow the effective flags.
    let lda8 = dec(
        &[0xA9, 0x01, 0x02],
        a,
        FlagState {
            m: true,
            ..native()
        },
    );
    assert_eq!(lda8.len, 2);
    let lda16 = dec(&[0xA9, 0x01, 0x02], a, native());
    assert_eq!(lda16.len, 3);
    let ldx8 = dec(&[0xA2, 0x01, 0x02], a, FlagState::RESET);
    assert_eq!(ldx8.len, 2, "emulation mode forces 8-bit");
}

#[test]
fn xce_variants() {
    let a = SnesAddress::new(0x00, 0x8000);
    // CLC; XCE → native; carry then holds the old E.
    let seq = run(&[0x18, 0xFB], a, FlagState::RESET);
    assert!(!seq[1].flags_after.e);
    assert_eq!(seq[1].flags_after.c, Some(true));
    assert_eq!(seq[1].assumptions & ASSUMED_XCE_CARRY, 0);
    // SEC; XCE → emulation, widths forced to 8-bit.
    let seq = run(&[0x38, 0xFB], a, native());
    assert!(seq[1].flags_after.e && seq[1].flags_after.m && seq[1].flags_after.x);
    // Unknown carry: assume native, flag the assumption.
    let x = dec(&[0xFB], a, FlagState::RESET);
    assert!(!x.flags_after.e);
    assert_ne!(x.assumptions & ASSUMED_XCE_CARRY, 0);
    // PLP keeps M/X but says so.
    let plp = dec(&[0x28], a, native());
    assert_ne!(plp.assumptions & ASSUMED_PLP, 0);
    assert_eq!(plp.flags_after.m, native().m);
    // Arithmetic and calls forget the carry.
    let adc = dec(
        &[0x69, 0x01, 0x00],
        a,
        FlagState {
            c: Some(true),
            ..native()
        },
    );
    assert_eq!(adc.flags_after.c, None);
    let jsr = dec(
        &[0x20, 0x00, 0x90],
        a,
        FlagState {
            c: Some(true),
            ..native()
        },
    );
    assert_eq!(jsr.flags_after.c, None);
}

#[test]
fn peepholes_recover_dbr_and_dp() {
    let a = SnesAddress::new(0x80, 0x8000);
    let unknown = FlagState {
        dbr: None,
        dp: None,
        ..native()
    };
    let seq = run(&[0x4B, 0xAB], a, unknown); // PHK; PLB
    assert_eq!(seq[1].flags_after.dbr, Some(0x80));
    let seq = run(
        &[0xA9, 0x7E, 0x48, 0xAB],
        a,
        FlagState { m: true, ..unknown },
    ); // LDA #$7E; PHA; PLB
    assert_eq!(seq[2].flags_after.dbr, Some(0x7E));
    let seq = run(&[0xA9, 0x00, 0x1F, 0x5B], a, unknown); // LDA #$1F00; TCD
    assert_eq!(seq[1].flags_after.dp, Some(0x1F00));
    let seq = run(&[0xF4, 0x00, 0x21, 0x2B], a, unknown); // PEA $2100; PLD
    assert_eq!(seq[1].flags_after.dp, Some(0x2100));
    let seq = run(&[0xF4, 0x7F, 0x7E, 0xAB, 0xAB], a, unknown); // PEA $7E7F; PLB; PLB
    assert_eq!(seq[1].flags_after.dbr, Some(0x7F));
    assert_eq!(seq[2].flags_after.dbr, Some(0x7E));
    // A bare PLB/PLD/TCD forgets.
    let plb = dec(&[0xAB], a, native());
    assert_eq!(plb.flags_after.dbr, None);
    let tcd = dec(&[0x5B], a, native());
    assert_eq!(tcd.flags_after.dp, None);
}

#[test]
fn target_rules() {
    let a = SnesAddress::new(0x80, 0x8440);
    // Backward branch: BPL $843C from $8440 (displacement -5 from $8441... BPL is at $843F in the listing).
    let bpl = dec(&[0x10, 0xFB], SnesAddress::new(0x80, 0x843F), native());
    let t = bpl.target.unwrap();
    assert_eq!(t.address, SnesAddress::new(0x80, 0x843C));
    assert_eq!(t.kind, TargetKind::Code);
    assert!(t.certain);
    // JML literal, mirror bank as written.
    let jml = dec(
        &[0x5C, 0x23, 0x84, 0x80],
        SnesAddress::new(0x00, 0x841F),
        FlagState::RESET,
    );
    assert_eq!(jml.target.unwrap().address, SnesAddress::new(0x80, 0x8423));
    // JSR abs → program bank.
    let jsr = dec(&[0x20, 0x00, 0x90], a, native());
    assert_eq!(jsr.target.unwrap().address, SnesAddress::new(0x80, 0x9000));
    // Data abs with DBR known / unknown.
    let sta = dec(
        &[0x8D, 0x0D, 0x42],
        a,
        FlagState {
            dbr: Some(0x7E),
            ..native()
        },
    );
    assert_eq!(sta.target.unwrap().address, SnesAddress::new(0x7E, 0x420D));
    assert_eq!(sta.assumptions & ASSUMED_DBR, 0);
    let sta = dec(
        &[0x8D, 0x0D, 0x42],
        a,
        FlagState {
            dbr: None,
            ..native()
        },
    );
    let t = sta.target.unwrap();
    assert_eq!(t.address, SnesAddress::new(0x80, 0x420D));
    assert!(!t.certain);
    assert_ne!(sta.assumptions & ASSUMED_DBR, 0);
    // Indexed keeps the base, uncertain.
    let ldx = dec(&[0xBD, 0x00, 0x90], a, native());
    let t = ldx.target.unwrap();
    assert_eq!(t.address, SnesAddress::new(0x80, 0x9000));
    assert!(!t.certain);
    // Direct page known / unknown.
    let sta = dec(
        &[0x85, 0x86],
        a,
        FlagState {
            dp: Some(0x0100),
            ..native()
        },
    );
    assert_eq!(sta.target.unwrap().address, SnesAddress::new(0x00, 0x0186));
    let sta = dec(
        &[0x85, 0x86],
        a,
        FlagState {
            dp: None,
            ..native()
        },
    );
    assert!(sta.target.is_none());
    assert_ne!(sta.assumptions & ASSUMED_DP, 0);
    // Pointers.
    let jmp = dec(&[0x6C, 0x00, 0x1F], a, native());
    let t = jmp.target.unwrap();
    assert_eq!(
        (t.address, t.kind, t.certain),
        (SnesAddress::new(0x00, 0x1F00), TargetKind::Pointer, true)
    );
    let jmpx = dec(&[0x7C, 0x00, 0x90], a, native());
    let t = jmpx.target.unwrap();
    assert_eq!(
        (t.address, t.kind, t.certain),
        (SnesAddress::new(0x80, 0x9000), TargetKind::Pointer, false)
    );
    let pei = dec(&[0xD4, 0x10], a, native());
    assert_eq!(pei.target.unwrap().kind, TargetKind::Pointer);
    // Stack, block move, immediates, PEA: none.
    for b in [
        [0xA3, 0x01, 0, 0],
        [0x54, 0x7E, 0x80, 0],
        [0xA9, 0x00, 0x00, 0],
        [0xF4, 0x34, 0x12, 0],
    ] {
        assert!(dec(&b, a, native()).target.is_none(), "{b:?}");
    }
}

#[test]
fn bank_wrap_is_flagged() {
    let i = dec(
        &[0x5C, 0x00, 0x80, 0x80],
        SnesAddress::new(0x80, 0xFFFE),
        native(),
    );
    assert_ne!(i.assumptions & BANK_WRAP, 0);
    assert_eq!(i.next_address(), SnesAddress::new(0x80, 0x0002));
    let ok = dec(&[0xEA], SnesAddress::new(0x80, 0xFFFF), native());
    assert_eq!(ok.assumptions & BANK_WRAP, 0);
}

#[test]
fn fixture_boot_code_decodes() {
    let seq = run(
        &fixtures::BOOT_CODE,
        SnesAddress::new(0x00, 0x8000),
        FlagState::RESET,
    );
    let names: Vec<&str> = seq.iter().map(|i| i.mnemonic.as_str()).collect();
    assert_eq!(
        names,
        [
            "SEI", "CLC", "XCE", "SEP", "LDA", "STA", "BRA", "NOP", "NOP", "RTI"
        ]
    );
    assert!(seq[1].flags_after.e);
    assert!(!seq[2].flags_after.e, "e flips after XCE");
    assert_eq!(seq[4].len, 2, "LDA #$80 under M=1");
    assert_eq!(
        seq[5].target.unwrap().address,
        SnesAddress::new(0x00, 0x2100)
    );
    assert_eq!(
        seq[6].target.unwrap().address,
        SnesAddress::new(0x00, 0x800A)
    );
    assert_eq!(seq[9].mnemonic, Mnemonic::RTI);
}
