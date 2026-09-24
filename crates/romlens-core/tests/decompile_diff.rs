//! Differential test: every level of the decompiler must do what the `lift`
//! level does, which is each instruction written out in full.
//!
//! Each routine is compiled at every level into one program, with its calls
//! going to stubs that mix the registers deterministically. The program runs
//! each routine from the same pseudo-random registers and memory at each
//! level, in a child process with a quarter-second alarm, and prints the
//! registers it leaves and every byte of memory it changed. A difference
//! between levels is a clean-up or structuring bug.
//!
//! Needs a C compiler and a POSIX system (fork, setitimer); it prints "skipped"
//! and passes otherwise. The development-ROM half also needs
//! `ROMLENS_ROM_DIR`.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;

use romlens_core::analysis::{AnalysisControl, AnalysisSnapshot, analyze};
use romlens_core::decompile::{self, DecompileOptions, Level};
use romlens_core::fixtures;
use romlens_core::model::Project;
use romlens_core::{RomImage, SnesAddress};

const SEEDS: u32 = 4;

fn compiler() -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_owned());
    Command::new(&cc)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| cc)
}

/// One run's result: the registers, then changed memory.
type Outcome = (String, BTreeMap<u32, u8>);

struct Program {
    exe: PathBuf,
}

fn build(
    cc: &str,
    dir: &std::path::Path,
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    entries: &[SnesAddress],
    level: Level,
) -> Option<Program> {
    let opts = DecompileOptions {
        level,
        names: false,
        ..Default::default()
    };
    let all = decompile::entries(snap);
    let mut src = String::from(PRELUDE);
    let mut funcs: BTreeSet<String> = BTreeSet::new();
    let mut tables: BTreeSet<String> = BTreeSet::new();
    let mut table_rows = Vec::new();
    for (i, &e) in entries.iter().enumerate() {
        let Ok(f) = decompile::discover(rom, snap, &all, e) else {
            continue;
        };
        let x8 = f.steps[f.index_of(f.entry_offset).unwrap()]
            .insn
            .flags_before
            .eff_x();
        let d = decompile::render(rom, project, snap, &f, &opts);
        for line in d.text.lines() {
            if let Some(name) = line
                .strip_prefix("void ")
                .and_then(|l| l.strip_suffix("(void);"))
            {
                funcs.insert(name.to_owned());
            }
            if let Some(name) = line
                .strip_prefix("extern void (*const ")
                .and_then(|l| l.strip_suffix("[])(void);"))
            {
                tables.insert(name.to_owned());
            }
        }
        src.push_str(&format!("#define {} entry_{i}\n", d.name));
        src.push_str(&d.text);
        src.push_str(&format!("#undef {}\n", d.name));
        table_rows.push(format!("{{entry_{i}, {}, {}}}", x8 as u8, i));
    }
    src.push_str("\n/* Stubs for everything called. */\n");
    for (k, name) in funcs.iter().enumerate() {
        src.push_str(&format!("void {name}(void) {{ stub({}); }}\n", k + 1));
    }
    for (k, name) in tables.iter().enumerate() {
        src.push_str(&format!(
            "static void {name}_stub(void) {{ stub({}); }}\n\
             void (*const {name}[32768])(void) = {{[0 ... 32767] = {name}_stub}};\n",
            0x10000 + k
        ));
    }
    src.push_str(&format!(
        "static const struct entry entries[] = {{{}}};\n\
         static const int n_entries = {};\n",
        table_rows.join(",\n"),
        table_rows.len()
    ));
    src.push_str(MAIN);
    let path = dir.join(format!("diff-{}.c", level.name()));
    std::fs::write(&path, src).unwrap();
    std::fs::write(dir.join("snes.h"), decompile::snes_h()).unwrap();
    let exe = dir.join(format!("diff-{}", level.name()));
    let out = Command::new(cc)
        .args(["-std=gnu11", "-O0", "-w", "-o"])
        .arg(&exe)
        .arg("-I")
        .arg(dir)
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the {} level does not build:\n{}",
        level.name(),
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .take(30)
            .collect::<Vec<_>>()
            .join("\n")
    );
    Some(Program { exe })
}

