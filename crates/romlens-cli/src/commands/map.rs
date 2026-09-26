//! `map`: the whole-ROM overview the shell's strip draws, as text
//! (checklist 2.6).

use std::path::Path;

use anyhow::Result;
use romlens_core::analysis::heuristics::EntropyProfile;
use romlens_core::viewmodel::atlas::{ArcEnd, call_arcs, items};
use romlens_core::viewmodel::region_summary::{SummaryBucket, summarize_window};

use crate::commands::session;

/// One character per kind. Code is `#` because it is what a reader is looking
/// for; unknown is a space so a map full of holes looks like one.
fn glyph(b: &SummaryBucket) -> char {
    use romlens_core::model::{DataKind, RegionKind};
    let c = match b.kind {
        RegionKind::Unknown => ' ',
        RegionKind::Code => '#',
        RegionKind::Data(d) => match d {
            DataKind::Graphics { .. } => 'g',
            DataKind::Palette => 'p',
            DataKind::Tilemap => 'm',
            DataKind::Compressed => 'z',
            DataKind::String => 's',
            DataKind::Pointer { .. } | DataKind::Table { .. } => 't',
            DataKind::Struct => 'h',
            DataKind::Sample => 'a',
            _ => '.',
        },
    };
    // A blended column is shown in lower case for code and marked with `~`
    // when it is data, so "mostly" never reads as "all".
    if b.mixed() && c == '#' { '+' } else { c }
}

/// What `map` shows beyond the columns.
pub struct MapOptions<'a> {
    /// The window's first byte, an address expression; the image's start
    /// when absent.
    pub start: Option<&'a str>,
    /// The window's length in bytes; to the image's end when absent.
    pub len: Option<&'a str>,
    /// The calls between columns (docs/22, A1).
    pub arcs: bool,
    /// The instructions and data rows in the window, up to this many.
    pub items: Option<usize>,
}

pub fn run(
    rom: &Path,
    project: Option<&Path>,
    buckets: u32,
    width: u32,
    json: bool,
    options: MapOptions<'_>,
) -> Result<()> {
    let s = session::open(rom, project, false)?;
    let n = s.rom.len() as u32;
    let start = match options.start {
        Some(e) => s.rom.resolve(e)?.file_offset.0,
        None => 0,
    };
    let len = match options.len {
        Some(l) => crate::commands::rec::number(l)?,
        None => n.saturating_sub(start),
    };
    let profile = EntropyProfile::build(&s.rom);
    let map = summarize_window(
        &s.snap,
        n,
        start,
        len,
        buckets,
        Some(&profile),
        s.project.coverage.as_deref(),
    );
    let arcs = if options.arcs {
        call_arcs(&s.snap, n, start, len, buckets)
    } else {
        Vec::new()
    };
    let found = options
        .items
        .map(|limit| items(&s.snap, &s.idx, start, len, limit));
    let end_name = |e: ArcEnd| match e {
        ArcEnd::Before => "before".to_owned(),
        ArcEnd::Column(c) => c.to_string(),
        ArcEnd::After => "after".to_owned(),
    };

    if json {
        println!("{{");
        println!("  \"romBytes\": {n},");
        println!("  \"buckets\": [");
        let rows: Vec<String> = map
            .iter()
            .map(|b| {
                format!(
                    "    {{\"start\": {}, \"len\": {}, \"kind\": \"{}\", \"confidence\": {:.2}, \
\"share\": {:.2}, \"mixed\": {}, \"entropy\": {:.2}, \"executed\": {:.2}}}",
                    b.start,
                    b.len,
                    b.kind.name(),
                    b.confidence,
                    b.share,
                    b.mixed(),
                    b.entropy,
                    b.executed
                )
            })
            .collect();
        println!("{}", rows.join(",\n"));
        if options.arcs {
            println!("  ],");
            println!("  \"arcs\": [");
            let rows: Vec<String> = arcs
                .iter()
                .map(|a| {
                    format!(
                        "    {{\"from\": \"{}\", \"to\": \"{}\", \"calls\": {}, \"observed\": {}}}",
                        end_name(a.from),
                        end_name(a.to),
                        a.calls,
                        a.observed
                    )
                })
                .collect();
            println!("{}", rows.join(",\n"));
        }
        if let Some((found, more)) = &found {
            println!("  ],");
            println!("  \"more\": {more},");
            println!("  \"items\": [");
            let rows: Vec<String> = found
                .iter()
                .map(|i| {
                    format!(
                        "    {{\"offset\": {}, \"len\": {}, \"instruction\": {}, \"kind\": \"{}\"}}",
                        i.offset,
                        i.len,
                        i.instruction,
                        i.kind.name()
                    )
                })
                .collect();
            println!("{}", rows.join(",\n"));
        }
        println!("  ]");
        println!("}}");
        return Ok(());
    }

    println!("# code   + mostly code   . byte   t table/pointer   s string");
    println!(
        "g graphics   p palette   m tilemap   z compressed   h header   a sample   (space) unknown"
    );
    println!();
    for (i, row) in map.chunks(width.max(1) as usize).enumerate() {
        let at = row.first().map(|b| b.start).unwrap_or(0);
        let strip: String = row.iter().map(glyph).collect();
        println!("{}  {strip}", romlens_core::FileOffset(at));
        let _ = i;
    }
    println!();
    let code = map.iter().filter(|b| b.kind.name() == "code").count();
    let unknown = map.iter().filter(|b| b.kind.name() == "unknown").count();
    let mixed = map.iter().filter(|b| b.mixed()).count();
    println!(
        "{} columns of {} bytes: {code} mostly code, {unknown} mostly unknown, {mixed} blended",
        map.len(),
        map.first().map(|b| b.len).unwrap_or(0)
    );
    if options.arcs {
        println!();
        println!("Calls between columns, most first:");
        for a in &arcs {
            println!(
                "  {:>6} -> {:<6} {} call{}{}",
                end_name(a.from),
                end_name(a.to),
                a.calls,
                if a.calls == 1 { "" } else { "s" },
                if a.observed { ", seen running" } else { "" }
            );
        }
    }
    if let Some((found, more)) = &found {
        println!();
        for i in found {
            let at = s
                .rom
                .snes_address_for(romlens_core::FileOffset(i.offset))
                .map_or_else(|| "-".to_owned(), |a| a.to_string());
            println!(
                "{}  {at}  {:>2} byte{}  {}{}",
                romlens_core::FileOffset(i.offset),
                i.len,
                if i.len == 1 { "" } else { "s" },
                if i.instruction {
                    "instruction in "
                } else {
                    "row of "
                },
                i.kind.name()
            );
        }
        if *more {
            println!("(more items in the window)");
        }
    }
    Ok(())
}
