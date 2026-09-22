//! `search`: byte patterns with `??` wildcards, or text (checklist 2.7).

use std::path::Path;

use anyhow::Result;
use romlens_core::{
    FileOffset, RomImage, parse_pattern, pattern_from_text, search_bytes, search_masked,
};

use crate::commands::session::{load_rom, rom_offset};

/// Bytes of context printed either side of a hit.
const CONTEXT: u32 = 8;

pub struct SearchArgs<'a> {
    pub rom: &'a Path,
    pub pattern: &'a str,
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub max: u32,
    /// Match `pattern` as text rather than hex.
    pub text: bool,
    pub ignore_case: bool,
}

pub fn run(args: SearchArgs<'_>) -> Result<()> {
    let rom = load_rom(args.rom)?;
    let start = match args.from {
        Some(e) => rom_offset(&rom, e)?,
        None => 0,
    };
    let end = match args.to {
        Some(e) => rom_offset(&rom, e)? + 1,
        None => rom.len() as u32,
    };
    let len = end.saturating_sub(start);
    let (hits, width) = if args.text || args.ignore_case {
        let (pattern, mask) = pattern_from_text(args.pattern, args.ignore_case)?;
        let n = pattern.len() as u32;
        (
            search_masked(rom.bytes(), &pattern, &mask, start, len, args.max),
            n,
        )
    } else {
        let pattern = parse_pattern(args.pattern)?;
        let n = pattern.len() as u32;
        (search_bytes(rom.bytes(), &pattern, start, len, args.max), n)
    };
    for h in &hits {
        println!("{}", hit_line(&rom, *h, width));
    }
    eprintln!(
        "{} match{}",
        hits.len(),
        if hits.len() == 1 { "" } else { "es" }
    );
    Ok(())
}

/// A hit with its bytes and their ASCII, so a reader can tell a real find from
/// a coincidence without a second command.
fn hit_line(rom: &RomImage, offset: u32, width: u32) -> String {
    let n = rom.len() as u32;
    let from = offset.saturating_sub(CONTEXT);
    let to = (offset + width + CONTEXT).min(n);
    let bytes = &rom.bytes()[from as usize..to as usize];
    let hex: String = bytes
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let at = from + i as u32;
            // The match is bracketed, so the eye finds it in the context.
            if at == offset {
                format!("[{b:02X}")
            } else if at == offset + width - 1 {
                format!("{b:02X}]")
            } else {
                format!("{b:02X}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .replace("[ ", "[")
        .replace(" ]", "]");
    let ascii: String = bytes
        .iter()
        .map(|b| {
            if (0x20..0x7F).contains(b) {
                *b as char
            } else {
                '.'
            }
        })
        .collect();
    format!(
        "{}  {}  {hex}  |{ascii}|",
        FileOffset(offset),
        rom.snes_address_for(FileOffset(offset))
            .map_or("--:----".to_owned(), |a| a.to_string())
    )
}
