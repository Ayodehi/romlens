//! asar-syntax listing. Every sizeable operand carries an explicit `.b/.w/.l`
//! suffix so reassembly is byte-exact whatever the assembler believes about
//! M and X; `; flags:` comments at labels are aids only. Labelled operands
//! are used only where the label's address equals the operand's resolved
//! address exactly; everything else stays numeric. Hardware registers stay
//! numeric with the register name as a comment.

use std::fmt::Write as _;

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::cpu65816::{AddressingMode, Instruction, Operand, format_instruction};
use crate::memory::address::{FileOffset, SnesAddress};
use crate::memory::map::MappingMode;
use crate::model::comment::CommentKind;
use crate::model::project::Project;
use crate::model::region::{DataKind, RegionKind};
use crate::model::symbols::Symbols;
use crate::rom::image::RomImage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsarOptions {
    /// `(start, len)` in file offsets; `None` for the whole image.
    pub range: Option<(u32, u32)>,
    /// Include user comments.
    pub comments: bool,
}

impl Default for AsarOptions {
    fn default() -> Self {
        Self {
            range: None,
            comments: true,
        }
    }
}

fn bank_size(rom: &RomImage) -> u32 {
    match rom.mapping() {
        MappingMode::LoRom => 0x8000,
        _ => 0x1_0000,
    }
}

/// `.b`, `.w`, `.l` for a mode and operand.
pub fn width_suffix(mode: AddressingMode, operand: Operand) -> &'static str {
    use AddressingMode::*;
    match mode {
        Implied | Accumulator | Relative8 | Relative16 | BlockMove => "",
        ImmediateM | ImmediateX => match operand {
            Operand::Word(_) => ".w",
            _ => ".b",
        },
        Immediate8 => ".b",
        AbsoluteLong | AbsoluteLongX => ".l",
        m if m.is_absolute() => ".w",
        _ => ".b",
    }
}

/// The instruction text for the listing.
pub fn asar_instruction(insn: &Instruction, symbols: &Symbols<'_>) -> String {
    // Use the label only when it names the resolved address exactly and the
    // mode is not direct page (DP arithmetic is not expressible as a label).
    struct Exact<'a> {
        inner: &'a Symbols<'a>,
        insn: &'a Instruction,
    }
    impl crate::cpu65816::SymbolLookup for Exact<'_> {
        fn name_for(&self, address: SnesAddress) -> Option<crate::cpu65816::Symbol> {
            if self.insn.mode.is_direct() {
                return None;
            }
            let label = self.inner.label_at(address)?;
            (label.address == address).then(|| crate::cpu65816::Symbol {
                name: label.name.clone(),
                user: label.source.is_user_or_imported(),
            })
        }
    }
    let f = format_instruction(
        insn,
        &Exact {
            inner: symbols,
            insn,
        },
    );
    let suffix = width_suffix(insn.mode, insn.operand);
    // asar takes a number after a branch as the displacement itself, not
    // the target: one with no label to name goes as its displacement.
    // asar writes a block move's operands in the order of its bytes,
    // destination bank first; the listing reads source first, as WDC does.
    if insn.mode == AddressingMode::BlockMove {
        let (dst, src) = (insn.bytes[1], insn.bytes[2]);
        return format!(
            "{} ${dst:02X},${src:02X}  ; from bank ${src:02X} to bank ${dst:02X}",
            insn.mnemonic.as_str()
        );
    }
    let relative = matches!(
        insn.mode,
        AddressingMode::Relative8 | AddressingMode::Relative16
    );
    let target = insn.target.map(|t| t.address).filter(|_| relative);
    if let Some(t) = target
        && symbols.label_at(t).is_none_or(|l| l.address != t)
    {
        let d = match insn.operand {
            Operand::Byte(b) => format!("${b:02X}"),
            Operand::Word(w) => format!("${w:04X}"),
            _ => String::new(),
        };
        return format!("{} {d}  ; to ${:06X}", insn.mnemonic.as_str(), t.as_u24());
    }
    let mut text = String::with_capacity(f.text.len() + 4);
    text.push_str(insn.mnemonic.as_str());
    text.push_str(suffix);
    if insn.mode == AddressingMode::Accumulator {
        text.push_str(" A");
    } else if !f.operand_text.is_empty() {
        text.push(' ');
        text.push_str(&f.operand_text);
    }
    text
}

