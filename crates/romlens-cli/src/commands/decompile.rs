//! `decompile`: pseudo-C for a routine, the `snes.h` it includes, and a
//! check over every routine that the output is valid C.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use romlens_core::decompile::{self, DecompileOptions, Decompiled, Level};
use romlens_core::{AddressExpr, SnesAddress};

use crate::commands::rom::json_str;
use crate::commands::session::{self, any_address};

pub struct DecompileArgs<'a> {
    pub rom: Option<&'a Path>,
    pub expr: Option<&'a str>,
    pub project: Option<&'a Path>,
    pub level: &'a str,
    pub json: bool,
    pub header: Option<&'a Path>,
    pub all: bool,
    pub check: bool,
    pub no_names: bool,
    pub assume_dp: Option<&'a str>,
    pub no_explain: bool,
}

pub fn run(args: DecompileArgs) -> Result<()> {
    if let Some(out) = args.header {
        std::fs::write(out, decompile::snes_h())
            .with_context(|| format!("writing {}", out.display()))?;
        println!("wrote {}", out.display());
        if args.rom.is_none() {
            return Ok(());
        }
    }
    let Some(rom) = args.rom else {
        bail!("a ROM is needed (or --header <out> alone to write snes.h)");
    };
    let level = Level::parse(args.level)
        .with_context(|| format!("--level {}: expected lift, clean or full", args.level))?;
    let assume_dp = match args.assume_dp {
        Some(s) => Some(
            u16::from_str_radix(s.trim_start_matches('$').trim_start_matches("0x"), 16)
                .with_context(|| format!("--assume-dp {s}: expected a 16-bit hex value"))?,
        ),
        None => None,
    };
    let opts = DecompileOptions {
        level,
        names: !args.no_names,
        assume_dp,
        explain: !args.no_explain,
    };
    let s = session::open(rom, args.project, false)?;

    if args.all {
        return all(&s, &opts, args.check);
    }
    let Some(expr) = args.expr else {
        bail!("give an address, or --all");
    };
    let addr = match any_address(expr)? {
        AddressExpr::Snes(a) => a,
        AddressExpr::File(off) => s
            .rom
            .snes_address_for(off)
            .with_context(|| format!("{off} has no SNES address"))?,
    };
    let d = decompile::decompile(&s.rom, &s.project, &s.snap, addr, &opts)?;
    if args.json {
        print_json(&d);
    } else {
        print!("{}", d.text);
    }
    Ok(())
}

fn print_json(d: &Decompiled) {
    println!("{{");
    println!("  \"name\": {},", json_str(&d.name));
    println!("  \"entry\": \"{}\",", d.entry);
    println!("  \"text\": {},", json_str(&d.text));
    let lines: Vec<String> = d
        .lines
        .iter()
        .map(|offs| {
            let o: Vec<String> = offs.iter().map(|o| o.0.to_string()).collect();
            format!("[{}]", o.join(", "))
        })
        .collect();
    println!("  \"lines\": [{}],", lines.join(", "));
    let warnings: Vec<String> = d.warnings.iter().map(|w| json_str(w)).collect();
    println!("  \"warnings\": [{}],", warnings.join(", "));
    println!(
        "  \"stats\": {{\"instructions\": {}, \"statements\": {}, \"blocks\": {}, \"gotos\": {}, \"asmComments\": {}}}",
        d.stats.instructions,
        d.stats.statements,
        d.stats.blocks,
        d.stats.gotos,
        d.stats.asm_comments
    );
    println!("}}");
}

/// Decompile every routine; with `check`, run each through the C compiler.
fn all(s: &session::Session, opts: &DecompileOptions, check: bool) -> Result<()> {
    let started = std::time::Instant::now();
    let program = decompile::program(&s.rom, &s.project, &s.snap, opts);
    let mut done: Vec<(SnesAddress, Decompiled)> = Vec::new();
    let refused = program.entries.len() - program.units.len();
    for (e, u) in &program.units {
        done.push((
            *e,
            decompile::render_with(&s.rom, &s.project, &s.snap, &u.f, opts, Some(&program)),
        ));
    }
    let elapsed = started.elapsed();
    let sum = |f: fn(&Decompiled) -> u32| done.iter().map(|(_, d)| f(d) as u64).sum::<u64>();
    println!(
        "{} routines decompiled ({} entries not decoded) in {} ms, level {}",
        done.len(),
        refused,
        elapsed.as_millis(),
        opts.level.name()
    );
    println!("  instructions:  {}", sum(|d| d.stats.instructions));
    println!("  statements:    {}", sum(|d| d.stats.statements));
    println!("  gotos:         {}", sum(|d| d.stats.gotos));
    println!("  asm comments:  {}", sum(|d| d.stats.asm_comments));
    println!(
        "  with warnings: {}",
        done.iter().filter(|(_, d)| !d.warnings.is_empty()).count()
    );
    if !check {
        return Ok(());
    }
    let Some(cc) = compiler() else {
        println!("  syntax check:  skipped (no C compiler found)");
        return Ok(());
    };
    let dir = std::env::temp_dir().join(format!("romlens-decompile-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("snes.h"), decompile::snes_h())?;
    let mut files: Vec<PathBuf> = Vec::new();
    for (e, d) in &done {
        let p = dir.join(format!("f_{:06X}.c", e.as_u24()));
        std::fs::write(&p, &d.text)?;
        files.push(p);
    }
    let mut failed: Vec<String> = Vec::new();
    for chunk in files.chunks(200) {
        let out = Command::new(&cc)
            .args(["-std=c11", "-fsyntax-only", "-Wall"])
            .arg("-I")
            .arg(&dir)
            .args(chunk)
            .output()
            .with_context(|| format!("running {cc}"))?;
        let text = String::from_utf8_lossy(&out.stderr);
        for line in text.lines() {
            if line.contains(": error:") || line.contains(": warning:") {
                failed.push(
                    line.replace(&*dir.to_string_lossy(), "")
                        .trim_start_matches('/')
                        .to_owned(),
                );
            }
        }
    }
    let bad: std::collections::BTreeSet<&str> =
        failed.iter().filter_map(|l| l.split(':').next()).collect();
    println!(
        "  syntax check:  {} of {} valid ({cc} -std=c11 -fsyntax-only -Wall)",
        done.len() - bad.len(),
        done.len()
    );
    for l in failed.iter().take(20) {
        println!("    {l}");
    }
    let _ = std::fs::remove_dir_all(&dir);
    if !bad.is_empty() {
        bail!("{} routines are not valid C", bad.len());
    }
    Ok(())
}

/// A C compiler, if one is installed: `$CC`, then `cc`.
pub fn compiler() -> Option<String> {
    let candidates = std::env::var("CC")
        .ok()
        .into_iter()
        .chain(["cc".to_owned()]);
    candidates.into_iter().find(|cc| {
        Command::new(cc)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}
