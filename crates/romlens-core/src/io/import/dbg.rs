//! ca65's debug information: the `.dbg` file ld65 writes with `--dbgfile`.
//!
//! Version 2 of the format, a line per record: `kind\tkey=value,…`, the
//! values numbers (decimal or `0x`), quoted strings, or ids joined by `+`.
//! What this reads:
//!
//! - `seg`: a segment, where it runs (`start`) and, for one written to the
//!   output, where it is in the file (`oname`, `ooffs`);
//! - `span`: bytes of a segment (`seg`, `start`, `size`);
//! - `line`: a source line (`file`, `line`, `type`: 0 the program's own, 1
//!   another tool's, 2 a macro's) and the spans it made;
//! - `file`: the source files, by name;
//! - `scope` and `sym`: the labels, with the scopes that qualify them.
//!
//! Symbols become labels the way the other symbol files' do
//! (`symbols::plan`), and the lines become a [`SourceMap`] the project
//! keeps. Where a segment was written is the only thing tying a span to the
//! ROM, so a ROM built from another output file, or a `.dbg` from another
//! build, maps nothing; the import says how much it placed.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::error::ProjectError;
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::source_map::{LineKind, SourceFile, SourceLine, SourceMap};
use crate::rom::image::RomImage;

use super::symbols::{SymbolFile, sanitize_imported_label, unique};

/// How a `.dbg` file starts.
pub const MAGIC: &[u8] = b"version\tmajor=2";

/// Whether `bytes` look like ld65's debug information.
pub fn looks_like(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// What an import found, before anything is applied.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DbgFile {
    /// The labels, for `symbols::plan`.
    pub symbols: SymbolFile,
    pub map: SourceMap,
    /// The output file whose segments were placed in the ROM.
    pub output: String,
    /// Symbols that are not labels: equates, imports, constants.
    pub not_labels: usize,
    /// Spans left out: in a segment not written to `output`, or past the
    /// ROM's end.
    pub spans_unplaced: usize,
}

/// One record's fields.
type Fields = HashMap<String, String>;

fn fields(text: &str) -> Fields {
    let mut out = Fields::new();
    let (mut key, mut value) = (String::new(), String::new());
    let (mut in_value, mut quoted) = (false, false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    value.push(n);
                }
            }
            '=' if !in_value && !quoted => in_value = true,
            ',' if !quoted => {
                out.insert(std::mem::take(&mut key), std::mem::take(&mut value));
                in_value = false;
            }
            _ if in_value => value.push(c),
            _ => key.push(c),
        }
    }
    if !key.is_empty() {
        out.insert(key, value);
    }
    out
}

fn number(f: &Fields, key: &str) -> Option<u32> {
    let v = f.get(key)?;
    match v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => v.parse().ok(),
    }
}

fn ids(f: &Fields, key: &str) -> Vec<u32> {
    f.get(key)
        .map(|v| v.split('+').filter_map(|p| p.parse().ok()).collect())
        .unwrap_or_default()
}

struct Seg {
    start: u32,
    size: u32,
    output: Option<(String, u32)>,
}

struct Span {
    seg: u32,
    start: u32,
    size: u32,
}

struct Scope {
    name: String,
    parent: Option<u32>,
}

struct Sym {
    name: String,
    kind: String,
    val: Option<u32>,
    seg: Option<u32>,
    scope: Option<u32>,
    parent: Option<u32>,
}

