//! `analyze`: run the analyzer and report.

use std::path::Path;

use anyhow::Result;
use romlens_core::RomImage;
use romlens_core::analysis::{AnalysisOptions, AnalysisStats, Warning};

use crate::commands::session;

pub fn run(
    rom: &Path,
    project: Option<&Path>,
    json: bool,
    progress: bool,
    warnings: bool,
    options: AnalysisOptions,
) -> Result<()> {
    let s = session::open_options(rom, project, progress, options)?;
    let st = s.snap.stats;
    eprintln!("elapsed: {} ms", st.elapsed_ms);
    if json {
        let mut out = stats_json(&st, s.rom.len() as u64);
        if warnings {
            // Replace the closing brace with the warning list.
            out.truncate(out.trim_end().len() - 1);
            out.push_str(",\n  \"warningList\": [\n");
            let items: Vec<String> = s
                .snap
                .warnings
                .iter()
                .map(|w| warning_json(&s.rom, w))
                .collect();
            out.push_str(&items.join(",\n"));
            out.push_str("\n  ]\n}\n");
        }
        print!("{out}");
    } else {
        print!("{}", stats_text(&st, s.rom.len() as u64));
        if warnings {
            println!();
            for w in &s.snap.warnings {
                println!("{}", warning_text(&s.rom, w));
            }
        }
    }
    Ok(())
}

fn warning_text(rom: &RomImage, w: &Warning) -> String {
    let snes = rom
        .snes_address_for(w.offset)
        .map_or_else(|| "--:----".to_owned(), |a| a.to_string());
    format!(
        "{}  {:<9}  {:<24} {}",
        w.offset,
        snes,
        w.kind.name(),
        w.text
    )
}

fn warning_json(rom: &RomImage, w: &Warning) -> String {
    let snes = rom
        .snes_address_for(w.offset)
        .map_or_else(|| "null".to_owned(), |a| format!("\"{a}\""));
    let text = w.text.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "    {{\"fileOffset\": {}, \"snesAddress\": {snes}, \"kind\": \"{}\", \"text\": \"{text}\"}}",
        w.offset.0,
        w.kind.name()
    )
}

fn pct(n: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        n as f64 * 100.0 / total as f64
    }
}

pub fn stats_text(st: &AnalysisStats, total: u64) -> String {
    format!(
        "Code:          {} bytes ({:.1}%) in {} blocks, {} instructions\n\
Data:          {} bytes ({:.1}%)\n\
Unknown:       {} bytes ({:.1}%)\n\
Regions:       {}\n\
Labels:        {}\n\
Xrefs:         {}\n\
Warnings:      {} ({} flag conflicts)\n",
        st.code_bytes,
        pct(st.code_bytes, total),
        st.blocks,
        st.instructions,
        st.data_bytes,
        pct(st.data_bytes, total),
        st.unknown_bytes,
        pct(st.unknown_bytes, total),
        st.regions,
        st.labels,
        st.xrefs,
        st.warnings,
        st.conflicts
    )
}

pub fn stats_json(st: &AnalysisStats, total: u64) -> String {
    format!(
        "{{\n  \"bytes\": {total},\n  \"codeBytes\": {},\n  \"dataBytes\": {},\n  \"unknownBytes\": {},\n  \"instructions\": {},\n  \"blocks\": {},\n  \"regions\": {},\n  \"labels\": {},\n  \"xrefs\": {},\n  \"warnings\": {},\n  \"conflicts\": {}\n}}\n",
        st.code_bytes,
        st.data_bytes,
        st.unknown_bytes,
        st.instructions,
        st.blocks,
        st.regions,
        st.labels,
        st.xrefs,
        st.warnings,
        st.conflicts
    )
}
