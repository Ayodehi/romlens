//! `xrefs`: references to and from an address.

use std::path::Path;

use anyhow::Result;
use romlens_core::model::Project;
use romlens_core::{AddressExpr, FileOffset, SnesAddress};

use crate::commands::session::{self, any_address};

pub fn run(rom: &Path, expr: &str, project: Option<&Path>) -> Result<()> {
    let s = session::open(rom, project, false)?;
    let (addr, offset) = match any_address(expr)? {
        AddressExpr::Snes(a) => (Project::canonical(&s.rom, a), s.rom.file_offset_for(a)),
        AddressExpr::File(off) => {
            let r = s.rom.resolve(expr)?;
            (
                r.snes_address.unwrap_or(SnesAddress::from_u24(0)),
                Some(off),
            )
        }
    };
    let to = s.snap.xrefs_to(addr);
    println!("references to {addr}: {}", to.len());
    for x in to {
        println!(
            "  from {}  {}  {}{}",
            x.from,
            s.rom
                .snes_address_for(x.from)
                .map_or("--:----".to_owned(), |a| a.to_string()),
            x.kind.name(),
            if x.certain { "" } else { " (uncertain)" }
        );
    }
    if let Some(off) = offset {
        let from = s.snap.xrefs_from(off);
        println!("references from {}: {}", FileOffset(off.0), from.len());
        for x in from {
            println!(
                "  to {}  {}  {}{}",
                x.to,
                x.to_offset.map_or("        ".to_owned(), |o| o.to_string()),
                x.kind.name(),
                if x.certain { "" } else { " (uncertain)" }
            );
        }
    }
    Ok(())
}
