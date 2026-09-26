//! The SPC700 against Tom Harte's single-step tests
//! (github.com/SingleStepTests/spc700, MIT): for every opcode, a thousand
//! cases of registers and memory before and after, and every bus cycle
//! between. The suite is not committed; point `ROMLENS_SPC_TESTS` at its
//! `v1` folder and this runs, otherwise it says so and passes.
//!
//! Memory in the suite is plain RAM, `$F0–$FF` and `$FFC0–$FFFF` included,
//! so the CPU runs on a flat bus here, not the APU's.

use romlens_core::apu::{Spc700, SpcBus};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Cycle {
    Read(u16, u8),
    Write(u16, u8),
    Idle,
}

struct Flat {
    ram: Vec<u8>,
    cycles: Vec<Cycle>,
}

impl SpcBus for Flat {
    fn read(&mut self, addr: u16) -> u8 {
        let v = self.ram[addr as usize];
        self.cycles.push(Cycle::Read(addr, v));
        v
    }

    fn write(&mut self, addr: u16, value: u8) {
        self.ram[addr as usize] = value;
        self.cycles.push(Cycle::Write(addr, value));
    }

    fn idle(&mut self) {
        self.cycles.push(Cycle::Idle);
    }
}

fn n(v: &Value, k: &str) -> u64 {
    v[k].as_u64().unwrap_or_else(|| panic!("no {k}"))
}

/// Why a case fails, or `None`.
fn run_case(case: &Value) -> Option<String> {
    let i = &case["initial"];
    let f = &case["final"];
    let mut bus = Flat {
        ram: vec![0; 0x10000],
        cycles: Vec::new(),
    };
    for p in i["ram"].as_array().unwrap() {
        bus.ram[p[0].as_u64().unwrap() as usize] = p[1].as_u64().unwrap() as u8;
    }
    let mut cpu = Spc700 {
        a: n(i, "a") as u8,
        x: n(i, "x") as u8,
        y: n(i, "y") as u8,
        sp: n(i, "sp") as u8,
        psw: n(i, "psw") as u8,
        pc: n(i, "pc") as u16,
        halted: false,
    };
    let want: Vec<Option<Cycle>> = case["cycles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let addr = c[0].as_u64().map(|a| a as u16);
            let value = c[1].as_u64().map(|v| v as u8);
            match (c[2].as_str().unwrap(), addr, value) {
                ("wait", ..) => Some(Cycle::Idle),
                ("read", Some(a), Some(v)) => Some(Cycle::Read(a, v)),
                ("write", Some(a), Some(v)) => Some(Cycle::Write(a, v)),
                // A read whose value the suite leaves open.
                _ => None,
            }
        })
        .collect();
    cpu.step(&mut bus);
    // SLEEP and STOP: the suite goes on counting cycles of the stopped CPU.
    while cpu.halted && bus.cycles.len() < want.len() {
        cpu.step(&mut bus);
    }
    let mut why = Vec::new();
    for (k, want) in [
        ("a", cpu.a as u64),
        ("x", cpu.x as u64),
        ("y", cpu.y as u64),
        ("sp", cpu.sp as u64),
        ("psw", cpu.psw as u64),
        ("pc", cpu.pc as u64),
    ] {
        if n(f, k) != want {
            why.push(format!("{k} {:#x} want {:#x}", want, n(f, k)));
        }
    }
    for p in f["ram"].as_array().unwrap() {
        let (a, v) = (
            p[0].as_u64().unwrap() as usize,
            p[1].as_u64().unwrap() as u8,
        );
        if bus.ram[a] != v {
            why.push(format!("ram[{a:#06x}] {:#04x} want {v:#04x}", bus.ram[a]));
        }
    }
    let got = &bus.cycles;
    let cycles_ok = got.len() == want.len()
        && got.iter().zip(&want).all(|(g, w)| match (g, w) {
            (_, None) => matches!(g, Cycle::Read(..)),
            (g, Some(w)) => match (g, w) {
                // The address of a dummy read is checked, not its value.
                (Cycle::Read(a, _), Cycle::Read(b, _)) => a == b,
                _ => g == w,
            },
        });
    if !cycles_ok {
        why.push(format!("cycles {got:?}\n   want {want:?}"));
    }
    if why.is_empty() {
        None
    } else {
        Some(why.join("; "))
    }
}

#[test]
fn every_opcode_matches_the_single_step_suite() {
    let Some(dir) = std::env::var_os("ROMLENS_SPC_TESTS") else {
        eprintln!("ROMLENS_SPC_TESTS is not set: the SPC700 single-step suite was not run");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let mut failed = Vec::new();
    let mut total = 0;
    for op in 0..=255u8 {
        let path = dir.join(format!("{op:02x}.json"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let cases: Vec<Value> = serde_json::from_str(&text).unwrap();
        let mut bad = 0;
        let mut first = None;
        for case in &cases {
            total += 1;
            if let Some(why) = run_case(case) {
                bad += 1;
                first.get_or_insert_with(|| format!("{}: {why}", case["name"]));
            }
        }
        if let Some(first) = first {
            failed.push(format!(
                "${op:02X}: {bad} of {} fail, first {first}",
                cases.len()
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "{} of 256 opcodes fail ({total} cases):\n{}",
        failed.len(),
        failed.join("\n")
    );
}
