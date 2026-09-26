//! Which source line assembled which ROM bytes, from an assembler's debug
//! information (ca65's `.dbg`, `io::import::dbg`).
//!
//! A line may have made bytes in several places (a macro's line, once per
//! expansion) and a byte may come from several lines (the line that invoked
//! a macro and the macro's own line inside it), so both directions are kept.

use crate::memory::address::FileOffset;

/// A source file the debug information names. The text is not kept: it is
/// found beside the `.dbg` when shown, as the assembler read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// As the assembler was given it, relative to where it ran.
    pub name: String,
    pub size: u32,
    /// Seconds since 1970, as the assembler saw the file; 0 when unknown.
    pub mtime: u32,
}

/// What kind of line made the bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LineKind {
    /// A line of the program's own source.
    Source,
    /// A line of a macro's body, once per expansion.
    Macro,
    /// A line of source the assembler was given from elsewhere (a
    /// compiler's output, cc65's `.dbg` type 1).
    External,
}

impl LineKind {
    pub const fn name(self) -> &'static str {
        match self {
            LineKind::Source => "source",
            LineKind::Macro => "macro",
            LineKind::External => "external",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "source" => Some(LineKind::Source),
            "macro" => Some(LineKind::Macro),
            "external" => Some(LineKind::External),
            _ => None,
        }
    }
}

/// One source line and the ROM bytes it made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLine {
    /// An index into `SourceMap::files`.
    pub file: u32,
    /// 1-based, as an editor counts.
    pub line: u32,
    pub kind: LineKind,
    /// Sorted, `(start, length)` in the ROM.
    pub ranges: Vec<(FileOffset, u32)>,
}

/// One debug-information file's lines, as imported.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceMap {
    /// The file it came from, as the import names it.
    pub source: String,
    /// The folder it was read from, where its sources are looked for.
    pub dir: String,
    pub files: Vec<SourceFile>,
    /// Sorted by file, then line, then kind.
    pub lines: Vec<SourceLine>,
}

impl SourceMap {
    /// The lines that made the byte at `offset`: the program's own line
    /// first, then the macro lines inside it, each narrowest first.
    pub fn lines_at(&self, offset: FileOffset) -> Vec<&SourceLine> {
        let mut hits: Vec<(&SourceLine, u32)> = self
            .lines
            .iter()
            .filter_map(|l| {
                l.ranges
                    .iter()
                    .find(|(s, n)| offset.0 >= s.0 && offset.0 < s.0 + n)
                    .map(|(_, n)| (l, *n))
            })
            .collect();
        hits.sort_by_key(|(l, n)| (l.kind, *n, l.file, l.line));
        hits.into_iter().map(|(l, _)| l).collect()
    }

    /// The line `line` of file `file`, where it made bytes.
    pub fn line(&self, file: u32, line: u32) -> Option<&SourceLine> {
        self.lines
            .iter()
            .find(|l| l.file == file && l.line == line && l.kind != LineKind::Macro)
            .or_else(|| self.lines.iter().find(|l| l.file == file && l.line == line))
    }

    /// Where source file `file` is looked for: its name, as the assembler
    /// was given it, from the folder the `.dbg` was read from.
    pub fn path_of(&self, file: u32) -> Option<std::path::PathBuf> {
        let f = self.files.get(file as usize)?;
        let name = std::path::Path::new(&f.name);
        Some(if name.is_absolute() {
            name.to_path_buf()
        } else {
            std::path::Path::new(&self.dir).join(name)
        })
    }

    /// The index of the file named `name`, or ending in it after a `/`.
    pub fn file_named(&self, name: &str) -> Option<u32> {
        let exact = self.files.iter().position(|f| f.name == name);
        exact
            .or_else(|| {
                self.files
                    .iter()
                    .position(|f| f.name.ends_with(&format!("/{name}")))
            })
            .map(|i| i as u32)
    }

    /// Every ROM byte some line made.
    pub fn bytes_covered(&self) -> u64 {
        let mut ranges: Vec<(u32, u32)> = self
            .lines
            .iter()
            .flat_map(|l| l.ranges.iter().map(|(s, n)| (s.0, s.0 + n)))
            .collect();
        ranges.sort_unstable();
        let (mut total, mut end) = (0u64, 0u32);
        for (s, e) in ranges {
            let s = s.max(end);
            if e > s {
                total += (e - s) as u64;
                end = e;
            }
        }
        total
    }
}
