//! `explain`: what an instruction does to the hardware, what a whole
//! routine does, or how much of a ROM can be explained (docs/20).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use romlens_core::cpu65816::format_instruction;
use romlens_core::decompile::function;
use romlens_core::explain::{Explained, Explanations, Idiom};
use romlens_core::model::symbols::Symbols;
use romlens_core::{AddressExpr, FileOffset};

use crate::commands::registers::{wrap, write_text};
use crate::commands::rom::json_str;
use crate::commands::session::{self, Session, any_address};

pub struct ExplainArgs<'a> {
    pub rom: &'a Path,
    pub expr: Option<&'a str>,
    pub project: Option<&'a Path>,
    pub routine: bool,
    pub stats: bool,
    pub idioms: Option<&'a str>,
    pub json: bool,
}

const KINDS: &[&str] = &[
    "wait",
    "dma",
    "hdma",
    "multiply",
    "divide",
    "clear-memory",
    "block-move",
    "apu-handshake",
    "decimal",
    "shared-entry",
    "shadow-register",
];

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
        println!(
            "source known:      {} ({:.0}%)",
            st.sourced,
            pct(st.sourced)
        );
        println!(
            "indexed, unknown:  {} ({:.0}%)",
            st.indexed,
            pct(st.indexed)
        );
        for (kind, n) in &st.idioms {
            println!("idiom {:<17}{n}", format!("{}:", kind.as_str()));
        }
        return Ok(());
    }
    if let Some(kind) = args.idioms {
        let all: Vec<&Idiom> = x
            .idioms()
            .iter()
            .filter(|i| kind == "all" || i.kind.as_str() == kind)
            .collect();
        if all.is_empty() && kind != "all" && !KINDS.contains(&kind) {
            return Err(anyhow!(
                "no idiom kind {kind:?}: use one of {}",
                KINDS.join(", ")
            ));
        }
        if args.json {
            print!("{}", json(&s, &[], &all));
        } else {
            for i in all {
                print!("{}", idiom_text(&s, i));
            }
        }
        return Ok(());
    }
    let expr = args
        .expr
        .ok_or_else(|| anyhow!("give an address, or --stats"))?;
    let off = match any_address(expr)? {
        AddressExpr::Snes(a) => s
            .rom
            .file_offset_for(a)
            .with_context(|| format!("{a} is not in ROM"))?,
        AddressExpr::File(off) => off,
    };
    let (writes, idioms): (Vec<&Explained>, Vec<&Idiom>) = if args.routine {
        let entries = function::entries(&s.snap);
        let f = function::containing(&s.rom, &s.snap, &entries, off)
            .with_context(|| format!("{off} is not in a routine the analysis found"))?;
        let mut idioms: Vec<&Idiom> = Vec::new();
        for st in &f.steps {
            for i in x.idioms_starting_at(st.insn.file_offset) {
                idioms.push(i);
            }
        }
        (
            f.steps
                .iter()
                .filter_map(|st| x.write_at(st.insn.file_offset))
                .collect(),
            idioms,
        )
    } else {
        (x.write_at(off).into_iter().collect(), x.idioms_at(off))
    };
    if args.json {
        print!("{}", json(&s, &writes, &idioms));
        return Ok(());
    }
    if writes.is_empty() && idioms.is_empty() {
        println!("{}: nothing to explain", address(&s, off));
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
        for i in &idioms {
            print!("{}", idiom_text(&s, i));
        }
    } else if writes.is_empty() {
        println!("{}  {}", address(&s, off), instruction(&s, off));
        for i in &idioms {
            print!("{}", idiom_text(&s, i));
        }
    } else {
        let e = writes[0];
        println!("{}  {}", address(&s, e.offset), instruction(&s, e.offset));
        println!("\n  {}", e.short());
        if let Some(a) = e.source {
            println!("  The value is loaded from {a}; its bits are listed without values.");
        }
        print!("{}", write_text(&e.write, "  "));
        for i in &idioms {
            print!("{}", idiom_text(&s, i));
        }
    }
    Ok(())
}

/// An idiom: its title, where it spans, what it does here and why.
fn idiom_text(s: &Session, i: &Idiom) -> String {
    let mut out = String::new();
    let last = *i.offsets.last().unwrap_or(&i.first());
    let _ = writeln!(
        out,
        "\n▸ {}  ({}–{}, {} instructions)",
        i.title,
        address(s, i.first()),
        address(s, last),
        i.offsets.len()
    );
    for line in wrap(&i.summary, 72) {
        let _ = writeln!(out, "  {line}");
    }
    out.push('\n');
    for line in wrap(i.why, 72) {
        let _ = writeln!(out, "  {line}");
    }
    out
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

fn json(s: &Session, writes: &[&Explained], idioms: &[&Idiom]) -> String {
    let mut out = String::from("{\"writes\": [");
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
    out.push_str(if writes.is_empty() { "]" } else { "\n]" });
    out.push_str(", \"idioms\": [");
    for (k, i) in idioms.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        let offsets: Vec<String> = i
            .offsets
            .iter()
            .map(|o| json_str(&address(s, *o)))
            .collect();
        let _ = write!(
            out,
            "\n  {{\"kind\": {}, \"title\": {}, \"summary\": {}, \"why\": {}, \"instructions\": [{}]}}",
            json_str(i.kind.as_str()),
            json_str(&i.title),
            json_str(&i.summary),
            json_str(i.why),
            offsets.join(", ")
        );
    }
    out.push_str(if idioms.is_empty() { "]}\n" } else { "\n]}\n" });
    out
}
