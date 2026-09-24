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
    let Some(cc) = compiler() else {
        eprintln!("skipped: no C compiler");
        return;
    };
    for level in LEVELS {
        let dir = scratch(&format!("semantic-{}", level.name()));
        std::fs::write(dir.join("snes.h"), decompile::snes_h()).unwrap();
        let mut includes = String::new();
        for at in [0x8020u16, 0x8030, 0x8040, 0x8050] {
            let opts = DecompileOptions {
                level,
                names: false,
                ..Default::default()
            };
            let d = decompile::decompile(&rom, &project, &snap, SnesAddress::new(0, at), &opts)
                .unwrap();
            let name = format!("r{at:04X}.c");
            std::fs::write(dir.join(&name), &d.text).unwrap();
            includes.push_str(&format!("#include \"{name}\"\n"));
        }
        let rom_bytes: Vec<String> = rom.bytes()[..0x100].iter().map(|b| b.to_string()).collect();
        let main = RUNTIME
            .replace("/*INCLUDES*/", &includes)
            .replace("/*ROM*/", &rom_bytes.join(","));
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
        let program = decompile::program(&rom, &snap, &opts);
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
/// ROM at `$00:8000`, then each routine checked on inputs. Only what a caller
/// reads is checked: the boot never reads `SUB_008030`'s carry or
/// `SUB_008040`'s high byte of A, so from the clean level on neither is kept.
const RUNTIME: &str = r#"
#include <stdint.h>
#include <stdio.h>
#include <string.h>
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
u16 bcd_add(u16 a, u16 b, int bits) { (void)bits; return a + b; }
u16 bcd_sub(u16 a, u16 b, int bits) { (void)bits; return a - b; }
void WAI(void) {}
void STP(void) {}
void BRK(u8 n) { (void)n; }
void COP(u8 n) { (void)n; }
/*INCLUDES*/
static const uint8_t rom[] = {/*ROM*/};
static int failures;
#define CHECK(what, got, want) do { unsigned g = (got), w = (want); \
    if (g != w) { printf("%s: got 0x%X, want 0x%X\n", what, g, w); failures++; } } while (0)
int main(void) {
    memcpy(&mem[0x8000], rom, sizeof rom);

    memset(&mem[0x0200], 0xAA, 0x20);
    X = 0x33;
    SUB_008020();
    for (int i = 0; i < 16; i++) CHECK("cleared", mem[0x0200 + i], 0);
    CHECK("past the end", mem[0x0210], 0xAA);
    CHECK("X after the loop", X, 0xFF);

    MEM16(0x10) = 0x1234; MEM16(0x12) = 0x0FF0; A = 0xBEEF;
    SUB_008030();
    CHECK("sum", MEM16(0x14), 0x2224);
    MEM16(0x10) = 0xFFFF; MEM16(0x12) = 2;
    SUB_008030();
    CHECK("wrapped sum", MEM16(0x14), 1);

    mem[0x20] = 5; mem[0x21] = 9; A = 0x1200;
    SUB_008040();
    CHECK("larger (second)", mem[0x22], 9);
    mem[0x20] = 200;
    SUB_008040();
    CHECK("larger (first)", mem[0x22], 200);

    X = 3; S = 0x01FF;
    SUB_008050();
    CHECK("table entry", mem[0x0300], 8);
    CHECK("X restored", X, 3);
    CHECK("S balanced", S, 0x01FF);
    return failures != 0;
}
"#;
