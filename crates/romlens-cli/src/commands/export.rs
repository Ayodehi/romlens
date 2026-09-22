//! `export asm` and `export sym`.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use romlens_core::io::{AsarOptions, export_asar, export_symbols};

use crate::commands::session::{self, rom_offset};

fn emit(out: &Path, text: &str) -> Result<()> {
    if out.as_os_str() == "-" {
        print!("{text}");
    } else {
        std::fs::write(out, text).with_context(|| format!("writing {}", out.display()))?;
        eprintln!("wrote {} ({} bytes)", out.display(), text.len());
    }
    Ok(())
}

pub fn asm(rom: &Path, out: &Path, project: Option<&Path>, range: Option<&str>) -> Result<()> {
    let s = session::open(rom, project, false)?;
    let range = match range {
        Some(r) => {
            let (a, b) = r
                .split_once("..")
                .ok_or_else(|| anyhow!("--range takes start..end, e.g. $80:8000..$80:9000"))?;
            let start = rom_offset(&s.rom, a)?;
            let end = rom_offset(&s.rom, b)?;
            if end <= start {
                return Err(anyhow!("--range end must be after its start"));
            }
            Some((start, end - start))
        }
        None => None,
    };
    let mut text = String::new();
    export_asar(
        &s.rom,
        &s.snap,
        &s.project,
        AsarOptions {
            range,
            comments: true,
        },
        &mut text,
    );
    emit(out, &text)
}

pub fn sym(rom: &Path, out: &Path, project: Option<&Path>, include_auto: bool) -> Result<()> {
    let s = session::open(rom, project, false)?;
    let mut text = String::new();
    export_symbols(&s.rom, &s.snap, &s.project, include_auto, &mut text);
    emit(out, &text)
}
