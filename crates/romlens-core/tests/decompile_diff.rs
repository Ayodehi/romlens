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
use romlens_core::decompile::dataflow::{Loc, LocSet};
use romlens_core::decompile::ir::{CType, Slot};
use romlens_core::decompile::signature::Abi;
use romlens_core::decompile::{self, DecompileOptions, Level, Program};
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

struct Program2 {
    exe: PathBuf,
}

/// Bits for registers and flags, as the C side masks them.
fn mask(set: &LocSet) -> u32 {
    let mut m = 0;
    for (l, bit) in [
        (Loc::Al, 1),
        (Loc::Ah, 2),
        (Loc::X, 4),
        (Loc::Y, 8),
        (Loc::C, 16),
        (Loc::N, 32),
        (Loc::V, 64),
        (Loc::Z, 128),
        (Loc::D, 256),
        (Loc::Dbr, 512),
    ] {
        if set.contains(&l) {
            m |= bit;
        }
    }
    m
}

const ALL: u32 = 1023;
/// What `Conventions::default()` says an unknown call reads: every register
/// and the carry, not N, V or Z.
const DEFAULT_READS: u32 = 1 | 2 | 4 | 8 | 16 | 256 | 512;

#[allow(clippy::too_many_arguments)]
fn build(
    cc: &str,
    dir: &std::path::Path,
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    program: &Program,
    entries: &[SnesAddress],
    level: Level,
) -> Option<Program2> {
    let opts = DecompileOptions {
        level,
        names: false,
        ..Default::default()
    };
    let mut src = String::from(PRELUDE);
    let mut funcs: BTreeMap<String, (u32, u32, Option<Abi>)> = BTreeMap::new();
    let mut exits_x8: BTreeSet<String> = BTreeSet::new();
    let abi_of = |at: &SnesAddress| {
        (level == Level::Full)
            .then(|| program.abis.get(at).cloned())
            .flatten()
    };
    let mut tables: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    let mut table_rows = Vec::new();
    for (i, &e) in entries.iter().enumerate() {
        let Some(u) = program.units.get(&e) else {
            continue;
        };
        let f = &u.f;
        let x8 = f.steps[f.index_of(f.entry_offset).unwrap()]
            .insn
            .flags_before
            .eff_x();
        let d = decompile::render_with(rom, project, snap, f, &opts, Some(program));
        for (name, at, table) in &d.callees {
            if *table {
                // What the analysis assumes of a table call: the union of
                // its targets' summaries.
                let targets: Vec<SnesAddress> = snap
                    .jump_tables
                    .iter()
                    .filter(|t| t.base_address == *at)
                    .flat_map(|t| t.targets.iter().map(|(a, _)| *a))
                    .collect();
                let mut m = (0, 0);
                for t in &targets {
                    match program.summaries.get(t) {
                        Some(s) => {
                            m.0 |= mask(&s.reads);
                            m.1 |= mask(&s.writes);
                        }
                        None => m = (DEFAULT_READS, ALL),
                    }
                }
                if targets.is_empty() {
                    m = (DEFAULT_READS, ALL);
                }
                tables.insert(name.clone(), m);
            } else {
                let m = program
                    .summaries
                    .get(at)
                    .map(|s| (mask(&s.reads), mask(&s.writes)))
                    .unwrap_or((DEFAULT_READS, ALL));
                funcs.insert(name.clone(), (m.0, m.1, abi_of(at)));
                if program
                    .units
                    .get(at)
                    .is_some_and(|u| decompile::signature::index_widths(u).1)
                {
                    exits_x8.insert(name.clone());
                }
            }
        }
        let s = &program.summaries[&e];
        src.push_str(&format!("#define {} entry_{i}\n", d.name));
        // After a call made with 8-bit index registers their high bytes are
        // zero, as the hardware keeps them; a stub does not know, so each
        // such call line says so, at every level alike.
        let x8_calls: BTreeSet<u32> = f
            .steps
            .iter()
            .filter(|st| st.insn.mnemonic.is_call() && st.insn.flags_after.eff_x())
            .map(|st| st.insn.file_offset.0)
            .collect();
        for (n, line) in d.text.lines().enumerate() {
            src.push_str(line);
            let t = line.trim();
            if t.ends_with("();")
                && d.lines
                    .get(n)
                    .is_some_and(|o| o.iter().any(|o| x8_calls.contains(&o.0)))
            {
                src.push_str(" X &= 0xFF; Y &= 0xFF;");
            }
            src.push('\n');
        }
        src.push_str(&format!("#undef {}\n", d.name));
        // At `full` a routine takes and returns its registers: a wrapper
        // moves them from and to the globals the runs compare.
        let fn_name = match abi_of(&e) {
            Some(abi) => {
                src.push_str(&abi.global_shim(&format!("wrap_{i}"), &format!("entry_{i}")));
                format!("wrap_{i}")
            }
            None => format!("entry_{i}"),
        };
        table_rows.push(format!(
            "{{{fn_name}, {}, {i}, {}, {}}}",
            x8 as u8,
            mask(&s.reads),
            mask(&s.writes)
        ));
    }
    src.push_str(
        "\n/* Stubs for everything called, reading and writing what its summary says. */\n",
    );
    for (k, (name, (r, w, abi))) in funcs.iter().enumerate() {
        match abi {
            Some(abi) => src.push_str(&abi_stub(name, abi, k + 1, *r, *w)),
            // A callee that returns with 8-bit index registers leaves
            // their high bytes clear, as the hardware does.
            None => src.push_str(&format!(
                "void {name}(void) {{ stub({}, {r}, {w}); {}}}\n",
                k + 1,
                if exits_x8.contains(name) {
                    "X &= 0xFF; Y &= 0xFF; "
                } else {
                    ""
                }
            )),
        }
    }
    for (k, (name, (r, w))) in tables.iter().enumerate() {
        src.push_str(&format!(
            "static void {name}_stub(void) {{ stub({}, {r}, {w}); }}\n\
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
    Some(Program2 { exe })
}

/// A global register read as a value of type `t`.
fn global_as(k: Slot, t: CType) -> String {
    match t {
        CType::U8 => format!("(u8){}", k.global()),
        _ => k.global().to_owned(),
    }
}

/// Store `v`, of type `t`, into the global register.
fn store_global(k: Slot, t: CType, v: &str) -> String {
    match (k, t) {
        (Slot::A, CType::U8) => format!("A = (A & 0xFF00) | (u8)({v});"),
        _ => format!("{} = {v};", k.global()),
    }
}

/// A stub with the callee's signature: its arguments into the globals,
/// the mixing stub, its results out of them.
fn abi_stub(name: &str, abi: &Abi, id: usize, r: u32, w: u32) -> String {
    let mut s = format!("{} {{\n", abi.c_signature(name));
    for (k, t) in &abi.params {
        s.push_str(&format!("    {}\n", store_global(*k, *t, k.name())));
    }
    s.push_str(&format!("    stub({id}, {r}, {w});\n"));
    for (k, t) in &abi.outs {
        s.push_str(&format!("    *{}_out = {};\n", k.name(), global_as(*k, *t)));
    }
    if let Some((k, t)) = abi.ret {
        s.push_str(&format!("    return {};\n", global_as(k, t)));
    }
    s.push_str("}\n");
    s
}

/// One run: final registers and flags (A X Y S D DBR C N V Z), the
/// initial ones, and the memory it changed.
#[derive(Debug, Clone, PartialEq)]
struct Outcome {
    regs: [u32; 10],
    initial: [u32; 10],
    mem: BTreeMap<u32, u8>,
}

/// Run every routine under every seed, perturbing what `variant` 1 says:
/// (routine index, seed) to outcome; timeouts and crashes are absent.
fn run(p: &Program2, rom_path: &std::path::Path, variant: u32) -> BTreeMap<(usize, u32), Outcome> {
    let out = Command::new(&p.exe)
        .arg(rom_path)
        .arg(SEEDS.to_string())
        .arg(variant.to_string())
        .output()
        .unwrap();
    let mut results = BTreeMap::new();
    let hex = |v: &str| u32::from_str_radix(v, 16).unwrap();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let parts: Vec<&str> = line.split('|').map(|p| p.trim()).collect();
        if parts.len() < 3 {
            continue;
        }
        let mut h = parts[0].split_whitespace();
        if h.next() != Some("R") {
            continue;
        }
        let i: usize = h.next().unwrap().parse().unwrap();
        let s: u32 = h.next().unwrap().parse().unwrap();
        let regs: Vec<u32> = parts[1].split_whitespace().map(hex).collect();
        let initial: Vec<u32> = parts[2].split_whitespace().map(hex).collect();
        let mut mem = BTreeMap::new();
        for pair in parts.get(3).unwrap_or(&"").split_whitespace() {
            let (a, v) = pair.split_once(':').unwrap();
            mem.insert(hex(a), hex(v) as u8);
        }
        results.insert(
            (i, s),
            Outcome {
                regs: regs.try_into().unwrap(),
                initial: initial.try_into().unwrap(),
                mem,
            },
        );
    }
    results
}

/// Registers by mask bit: which part of which outcome field.
fn reg_diffs(a: &Outcome, b: &Outcome, bits: u32) -> Vec<String> {
    const NAMES: [&str; 10] = ["A", "X", "Y", "S", "D", "DBR", "C", "N", "V", "Z"];
    let mut out = Vec::new();
    let mut check = |field: usize, m: u32, name: &str| {
        if a.regs[field] & m != b.regs[field] & m {
            out.push(format!(
                "{name} {:X} (lift: {:X})",
                a.regs[field] & m,
                b.regs[field] & m
            ));
        }
    };
    if bits & 1 != 0 {
        check(0, 0xFF, "A.lo");
    }
    if bits & 2 != 0 {
        check(0, 0xFF00, "A.hi");
    }
    for (bit, field) in [
        (4, 1),
        (8, 2),
        (256, 4),
        (512, 5),
        (16, 6),
        (32, 7),
        (64, 8),
        (128, 9),
    ] {
        if bits & bit != 0 {
            check(field, 0xFFFF, NAMES[field]);
        }
    }
    check(3, 0xFFFF, "S");
    out
}

fn mem_diffs(a: &Outcome, b: &Outcome) -> Vec<String> {
    let addrs: BTreeSet<&u32> = a.mem.keys().chain(b.mem.keys()).collect();
    addrs
        .into_iter()
        .filter(|x| a.mem.get(x) != b.mem.get(x))
        .map(|x| {
            format!(
                "${x:06X} {:02X?} (lift: {:02X?})",
                a.mem.get(x),
                b.mem.get(x)
            )
        })
        .collect()
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
    let program = decompile::program(rom, project, snap, &DecompileOptions::default());
    let lift = build(
        &cc,
        &dir,
        rom,
        project,
        snap,
        &program,
        entries,
        Level::Lift,
    )?;
    let base = run(&lift, &rom_path, 0);
    let mut compared = 0;
    let mut bad = Vec::new();
    let summary = |i: usize| &program.summaries[&entries[i]];
    let say = |bad: &mut Vec<String>, i: usize, what: &str, seed: u32, diff: Vec<String>| {
        bad.push(format!(
            "{} {what}, seed {seed}: {}",
            entries[i],
            diff.into_iter().take(6).collect::<Vec<_>>().join("; ")
        ));
    };

    // The summary is true of the lift level: perturbing what it says is not
    // read changes nothing, and what it says is not written is unchanged.
    let perturbed = run(&lift, &rom_path, 1);
    for (key, want) in &base {
        let s = summary(key.0);
        let seen = mask(&s.writes);
        for (bit, field, m) in [
            (1u32, 0usize, 0xFFu32),
            (2, 0, 0xFF00),
            (4, 1, 0xFFFF),
            (8, 2, 0xFFFF),
            (256, 4, 0xFFFF),
            (512, 5, 0xFF),
            (16, 6, 1),
        ] {
            if seen & bit == 0 && want.regs[field] & m != want.initial[field] & m {
                say(
                    &mut bad,
                    key.0,
                    "changes what its summary says it does not write",
                    key.1,
                    vec![format!("field {field}")],
                );
            }
        }
        let Some(have) = perturbed.get(key) else {
            continue;
        };
        compared += 1;
        // Only what callers read: a register written on some paths and not
        // others passes its input through on the rest, which no one reads.
        let mut diff = reg_diffs(have, want, mask(&s.returns));
        diff.extend(mem_diffs(have, want));
        if !diff.is_empty() {
            say(
                &mut bad,
                key.0,
                "depends on what its summary says it does not read",
                key.1,
                diff,
            );
        }
    }

    for level in [Level::Clean, Level::Full] {
        let got = run(
            &build(&cc, &dir, rom, project, snap, &program, entries, level)?,
            &rom_path,
            0,
        );
        for (key, want) in &base {
            let Some(have) = got.get(key) else {
                continue;
            };
            compared += 1;
            let s = summary(key.0);
            // At `full` the registers are each routine's own variables:
            // what it preserves is kept by its callers, not put back, so
            // only S, D and DBR, still globals, must come back.
            let kept: LocSet = if level == Level::Full {
                s.preserves
                    .iter()
                    .filter(|l| matches!(l, Loc::S | Loc::D | Loc::Dbr))
                    .copied()
                    .collect()
            } else {
                s.preserves.clone()
            };
            let live: LocSet = s.returns.union(&kept).copied().collect();
            let mut diff = reg_diffs(have, want, mask(&live));
            // An index result the signature makes 8 bits (every caller goes
            // on with 8-bit index registers) passes only its low byte.
            if level == Level::Full
                && let Some(abi) = program.abis.get(&entries[key.0])
            {
                let narrow: Vec<&str> = abi
                    .ret
                    .iter()
                    .chain(&abi.outs)
                    .filter(|(k, t)| *t == CType::U8 && matches!(k, Slot::X | Slot::Y))
                    .map(|(k, _)| k.global())
                    .collect();
                diff.retain(|d| {
                    let reg = d.split(' ').next().unwrap_or("");
                    if !narrow.contains(&reg) {
                        return true;
                    }
                    let v: Vec<&str> = d.split([' ', ')']).collect();
                    let got = u32::from_str_radix(v[1], 16).unwrap_or(0);
                    let lift = u32::from_str_radix(v[3], 16).unwrap_or(0);
                    got & 0xFF != lift & 0xFF
                });
            }
            diff.extend(mem_diffs(have, want));
            if !diff.is_empty() {
                say(
                    &mut bad,
                    key.0,
                    &format!("at {} level", level.name()),
                    key.1,
                    diff,
                );
            }
        }
    }
    // ROMLENS_DIFF_KEEP leaves the programs for a look.
    if std::env::var_os("ROMLENS_DIFF_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    } else {
        eprintln!("kept {}", dir.display());
    }
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
    assert!(bad.is_empty(), "{}", bad.join("\n"));
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
/* The stack is its own memory: at lift the pushes write it and at clean
 * the saved temporaries do not, so it must not be memory a routine reads. */
static uint8_t stack[0x10000];
void push8(u8 v) { stack[S] = v; S--; }
void push16(u16 v) { push8(v >> 8); push8(v & 0xFF); }
u8 pull8(void) { S++; return stack[S]; }
u16 pull16(void) { u16 lo = pull8(); return lo | (u16)(pull8() << 8); }
void PHP(void) { push8((u8)(N << 7 | V << 6 | Z << 1 | C)); }
void PLP(void) { u8 p = pull8(); N = p >> 7 & 1; V = p >> 6 & 1; Z = p >> 1 & 1; C = p & 1; }
void mvn(u8 d, u8 s) { int n = 0; do { MEM8(d << 16 | Y) = MEM8(s << 16 | X); X++; Y++; } while (A-- != 0 && ++n < 0x10000); }
void mvp(u8 d, u8 s) { int n = 0; do { MEM8(d << 16 | Y) = MEM8(s << 16 | X); X--; Y--; } while (A-- != 0 && ++n < 0x10000); }
void mvn8(u8 d, u8 s) { int n = 0; do { MEM8(d << 16 | Y) = MEM8(s << 16 | X); X = (X + 1) & 0xFF; Y = (Y + 1) & 0xFF; } while (A-- != 0 && ++n < 0x10000); }
void mvp8(u8 d, u8 s) { int n = 0; do { MEM8(d << 16 | Y) = MEM8(s << 16 | X); X = (X - 1) & 0xFF; Y = (Y - 1) & 0xFF; } while (A-- != 0 && ++n < 0x10000); }
u16 bcd_add(u16 a, u16 b, int bits) { u32 r = a + b + C; C = r >> bits & 1; V = 0; return r; }
u16 bcd_sub(u16 a, u16 b, int bits) { u32 r = a - b - !C; C = !(r >> bits & 1); V = 0; return r; }
void WAI(void) {}
void STP(void) { _exit(3); }
void BRK(u8 n) { A ^= n; }
void COP(u8 n) { A ^= n; }
_Noreturn void table_overrun(void) { _exit(4); }
/* A call: mixes what it reads (mask r) into what it writes (mask w). */
static void stub(uint32_t id, unsigned r, unsigned w) {
    uint32_t in = id;
    if (r & 1) in ^= (A & 0xFF) * 31;
    if (r & 2) in ^= (A >> 8) * 37;
    if (r & 4) in ^= X * 17;
    if (r & 8) in ^= Y * 7;
    if (r & 16) in ^= C * 3;
    if (r & 32) in ^= N * 5;
    if (r & 64) in ^= V * 11;
    if (r & 128) in ^= Z * 13;
    if (r & 256) in ^= D * 19;
    if (r & 512) in ^= DBR * 23;
    uint32_t h = hash(in);
    if (w & 1) A = (A & 0xFF00) | (h & 0xFF);
    if (w & 2) A = (A & 0x00FF) | (h & 0xFF00);
    if (w & 4) X = h >> 8;
    if (w & 8) Y = h >> 16;
    if (w & 16) C = h & 1;
    if (w & 32) N = h >> 1 & 1;
    if (w & 64) V = h >> 2 & 1;
    if (w & 128) Z = h >> 3 & 1;
    if (w & 256) D = h >> 4;
    if (w & 512) DBR = h >> 12;
    if (h & 0x10) MEM8(0x7E0000 | (h >> 8 & 0xFFFF)) = h >> 24;
}
struct entry { void (*fn)(void); int x8; int index; unsigned reads, writes; };
"#;

const MAIN: &str = r#"
int main(int argc, char **argv) {
    FILE *f = fopen(argv[1], "rb");
    fseek(f, 0, SEEK_END); rom_len = ftell(f); fseek(f, 0, SEEK_SET);
    rom = malloc(rom_len); fread(rom, 1, rom_len, f); fclose(f);
    mem = calloc((1 << 24) + 4, 1);
    touched = calloc((1 << 21) + 1, 1);
    int seeds = atoi(argv[2]);
    int variant = atoi(argv[3]);
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
                S = 0x01FF; C = r & 1; N = r >> 1 & 1; V = r >> 2 & 1; Z = r >> 3 & 1;
                if (variant) {
                    /* Change what the summary says is not read. */
                    unsigned k = ~entries[i].reads;
                    if (k & 1) A ^= 0x5A;
                    if (k & 2) A ^= 0xA500;
                    if (k & 4) X ^= entries[i].x8 ? 0x33 : 0x3C3;
                    if (k & 8) Y ^= entries[i].x8 ? 0x55 : 0x5A5;
                    if (k & 16) C ^= 1;
                    if (k & 32) N ^= 1;
                    if (k & 64) V ^= 1;
                    if (k & 128) Z ^= 1;
                    if (k & 256) D ^= 0x1100;
                    if (k & 512) DBR ^= 0x01;
                }
                u16 a0 = A, x0 = X, y0 = Y, d0 = D; u8 dbr0 = DBR, c0 = C, n0 = N, v0 = V, z0 = Z;
                entries[i].fn();
                printf("R %d %d | %04X %04X %04X %04X %04X %02X %X %X %X %X | %04X %04X %04X 01FF %04X %02X %X %X %X %X |",
                       entries[i].index, s, A, X, Y, S, D, DBR, C, N, V, Z, a0, x0, y0, d0, dbr0, c0, n0, v0, z0);
                for (uint32_t a = 0; a < (1u << 24); a += 8) {
                    if (!touched[a >> 3]) continue;
                    for (uint32_t b = a; b < a + 8; b++) {
                        if (!(touched[b >> 3] & (1 << (b & 7)))) continue;
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

/// The same on any ROM, when `ROMLENS_DIFF_ROM` names one.
#[test]
fn every_level_does_what_lift_does_on_a_rom_you_name() {
    let Some(path) = std::env::var_os("ROMLENS_DIFF_ROM") else {
        return;
    };
    let rom = RomImage::load(std::path::Path::new(&path)).unwrap();
    // With ROMLENS_DIFF_PROJECT, the project's traces and marks too.
    let (rom, project, snap) = match std::env::var_os("ROMLENS_DIFF_PROJECT") {
        Some(dir) => {
            let files = romlens_core::io::read_package(std::path::Path::new(&dir)).unwrap();
            let project = romlens_core::io::from_files(&rom, &files).unwrap();
            let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
            (rom, project, snap)
        }
        None => setup(rom),
    };
    let mut entries: Vec<SnesAddress> = decompile::entries(&snap).into_iter().collect();
    // ROMLENS_DIFF_ONLY=009BC9 narrows it to one routine.
    if let Some(only) = std::env::var("ROMLENS_DIFF_ONLY")
        .ok()
        .and_then(|s| u32::from_str_radix(&s, 16).ok())
    {
        entries.retain(|a| a.as_u24() == only);
    }
    let Some((compared, bad)) = compare("named", &rom, &project, &snap, &entries) else {
        eprintln!("skipped: no C compiler or not POSIX");
        return;
    };
    eprintln!("{compared} runs compared, {} differ", bad.len());
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
