//! `glossary`: the SNES's acronyms and initialisms spelt out (docs/27),
//! as the tutor's answers show them when one is clicked.

use anyhow::{Result, anyhow};
use romlens_core::explain::glossary::{self, Kind};

pub fn run(term: Option<&str>, registers: bool) -> Result<()> {
    match term {
        Some(t) => {
            let e = glossary::lookup(t).ok_or_else(|| anyhow!("{t} is not in the glossary"))?;
            println!("{}: {}", e.term, e.words);
            if !e.also.is_empty() {
                println!("  also written {}", e.also.join(", "));
            }
            println!("  {}", e.about);
        }
        None => {
            for e in glossary::entries() {
                if e.kind == Kind::Term || registers {
                    println!("{:<9} {}", e.term, e.words);
                }
            }
        }
    }
    Ok(())
}
