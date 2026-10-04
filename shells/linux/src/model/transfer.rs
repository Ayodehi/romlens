//! File › Import and File › Export: what each kind is called, which files it
//! takes, the text an export makes, and the words a report uses. No GTK, so
//! the wording and the byte columns are tested. The macOS twins are
//! `ImportController` and `ExportController`.

use romlens_ffi::{ImportResult, Workbench};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    Trace,
    Symbols,
    Dbg,
}

impl ImportKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Trace => "Import Execution Trace",
            Self::Symbols => "Import Symbols",
            Self::Dbg => "Import ca65 Debug Information",
        }
    }

    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Trace => &["cdl", "map", "bin", "usage", "mxlog"],
            Self::Symbols => &["sym", "lbl", "txt"],
            Self::Dbg => &["dbg"],
        }
    }

    pub fn filter_name(self) -> &'static str {
        match self {
            Self::Trace => "A .cdl, .mxlog or usage map",
            Self::Symbols => "A .sym or .lbl file",
            Self::Dbg => "A ca65 .dbg file",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    Assembly,
    Annotations,
    Symbols,
}

impl ExportKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Assembly => "Export Assembly Listing",
            Self::Annotations => "Export Labels and Comments",
            Self::Symbols => "Export Symbol File",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Assembly => "asm",
            Self::Annotations | Self::Symbols => "sym",
        }
    }
}

/// The notice shown with the assembly listing (docs/12): it holds ROM bytes,
/// so it is not for sharing.
pub const ASSEMBLY_MESSAGE: &str = "Holds ROM bytes: keep it local";

/// The text for an export. Byte columns are added from the listing's own
/// lines so the core's export stays byte-exact.
pub fn generate(kind: ExportKind, include_bytes: bool, wb: &Workbench) -> String {
    match kind {
        ExportKind::Assembly => {
            let listing = wb.export_asar(None, None);
            if include_bytes {
                with_byte_columns(&listing, wb)
            } else {
                listing
            }
        }
        ExportKind::Annotations => wb.export_symbols(false),
        ExportKind::Symbols => wb.export_symbols(true),
    }
}

/// Width in bytes of a `db`, `dw` or `dl` directive's head, if it is one.
fn data_width(head: &str) -> Option<u32> {
    match head {
        h if h.starts_with("db") => Some(1),
        h if h.starts_with("dw") => Some(2),
        h if h.starts_with("dl") => Some(3),
        _ => None,
    }
}

