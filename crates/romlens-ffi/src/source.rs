//! Source lines from an imported `.dbg`, for the Source tab (docs/22, S2).

use romlens_core::FileOffset;
use romlens_core::model::source_map::{LineKind, SourceLine};

use crate::records::ByteRange;
use crate::workbench::Workbench;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SourceLineKind {
    /// The program's own line.
    Source,
    /// A macro's line, once per expansion.
    Macro,
    /// Another tool's line (a compiler's).
    External,
}

impl From<LineKind> for SourceLineKind {
    fn from(k: LineKind) -> Self {
        match k {
            LineKind::Source => SourceLineKind::Source,
            LineKind::Macro => SourceLineKind::Macro,
            LineKind::External => SourceLineKind::External,
        }
    }
}

/// A source file of an import.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SourceFileInfo {
    /// Which import, an index into `source_files`' maps.
    pub map: u32,
    /// Which file of it.
    pub file: u32,
    /// As the assembler was given it.
    pub name: String,
    /// Where it is looked for: the name from the `.dbg`'s folder.
    pub path: String,
    /// The `.dbg` it came from.
    pub import: String,
    /// Its size when assembled, to tell whether the file found is that one.
    pub size: u32,
    /// Lines of it that made bytes.
    pub lines: u32,
}

/// A line that made bytes.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SourceLineInfo {
    pub map: u32,
    pub file: u32,
    /// 1-based.
    pub line: u32,
    pub kind: SourceLineKind,
    /// Sorted.
    pub ranges: Vec<ByteRange>,
}

fn info(map: u32, l: &SourceLine) -> SourceLineInfo {
    SourceLineInfo {
        map,
        file: l.file,
        line: l.line,
        kind: l.kind.into(),
        ranges: l
            .ranges
            .iter()
            .map(|(s, n)| ByteRange {
                start: s.0,
                len: *n,
            })
            .collect(),
    }
}

#[uniffi::export]
impl Workbench {
    /// Every source file the project's imported `.dbg` files name.
    pub fn source_files(&self) -> Vec<SourceFileInfo> {
        self.with_project(|p| {
            let mut out = Vec::new();
            for (mi, m) in p.source_maps.iter().enumerate() {
                for (fi, f) in m.files.iter().enumerate() {
                    out.push(SourceFileInfo {
                        map: mi as u32,
                        file: fi as u32,
                        name: f.name.clone(),
                        path: m
                            .path_of(fi as u32)
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                        import: m.source.clone(),
                        size: f.size,
                        lines: m.lines.iter().filter(|l| l.file == fi as u32).count() as u32,
                    });
                }
            }
            out
        })
    }

    /// The lines that made the byte at `file_offset`: the program's own
    /// first, then a macro's inside it, each narrowest first.
    pub fn source_lines_at(&self, file_offset: u32) -> Vec<SourceLineInfo> {
        self.with_project(|p| {
            p.source_maps
                .iter()
                .enumerate()
                .flat_map(|(mi, m)| {
                    m.lines_at(FileOffset(file_offset))
                        .into_iter()
                        .map(move |l| info(mi as u32, l))
                })
                .collect()
        })
    }

    /// Every line of one file that made bytes, in line order.
    pub fn source_file_lines(&self, map: u32, file: u32) -> Vec<SourceLineInfo> {
        self.with_project(|p| {
            p.source_maps
                .get(map as usize)
                .map(|m| {
                    m.lines
                        .iter()
                        .filter(|l| l.file == file)
                        .map(|l| info(map, l))
                        .collect()
                })
                .unwrap_or_default()
        })
    }
}