/// A branch whose target is reached by the program counter wrapping
/// within the bank, and that target.
fn wrapped_branch(insn: &Instruction) -> Option<SnesAddress> {
    let d = match (insn.mode, insn.operand) {
        (AddressingMode::Relative8, Operand::Byte(b)) => b as i8 as i32,
        (AddressingMode::Relative16, Operand::Word(w)) => w as i16 as i32,
        _ => return None,
    };
    let next = insn.address.offset() as i32 + insn.len as i32;
    let to = next + d;
    (!(0..=0xFFFF).contains(&to))
        .then(|| SnesAddress::new(insn.address.bank(), (to & 0xFFFF) as u16))
}

fn data_directive(kind: RegionKind, len: u32) -> (&'static str, u32) {
    match kind {
        RegionKind::Data(DataKind::Word | DataKind::Palette | DataKind::Tilemap) => ("dw", 2),
        RegionKind::Data(DataKind::Long) => ("dl", 3),
        RegionKind::Data(DataKind::Pointer { .. })
            if len.is_multiple_of(3) && !len.is_multiple_of(2) =>
        {
            ("dl", 3)
        }
        RegionKind::Data(DataKind::Pointer { .. }) => ("dw", 2),
        _ => ("db", 1),
    }
}

/// Write the listing for `[start, start + len)`.
pub fn export_asar(
    rom: &RomImage,
    snap: &AnalysisSnapshot,
    project: &Project,
    options: AsarOptions,
    out: &mut String,
) {
    let symbols = Symbols::new(rom, project, &snap.auto_labels);
    let (start, len) = options.range.unwrap_or((0, rom.len() as u32));
    let end = start.saturating_add(len).min(rom.len() as u32);
    let bank = bank_size(rom);
    let h = rom.header();
    let _ = writeln!(out, "; Romlens export of {:?}", h.title);
    let _ = writeln!(out, "; SHA-256 {}", rom.sha256_hex());
    let _ = writeln!(
        out,
        "; {} {}; range {}..{}",
        rom.mapping(),
        if h.is_fast_rom() {
            "FastROM"
        } else {
            "SlowROM"
        },
        FileOffset(start),
        FileOffset(end)
    );
    let _ = writeln!(
        out,
        "; Reassemble with asar: every operand carries an explicit width, so no"
    );
    let _ = writeln!(
        out,
        "; flag tracking is needed; `; flags:` comments are notes only."
    );
    let _ = writeln!(
        out,
        "{}",
        match rom.mapping() {
            MappingMode::LoRom => "lorom",
            MappingMode::HiRom => "hirom",
            MappingMode::ExHiRom => "exhirom",
        }
    );
    // Labels outside ROM (RAM, hardware) that operands may name.
    let mut defined = false;
    for l in project.labels.values() {
        if rom.file_offset_for(l.address).is_none() {
            if !defined {
                out.push_str("\n; RAM and register labels\n");
                defined = true;
            }
            let _ = writeln!(out, "{} = ${:06X}", l.name, l.address.as_u24());
        }
    }
    // Labels no line of this listing starts at: inside an instruction (a
    // table whose base is the last byte of a JMP before it, read with an
    // index that is never 0), or outside the range. Operands still name
    // them, so they are defined by value.
    let unplaced: Vec<&crate::model::label::Label> = symbols
        .all_labels()
        .into_iter()
        .filter(|l| {
            rom.file_offset_for(l.address).is_some_and(|o| {
                let inside = snap
                    .instruction_at(o)
                    .is_some_and(|r| r.offset != o.0 && r.offset >= start && r.end() <= end);
                inside || o.0 < start || o.0 >= end
            })
        })
        .collect();
    let body_at = out.len();
    let mut pos = start;
    let mut need_org = true;
    let mut last_region_kind: Option<(RegionKind, u8)> = None;
    let mut rec_i = snap.instruction_index_from(start);
    while pos < end {
        if pos % bank == 0 {
            need_org = true;
        }
        let canonical = rom.snes_address_for(FileOffset(pos));
        if need_org {
            if let Some(a) = canonical {
                let _ = writeln!(out, "\norg ${:06X}", a.as_u24());
            }
            need_org = false;
        }
        let region = snap.region_at(FileOffset(pos));
        let region_key = region.map(|r| (r.kind, (r.confidence * 100.0).round() as u8));
        if region_key != last_region_kind {
            if let Some(r) = region {
                let _ = writeln!(
                    out,
                    "; ---- {} ({}%) ----",
                    r.kind.name(),
                    (r.confidence * 100.0).round() as u32
                );
            }
            last_region_kind = region_key;
        }
        if options.comments
            && let Some(a) = canonical
            && let Some(c) = project.comment_at(a, CommentKind::Block)
        {
            for l in c.text.lines() {
                let _ = writeln!(out, "; {l}");
            }
        }
        let line_comment = if options.comments {
            canonical
                .and_then(|a| project.comment_at(a, CommentKind::Line))
                .map(|c| c.text.clone())
        } else {
            None
        };
        while rec_i < snap.instructions.len() && snap.instructions[rec_i].offset < pos {
            rec_i += 1;
        }
        let rec = snap
            .instructions
            .get(rec_i)
            .filter(|r| r.offset == pos)
            .copied();
        if let Some(a) = canonical
            && let Some(label) = symbols.label_at(a)
        {
            let _ = write!(out, "{}:", label.name);
            if let Some(r) = rec {
                let _ = write!(out, "  ; flags: {}", r.flags_before());
            }
            out.push('\n');
        }
        // An instruction whose bytes cross a bank end is one the CPU never
        // runs as such (its program counter wraps within the bank), and asar
        // refuses it; it goes out as data rows.
        if let Some(r) = rec
            && let Some(insn) = snap.decode_at(rom, &r)
            && r.end() <= end
            && r.offset / bank == (r.end() - 1) / bank
        {
            // A branch that reaches its target only by wrapping around the
            // bank, as the 65816's program counter does: asar measures the
            // distance without the wrap, so the bytes go as they are.
            if let Some(target) = wrapped_branch(&insn) {
                let bytes: Vec<String> = insn.bytes[..insn.len as usize]
                    .iter()
                    .map(|b| format!("${b:02X}"))
                    .collect();
                let _ = writeln!(
                    out,
                    "  db {}  ; {} ${:06X}, wrapping around the bank",
                    bytes.join(","),
                    insn.mnemonic.as_str(),
                    target.as_u24()
                );
                pos = r.end();
                rec_i += 1;
                continue;
            }
            let mut text = asar_instruction(&insn, &symbols);
            let register = crate::cpu65816::register_for(&insn).map(|r| r.name);
            match (line_comment, register) {
                (Some(c), _) => {
                    let _ = write!(text, "  ; {c}");
                }
                (None, Some(r)) => {
                    let _ = write!(text, "  ; {r}");
                }
                _ => {}
            }
            let _ = writeln!(out, "  {text}");
            pos = r.end();
            rec_i += 1;
            continue;
        }
        // Data row: up to 16 bytes, never spanning a label, a record, a
        // region end, a bank end or the range end.
        let region_end = region.map_or(end, |r| r.end()).min(end);
        let (directive, width) = data_directive(
            region.map_or(RegionKind::Unknown, |r| r.kind),
            region_end - pos,
        );
        let mut row_end = (pos + (16 / width) * width).min(region_end);
        match rec {
            // An instruction the range cuts short: its bytes as one row.
            Some(r) => row_end = row_end.min(r.end()),
            None => {
                if let Some(r) = snap.instructions.get(rec_i) {
                    row_end = row_end.min(r.offset.max(pos + 1));
                }
            }
        }
        row_end = row_end.min((pos / bank + 1) * bank);
        for b in pos + 1..row_end {
            if let Some(a) = rom.snes_address_for(FileOffset(b))
                && (symbols.label_at(a).is_some()
                    || (options.comments && project.comment_at(a, CommentKind::Block).is_some()))
            {
                row_end = b;
                break;
            }
        }
        let bytes = &rom.bytes()[pos as usize..row_end as usize];
        // A table of addresses exports as the labels it names. asar resolves
        // them on reassembly exactly as it resolves a branch target, so this
        // is both more readable and no less exact than the numbers.
        let entries = match region.map(|r| r.kind) {
            Some(RegionKind::Data(d)) => d.entry_rule().filter(|(w, ..)| (2..=4).contains(w)),
            _ => None,
        };
        let (directive, width) = match entries {
            Some((w, ..)) => (if w == 3 { "dl" } else { "dw" }, w),
            None => (directive, width),
        };
        let whole = bytes.len() / width as usize * width as usize;
        if whole > 0 {
            let _ = write!(out, "  {directive} ");
            let table_bank = rom
                .snes_address_for(FileOffset(pos))
                .map_or(0, |a| a.bank());
            let mut i = 0;
            while i < whole {
                if i > 0 {
                    out.push(',');
                }
                let w = width as usize;
                if let Some((_, bank, _)) = entries
                    && let Some(target) = bank.target(&bytes[i..], width, table_bank)
                    && let Some(label) = symbols.label_at(target)
                {
                    out.push_str(&label.name);
                    i += w;
                    continue;
                }
                let mut v = 0u32;
                for k in (0..w).rev() {
                    v = (v << 8) | bytes[i + k] as u32;
                }
                let _ = write!(out, "${:0w$X}", v, w = w * 2);
                i += w;
            }
            if whole == bytes.len()
                && let Some(c) = &line_comment
            {
                let _ = write!(out, "  ; {c}");
            }
            out.push('\n');
        }
        if whole < bytes.len() {
            // A short tail is written byte by byte so nothing is invented.
            let _ = write!(out, "  db ");
            for (k, b) in bytes[whole..].iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                let _ = write!(out, "${b:02X}");
            }
            if let Some(c) = &line_comment {
                let _ = write!(out, "  ; {c}");
            }
            out.push('\n');
        }
        pos = row_end;
    }
    // The ones the listing names, defined by value ahead of it.
    let used: std::collections::HashSet<&str> = out[body_at..]
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .collect();
    let mut defs = String::new();
    for l in unplaced.iter().filter(|l| used.contains(l.name.as_str())) {
        if defs.is_empty() {
            defs.push_str("\n; Labels inside an instruction or outside the range\n");
        }
        let _ = writeln!(defs, "{} = ${:06X}", l.name, l.address.as_u24());
    }
    out.insert_str(body_at, &defs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu65816::{FlagState, decode};

    fn at(bytes: &[u8], bank: u8, offset: u16) -> Instruction {
        decode(
            bytes,
            SnesAddress::new(bank, offset),
            FileOffset(0),
            FlagState::NATIVE_VECTOR,
        )
        .unwrap()
    }

    #[test]
    fn a_branch_that_wraps_around_the_bank_is_found() {
        // BRL at $C0:1000 to $C0:F000: -$2003 from $1003, past $0000.
        let brl = at(&[0x82, 0xFD, 0xDF], 0xC0, 0x1000);
        assert_eq!(wrapped_branch(&brl), Some(SnesAddress::new(0xC0, 0xF000)));
        // The same distance forward stays in the bank.
        let near = at(&[0x82, 0xFD, 0x1F], 0xC0, 0x1000);
        assert_eq!(wrapped_branch(&near), None);
        // BRA at $80:FFFE to $80:0002, over the top.
        let bra = at(&[0x80, 0x02], 0x80, 0xFFFE);
        assert_eq!(wrapped_branch(&bra), Some(SnesAddress::new(0x80, 0x0002)));
    }
}