/// Prefix each instruction line with its bytes as a comment column.
pub fn with_byte_columns(listing: &str, wb: &Workbench) -> String {
    let rom = wb.rom();
    let mut out = String::with_capacity(listing.len() + listing.len() / 4);
    let mut pc: Option<u32> = None;
    for line in listing.lines() {
        let trimmed = line.trim();
        if let Some(hex) = trimmed.strip_prefix("org $")
            && let Ok(v) = u32::from_str_radix(hex, 16)
        {
            pc = rom.file_offset_for(v);
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if line.starts_with("  ")
            && let Some(offset) = pc
            && !trimmed.starts_with(';')
        {
            let head = trimmed.split(' ').next().unwrap_or("");
            let len = if let Some(width) = data_width(head) {
                let body = trimmed.split(';').next().unwrap_or(trimmed);
                body[head.len().min(body.len())..].split(',').count() as u32 * width
            } else if let Some(insn) = wb.instruction_at(offset)
                && insn.file_offset == offset
            {
                u32::from(insn.len)
            } else {
                0
            };
            if len > 0 {
                let bytes = wb
                    .disassemble(offset, 1, None)
                    .first()
                    .map(|i| i.bytes.clone())
                    .unwrap_or_default();
                let hex = bytes
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push_str(&format!(
                    "  ; {}  {hex:<11}\n",
                    romlens_ffi::format_file_offset(offset)
                ));
                pc = Some(offset + len);
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    // Every line was given a newline; keep the text's own ending.
    if !listing.ends_with('\n') {
        out.pop();
    }
    out
}

fn rewritten_lines(r: &ImportResult) -> Option<String> {
    if r.rewritten.is_empty() {
        return None;
    }
    let shown = r
        .rewritten
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n  ");
    let more = if r.rewritten.len() > 8 {
        format!("\n  … and {} more", r.rewritten.len() - 8)
    } else {
        String::new()
    };
    Some(format!(
        "{} names rewritten to be usable:\n  {shown}{more}",
        r.rewritten.len()
    ))
}

/// Everything the core reported, including what it could not use. An importer
/// that rewrote forty names and did not say so would be the exact failure the
/// importers are written to avoid.
pub fn summary(kind: ImportKind, r: &ImportResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    match kind {
        ImportKind::Trace => {
            lines.push(format!(
                "{} bytes executed, {} read, as {}.",
                r.executed_bytes, r.read_bytes, r.format
            ));
            if r.has_widths {
                lines.push("The recorded M/X widths now steer the disassembler.".into());
            }
            if !r.detail.is_empty() {
                lines.push(format!(
                    "Execution log: {}. Its calls, jumps and reads join the references, marked “seen”.",
                    r.detail
                ));
            }
        }
        ImportKind::Symbols => {
            lines.push(format!(
                "{} labels added, {} replaced, {} comments added, as {}.",
                r.labels_added, r.labels_replaced, r.comments_added, r.format
            ));
            if r.kept_user > 0 {
                lines.push(format!("{} kept: you had already named them.", r.kept_user));
            }
            lines.extend(rewritten_lines(r));
            if !r.skipped.is_empty() {
                lines.push(format!("{} lines not understood.", r.skipped.len()));
            }
        }
        ImportKind::Dbg => {
            lines.push(format!(
                "{} labels added, {} replaced. {}.",
                r.labels_added, r.labels_replaced, r.detail
            ));
            if r.kept_user > 0 {
                lines.push(format!("{} kept: you had already named them.", r.kept_user));
            }
            lines.extend(rewritten_lines(r));
            if !r.skipped.is_empty() {
                lines.push(format!("{} records not understood.", r.skipped.len()));
            }
            lines.push("The Source tab shows each line beside the bytes it made.".into());
        }
    }
    if !r.notice.is_empty() {
        lines.push(format!("Notice kept with the project:\n{}", r.notice));
    }
    lines.join("\n\n")
}

/// The core call for an import, run wherever the caller likes (it is `Send`).
pub fn run_import(
    kind: ImportKind,
    wb: &Workbench,
    path: &std::path::Path,
) -> Result<ImportResult, romlens_ffi::RomlensError> {
    let io = |e: std::io::Error| romlens_ffi::RomlensError::Io {
        msg: format!("{}: {e}", path.display()),
    };
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    match kind {
        ImportKind::Trace => wb.import_trace(name, std::fs::read(path).map_err(io)?),
        ImportKind::Symbols => wb.import_symbols(name, read_text(path).map_err(io)?),
        ImportKind::Dbg => {
            let dir = path
                .parent()
                .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
            wb.import_dbg(name, dir, read_text(path).map_err(io)?)
        }
    }
}

/// A text file as UTF-8, replacing what is not: symbol files from old tools
/// are not always clean, and one bad byte should not refuse the whole file.
fn read_text(path: &std::path::Path) -> std::io::Result<String> {
    Ok(String::from_utf8_lossy(&std::fs::read(path)?).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing::test_rom;

    fn result() -> ImportResult {
        ImportResult {
            source: "game.sym".into(),
            format: "WLA-DX".into(),
            labels_added: 12,
            labels_replaced: 2,
            comments_added: 3,
            kept_user: 0,
            rewritten: vec![],
            skipped: vec![],
            notice: String::new(),
            detail: String::new(),
            executed_bytes: 0,
            read_bytes: 0,
            has_widths: false,
        }
    }

    fn analysed() -> std::sync::Arc<Workbench> {
        let wb = Workbench::new(test_rom());
        wb.analyze_blocking().unwrap();
        wb
    }

    #[test]
    fn a_symbol_report_says_what_it_did_and_what_it_could_not() {
        let mut r = result();
        let plain = summary(ImportKind::Symbols, &r);
        assert_eq!(
            plain,
            "12 labels added, 2 replaced, 3 comments added, as WLA-DX."
        );
        r.kept_user = 4;
        r.skipped = vec!["junk".into(); 2];
        r.rewritten = (0..10).map(|i| format!("a.{i} -> a_{i}")).collect();
        r.notice = "License: MIT".into();
        let s = summary(ImportKind::Symbols, &r);
        assert!(s.contains("4 kept: you had already named them."));
        assert!(s.contains("10 names rewritten to be usable:\n  a.0 -> a_0"));
        assert!(s.contains("… and 2 more"));
        assert!(!s.contains("a.8"), "only eight are listed");
        assert!(s.contains("2 lines not understood."));
        assert!(s.ends_with("Notice kept with the project:\nLicense: MIT"));
    }

    #[test]
    fn a_trace_report_mentions_widths_and_logs() {
        let mut r = result();
        r.executed_bytes = 4096;
        r.read_bytes = 100;
        r.format = "Mesen CDL".into();
        assert_eq!(
            summary(ImportKind::Trace, &r),
            "4096 bytes executed, 100 read, as Mesen CDL."
        );
        r.has_widths = true;
        r.detail = "770 instructions".into();
        let s = summary(ImportKind::Trace, &r);
        assert!(s.contains("M/X widths now steer the disassembler"));
        assert!(s.contains("Execution log: 770 instructions."));
    }

    #[test]
    fn a_dbg_report_points_at_the_source_tab() {
        let mut r = result();
        r.detail = "5 files".into();
        r.skipped = vec!["x".into()];
        let s = summary(ImportKind::Dbg, &r);
        assert!(s.starts_with("12 labels added, 2 replaced. 5 files."));
        assert!(s.contains("1 records not understood."));
        assert!(s.ends_with("The Source tab shows each line beside the bytes it made."));
    }

    #[test]
    fn kinds_name_their_files() {
        assert_eq!(ExportKind::Assembly.extension(), "asm");
        assert_eq!(ExportKind::Annotations.extension(), "sym");
        assert!(ImportKind::Trace.extensions().contains(&"mxlog"));
        assert_eq!(ImportKind::Dbg.extensions(), ["dbg"]);
    }

    #[test]
    fn the_exports_are_what_the_core_writes() {
        let wb = analysed();
        let asm = generate(ExportKind::Assembly, false, &wb);
        assert!(asm.contains("org $"), "{asm}");
        let user = generate(ExportKind::Annotations, false, &wb);
        let all = generate(ExportKind::Symbols, false, &wb);
        // Only a person's names are share-safe; the full file adds the analyzer's.
        assert!(all.len() >= user.len());
        assert_eq!(asm, wb.export_asar(None, None));
    }

    #[test]
    fn byte_columns_follow_each_instruction_and_change_nothing_else() {
        let wb = analysed();
        let plain = generate(ExportKind::Assembly, false, &wb);
        let with = generate(ExportKind::Assembly, true, &wb);
        assert!(with.len() > plain.len());
        // Every original line is still there, in order.
        let kept: Vec<_> = with
            .lines()
            .filter(|l| !l.trim_start().starts_with("; 0x"))
            .collect();
        assert_eq!(kept, plain.lines().collect::<Vec<_>>());
        // The test ROM starts with SEI (78) then CLC (18).
        assert!(with.contains("; 0x000000  78"), "{with}");
        assert!(with.contains("; 0x000001  18"), "{with}");
    }

    #[test]
    fn importing_reads_the_file_and_reports_the_cores_message() {
        let wb = analysed();
        let dir = std::env::temp_dir().join(format!("romlens-imp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sym = dir.join("game.sym");
        std::fs::write(&sym, "[labels]\n00:8000 Start\n").unwrap();
        let r = run_import(ImportKind::Symbols, &wb, &sym).unwrap();
        assert_eq!(r.source, "game.sym");
        assert_eq!(r.labels_added, 1);
        assert_eq!(wb.labels().iter().filter(|l| l.name == "Start").count(), 1);
        let missing = run_import(ImportKind::Symbols, &wb, &dir.join("nope.sym")).unwrap_err();
        assert!(missing.to_string().contains("nope.sym"), "{missing}");
        let empty = dir.join("empty.cdl");
        std::fs::write(&empty, b"").unwrap();
        assert!(run_import(ImportKind::Trace, &wb, &empty).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