/// Read a `.dbg` against the ROM it describes. `source` names the file and
/// `dir` is the folder it was read from.
pub fn read(text: &str, rom: &RomImage, source: &str, dir: &str) -> Result<DbgFile, ProjectError> {
    let mut version = None;
    let mut files: BTreeMap<u32, SourceFile> = BTreeMap::new();
    let mut segs: HashMap<u32, Seg> = HashMap::new();
    let mut spans: HashMap<u32, Span> = HashMap::new();
    let mut scopes: HashMap<u32, Scope> = HashMap::new();
    let mut syms: BTreeMap<u32, Sym> = BTreeMap::new();
    let mut lines: Vec<(u32, u32, LineKind, Vec<u32>)> = Vec::new();
    let mut skipped = Vec::new();

    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let Some((kind, rest)) = line.split_once(['\t', ' ']) else {
            skipped.push(line.to_owned());
            continue;
        };
        let f = fields(rest.trim());
        let id = number(&f, "id");
        match (kind, id) {
            ("version", _) => version = number(&f, "major").zip(number(&f, "minor")),
            ("info", _) => {}
            ("file", Some(id)) => {
                files.insert(
                    id,
                    SourceFile {
                        name: f.get("name").cloned().unwrap_or_default(),
                        size: number(&f, "size").unwrap_or(0),
                        mtime: number(&f, "mtime").unwrap_or(0),
                    },
                );
            }
            ("seg", Some(id)) => {
                let output = f.get("oname").cloned().zip(number(&f, "ooffs"));
                segs.insert(
                    id,
                    Seg {
                        start: number(&f, "start").unwrap_or(0),
                        size: number(&f, "size").unwrap_or(0),
                        output,
                    },
                );
            }
            ("span", Some(id)) => {
                if let (Some(seg), Some(start), Some(size)) =
                    (number(&f, "seg"), number(&f, "start"), number(&f, "size"))
                {
                    spans.insert(id, Span { seg, start, size });
                }
            }
            ("line", Some(_)) => {
                let (Some(file), Some(n)) = (number(&f, "file"), number(&f, "line")) else {
                    skipped.push(line.to_owned());
                    continue;
                };
                let kind = match number(&f, "type").unwrap_or(0) {
                    2 => LineKind::Macro,
                    1 => LineKind::External,
                    _ => LineKind::Source,
                };
                let span = ids(&f, "span");
                if !span.is_empty() {
                    lines.push((file, n, kind, span));
                }
            }
            ("scope", Some(id)) => {
                scopes.insert(
                    id,
                    Scope {
                        name: f.get("name").cloned().unwrap_or_default(),
                        parent: number(&f, "parent"),
                    },
                );
            }
            ("sym", Some(id)) => {
                syms.insert(
                    id,
                    Sym {
                        name: f.get("name").cloned().unwrap_or_default(),
                        kind: f.get("type").cloned().unwrap_or_default(),
                        val: number(&f, "val"),
                        seg: number(&f, "seg"),
                        scope: number(&f, "scope"),
                        parent: number(&f, "parent"),
                    },
                );
            }
            ("mod" | "lib" | "type" | "csym", _) => {}
            _ => skipped.push(line.to_owned()),
        }
    }
    match version {
        Some((2, _)) => {}
        Some((major, minor)) => {
            return Err(ProjectError::BadFormat(format!(
                "ld65 debug information version {major}.{minor}; this reads version 2"
            )));
        }
        None => {
            return Err(ProjectError::BadFormat(
                "not ld65 debug information: no version record".into(),
            ));
        }
    }

    // The output whose segments are this ROM: the one named like the ROM,
    // else the one with the most bytes.
    let mut outputs: BTreeMap<&str, u64> = BTreeMap::new();
    for s in segs.values() {
        if let Some((name, _)) = &s.output {
            *outputs.entry(name.as_str()).or_default() += s.size as u64;
        }
    }
    let rom_name = std::path::Path::new(rom.source_name())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let output = outputs
        .keys()
        .find(|n| {
            std::path::Path::new(n)
                .file_name()
                .is_some_and(|f| f.to_string_lossy() == rom_name)
        })
        .or_else(|| outputs.iter().max_by_key(|(_, n)| **n).map(|(k, _)| k))
        .map(|n| n.to_string())
        .unwrap_or_default();
    // ld65's offsets count from the file's start; a copier header is not
    // part of what the ROM image holds.
    let header = if rom.has_copier_header() { 512 } else { 0 };
    let rom_len = rom.len() as u32;
    let place = |seg: &Seg, at: u32| -> Option<FileOffset> {
        let (name, ooffs) = seg.output.as_ref()?;
        if *name != output {
            return None;
        }
        let off = (ooffs + at).checked_sub(header)?;
        (off < rom_len).then_some(FileOffset(off))
    };

    let mut file = DbgFile {
        output: output.clone(),
        ..Default::default()
    };
    let index: BTreeMap<u32, u32> = files
        .keys()
        .enumerate()
        .map(|(i, id)| (*id, i as u32))
        .collect();
    let mut map_lines = Vec::new();
    for (f, n, kind, span_ids) in lines {
        let Some(&fi) = index.get(&f) else { continue };
        let mut ranges = Vec::new();
        for id in span_ids {
            let placed = spans.get(&id).and_then(|sp| {
                let seg = segs.get(&sp.seg)?;
                let start = place(seg, sp.start)?;
                Some((start, sp.size.min(rom_len - start.0)))
            });
            match placed {
                Some(r) if r.1 > 0 => ranges.push(r),
                Some(_) => {}
                None => {
                    // A span in RAM (BSS, zero page) makes no ROM bytes, so it
                    // is not a loss; one in another output is.
                    if spans
                        .get(&id)
                        .and_then(|sp| segs.get(&sp.seg))
                        .is_some_and(|s| s.output.is_some())
                    {
                        file.spans_unplaced += 1;
                    }
                }
            }
        }
        if ranges.is_empty() {
            continue;
        }
        ranges.sort_unstable_by_key(|r| r.0);
        ranges.dedup();
        map_lines.push(SourceLine {
            file: fi,
            line: n,
            kind,
            ranges,
        });
    }
    map_lines.sort_by_key(|l| (l.file, l.line, l.kind));
    file.map = SourceMap {
        source: source.to_owned(),
        dir: dir.to_owned(),
        files: files.into_values().collect(),
        lines: map_lines,
    };

    // Labels: each symbol at the address it names, qualified by its scopes
    // (`Player::Draw`) and a cheap local by its parent (`ClearBuffer@loop`).
    let scope_path = |mut id: Option<u32>| -> String {
        let mut parts = Vec::new();
        while let Some(s) = id.and_then(|i| scopes.get(&i)) {
            if !s.name.is_empty() {
                parts.push(s.name.as_str());
            }
            id = s.parent;
        }
        parts.reverse();
        parts.join("::")
    };
    let full_name = |sym: &Sym| -> String {
        let prefix = match (sym.parent, sym.scope) {
            (Some(p), _) => syms.get(&p).map(|p| {
                let outer = scope_path(p.scope);
                if outer.is_empty() {
                    p.name.clone()
                } else {
                    format!("{outer}::{}", p.name)
                }
            }),
            (None, s) => Some(scope_path(s)),
        }
        .unwrap_or_default();
        match (prefix.is_empty(), sym.name.starts_with('@')) {
            (true, _) => sym.name.clone(),
            (false, true) => format!("{prefix}{}", sym.name),
            (false, false) => format!("{prefix}::{}", sym.name),
        }
    };
    let mut named: BTreeMap<SnesAddress, String> = BTreeMap::new();
    for sym in syms.values() {
        let Some(val) = sym.val.filter(|_| sym.kind == "lab") else {
            file.not_labels += 1;
            continue;
        };
        let seg = sym.seg.and_then(|s| segs.get(&s));
        let address = match seg {
            // In the ROM: where the segment was written.
            Some(seg) if seg.output.is_some() => {
                let at = val
                    .checked_sub(seg.start)
                    .filter(|d| *d < seg.size.max(1))
                    .or_else(|| {
                        (val & 0xFFFF)
                            .checked_sub(seg.start & 0xFFFF)
                            .filter(|d| *d < seg.size.max(1))
                    });
                at.and_then(|at| place(seg, at))
                    .and_then(|off| rom.snes_address_for(off))
            }
            // RAM, or anywhere else: the address as the program uses it.
            _ => (val <= 0xFF_FFFF).then(|| SnesAddress::from_u24(val)),
        };
        let Some(address) = address else {
            file.spans_unplaced += 1;
            continue;
        };
        // The first symbol at an address names it, but a routine's name
        // beats a cheap local at its first byte.
        let name = full_name(sym);
        match named.get(&address) {
            None => {
                named.insert(address, name);
            }
            Some(e) if e.contains('@') && !name.contains('@') => {
                named.insert(address, name);
            }
            Some(_) => {}
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    for (address, name) in named {
        let clean = unique(sanitize_imported_label(&name), &mut seen);
        if clean != name {
            file.symbols.rewritten.push((name, clean.clone()));
        }
        file.symbols.labels.insert(address, clean);
    }
    file.symbols.skipped = skipped;
    if file.map.lines.is_empty() && file.symbols.labels.is_empty() {
        return Err(ProjectError::BadFormat(format!(
            "{source} places nothing in this ROM: none of its segments were written to {}",
            if output.is_empty() {
                "an output file".to_owned()
            } else {
                output
            }
        )));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_keep_quoted_commas_and_lists() {
        let f = fields(r#"id=3,name="a,b",span=4+5,val=0x8000"#);
        assert_eq!(f["name"], "a,b");
        assert_eq!(ids(&f, "span"), vec![4, 5]);
        assert_eq!(number(&f, "val"), Some(0x8000));
        assert_eq!(number(&f, "id"), Some(3));
    }
}
