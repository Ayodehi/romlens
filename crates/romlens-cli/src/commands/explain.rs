//! `explain`: what an instruction does to the hardware, what a whole
//! routine does, or how much of a ROM can be explained (docs/20).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use romlens_core::cpu65816::format_instruction;
use romlens_core::decompile::function;
use romlens_core::explain::{Explained, Explanations};
use romlens_core::model::symbols::Symbols;
use romlens_core::{AddressExpr, FileOffset};

use crate::commands::registers::write_text;
use crate::commands::rom::json_str;
use crate::commands::session::{self, Session, any_address};

pub struct ExplainArgs<'a> {
    pub rom: &'a Path,
    pub expr: Option<&'a str>,
    pub project: Option<&'a Path>,
    pub routine: bool,
    pub stats: bool,
    pub json: bool,
}

pub fn run(args: ExplainArgs) -> Result<()> {
    let s = session::open(args.rom, args.project, false)?;
    let x = Explanations::build(&s.rom, &s.project, &s.snap);
    if args.stats {
        let st = x.stats();
        let pct = |n: usize| {
            if st.stores == 0 {
                0.0
            } else {
                100.0 * n as f64 / st.stores as f64
            }
        };
        println!("routines:          {}", st.routines);
        println!("hardware stores:   {}", st.stores);
        println!("value known:       {} ({:.0}%)", st.known, pct(st.known));
        println!("source known:      {} ({:.0}%)", st.sourced, pct(st.sourced));
        println!("indexed, unknown:  {} ({:.0}%)", st.indexed, pct(st.indexed));
        return Ok(());
    }
    let expr = args.expr.ok_or_else(|| anyhow!("give an address, or --stats"))?;
    let off = match any_address(expr)? {
        AddressExpr::Snes(a) => s
            .rom
            .file_offset_for(a)
            .with_context(|| format!("{a} is not in ROM"))?,
        AddressExpr::File(off) => off,
    };
    let writes: Vec<&Explained> = if args.routine {
        let entries = function::entries(&s.snap);
        let f = function::containing(&s.rom, &s.snap, &entries, off)
            .with_context(|| format!("{off} is not in a routine the analysis found"))?;
        f.steps
            .iter()
            .filter_map(|st| x.write_at(st.insn.file_offset))
            .collect()
    } else {
        x.write_at(off).into_iter().collect()
    };
    if args.json {
        print!("{}", json(&s, &writes));
        return Ok(());
    }
    if writes.is_empty() {
        println!("{}: no store to a hardware register", address(&s, off));
        return Ok(());
    }
    if args.routine {
        for e in &writes {
            println!(
                "{}  {:<16} {}",
                address(&s, e.offset),
                instruction(&s, e.offset),
                e.short()
            );
        }
    } else {
        let e = writes[0];
        println!("{}  {}", address(&s, e.offset), instruction(&s, e.offset));
        println!("\n  {}", e.short());
        if let Some(a) = e.source {
            println!("  The value is loaded from {a}; its bits are listed without values.");
        }
        print!("{}", write_text(&e.write, "  "));
    }
    Ok(())
}

fn address(s: &Session, off: FileOffset) -> String {
    s.rom
        .snes_address_for(off)
        .map_or_else(|| format!("{off}"), |a| format!("{a}"))
}

fn instruction(s: &Session, off: FileOffset) -> String {
    let symbols = Symbols::new(&s.rom, &s.project, &s.snap.auto_labels);
    s.snap
        .instruction_at(off)
        .and_then(|r| s.snap.decode_at(&s.rom, r))
        .map(|i| format_instruction(&i, &symbols).text)
        .unwrap_or_default()
}

fn json(s: &Session, writes: &[&Explained]) -> String {
    let mut out = String::from("[");
    for (i, e) in writes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "\n  {{\"address\": {}, \"instruction\": {}, \"short\": {}, \"value\": {}, \"source\": {}, \"parts\": [",
            json_str(&address(s, e.offset)),
            json_str(&instruction(s, e.offset)),
            json_str(&e.short()),
            e.write.value.map_or("null".to_owned(), |v| v.to_string()),
            e.source
                .map_or("null".to_owned(), |a| json_str(&format!("{a}"))),
        );
        for (j, p) in e.write.parts.iter().enumerate() {
            if j > 0 {
                out.push_str(", ");
            }
            let _ = write!(
                out,
                "{{\"register\": {}, \"name\": {}, \"summary\": {}, \"fields\": [",
                p.address,
                json_str(&p.name),
                p.summary.as_deref().map_or("null".to_owned(), json_str),
            );
            for (k, f) in p.fields.iter().enumerate() {
                if k > 0 {
                    out.push_str(", ");
                }
                let _ = write!(
                    out,
                    "{{\"bits\": {}, \"name\": {}, \"raw\": {}, \"meaning\": {}}}",
                    json_str(&f.bits),
                    json_str(f.name),
                    f.raw.map_or("null".to_owned(), |v| v.to_string()),
                    f.meaning.as_deref().map_or("null".to_owned(), json_str),
                );
            }
            out.push_str("]}");
        }
        out.push_str("]}");
    }
    out.push_str(if writes.is_empty() { "]\n" } else { "\n]\n" });
    out
}
