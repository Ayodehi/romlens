//! `heuristics`: what the scored guesses found, and how strongly
//! (checklist 2.2).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use romlens_core::analysis::heuristics::HeuristicHit;

use crate::commands::session;

pub fn run(
    rom: &Path,
    project: Option<&Path>,
    kind: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    json: bool,
) -> Result<()> {
    let s = session::open(rom, project, false)?;
    let start = from.map(|e| session::rom_offset(&s.rom, e)).transpose()?;
    let end = to.map(|e| session::rom_offset(&s.rom, e)).transpose()?;
    let hits: Vec<&HeuristicHit> = s
        .snap
        .heuristic_hits
        .iter()
        .filter(|h| kind.is_none_or(|k| h.name == k))
        .filter(|h| start.is_none_or(|f| h.end() > f) && end.is_none_or(|t| h.start < t))
        .collect();

    if json {
        println!("{{");
        println!("  \"hits\": [");
        let rows: Vec<String> = hits
            .iter()
            .map(|h| {
                let detail = h.detail.replace('\\', "\\\\").replace('"', "\\\"");
                format!(
                    "    {{\"start\": {}, \"len\": {}, \"kind\": \"{}\", \"name\": \"{}\", \
\"score\": {:.2}, \"confidence\": {}, \"classifies\": {}, \"detail\": \"{detail}\"}}",
                    h.start,
                    h.len,
                    h.kind.name(),
                    h.name,
                    h.score,
                    h.confidence(),
                    h.classifies
                )
            })
            .collect();
        println!("{}", rows.join(",\n"));
        println!("  ]");
        println!("}}");
        return Ok(());
    }

    // A per-heuristic summary first: the list is long, and what a reader wants
    // to know before reading it is which guesses carried the map.
    let mut totals: BTreeMap<&str, (usize, u64, bool)> = BTreeMap::new();
    for h in &hits {
        let e = totals.entry(h.name).or_insert((0, 0, h.classifies));
        e.0 += 1;
        e.1 += h.len as u64;
    }
    for (name, (spans, bytes, classifies)) in &totals {
        println!(
            "{name:<10} {spans:>6} {:<6} {bytes:>9} bytes{}",
            if *spans == 1 { "span" } else { "spans" },
            if *classifies {
                ""
            } else {
                "   (annotates only)"
            }
        );
    }
    println!();
    for h in &hits {
        println!(
            "{}  {:>8} bytes  {:<10} {:<10} {:.2}  {}{}",
            romlens_core::FileOffset(h.start),
            h.len,
            h.name,
            h.kind.name(),
            h.score,
            h.detail,
            if h.classifies {
                ""
            } else {
                "  (annotates only)"
            }
        );
    }
    Ok(())
}
