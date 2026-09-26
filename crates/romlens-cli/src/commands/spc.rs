//! `spc`: the sound CPU's code (docs/23).

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use romlens_core::cpu65816::format::{Symbol, format_bytes};
use romlens_core::spc700::{AramSymbols, NoAramSymbols, aram, decode_at, format_instruction};

pub struct DisasmArgs<'a> {
    pub image: &'a Path,
    /// Where the file's first byte goes in audio RAM.
    pub base: Option<&'a str>,
    pub address: Option<&'a str>,
    pub count: usize,
    /// List only the code reached from `address`.
    pub walk: bool,
}

/// The routines a walk found, named `SUB_xxxx`.
struct Routines(BTreeSet<u16>);

impl AramSymbols for Routines {
    fn name_for(&self, address: u16) -> Option<Symbol> {
        self.0.contains(&address).then(|| Symbol {
            name: format!("SUB_{address:04X}"),
            user: false,
        })
    }
}

fn address(text: &str) -> Result<u16> {
    let v = super::rec::number(text)?;
    u16::try_from(v).map_err(|_| anyhow!("{text} is past $FFFF: audio RAM is 64 KB"))
}

pub fn disasm(args: DisasmArgs) -> Result<()> {
    let bytes =
        std::fs::read(args.image).with_context(|| format!("reading {}", args.image.display()))?;
    let base = args.base.map(address).transpose()?.unwrap_or(0);
    if base as usize + bytes.len() > 0x10000 {
        return Err(anyhow!(
            "{} bytes from ${base:04X} run past $FFFF: audio RAM is 64 KB",
            bytes.len()
        ));
    }
    let mut image = vec![0u8; 0x10000];
    image[base as usize..base as usize + bytes.len()].copy_from_slice(&bytes);
    let from = args.address.map(address).transpose()?.unwrap_or(base);
    if !args.walk {
        for (insn, f) in aram::disassemble(&image, from, args.count, &NoAramSymbols) {
            println!(
                "${:04X}  {:<9} {}",
                insn.address,
                format_bytes(insn.raw()),
                f.text
            );
        }
        return Ok(());
    }
    let walk = aram::walk(&image, &[from]);
    let names = Routines(walk.routines.clone());
    let mut last: Option<u16> = None;
    for &at in &walk.starts {
        let insn = decode_at(&image, at);
        if last.is_some_and(|l| l != at) {
            println!();
        }
        if walk.routines.contains(&at) {
            println!("SUB_{at:04X}:");
        }
        let f = format_instruction(&insn, &names);
        let line = match f.register {
            Some(r) => format!("{:<22}; {}", f.text, r.description),
            None => f.text,
        };
        println!("${at:04X}  {:<9} {line}", format_bytes(insn.raw()));
        last = Some(insn.next());
    }
    println!();
    println!(
        "{} instructions, {} bytes of code, {} routines{}",
        walk.starts.len(),
        walk.code.len(),
        walk.routines.len(),
        if walk.unresolved.is_empty() {
            String::new()
        } else {
            format!(
                "; jumps through a table not followed at {}",
                walk.unresolved
                    .iter()
                    .map(|a| format!("${a:04X}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );
    Ok(())
}
