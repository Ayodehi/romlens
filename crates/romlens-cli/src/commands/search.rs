//! `search`: byte patterns with `??` wildcards.

use std::path::Path;

use anyhow::Result;
use romlens_core::{FileOffset, parse_pattern, search_bytes};

use crate::commands::session::{load_rom, rom_offset};

pub fn run(
    rom: &Path,
    pattern: &str,
    from: Option<&str>,
    to: Option<&str>,
    max: u32,
) -> Result<()> {
    let rom = load_rom(rom)?;
    let pattern = parse_pattern(pattern)?;
    let start = match from {
        Some(e) => rom_offset(&rom, e)?,
        None => 0,
    };
    let end = match to {
        Some(e) => rom_offset(&rom, e)? + 1,
        None => rom.len() as u32,
    };
    let hits = search_bytes(rom.bytes(), &pattern, start, end.saturating_sub(start), max);
    for h in &hits {
        println!(
            "{}  {}",
            FileOffset(*h),
            rom.snes_address_for(FileOffset(*h))
                .map_or("--:----".to_owned(), |a| a.to_string())
        );
    }
    eprintln!(
        "{} match{}",
        hits.len(),
        if hits.len() == 1 { "" } else { "es" }
    );
    Ok(())
}
