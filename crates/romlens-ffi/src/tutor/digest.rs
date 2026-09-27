//! The ROM digest (docs/24): what a conversation is told about the ROM
//! once, when it starts, so it caches with the system prompt. Anything
//! that changes later the model reads through its tools.

use super::tools::addr;
use crate::records::LabelSource;
use crate::workbench::Workbench;

pub fn digest(wb: &Workbench) -> String {
    let i = wb.rom().info();
    let s = wb.stats();
    let named = |a: u16| {
        let full = a as u32;
        match wb.label_at(full) {
            Some(l) => format!("{} ({})", addr(full), l.name),
            None => addr(full),
        }
    };
    let labels = wb.labels();
    let count = |k: LabelSource| labels.iter().filter(|l| l.source == k).count();
    let mut out = format!(
        "# The ROM the student has open\n\n\
         - Title: {}\n- Mapping: {}{}, {} bytes\n- Region: {}\n- Checksum: {}\n\
         - RESET {}, NMI {}, IRQ {}\n\
         - Classified: {} bytes of code, {} of data, {} unknown; {} instructions\n\
         - Labels: {} from the student, {} imported, {} from the analysis",
        i.title.trim(),
        i.mapping_name,
        if i.fast_rom { ", FastROM" } else { "" },
        i.byte_len,
        i.region_name,
        if i.checksum_ok {
            "matches"
        } else {
            "does not match (a hack or a bad dump)"
        },
        named(i.emulation.reset),
        named(i.native.nmi),
        named(i.native.irq),
        s.code_bytes,
        s.data_bytes,
        s.unknown_bytes,
        s.instructions,
        count(LabelSource::User),
        count(LabelSource::Imported),
        count(LabelSource::Auto),
    );
    let imports = wb.imports();
    if !imports.is_empty() {
        out.push_str("\n- Imported: ");
        let names: Vec<String> = imports
            .iter()
            .map(|m| format!("{} ({}, {} labels)", m.source, m.format, m.labels_added))
            .collect();
        out.push_str(&names.join("; "));
    }
    let recs = wb.recordings();
    if !recs.is_empty() {
        out.push_str(&format!(
            "\n- Recordings attached to the project: {}",
            recs.len()
        ));
    }
    out
}
