//! `import`: read what other tools know about a ROM (checklist 2.3).

use std::path::Path;

use anyhow::{Context, Result, bail};
use romlens_core::io::import::symbols::{self, SymbolFormat};
use romlens_core::io::import::{Trace, TraceFormat, read};
use romlens_core::model::{ImportRecord, Origin, TraceRecord};

use crate::commands::session::{load_project, rom_for_project, save_project};

pub fn trace(dir: &Path, rom: Option<&Path>, file: &Path, format: Option<&str>) -> Result<()> {
    let format = match format {
        Some(name) => match TraceFormat::parse(name) {
            Some(f) => Some(f),
            None => bail!("unknown trace format {name:?}; use cdl, usage or mxlog"),
        },
        None => None,
    };
    let rom = rom_for_project(dir, rom)?;
    let mut project = load_project(&rom, dir)?;
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let Trace {
        format,
        coverage,
        exec_log,
    } = read(&bytes, &rom, format)?;
    if coverage.is_empty() && exec_log.as_ref().is_none_or(|l| l.is_empty()) {
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
    if let Some(log) = &exec_log {
        project.add_exec_log(log);
    }
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
    if let Some(log) = &exec_log {
        println!(
            "execution log: {} instructions ({} in more than one width), {} access runs, \
{} transfers, {} DMA runs",
            log.insns.len(),
            log.insns.iter().filter(|i| i.mixed_widths()).count(),
            log.accesses.len(),
            log.flows.len(),
            log.dma.len()
        );
    }
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

pub fn dbg(dir: &Path, rom: Option<&Path>, file: &Path) -> Result<()> {
    use romlens_core::io::import::dbg;
    let rom = rom_for_project(dir, rom)?;
    let mut project = load_project(&rom, dir)?;
    let text =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.display().to_string());
    let folder = std::path::absolute(file)
        .ok()
        .and_then(|p| p.parent().map(|d| d.display().to_string()))
        .unwrap_or_default();
    let parsed = dbg::read(&text, &rom, &name, &folder)?;
    let plan = symbols::plan(&rom, &project, &parsed.symbols);
    let labels = parsed.symbols.labels.len();
    let undo = if plan.commands.is_empty() {
        None
    } else {
        Some(project.apply_batch(&rom, plan.commands, Origin::Import(name.clone()))?)
    };
    project.add_import(ImportRecord {
        source: name.clone(),
        format: "dbg".to_owned(),
        labels: labels as u64,
        comments: 0,
        notice: String::new(),
    });
    let map = parsed.map;
    let (lines, bytes, files) = (map.lines.len(), map.bytes_covered(), map.files.len());
    let per_file: Vec<(String, usize)> = map
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let n = map.lines.iter().filter(|l| l.file == i as u32).count();
            (f.name.clone(), n)
        })
        .collect();
    project.add_source_map(map);
    save_project(&rom, dir, &project)?;

    println!(
        "imported {name}, ld65 debug information for {}: {} {} added, {} replaced",
        if parsed.output.is_empty() {
            "an unnamed output"
        } else {
            &parsed.output
        },
        plan.labels_added,
        if plan.labels_added == 1 {
            "label"
        } else {
            "labels"
        },
        plan.replaced
    );
    if let Some(entry) = undo {
        println!("undo entry: {}", entry.title);
    }
    if !plan.kept_user.is_empty() {
        println!("{} kept: you had already named them", plan.kept_user.len());
    }
    println!(
        "{lines} source lines made {bytes} bytes ({:.1}% of the ROM), from {files} files:",
        bytes as f64 * 100.0 / rom.len() as f64
    );
    for (file, n) in per_file {
        println!("  {file}: {n} lines");
    }
    if parsed.not_labels > 0 {
        println!(
            "{} symbols are not labels (equates, imports, constants)",
            parsed.not_labels
        );
    }
    if parsed.spans_unplaced > 0 {
        println!(
            "{} spans or symbols could not be placed in this ROM",
            parsed.spans_unplaced
        );
    }
    if !parsed.symbols.rewritten.is_empty() {
        println!(
            "{} names rewritten to be usable:",
            parsed.symbols.rewritten.len()
        );
        for (from, to) in parsed.symbols.rewritten.iter().take(10) {
            println!("  {from} -> {to}");
        }
    }
    if !parsed.symbols.skipped.is_empty() {
        println!("{} records not understood", parsed.symbols.skipped.len());
    }
    Ok(())
}
