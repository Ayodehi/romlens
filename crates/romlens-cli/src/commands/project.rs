//! `project`: create and edit a `.romlens` package from the shell.

use std::path::Path;

use anyhow::{Result, anyhow};
use romlens_core::io::{to_files, write_package};
use romlens_core::model::{
    BankRule, Command, CommentKind, DataKind, FlagOverride, LabelSource, OverrideKind, Project,
};
use romlens_core::{AddressExpr, FileOffset, RomImage, SnesAddress};

use crate::commands::session::{
    any_address, load_project, load_rom, rom_for_project, rom_offset, save_project,
};

pub fn init(dir: &Path, rom: &Path) -> Result<()> {
    let rom = load_rom(rom)?;
    if dir.exists() {
        return Err(anyhow!("{} already exists", dir.display()));
    }
    let project = Project::new(&rom);
    write_package(dir, &to_files(&rom, &project))?;
    println!("created {} for {:?}", dir.display(), rom.header().title);
    Ok(())
}

fn snes_of(rom: &RomImage, expr: &str) -> Result<SnesAddress> {
    Ok(match any_address(expr)? {
        AddressExpr::Snes(a) => a,
        AddressExpr::File(_) => rom
            .resolve(expr)?
            .snes_address
            .ok_or_else(|| anyhow!("{expr} has no CPU address"))?,
    })
}

fn apply(
    dir: &Path,
    rom: Option<&Path>,
    build: impl FnOnce(&RomImage) -> Result<Command>,
) -> Result<()> {
    let rom = rom_for_project(dir, rom)?;
    let mut project = load_project(&rom, dir)?;
    let cmd = build(&rom)?;
    let entry = project.apply(&rom, cmd)?;
    save_project(&rom, dir, &project)?;
    println!("{}", entry.title);
    Ok(())
}

/// `-` means "remove".
fn text_arg(text: &str) -> Option<String> {
    if text == "-" {
        None
    } else {
        Some(text.to_owned())
    }
}

pub fn label(dir: &Path, rom: Option<&Path>, expr: &str, name: &str) -> Result<()> {
    apply(dir, rom, |r| {
        Ok(Command::SetLabel {
            address: snes_of(r, expr)?,
            name: text_arg(name),
        })
    })
}

pub fn comment(dir: &Path, rom: Option<&Path>, expr: &str, block: bool, text: &str) -> Result<()> {
    apply(dir, rom, |r| {
        Ok(Command::SetComment {
            address: snes_of(r, expr)?,
            kind: if block {
                CommentKind::Block
            } else {
                CommentKind::Line
            },
            text: text_arg(text),
        })
    })
}

pub struct MarkArgs<'a> {
    pub dir: &'a Path,
    pub rom: Option<&'a Path>,
    pub expr: &'a str,
    pub len: u32,
    pub kind: &'a str,
    pub stride: Option<u8>,
    pub bpp: Option<u8>,
    pub elem: Option<&'a str>,
    pub bank: Option<&'a str>,
}

pub fn mark(args: MarkArgs<'_>) -> Result<()> {
    apply(args.dir, args.rom, |r| {
        let bank = match args.bank {
            Some(text) => Some(BankRule::parse(text).ok_or_else(|| {
                anyhow!("unknown bank rule {text:?}; use same, entry or a bank such as $C0")
            })?),
            None => None,
        };
        let kind = match args.kind {
            "code" => OverrideKind::Code,
            "unknown" => OverrideKind::Unknown,
            other => OverrideKind::Data(
                DataKind::parse_with(other, args.stride, args.bpp, args.elem, bank)
                    .ok_or_else(|| anyhow!("unknown kind {other:?} or element {:?}; kinds are code, byte, word, long, pointer, table, string, graphics, tilemap, palette, compressed, struct and unknown; elements are raw, pointer and code", args.elem))?,
            ),
        };
        Ok(Command::MarkRegion {
            start: FileOffset(rom_offset(r, args.expr)?),
            len: args.len,
            kind,
        })
    })
}

pub fn clear(dir: &Path, rom: Option<&Path>, expr: &str, len: u32) -> Result<()> {
    apply(dir, rom, |r| {
        Ok(Command::ClearRegionOverride {
            start: FileOffset(rom_offset(r, expr)?),
            len,
        })
    })
}

pub struct FlagArgs<'a> {
    pub m: Option<u8>,
    pub x: Option<u8>,
    pub e: Option<u8>,
    pub dbr: Option<&'a str>,
    pub dp: Option<&'a str>,
    pub remove: bool,
}

fn hex_arg(text: &str, digits: usize) -> Result<u32> {
    let t = text.trim().trim_start_matches('$');
    if t.is_empty() || t.len() > digits {
        return Err(anyhow!("bad value {text:?}"));
    }
    u32::from_str_radix(t, 16).map_err(|_| anyhow!("bad value {text:?}"))
}

pub fn flags(dir: &Path, rom: Option<&Path>, expr: &str, args: FlagArgs<'_>) -> Result<()> {
    apply(dir, rom, |r| {
        let flags = if args.remove {
            None
        } else {
            Some(FlagOverride {
                m: args.m.map(|v| v != 0),
                x: args.x.map(|v| v != 0),
                e: args.e.map(|v| v != 0),
                dbr: args
                    .dbr
                    .map(|t| hex_arg(t, 2))
                    .transpose()?
                    .map(|v| v as u8),
                dp: args
                    .dp
                    .map(|t| hex_arg(t, 4))
                    .transpose()?
                    .map(|v| v as u16),
            })
        };
        Ok(Command::SetFlagOverride {
            offset: FileOffset(rom_offset(r, expr)?),
            flags,
        })
    })
}

/// What the project holds, since the undo stack lives only in a session.
pub fn history(dir: &Path, rom: Option<&Path>) -> Result<()> {
    let rom = rom_for_project(dir, rom)?;
    let p = load_project(&rom, dir)?;
    println!(
        "{} for {:?} ({}): {} labels, {} comments, {} region marks, {} flag overrides",
        dir.display(),
        p.rom.title,
        &p.rom.sha256[..16],
        p.labels.len(),
        p.comments.len(),
        p.region_overrides.len(),
        p.flag_overrides.len()
    );
    for l in p.labels.values() {
        let src = match &l.source {
            LabelSource::Imported(s) => format!("imported:{s}"),
            _ => "user".to_owned(),
        };
        println!("  label    {}  {}  ({src})", l.address, l.name);
    }
    for c in p.comments.values() {
        println!(
            "  comment  {}  {}: {}",
            c.address,
            c.kind.as_str(),
            c.text.replace('\n', " | ")
        );
    }
    for r in &p.region_overrides {
        println!("  mark     {}  {} bytes  {}", r.start, r.len, r.kind.name());
    }
    for (off, f) in &p.flag_overrides {
        println!("  flags    {}  {:?}", off, f);
    }
    Ok(())
}
