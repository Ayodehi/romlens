//! `screen`: what the screen is set up to be when an instruction runs
//! (docs/21).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};
use romlens_core::explain::screen::{Link, ScreenSetup, screen_at};
use romlens_core::explain::{Explanations, UploadIndex};
use romlens_core::{AddressExpr, FileOffset, RomImage};

use crate::commands::session::{self, any_address};

pub fn run(rom: &Path, expr: &str, project: Option<&Path>) -> Result<()> {
    let s = session::open(rom, project, false)?;
    let off = match any_address(expr)? {
        AddressExpr::Snes(a) => s
            .rom
            .file_offset_for(a)
            .with_context(|| format!("{a} is not in ROM"))?,
        AddressExpr::File(off) => off,
    };
    let x = Explanations::build(&s.rom, &s.project, &s.snap);
    let uploads = UploadIndex {
        explain: &x,
        rom: &s.rom,
    };
    let setup = screen_at(&s.rom, &s.project, &s.snap, off, Some(&uploads))
        .with_context(|| format!("{off} is not in a routine the analysis found"))?;
    print!("{}", text(&s.rom, off, &setup));
    Ok(())
}

fn at(rom: &RomImage, off: FileOffset) -> String {
    rom.snes_address_for(off)
        .map_or_else(|| format!("{off}"), |a| format!("{a}"))
}

pub fn text(rom: &RomImage, off: FileOffset, s: &ScreenSetup) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "The screen when {} runs (in the routine at {}):",
        at(rom, off),
        s.routine
    );
    for sec in &s.sections {
        let _ = writeln!(out, "\n{}", sec.title);
        let w = sec
            .rows
            .iter()
            .map(|r| r.label.chars().count())
            .max()
            .unwrap_or(0);
        for r in &sec.rows {
            let set: Vec<String> = r.set_at.iter().map(|o| at(rom, *o)).collect();
            let pad = w - r.label.chars().count();
            let _ = write!(out, "  {}{}  {}", r.label, " ".repeat(pad), r.text);
            if !set.is_empty() {
                let _ = write!(out, "  [{}]", set.join(", "));
            }
            out.push('\n');
            if let Some(src) = &r.source {
                let _ = writeln!(out, "  {:<w$}  {src}", "");
            }
            if let Some(l) = r.link {
                let view = match l {
                    Link::Tiles { rom: o, bpp } => format!("tiles at {} ({bpp}bpp)", at(rom, o)),
                    Link::Tilemap { rom: o } => format!("tilemap at {}", at(rom, o)),
                    Link::Palette { rom: o } => format!("palette at {}", at(rom, o)),
                };
                let _ = writeln!(out, "  {:<w$}  view: {view}", "");
            }
        }
    }
    out
}
