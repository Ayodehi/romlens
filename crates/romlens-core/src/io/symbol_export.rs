//! bsnes-plus / WLA style `.sym`: `[labels]` with `bb:aaaa NAME` lines, then
//! `[comments]`. `include_auto = false` is the share-safe export (labels and
//! comments only, no ROM bytes; docs/12).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::model::comment::CommentKind;
use crate::model::label::LabelSource;
use crate::model::project::Project;
use crate::model::symbols::Symbols;
use crate::rom::image::RomImage;

pub fn export_symbols(
    rom: &RomImage,
    snap: &AnalysisSnapshot,
    project: &Project,
    include_auto: bool,
    out: &mut String,
) {
    let symbols = Symbols::new(rom, project, &snap.auto_labels);
    let _ = writeln!(
        out,
        "; Romlens symbols for {:?} (SHA-256 {})",
        rom.header().title,
        rom.sha256_hex()
    );
    out.push_str("[labels]\n");
    for l in symbols.all_labels() {
        if !include_auto && l.source == LabelSource::Auto {
            continue;
        }
        let _ = writeln!(
            out,
            "{:02X}:{:04X} {}",
            l.address.bank(),
            l.address.offset(),
            l.name
        );
    }
    out.push_str("\n[comments]\n");
    let mut by_address: BTreeMap<_, (Option<&str>, Option<&str>)> = BTreeMap::new();
    for ((addr, kind), c) in &project.comments {
        let e = by_address.entry(*addr).or_default();
        match kind {
            CommentKind::Line => e.0 = Some(c.text.as_str()),
            CommentKind::Block => e.1 = Some(c.text.as_str()),
        }
    }
    for (addr, (line, block)) in by_address {
        let mut parts: Vec<&str> = Vec::new();
        if let Some(l) = line {
            parts.push(l);
        }
        if let Some(b) = block {
            parts.extend(b.lines());
        }
        let _ = writeln!(
            out,
            "{:02X}:{:04X} {}",
            addr.bank(),
            addr.offset(),
            parts.join(" | ")
        );
    }
}
