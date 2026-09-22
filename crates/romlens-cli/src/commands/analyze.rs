//! `analyze`: run the analyzer and report.

use std::path::Path;

use anyhow::Result;
use romlens_core::analysis::AnalysisStats;

use crate::commands::session;

pub fn run(rom: &Path, project: Option<&Path>, json: bool, progress: bool) -> Result<()> {
    let s = session::open(rom, project, progress)?;
    let st = s.snap.stats;
    eprintln!("elapsed: {} ms", st.elapsed_ms);
    if json {
        print!("{}", stats_json(&st, s.rom.len() as u64));
    } else {
        print!("{}", stats_text(&st, s.rom.len() as u64));
    }
    Ok(())
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
        st.labels,
        st.xrefs,
        st.warnings,
        st.conflicts
    )
}

pub fn stats_json(st: &AnalysisStats, total: u64) -> String {
    format!(
        "{{\n  \"bytes\": {total},\n  \"codeBytes\": {},\n  \"dataBytes\": {},\n  \"unknownBytes\": {},\n  \"instructions\": {},\n  \"blocks\": {},\n  \"labels\": {},\n  \"xrefs\": {},\n  \"warnings\": {},\n  \"conflicts\": {}\n}}\n",
        st.code_bytes,
        st.data_bytes,
        st.unknown_bytes,
        st.instructions,
        st.blocks,
        st.labels,
        st.xrefs,
        st.warnings,
        st.conflicts
    )
}
