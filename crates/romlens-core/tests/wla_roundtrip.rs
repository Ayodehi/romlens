//! Independent assembler oracle: assemble `tests/data/all_opcodes.wla.asm`
//! with WLA-DX, then check that the decoder reads back every instruction with
//! the same opcode, length and operand. Skipped when `wla-65816` is absent.
//!
//! Learned here: WLA-DX writes `MVN src,dst` as bytes `dst, src`, the same
//! order the decoder uses; a numeric branch operand is the displacement.

use std::path::Path;
use std::process::Command;

use romlens_core::cpu65816::{FlagState, decode};
use romlens_core::fixtures;
use romlens_core::{FileOffset, SnesAddress};

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("-h")
        .output()
        .map(|_| true)
        .unwrap_or(false)
}

#[test]
fn wla_dx_agrees_with_the_decoder() {
    if !have("wla-65816") || !have("wlalink") {
        eprintln!("skipped: wla-65816 / wlalink not installed");
        return;
    }
    let dir = std::env::temp_dir().join(format!("romlens-wla-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/all_opcodes.wla.asm");
    let asm = dir.join("all_opcodes.asm");
    std::fs::copy(&src, &asm).unwrap();
    let obj = dir.join("all.o");
    let status = Command::new("wla-65816")
        .current_dir(&dir)
        .args(["-o", obj.to_str().unwrap(), asm.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "wla-65816 failed");
    std::fs::write(dir.join("link.txt"), "[objects]\nall.o\n").unwrap();
    let bin = dir.join("all.bin");
    let status = Command::new("wlalink")
        .current_dir(&dir)
        .args(["-r", "link.txt", bin.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "wlalink failed");
    let bytes = std::fs::read(&bin).unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    // The fixture holds exactly the byte stream the assembler should produce.
    let fixture = fixtures::all_opcodes_lorom();
    let flags = FlagState {
        m: false,
        x: false,
        e: false,
        ..FlagState::NATIVE_VECTOR
    };
    let mut pos = 0usize;
    for op in 0..=255u8 {
        let insn = decode(
            &bytes[pos..],
            SnesAddress::new(0x00, 0x8000 + pos as u16),
            FileOffset(pos as u32),
            flags,
        )
        .unwrap();
        assert_eq!(insn.opcode, op, "opcode at {pos:#x}");
        let expect = &fixture[pos..pos + insn.len as usize];
        assert_eq!(
            insn.bytes(),
            expect,
            "bytes of ${op:02X} ({})",
            insn.mnemonic
        );
        pos += insn.len as usize;
    }
    assert_eq!(pos, 571);
    assert!(bytes[pos..].iter().all(|&b| b == 0));
}
