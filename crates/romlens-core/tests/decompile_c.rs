//! The decompiler's output is valid C, and means what the code does.
//!
//! Both checks need a C compiler (`$CC` or `cc`) and print "skipped" and
//! pass without one, as the development-ROM tests do without the ROM.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use romlens_core::analysis::{AnalysisControl, AnalysisSnapshot, analyze};
use romlens_core::cpu65816::{FlagState, Mnemonic, decode};
use romlens_core::decompile::{
    self, Callee, DecompileOptions, Dest, Function, Level, Step, Transfer,
};
use romlens_core::fixtures;
use romlens_core::model::Project;
use romlens_core::{FileOffset, RomImage, SnesAddress};

const LEVELS: [Level; 3] = [Level::Lift, Level::Clean, Level::Full];

fn compiler() -> Option<String> {
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_owned());
    Command::new(&cc)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| cc)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("romlens-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Compile `files` with `-fsyntax-only -Wall`; the diagnostics, if any.
fn syntax_errors(cc: &str, dir: &Path, files: &[PathBuf]) -> String {
    let mut all = String::new();
    for chunk in files.chunks(128) {
        let out = Command::new(cc)
            .args([
                "-std=c11",
                "-fsyntax-only",
                "-Wall",
                "-Wextra",
                "-Wno-unused-parameter",
            ])
            .arg("-I")
            .arg(dir)
            .args(chunk)
            .output()
            .unwrap();
        all.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    all
}

fn setup(bytes: Vec<u8>) -> (RomImage, Project, AnalysisSnapshot) {
    let rom = RomImage::from_bytes(bytes, "t.sfc").unwrap();
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    (rom, project, snap)
}

/// Every opcode, alone as a routine, in three states: 8-bit native with the
/// data bank and direct page known, 16-bit with both unknown, and emulation
/// mode. Each lifts, and the result compiles without a warning.
#[test]
fn every_opcode_lifts_to_valid_c() {
    let (rom, project, snap) = setup(fixtures::all_opcodes_lorom());
    let states = [
        FlagState {
            m: true,
            x: true,
            e: false,
            dbr: Some(0x7E),
            dp: Some(0),
            c: Some(false),
        },
        FlagState {
            m: false,
            x: false,
            e: false,
            dbr: None,
            dp: None,
            c: None,
        },
        FlagState::RESET,
    ];
    let at = SnesAddress::new(0, 0x8000);
    let mut units = Vec::new();
    for (si, flags) in states.iter().enumerate() {
        for op in 0..=255u8 {
            let bytes = [op, 0x12, 0x34, 0x56];
            let insn = decode(&bytes, at, FileOffset(0), *flags).unwrap();
            let end = || Dest::Unknown("end of the test".into());
            let m = insn.mnemonic;
            let transfer = match m {
                _ if m.is_branch() => Transfer::Branch {
                    taken: end(),
                    next: end(),
                },
                Mnemonic::JSR | Mnemonic::JSL => Transfer::Call {
                    callee: match insn.target {
                        Some(t) if !insn.mode.is_indirect() => Callee::Direct(t.address),
                        _ => Callee::Indirect,
                    },
                    inline: 0,
                    next: end(),
                },
                Mnemonic::RTS | Mnemonic::RTL | Mnemonic::RTI => Transfer::Return,
                Mnemonic::BRK | Mnemonic::STP => Transfer::Halt,
                _ if m.is_jump() => Transfer::Jump(end()),
                _ => Transfer::Next(end()),
            };
            let f = Function {
                entry: at,
                entry_offset: FileOffset(0),
                steps: vec![Step { insn, transfer }],
                truncated: false,
            };
            let opts = DecompileOptions {
                level: Level::Lift,
                ..Default::default()
            };
            let d = decompile::render(&rom, &project, &snap, &f, &opts);
            assert!(d.text.contains("#include \"snes.h\""));
            units.push((format!("op{op:02X}_s{si}.c"), d.text));
        }
    }
    let Some(cc) = compiler() else {
        eprintln!("skipped the syntax check: no C compiler");
        return;
    };
    let dir = scratch("opcodes");
    std::fs::write(dir.join("snes.h"), decompile::snes_h()).unwrap();
    let files: Vec<PathBuf> = units
        .iter()
        .map(|(name, text)| {
            let p = dir.join(name);
            std::fs::write(&p, text).unwrap();
            p
        })
        .collect();
    let errors = syntax_errors(&cc, &dir, &files);
    assert!(errors.is_empty(), "{errors}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The fixture's routines, compiled at every level against an `snes.h` whose
/// memory is an array, run on inputs; the memory and registers they leave
/// must be what the 65816 would leave.
#[test]
fn the_routines_mean_what_the_code_does() {
    let (rom, project, snap) = setup(fixtures::routines_lorom());
    run_checks(
        "semantic",
        &rom,
        &project,
        &snap,
        &[0x8020, 0x8030, 0x8040, 0x8050],
        ROUTINE_CHECKS,
    );
}

/// Tests that choose between the same two ways, read as `&&` and `||`, and
/// a pushed word pulled a byte at a time, still do what the code does:
///
/// ```text
/// $8000  SEI; CLC; XCE; REP #$10; SEP #$20
/// $8007  JSR $8020; STY $30; JSR each of the rest; ROL $42; BRA self
///
/// $8020  INY / BEQ $8027          ; Y += 2, into the next bank past $FFFF
/// $8023  INY / BEQ $8027
/// $8026  RTS
/// $8027  INC $02 / PEI ($01) / PLB / PLB / LDY #$8000 / RTS
///
/// $8040  LDA $10 / BEQ $804C      ; $12 = 1 when $10 and $11 are both set
/// $8044  LDA $11 / BEQ $804C
/// $8048  LDA #$01 / STA $12
/// $804C  RTS
///
/// $8060  LDA $20 / CMP #$03 / BEQ $806D   ; $21 = ($20 is 3 or 5)
/// $8066  CMP #$05 / BEQ $806D
/// $806A  STZ $21 / RTS
/// $806D  LDA #$01 / STA $21 / RTS
///
/// $8080  PEA $1234 / PLB / PLB / RTS      ; DBR = $12
///
/// $80A0  LDA $40 / AND $41 / BNE $80A8    ; carry = ($40 & $41) != 0
/// $80A6  CLC / RTS
/// $80A8  SEC / RTS
/// ```
fn conditions_rom() -> RomImage {
    let mut code = vec![0u8; 0x100];
    let mut put = |at: u16, b: &[u8]| {
        let i = (at - 0x8000) as usize;
        code[i..i + b.len()].copy_from_slice(b);
    };
    put(
        0x8000,
        &[
            0x78, 0x18, 0xFB, 0xC2, 0x10, 0xE2, 0x20, 0x20, 0x20, 0x80, 0x84, 0x30, 0x20, 0x40,
            0x80, 0x20, 0x60, 0x80, 0x20, 0x80, 0x80, 0x20, 0xA0, 0x80, 0x26, 0x42, 0x80, 0xFE,
        ],
    );
    put(
        0x8020,
        &[
            0xC8, 0xF0, 0x04, 0xC8, 0xF0, 0x01, 0x60, 0xE6, 0x02, 0xD4, 0x01, 0xAB, 0xAB, 0xA0,
            0x00, 0x80, 0x60,
        ],
    );
    put(
        0x8040,
        &[
            0xA5, 0x10, 0xF0, 0x08, 0xA5, 0x11, 0xF0, 0x04, 0xA9, 0x01, 0x85, 0x12, 0x60,
        ],
    );
    put(
        0x8060,
        &[
            0xA5, 0x20, 0xC9, 0x03, 0xF0, 0x07, 0xC9, 0x05, 0xF0, 0x03, 0x64, 0x21, 0x60, 0xA9,
            0x01, 0x85, 0x21, 0x60,
        ],
    );
    put(0x8080, &[0xF4, 0x34, 0x12, 0xAB, 0xAB, 0x60]);
    put(
        0x80A0,
        &[0xA5, 0x40, 0x25, 0x41, 0xD0, 0x02, 0x18, 0x60, 0x38, 0x60],
    );
    put(0x80F0, &[0x40]);
    let mut vectors = [0x80F0; 12];
    vectors[10] = 0x8000;
    RomImage::from_bytes(
        fixtures::build_custom(
            romlens_core::MappingMode::LoRom,
            0x8000,
            false,
            &code,
            "CONDITIONS",
            vectors,
        ),
        "c.sfc",
    )
    .unwrap()
}

#[test]
fn combined_tests_mean_what_the_branches_do() {
    let rom = conditions_rom();
    let (rom, project, snap) = setup(rom.bytes().to_vec());
    let full = |at: u16| {
        decompile::decompile(
            &rom,
            &project,
            &snap,
            SnesAddress::new(0, at),
            &DecompileOptions::default(),
        )
        .unwrap()
        .text
    };
    // As C is written: one test, no goto, the bank from memory in one line.
    let advance = full(0x8020);
    assert!(advance.contains("if (++y == 0 || ++y == 0) {"), "{advance}");
    assert!(advance.contains("DBR = ADDR_7E0002;"), "{advance}");
    assert!(
        !advance.contains("goto") && !advance.contains("pull8"),
        "{advance}"
    );
    let both = full(0x8040);
    assert!(
        both.contains("if (ADDR_7E0010 != 0 && ADDR_7E0011 != 0) {"),
        "{both}"
    );
    let either = full(0x8060);
    assert!(either.contains("||"), "{either}");
    assert!(!either.contains("goto"), "{either}");
    // Two ways out alike but for the carry: one test, one return.
    let carry = full(0x80A0);
    assert!(carry.contains(" != 0;"), "{carry}");
    assert!(!carry.contains("if ("), "{carry}");
    let bank = full(0x8080);
    assert!(bank.contains("DBR = 0x12;"), "{bank}");
    run_checks(
        "conditions",
        &rom,
        &project,
        &snap,
        &[0x8020, 0x8040, 0x8060, 0x8080, 0x80A0],
        CONDITION_CHECKS,
    );
}

/// Each routine at `entries` compiled at every level into one program with
/// `checks` as its `main`, which must build and pass.
fn run_checks(
    name: &str,
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    entries: &[u16],
    checks: &str,
) {
    let Some(cc) = compiler() else {
        eprintln!("skipped: no C compiler");
        return;
    };
    for level in LEVELS {
        let dir = scratch(&format!("{name}-{}", level.name()));
        std::fs::write(dir.join("snes.h"), decompile::snes_h()).unwrap();
        let mut includes = String::new();
        let mut shims = String::new();
        for &at in entries {
            let opts = DecompileOptions {
                level,
                names: false,
                ..Default::default()
            };
            let d =
                decompile::decompile(rom, project, snap, SnesAddress::new(0, at), &opts).unwrap();
            let name = format!("r{at:04X}.c");
            std::fs::write(dir.join(&name), &d.text).unwrap();
            includes.push_str(&format!("#include \"{name}\"\n"));
            // At `full` a routine takes and returns its registers: call it
            // from the globals the checks read.
            let entry = SnesAddress::new(0, at);
            let shim = format!("call_{at:04X}");
            let program = decompile::program(rom, project, snap, &opts);
            match program.abis.get(&entry).filter(|_| level == Level::Full) {
                Some(abi) => shims.push_str(&abi.global_shim(&shim, &d.name)),
                None => shims.push_str(&format!("static void {shim}(void) {{ {}(); }}\n", d.name)),
            }
        }
        includes.push_str(&shims);
        let rom_bytes: Vec<String> = rom.bytes()[..0x100].iter().map(|b| b.to_string()).collect();
        let main = RUNTIME
            .replace("/*INCLUDES*/", &includes)
            .replace("/*ROM*/", &rom_bytes.join(","))
            .replace("/*CHECKS*/", checks);
        std::fs::write(dir.join("main.c"), main).unwrap();
        let exe = dir.join("main");
        let out = Command::new(&cc)
            .args(["-std=c11", "-Wall", "-O0", "-o"])
            .arg(&exe)
            .arg("-I")
            .arg(&dir)
            .arg(dir.join("main.c"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{} level does not build:\n{}",
            level.name(),
            String::from_utf8_lossy(&out.stderr)
        );
        let run = Command::new(&exe).output().unwrap();
        assert!(
            run.status.success(),
            "{} level:\n{}{}",
            level.name(),
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Every routine on the development ROM, at every level, compiles without
/// a warning.
#[test]
fn every_routine_on_the_development_rom_is_valid_c() {
    let Some(rom) = common::dev_rom() else {
        return;
    };
    let Some(cc) = compiler() else {
        eprintln!("skipped: no C compiler");
        return;
    };
    let (rom, project, snap) = setup(rom.bytes().to_vec());
    for level in LEVELS {
        let opts = DecompileOptions {
            level,
            ..Default::default()
        };
        let program = decompile::program(&rom, &project, &snap, &opts);
        let dir = scratch(&format!("devrom-{}", level.name()));
        std::fs::write(dir.join("snes.h"), decompile::snes_h()).unwrap();
        let files: Vec<PathBuf> = program
            .units
            .iter()
            .map(|(a, u)| {
                let d = decompile::render_with(&rom, &project, &snap, &u.f, &opts, Some(&program));
                let p = dir.join(format!("f_{:06X}.c", a.as_u24()));
                std::fs::write(&p, d.text).unwrap();
                p
            })
            .collect();
        assert!(files.len() > 700, "{}", files.len());
        let errors = syntax_errors(&cc, &dir, &files);
        assert!(errors.is_empty(), "{} level:\n{errors}", level.name());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A runtime over a flat 16 MB array, with the fixture's first 256 bytes of
/// ROM at `$00:8000`, then the checks.
const RUNTIME: &str = r#"
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
static uint8_t mem[1 << 24];
#define MEM8(a) (mem[(uint32_t)(a) & 0xFFFFFF])
#define MEM16(a) (*(uint16_t *)&mem[(uint32_t)(a) & 0xFFFFFF])
#include "snes.h"
u16 A, X, Y, S = 0x01FF, D;
u8 DBR;
u8 N, V, Z, C;
void SEI(void) {}
void CLI(void) {}
void SED(void) {}
void CLD(void) {}
void native_mode(void) {}
void emulation_mode(void) {}
void push8(u8 v) { mem[S--] = v; }
void push16(u16 v) { push8(v >> 8); push8(v & 0xFF); }
u8 pull8(void) { return mem[++S]; }
u16 pull16(void) { u16 lo = pull8(); return lo | (u16)(pull8() << 8); }
void PHP(void) { push8((u8)(N << 7 | V << 6 | Z << 1 | C)); }
void PLP(void) { u8 p = pull8(); N = p >> 7 & 1; V = p >> 6 & 1; Z = p >> 1 & 1; C = p & 1; }
void mvn(u8 d, u8 s) { do { mem[d << 16 | Y++] = mem[s << 16 | X++]; } while (A-- != 0); }
void mvp(u8 d, u8 s) { do { mem[d << 16 | Y--] = mem[s << 16 | X--]; } while (A-- != 0); }
void mvn8(u8 d, u8 s) { do { mem[d << 16 | Y] = mem[s << 16 | X]; X = (X + 1) & 0xFF; Y = (Y + 1) & 0xFF; } while (A-- != 0); }
void mvp8(u8 d, u8 s) { do { mem[d << 16 | Y] = mem[s << 16 | X]; X = (X - 1) & 0xFF; Y = (Y - 1) & 0xFF; } while (A-- != 0); }
u16 bcd_add(u16 a, u16 b, int bits) { (void)bits; return a + b; }
u16 bcd_sub(u16 a, u16 b, int bits) { (void)bits; return a - b; }
void WAI(void) {}
void STP(void) {}
void BRK(u8 n) { (void)n; }
void COP(u8 n) { (void)n; }
_Noreturn void table_overrun(void) { printf("table overrun\n"); _Exit(2); }
/*INCLUDES*/
static const uint8_t rom[] = {/*ROM*/};
static int failures;
#define CHECK(what, got, want) do { unsigned g = (got), w = (want); \
    if (g != w) { printf("%s: got 0x%X, want 0x%X\n", what, g, w); failures++; } } while (0)
int main(void) {
    memcpy(&mem[0x8000], rom, sizeof rom);
/*CHECKS*/
    return failures != 0;
}
"#;

/// `routines_lorom`'s routines on inputs. Only what a caller reads is
/// checked: the boot never reads `SUB_008030`'s carry or `SUB_008040`'s high
/// byte of A, so from the clean level on neither is kept.
const ROUTINE_CHECKS: &str = r#"

    memset(&mem[0x0200], 0xAA, 0x20);
    X = 0x33;
    call_8020();
    for (int i = 0; i < 16; i++) CHECK("cleared", mem[0x0200 + i], 0);
    CHECK("past the end", mem[0x0210], 0xAA);
    CHECK("X after the loop", X, 0xFF);

    MEM16(0x10) = 0x1234; MEM16(0x12) = 0x0FF0; A = 0xBEEF;
    call_8030();
    CHECK("sum", MEM16(0x14), 0x2224);
    MEM16(0x10) = 0xFFFF; MEM16(0x12) = 2;
    call_8030();
    CHECK("wrapped sum", MEM16(0x14), 1);

    mem[0x20] = 5; mem[0x21] = 9; A = 0x1200;
    call_8040();
    CHECK("larger (second)", mem[0x22], 9);
    mem[0x20] = 200;
    call_8040();
    CHECK("larger (first)", mem[0x22], 200);

    X = 3; S = 0x01FF;
    call_8050();
    CHECK("table entry", mem[0x0300], 8);
    CHECK("X restored", X, 3);
    CHECK("S balanced", S, 0x01FF);
"#;

/// `conditions_rom`'s routines on inputs.
const CONDITION_CHECKS: &str = r#"
    Y = 0x1234; DBR = 0x80; mem[2] = 5;
    call_8020();
    CHECK("stepped twice", Y, 0x1236);
    CHECK("bank kept", DBR, 0x80);
    CHECK("bank byte kept", mem[2], 5);
    Y = 0xFFFE;
    call_8020();
    CHECK("second step wraps", Y, 0x8000);
    CHECK("next bank", mem[2], 6);
    CHECK("data bank", DBR, 6);
    Y = 0xFFFF;
    call_8020();
    CHECK("first step wraps", Y, 0x8000);
    CHECK("next bank again", DBR, 7);
    CHECK("S balanced", S, 0x01FF);

    int both[4][2] = {{0, 0}, {1, 0}, {0, 1}, {1, 1}};
    for (int i = 0; i < 4; i++) {
        mem[0x10] = both[i][0]; mem[0x11] = both[i][1]; mem[0x12] = 0;
        call_8040();
        CHECK("both set", mem[0x12], both[i][0] && both[i][1]);
    }

    for (int v = 0; v < 8; v++) {
        mem[0x20] = v; mem[0x21] = 0xAA;
        call_8060();
        CHECK("3 or 5", mem[0x21], v == 3 || v == 5);
    }

    DBR = 0;
    call_8080();
    CHECK("PEA bank", DBR, 0x12);
    CHECK("S balanced after PEA", S, 0x01FF);

    for (int i = 0; i < 4; i++) {
        mem[0x40] = i & 1 ? 0x0C : 0x03; mem[0x41] = i & 2 ? 0x04 : 0x10; C = 1 - (i & 1);
        call_80A0();
        CHECK("carry is the test", C, (mem[0x40] & mem[0x41]) != 0);
    }
"#;
