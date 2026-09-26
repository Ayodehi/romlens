//! `source`: the source lines an imported `.dbg` ties to the ROM.

use std::path::Path;

use anyhow::{Result, bail};
use romlens_core::model::source_map::{SourceLine, SourceMap};
use romlens_core::{FileOffset, RomImage};

use crate::commands::session::{load_project, load_rom, rom_offset};

pub struct SourceArgs<'a> {
    pub rom: &'a Path,
    pub project: &'a Path,
    pub address: Option<&'a str>,
    /// `FILE:LINE`.
    pub line: Option<&'a str>,
}

fn text_of(map: &SourceMap, l: &SourceLine) -> Option<String> {
    let text = std::fs::read_to_string(map.path_of(l.file)?).ok()?;
    text.lines()
        .nth(l.line.checked_sub(1)? as usize)
        .map(|t| t.trim_end().to_owned())
}

fn range(rom: &RomImage, (start, len): (FileOffset, u32)) -> String {
    let at = |o: u32| {
        rom.snes_address_for(FileOffset(o))
            .map_or(format!("{}", FileOffset(o)), |a| a.to_string())
    };
    if len == 1 {
        format!("{} (1 byte)", at(start.0))
    } else {
        format!("{}–{} ({len} bytes)", at(start.0), at(start.0 + len - 1))
    }
}

pub fn run(args: SourceArgs) -> Result<()> {
    let rom = load_rom(args.rom)?;
    let project = load_project(&rom, args.project)?;
    if project.source_maps.is_empty() {
        bail!("the project has no source lines; import a .dbg with `romlens import dbg`");
    }
    if let Some(expr) = args.address {
        let offset = FileOffset(rom_offset(&rom, expr)?);
        let mut any = false;
        for map in &project.source_maps {
            for l in map.lines_at(offset) {
                any = true;
                let name = &map.files[l.file as usize].name;
                println!(
                    "{name}:{} ({}): {}",
                    l.line,
                    l.kind.name(),
                    text_of(map, l).unwrap_or_else(|| "(source not found)".into())
                );
                for r in &l.ranges {
                    println!("  {}", range(&rom, *r));
                }
            }
        }
        if !any {
            println!("no source line made {offset}");
        }
        return Ok(());
    }
    if let Some(spec) = args.line {
        let Some((name, n)) = spec.rsplit_once(':') else {
            bail!("--line takes FILE:LINE, such as main.s:24");
        };
        let n: u32 = n.parse()?;
        for map in &project.source_maps {
            let Some(file) = map.file_named(name) else {
                continue;
            };
            match map.line(file, n) {
                Some(l) => {
                    println!(
                        "{}:{n} ({}): {}",
                        map.files[file as usize].name,
                        l.kind.name(),
                        text_of(map, l).unwrap_or_else(|| "(source not found)".into())
                    );
                    for r in &l.ranges {
                        println!("  {}", range(&rom, *r));
                    }
                }
                None => println!("{name}:{n} made no bytes"),
            }
            return Ok(());
        }
        bail!("no imported .dbg names a file {name:?}");
    }
    for map in &project.source_maps {
        println!(
            "{}: {} lines making {} bytes, sources in {}",
            map.source,
            map.lines.len(),
            map.bytes_covered(),
            map.dir
        );
        for (i, f) in map.files.iter().enumerate() {
            let found = map.path_of(i as u32).is_some_and(|p| p.is_file());
            println!(
                "  {}: {} bytes{}",
                f.name,
                f.size,
                if found { "" } else { ", not found" }
            );
        }
    }
    Ok(())
}
