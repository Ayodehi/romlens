//! `map`: the whole-ROM overview the shell's strip draws, as text
//! (checklist 2.6).

use std::path::Path;

use anyhow::Result;
use romlens_core::analysis::heuristics::EntropyProfile;
use romlens_core::viewmodel::region_summary::{SummaryBucket, summarize};

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
            _ => '.',
        },
    };
    // A blended column is shown in lower case for code and marked with `~`
    // when it is data, so "mostly" never reads as "all".
    if b.mixed() && c == '#' { '+' } else { c }
}

pub fn run(rom: &Path, project: Option<&Path>, buckets: u32, width: u32, json: bool) -> Result<()> {
    let s = session::open(rom, project, false)?;
    let n = s.rom.len() as u32;
    let profile = EntropyProfile::build(&s.rom);
    let map = summarize(
        &s.snap,
        n,
        buckets,
        Some(&profile),
        s.project.coverage.as_deref(),
    );

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
        println!("  ]");
        println!("}}");
        return Ok(());
    }

    println!("# code   + mostly code   . byte   t table/pointer   s string");
    println!("g graphics   p palette   m tilemap   z compressed   h header   (space) unknown");
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
    Ok(())
}
