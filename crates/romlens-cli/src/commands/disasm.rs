//! `disasm`: the analyzed listing, or a raw linear decode under given flags.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use romlens_core::cpu65816::{FlagState, NoSymbols, decode, format_bytes, format_instruction};
use romlens_core::explain::Explanations;
use romlens_core::{AddressStyle, FileOffset, LineIndex, LineKind, TextOptions, format_lines_text};

use crate::commands::session::{self, load_rom, rom_offset};

pub struct DisasmArgs<'a> {
    pub rom: &'a Path,
    pub from: Option<&'a str>,
    pub count: u32,
    pub project: Option<&'a Path>,
    pub flags: Option<&'a str>,
    pub style: AddressStyle,
    pub verbose: bool,
    /// Explain hardware writes and note idioms (docs/20).
    pub explain: bool,
}

pub fn run(args: DisasmArgs<'_>) -> Result<()> {
    if let Some(flags) = args.flags {
        let flags = FlagState::parse(flags).ok_or_else(|| {
            anyhow!("--flags takes letters m, x, e each followed by 0 or 1, e.g. m1x0e0")
        })?;
        let rom = load_rom(args.rom)?;
        let start = match args.from {
            Some(e) => rom_offset(&rom, e)?,
            None => 0,
        };
        print!(
            "{}",
            linear(&rom, start, args.count, flags, args.style, args.verbose)
        );
        return Ok(());
    }
    let mut s = session::open(args.rom, args.project, false)?;
    if args.explain {
        let x = Arc::new(Explanations::build(&s.rom, &s.project, &s.snap));
        s.idx = LineIndex::build_explained(&s.rom, &s.snap, &s.project, x);
    }
    let start_line = match args.from {
        Some(e) => {
            let off = rom_offset(&s.rom, e)?;
            let line = s
                .idx
                .line_for_offset(off)
                .ok_or_else(|| anyhow!("no line covers {}", FileOffset(off)))?;
            // Show the label and comments above the content line too.
            let mut first = line;
            while first > 0
                && s.idx.lines[first - 1].offset == s.idx.lines[line].offset
                && s.idx.lines[first - 1].kind != LineKind::Blank
            {
                first -= 1;
            }
            first as u32
        }
        None => 0,
    };
    print!(
        "{}",
        format_lines_text(
            &s.rom,
            &s.snap,
            &s.project,
            &s.idx,
            start_line,
            args.count,
            TextOptions {
                style: args.style,
                verbose: args.verbose,
            },
        )
    );
    Ok(())
}

/// Straight-line decode ignoring the analysis: the tutor's hypothesis tool.
pub fn linear(
    rom: &romlens_core::RomImage,
    start: u32,
    count: u32,
    flags: FlagState,
    style: AddressStyle,
    verbose: bool,
) -> String {
    let mut out = String::new();
    let mut pos = start;
    let mut flags = flags;
    let mut history = Vec::new();
    for _ in 0..count {
        let Some(addr) = rom.snes_address_for(FileOffset(pos)) else {
            break;
        };
        let Some(mut insn) = decode(&rom.bytes()[pos as usize..], addr, FileOffset(pos), flags)
        else {
            break;
        };
        romlens_core::analysis::flow::refine(&history, &mut insn);
        if style != AddressStyle::Snes {
            let _ = write!(out, "{}  ", FileOffset(pos));
        }
        if style != AddressStyle::File {
            let _ = write!(out, "{addr}  ");
        }
        let _ = write!(out, "{:<12} ", format_bytes(insn.bytes()));
        if verbose {
            let f = insn.flags_before;
            let _ = write!(
                out,
                "{} {}:{}  ",
                f.short(),
                f.dbr.map_or("$??".to_owned(), |b| format!("${b:02X}")),
                f.dp.map_or("$????".to_owned(), |d| format!("${d:04X}"))
            );
        }
        let f = format_instruction(&insn, &NoSymbols);
        out.push_str(&f.text);
        if let Some(r) = f.register {
            let pad = 44usize.saturating_sub(f.text.len());
            for _ in 0..pad {
                out.push(' ');
            }
            let _ = write!(out, "; {}", r.name);
        }
        out.push('\n');
        flags = insn.flags_after;
        pos += insn.len as u32;
        history.push(insn);
        if history.len() > 2 {
            history.remove(0);
        }
    }
    out
}
