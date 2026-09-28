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

pub fn asm(
    rom: &Path,
    out: &Path,
    project: Option<&Path>,
    range: Option<&str>,
    check: bool,
) -> Result<()> {
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
    emit(out, &text)?;
    if check {
        let (start, len) = range.unwrap_or((0, s.rom.len() as u32));
        reassemble(&s.rom, &text, start, len)?;
    }
    Ok(())
}

/// Runs asar on the listing and compares what it made with the ROM over
/// `start..start + len`: the first difference and how many, or byte-exact.
fn reassemble(rom: &romlens_core::RomImage, listing: &str, start: u32, len: u32) -> Result<()> {
    let dir = std::env::temp_dir().join(format!("romlens-asar-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let asm = dir.join("listing.asm");
    let built = dir.join("built.sfc");
    std::fs::write(&asm, listing)?;
    // asar patches a ROM file: an empty one the size of this ROM.
    std::fs::write(&built, vec![0u8; rom.len()])?;
    // The listing carries the header's checksum as it is: asar must not
    // work out its own.
    let run = std::process::Command::new("asar")
        .arg("--no-title-check")
        .arg("--fix-checksum=off")
        .arg(&asm)
        .arg(&built)
        .output();
    let result = match run {
        Err(e) => Err(anyhow!("could not run asar (is it on the PATH?): {e}")),
        Ok(o) if !o.status.success() => {
            let said = String::from_utf8_lossy(&o.stdout).to_string()
                + &String::from_utf8_lossy(&o.stderr);
            let errors: Vec<&str> = said
                .lines()
                .filter(|l| l.contains("error"))
                .take(10)
                .collect();
            Err(anyhow!(
                "asar did not assemble the listing:\n{}",
                errors.join("\n")
            ))
        }
        Ok(_) => {
            let made = std::fs::read(&built)?;
            let (a, b) = (start as usize, (start + len) as usize);
            let theirs = &rom.bytes()[a..b];
            let ours = made.get(a..b).unwrap_or(&[]);
            let differ: Vec<usize> = (0..theirs.len())
                .filter(|&i| ours.get(i) != Some(&theirs[i]))
                .collect();
            match differ.first() {
                None => {
                    println!("asar check: byte-exact, {} bytes", theirs.len());
                    Ok(())
                }
                Some(&i) => {
                    let at = romlens_core::FileOffset((a + i) as u32);
                    let cpu = rom
                        .snes_address_for(at)
                        .map(|x| format!(" ({x})"))
                        .unwrap_or_default();
                    Err(anyhow!(
                        "asar check: {} of {} bytes differ, the first at {at}{cpu}: ROM ${:02X}, asar ${:02X}",
                        differ.len(),
                        theirs.len(),
                        theirs[i],
                        ours.get(i).copied().unwrap_or(0)
                    ))
                }
            }
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    result
}

pub fn sym(rom: &Path, out: &Path, project: Option<&Path>, include_auto: bool) -> Result<()> {
    let s = session::open(rom, project, false)?;
    let mut text = String::new();
    export_symbols(&s.rom, &s.snap, &s.project, include_auto, &mut text);
    emit(out, &text)
}
