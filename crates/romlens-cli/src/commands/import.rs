//! `import`: read what other tools know about a ROM (checklist 2.3).

use std::path::Path;

use anyhow::{Context, Result, bail};
use romlens_core::io::import::symbols::{self, SymbolFormat};
use romlens_core::io::import::{TraceFormat, read_trace};
use romlens_core::model::{ImportRecord, Origin, TraceRecord};

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

pub fn symbols(
    dir: &Path,
    rom: Option<&Path>,
    file: &Path,
    format: Option<&str>,
    source: Option<&str>,
) -> Result<()> {
    let format = match format {
        Some(name) => match SymbolFormat::parse(name) {
            Some(f) => Some(f),
            None => bail!("unknown symbol format {name:?}; use wla, nocash or lbl"),
        },
        None => None,
    };
    let rom = rom_for_project(dir, rom)?;
    let mut project = load_project(&rom, dir)?;
    let text =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let parsed = symbols::read(&text, format)?;
    let format = format.unwrap_or_else(|| symbols::detect(&text));
    let name = source.map(str::to_owned).unwrap_or_else(|| {
        file.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.display().to_string())
    });

    let plan = symbols::plan(&rom, &project, &parsed);
    if plan.commands.is_empty() {
        println!(
            "{} adds nothing: {} labels and {} comments are already as it says",
            file.display(),
            parsed.labels.len(),
            parsed.comments.len()
        );
        return Ok(());
    }
    // One undo entry, not twenty thousand. All or nothing: a file with one bad
    // entry must not leave a project half-imported.
    let entry = project.apply_batch(&rom, plan.commands, Origin::Import(name.clone()))?;
    project.add_import(ImportRecord {
        source: name.clone(),
        format: format.name().to_owned(),
        labels: parsed.labels.len() as u64,
        comments: parsed.comments.len() as u64,
        notice: parsed.notice.clone(),
    });
    save_project(&rom, dir, &project)?;

    println!(
        "imported {name} as {}: {} {} added, {} replaced, {} {} added",
        format.name(),
        plan.labels_added,
        if plan.labels_added == 1 {
            "label"
        } else {
            "labels"
        },
        plan.replaced,
        plan.comments_added,
        if plan.comments_added == 1 {
            "comment"
        } else {
            "comments"
        }
    );
    println!("undo entry: {}", entry.title);
    if !plan.kept_user.is_empty() {
        println!("{} kept: you had already named them", plan.kept_user.len());
    }
    if !parsed.rewritten.is_empty() {
        println!("{} names rewritten to be usable:", parsed.rewritten.len());
        for (from, to) in parsed.rewritten.iter().take(10) {
            println!("  {from} -> {to}");
        }
        if parsed.rewritten.len() > 10 {
            println!("  … and {} more", parsed.rewritten.len() - 10);
        }
    }
    if !parsed.skipped.is_empty() {
        println!(
            "{} {} not understood:",
            parsed.skipped.len(),
            if parsed.skipped.len() == 1 {
                "line"
            } else {
                "lines"
            }
        );
        for line in parsed.skipped.iter().take(5) {
            println!("  {line}");
        }
    }
    if !parsed.notice.is_empty() {
        println!("notice kept in project.json:");
        for line in parsed.notice.lines() {
            println!("  {line}");
        }
    }
    Ok(())
}
