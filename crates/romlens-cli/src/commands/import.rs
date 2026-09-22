//! `import`: read what other tools know about a ROM (checklist 2.3).

use std::path::Path;

use anyhow::{Context, Result, bail};
use romlens_core::io::import::{TraceFormat, read_trace};
use romlens_core::model::TraceRecord;

use crate::commands::session::{load_project, rom_for_project, save_project};

pub fn trace(dir: &Path, rom: Option<&Path>, file: &Path, format: Option<&str>) -> Result<()> {
    let format = match format {
        Some(name) => match TraceFormat::parse(name) {
            Some(f) => Some(f),
            None => bail!("unknown trace format {name:?}; use cdl or usage"),
        },
        None => None,
    };
    let rom = rom_for_project(dir, rom)?;
    let mut project = load_project(&rom, dir)?;
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let (format, coverage) = read_trace(&bytes, &rom, format)?;
    if coverage.is_empty() {
        bail!(
            "{} records nothing for this ROM; it was probably taken from another image",
            file.display()
        );
    }
    let source = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.display().to_string());
    let executed_bytes = coverage.executed.count();
    let read_bytes = coverage.read.count();
    let entries = coverage.entry.count();
    let widths = coverage.flags.recorded;
    project.add_trace(
        TraceRecord {
            source: source.clone(),
            format: format.name().to_owned(),
            executed_bytes,
            read_bytes,
        },
        coverage,
    );
    save_project(&rom, dir, &project)?;

    let total = rom.len() as f64;
    println!(
        "imported {source} as {}: {executed_bytes} bytes executed ({:.1}%), \
{read_bytes} read ({:.1}%), {entries} subroutine entries{}",
        format.name(),
        executed_bytes as f64 * 100.0 / total,
        read_bytes as f64 * 100.0 / total,
        if widths { ", with M/X widths" } else { "" }
    );
    if project.traces.len() > 1 {
        println!("merged with {} earlier traces", project.traces.len() - 1);
    }
    Ok(())
}