/// Run every routine under every seed: (routine index, seed) to outcome;
/// timeouts and crashes are absent.
fn run(p: &Program, rom_path: &std::path::Path) -> BTreeMap<(usize, u32), Outcome> {
    let out = Command::new(&p.exe)
        .arg(rom_path)
        .arg(SEEDS.to_string())
        .output()
        .unwrap();
    let mut results = BTreeMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = line.split(" | ");
        let head = parts.next().unwrap_or("");
        let mut h = head.split_whitespace();
        if h.next() != Some("R") {
            continue;
        }
        let i: usize = h.next().unwrap().parse().unwrap();
        let s: u32 = h.next().unwrap().parse().unwrap();
        let regs: Vec<&str> = h.collect();
        let mut mem = BTreeMap::new();
        for pair in parts.next().unwrap_or("").split_whitespace() {
            let (a, v) = pair.split_once(':').unwrap();
            mem.insert(
                u32::from_str_radix(a, 16).unwrap(),
                u8::from_str_radix(v, 16).unwrap(),
            );
        }
        results.insert((i, s), (regs.join(" "), mem));
    }
    results
}

fn compare(
    name: &str,
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    entries: &[SnesAddress],
) -> Option<(usize, Vec<String>)> {
    let cc = compiler()?;
    let dir = std::env::temp_dir().join(format!("romlens-diff-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rom_path = dir.join("rom.sfc");
    std::fs::write(&rom_path, rom.bytes()).unwrap();
    let base = run(
        &build(&cc, &dir, rom, project, snap, entries, Level::Lift)?,
        &rom_path,
    );
    let mut compared = 0;
    let mut bad = Vec::new();
    for level in [Level::Clean, Level::Full] {
        let got = run(
            &build(&cc, &dir, rom, project, snap, entries, level)?,
            &rom_path,
        );
        for (key, want) in &base {
            let Some(have) = got.get(key) else {
                continue;
            };
            compared += 1;
            if have != want {
                let mut diff: Vec<String> = Vec::new();
                if have.0 != want.0 {
                    diff.push(format!("registers {} (lift: {})", have.0, want.0));
                }
                let addrs: BTreeSet<&u32> = have.1.keys().chain(want.1.keys()).collect();
                for a in addrs {
                    let (h, w) = (have.1.get(a), want.1.get(a));
                    if h != w {
                        diff.push(format!("${a:06X} {h:02X?} (lift: {w:02X?})"));
                    }
                }
                bad.push(format!(
                    "{} at {} level, seed {}: {}",
                    entries[key.0],
                    level.name(),
                    key.1,
                    diff.into_iter().take(6).collect::<Vec<_>>().join("; ")
                ));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Some((compared, bad))
}

fn setup(rom: RomImage) -> (RomImage, Project, AnalysisSnapshot) {
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    (rom, project, snap)
}

#[test]
fn every_level_does_what_lift_does_on_the_fixture() {
    let (rom, project, snap) =
        setup(RomImage::from_bytes(fixtures::routines_lorom(), "r.sfc").unwrap());
    let entries: Vec<SnesAddress> = decompile::entries(&snap).into_iter().collect();
    let Some((compared, bad)) = compare("fixture", &rom, &project, &snap, &entries) else {
        eprintln!("skipped: no C compiler or not POSIX");
        return;
    };
    assert!(compared >= 16, "{compared} runs compared");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn every_level_does_what_lift_does_on_the_development_rom() {
    let Some(rom) = common::dev_rom() else {
        return;
    };
    let (rom, project, snap) = setup(rom);
    let entries: Vec<SnesAddress> = decompile::entries(&snap).into_iter().collect();
    let Some((compared, bad)) = compare("devrom", &rom, &project, &snap, &entries) else {
        eprintln!("skipped: no C compiler or not POSIX");
        return;
    };
    eprintln!("{compared} runs compared, {} differ", bad.len());
    assert!(
        bad.is_empty(),
        "{}",
        bad.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// Memory is 16 MB, filled on first touch: ROM bytes (LoROM) where the
/// address maps to the ROM, else a hash of the seed and the address.
const PRELUDE: &str = r#"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>
#include <sys/wait.h>
#include <sys/time.h>
static uint8_t *mem, *touched, *rom;
static long rom_len;
static uint32_t seed;
static uint32_t hash(uint32_t a) {
    uint32_t h = a * 2654435761u ^ (seed + 1) * 40503u;
    h ^= h >> 15; h *= 2246822519u; h ^= h >> 13;
    return h;
}
static uint8_t initial(uint32_t a) {
    uint8_t bank = a >> 16; uint16_t off = a & 0xFFFF;
    if (off >= 0x8000 && bank != 0x7E && bank != 0x7F) {
        long f = (long)(bank & 0x7F) * 0x8000 + (off - 0x8000);
        if (f < rom_len) return rom[f];
    }
    return (uint8_t)hash(a);
}
static uint8_t *touch(uint32_t a) {
    a &= 0xFFFFFF;
    if (!(touched[a >> 3] & (1 << (a & 7)))) {
        touched[a >> 3] |= 1 << (a & 7);
        mem[a] = initial(a);
    }
    return &mem[a];
}
static uint16_t *touch16(uint32_t a) {
    touch(a); touch(a + 1);
    return (uint16_t *)&mem[a & 0xFFFFFF];
}
#define MEM8(a) (*touch((uint32_t)(a)))
#define MEM16(a) (*touch16((uint32_t)(a)))
#include "snes.h"
u16 A, X, Y, S, D;
u8 DBR;
u8 N, V, Z, C;
void SEI(void) {}
void CLI(void) {}
void SED(void) {}
void CLD(void) {}
void native_mode(void) {}
void emulation_mode(void) {}
void push8(u8 v) { MEM8(S) = v; S--; }
void push16(u16 v) { push8(v >> 8); push8(v & 0xFF); }
u8 pull8(void) { S++; return MEM8(S); }
u16 pull16(void) { u16 lo = pull8(); return lo | (u16)(pull8() << 8); }
void PHP(void) { push8((u8)(N << 7 | V << 6 | Z << 1 | C)); }
void PLP(void) { u8 p = pull8(); N = p >> 7 & 1; V = p >> 6 & 1; Z = p >> 1 & 1; C = p & 1; }
void mvn(u8 d, u8 s) { int n = 0; do { MEM8(d << 16 | Y) = MEM8(s << 16 | X); X++; Y++; } while (A-- != 0 && ++n < 0x10000); }
void mvp(u8 d, u8 s) { int n = 0; do { MEM8(d << 16 | Y) = MEM8(s << 16 | X); X--; Y--; } while (A-- != 0 && ++n < 0x10000); }
u16 bcd_add(u16 a, u16 b, int bits) { u32 r = a + b + C; C = r >> bits & 1; V = 0; return r; }
u16 bcd_sub(u16 a, u16 b, int bits) { u32 r = a - b - !C; C = !(r >> bits & 1); V = 0; return r; }
void WAI(void) {}
void STP(void) { _exit(3); }
void BRK(u8 n) { A ^= n; }
void COP(u8 n) { A ^= n; }
/* A call: mixes what it reads into what it writes. */
static void stub(uint32_t id) {
    uint32_t h = hash(id ^ A * 31 ^ X * 17 ^ Y * 7 ^ C);
    A = h; X = h >> 8; Y = h >> 16; C = h & 1; N = h >> 1 & 1; V = h >> 2 & 1; Z = h >> 3 & 1;
    if (h & 0x10) MEM8(0x7E0000 | (h >> 8 & 0xFFFF)) = h >> 24;
}
struct entry { void (*fn)(void); int x8; int index; };
"#;

const MAIN: &str = r#"
int main(int argc, char **argv) {
    FILE *f = fopen(argv[1], "rb");
    fseek(f, 0, SEEK_END); rom_len = ftell(f); fseek(f, 0, SEEK_SET);
    rom = malloc(rom_len); fread(rom, 1, rom_len, f); fclose(f);
    mem = calloc((1 << 24) + 4, 1);
    touched = calloc((1 << 21) + 1, 1);
    int seeds = atoi(argv[2]);
    for (int i = 0; i < n_entries; i++) {
        for (int s = 0; s < seeds; s++) {
            fflush(stdout);
            pid_t pid = fork();
            if (pid == 0) {
                seed = s * 7919 + 1;
                struct itimerval t = {{0, 0}, {0, 250000}};
                setitimer(ITIMER_REAL, &t, NULL);
                uint32_t r = hash(0xC0FFEE);
                A = r; X = r >> 8; Y = r >> 12; D = hash(1) & 0xFF00; DBR = 0x7E;
                if (entries[i].x8) { X &= 0xFF; Y &= 0xFF; }
                S = 0x1F80; C = r & 1; N = r >> 1 & 1; V = r >> 2 & 1; Z = r >> 3 & 1;
                entries[i].fn();
                printf("R %d %d %04X %04X %04X %04X %04X %02X %d |", entries[i].index, s, A, X, Y, S, D, DBR, C);
                for (uint32_t a = 0; a < (1u << 24); a += 8) {
                    if (!touched[a >> 3]) continue;
                    for (uint32_t b = a; b < a + 8; b++) {
                        if (!(touched[b >> 3] & (1 << (b & 7)))) continue;
                        /* The stack page the routine may use: slots make it differ. */
                        if (b >= 0x1C00 && b <= 0x1F84) continue;
                        if (mem[b] != initial(b)) printf(" %06X:%02X", b, mem[b]);
                    }
                }
                printf("\n");
                fflush(stdout);
                _exit(0);
            }
            int status;
            waitpid(pid, &status, 0);
        }
    }
    return 0;
}
"#;
