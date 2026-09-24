//! The disassembly view: a line index over the snapshot, a flat batch
//! encoder for the hot path (docs/10) and a text formatter for the CLI.
//!
//! Lines per address, in order: Section (region kind or confidence class
//! change, or a bank start), Blank (after a block end when the next content
//! carries a label or section), Label, one Comment line per block-comment
//! line, one Note line per idiom starting there (with explanations,
//! docs/20), then the Instruction or a Data row of up to 16 bytes (split at
//! labelled, referenced or commented addresses and at region ends).
//!
//! Batch layout, little-endian:
//!
//! ```text
//! header (16): u16 version = 1, u16 stride = 96, u32 lines, u32 text_area_off, u32 text_area_len
//! record (96):
//!   0  u32 file_offset             4  u32 canonical SNES address (0xFFFFFFFF none)
//!   8  u8  kind (LineKind)         9  u8  byte_count (0..=16)
//!   10 u8  region_kind (0 unknown, 1 code, 2 byte, 3 word, 4 long, 5 pointer, 6 table,
//!          7 string, 8 graphics, 9 tilemap, 10 palette, 11 compressed, 12 struct)
//!   11 u8  confidence 0..=100
//!   12 u8  flags_before packed (bit0 m, bit1 x, bit2 e, bit3 dbr known, bit4 dp known,
//!          bit5 flag override here, bit6 warning here, bit7 assumption)
//!   13 u8  line_flags (bit0 line comment, bit1 block comment above, bit2 labelled,
//!          bit3 block end, bit4 has incoming xrefs, bit5 call target,
//!          bit6 low confidence (<0.5), bit7 user region override)
//!   14 u8  token_count (≤ 6)      15 u8  dbr (valid when bit3)   16 u16 dp (valid when bit4)
//!   18 u16 text_len               20 u32 text_off (absolute into the buffer)
//!   24 u32 target SNES address (0xFFFFFFFF none)   28 u32 target file offset (0xFFFFFFFF none)
//!   32 u16 xref_in_count          34 u16 reserved
//!   36 16 × u8 bytes (zero padded)
//!   52 6 × token { u8 kind, u8 reserved, u16 start, u16 len }  (36 bytes)
//!   88 8 reserved
//! text area: UTF-8 slices; the text holds mnemonic/operand (or `db …`, `NAME:`,
//! `; comment`, section text) with `  ; comment` appended.
//! ```

use std::fmt::Write as _;
use std::sync::Arc;

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::cpu65816::{Token, TokenKind, format_bytes, format_instruction};
use crate::explain::Explanations;
use crate::memory::address::{FileOffset, SnesAddress};
use crate::memory::map::MappingMode;
use crate::model::comment::CommentKind;
use crate::model::label::LabelSource;
use crate::model::project::Project;
use crate::model::region::{BankRule, DataKind, Region, RegionKind};
use crate::model::symbols::Symbols;
use crate::model::xref::XRefKind;
use crate::rom::image::RomImage;
use crate::viewmodel::hex_rows::AddressStyle;

pub const ASM_LINE_VERSION: u16 = 1;
pub const ASM_LINE_STRIDE: u16 = 96;
pub const ASM_BATCH_HEADER_LEN: usize = 16;
pub const MAX_TOKENS: usize = 6;
pub const NONE_ADDRESS: u32 = 0xFFFF_FFFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LineKind {
    Instruction = 1,
    Data = 2,
    Label = 3,
    Blank = 4,
    Comment = 5,
    Section = 6,
    /// An idiom's note (docs/20); `sub` picks which of those starting there.
    Note = 7,
}

impl LineKind {
    pub const fn from_u8(v: u8) -> Option<LineKind> {
        Some(match v {
            1 => LineKind::Instruction,
            2 => LineKind::Data,
            3 => LineKind::Label,
            4 => LineKind::Blank,
            5 => LineKind::Comment,
            6 => LineKind::Section,
            7 => LineKind::Note,
            _ => return None,
        })
    }

    pub const fn is_content(self) -> bool {
        matches!(self, LineKind::Instruction | LineKind::Data)
    }
}

/// One line: 8 bytes. `sub` is the byte count of a data row, the line number
/// within a block comment, or 1 for a bank-start section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRef {
    pub offset: u32,
    pub kind: LineKind,
    pub sub: u8,
}

#[derive(Debug, Clone, Default)]
pub struct LineIndex {
    pub lines: Vec<LineRef>,
    /// With explanations the listing's automatic comments explain each
    /// hardware write, and idioms get a note line (docs/20).
    pub explain: Option<Arc<Explanations>>,
}

fn confidence_class(c: f32) -> u8 {
    if c >= 0.8 {
        2
    } else if c >= 0.5 {
        1
    } else {
        0
    }
}

fn bank_size(rom: &RomImage) -> u32 {
    match rom.mapping() {
        MappingMode::LoRom => 0x8000,
        _ => 0x1_0000,
    }
}

/// Bytes per data row for a kind, and whether rows must end on an element.
fn row_len(kind: RegionKind) -> u32 {
    match kind {
        RegionKind::Data(d) => {
            let e = d.element_len().max(1);
            if e >= 16 { e } else { (16 / e) * e }
        }
        _ => 16,
    }
}

impl LineIndex {
    pub fn build(rom: &RomImage, snap: &AnalysisSnapshot, project: &Project) -> Self {
        Self::build_with(rom, snap, project, None)
    }

    /// The listing with its explanations: explained automatic comments and
    /// a note line above each idiom.
    pub fn build_explained(
        rom: &RomImage,
        snap: &AnalysisSnapshot,
        project: &Project,
        explain: Arc<Explanations>,
    ) -> Self {
        Self::build_with(rom, snap, project, Some(explain))
    }

    fn build_with(
        rom: &RomImage,
        snap: &AnalysisSnapshot,
        project: &Project,
        explain: Option<Arc<Explanations>>,
    ) -> Self {
        let n = rom.len() as u32;
        let bank = bank_size(rom);
        // Addresses that force a data-row split: labels, xref targets, comments.
        let mut splits: Vec<u32> = Vec::new();
        for a in project.labels.keys().chain(snap.auto_labels.keys()) {
            if let Some(off) = rom.file_offset_for(*a) {
                splits.push(off.0);
            }
        }
        for x in &snap.xrefs_by_target {
            if let Some(off) = x.to_offset {
                splits.push(off.0);
            }
        }
        for (a, _) in project.comments.keys() {
            if let Some(off) = rom.file_offset_for(*a) {
                splits.push(off.0);
            }
        }
        splits.sort_unstable();
        splits.dedup();

        let mut lines: Vec<LineRef> = Vec::with_capacity((n / 12) as usize);
        let mut last_section: Option<(u8, u8)> = None; // (kind code, confidence class)
        let mut prev_block_end = false;
        let mut has_content = false;
        let mut rec_i = 0usize;
        let recs = &snap.instructions;
        for region in &snap.regions {
            let mut pos = region.start.0;
            let end = region.end();
            let class = (region.kind.code(), confidence_class(region.confidence));
            while pos < end {
                let bank_start = pos % bank == 0;
                let mut section = false;
                if bank_start || last_section != Some(class) {
                    section = true;
                    last_section = Some(class);
                }
                let canonical = rom.snes_address_for(FileOffset(pos));
                let labelled = canonical.is_some_and(|a| {
                    project.labels.contains_key(&a) || snap.auto_labels.contains_key(&a)
                });
                if section {
                    if has_content {
                        lines.push(LineRef {
                            offset: pos,
                            kind: LineKind::Blank,
                            sub: 0,
                        });
                    }
                    lines.push(LineRef {
                        offset: pos,
                        kind: LineKind::Section,
                        sub: bank_start as u8,
                    });
                } else if labelled && prev_block_end {
                    lines.push(LineRef {
                        offset: pos,
                        kind: LineKind::Blank,
                        sub: 0,
                    });
                }
                if labelled {
                    lines.push(LineRef {
                        offset: pos,
                        kind: LineKind::Label,
                        sub: 0,
                    });
                }
                if let Some(a) = canonical
                    && let Some(c) = project.comment_at(a, CommentKind::Block)
                {
                    for (i, _) in c.text.lines().enumerate() {
                        lines.push(LineRef {
                            offset: pos,
                            kind: LineKind::Comment,
                            sub: i.min(255) as u8,
                        });
                    }
                }
                if let Some(x) = &explain {
                    let n = x.idioms_starting_at(FileOffset(pos)).len();
                    for k in 0..n.min(256) {
                        lines.push(LineRef {
                            offset: pos,
                            kind: LineKind::Note,
                            sub: k as u8,
                        });
                    }
                }
                while rec_i < recs.len() && recs[rec_i].offset < pos {
                    rec_i += 1;
                }
                let rec = recs.get(rec_i).filter(|r| r.offset == pos).copied();
                if let Some(r) = rec {
                    lines.push(LineRef {
                        offset: pos,
                        kind: LineKind::Instruction,
                        sub: r.len,
                    });
                    prev_block_end = crate::cpu65816::OPCODES[r.opcode as usize]
                        .mnemonic
                        .is_block_end();
                    pos = r.end();
                    rec_i += 1;
                } else {
                    let mut len = row_len(region.kind).min(end - pos);
                    if let Some(r) = recs.get(rec_i) {
                        len = len.min(r.offset - pos);
                    }
                    let si = splits.partition_point(|s| *s <= pos);
                    if let Some(next) = splits.get(si) {
                        len = len.min(next - pos);
                    }
                    let bank_end = (pos / bank + 1) * bank;
                    len = len.min(bank_end - pos).max(1);
                    lines.push(LineRef {
                        offset: pos,
                        kind: LineKind::Data,
                        sub: len as u8,
                    });
                    prev_block_end = false;
                    pos += len;
                }
                has_content = true;
            }
        }
        Self { lines, explain }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The content line whose bytes contain `off`.
    pub fn line_for_offset(&self, off: u32) -> Option<usize> {
        let i = self.lines.partition_point(|l| l.offset <= off);
        // The last line at or before `off`; content lines follow their
        // label/comment lines, so step back over the few decorations.
        let mut j = i.checked_sub(1)?;
        loop {
            let l = self.lines[j];
            if l.kind.is_content() {
                return (off >= l.offset && off < l.offset + l.sub as u32).then_some(j);
            }
            if j == 0 || self.lines[j - 1].offset < l.offset && self.lines[j - 1].kind.is_content()
            {
                // A decoration at `off` preceded by an earlier content line:
                // `off` can only be inside that earlier line.
                if j == 0 {
                    return None;
                }
                let p = self.lines[j - 1];
                return (off >= p.offset && off < p.offset + p.sub as u32).then_some(j - 1);
            }
            j -= 1;
        }
    }

    pub fn offset_for_line(&self, line: usize) -> Option<u32> {
        self.lines.get(line).map(|l| l.offset)
    }

    /// `(offset, len)` of a line; decorations have length 0.
    pub fn item_range(&self, line: usize) -> Option<(u32, u32)> {
        self.lines
            .get(line)
            .map(|l| (l.offset, if l.kind.is_content() { l.sub as u32 } else { 0 }))
    }

    /// One line number per byte of `[start, start + len)`, for the lockstep
    /// bracket. `NONE_ADDRESS` where no content line covers a byte.
    pub fn line_numbers_for_bytes(&self, start: u32, len: u32) -> Vec<u32> {
        let mut out = Vec::with_capacity(len as usize);
        let mut cached: Option<(usize, u32, u32)> = None;
        for off in start..start.saturating_add(len) {
            if let Some((line, s, e)) = cached
                && off >= s
                && off < e
            {
                out.push(line as u32);
                continue;
            }
            match self.line_for_offset(off) {
                Some(line) => {
                    let l = self.lines[line];
                    cached = Some((line, l.offset, l.offset + l.sub as u32));
                    out.push(line as u32);
                }
                None => out.push(NONE_ADDRESS),
            }
        }
        out
    }
}

/// The formatted content of one line, shared by the batch and the text
/// formatter.
#[derive(Debug, Clone, Default)]
pub struct LineText {
    pub text: String,
    pub tokens: Vec<Token>,
    /// `  ; comment` appended to the text (already included in `text`).
    pub comment: Option<String>,
    pub target: Option<SnesAddress>,
    pub target_offset: Option<FileOffset>,
    pub flags_packed: u8,
    pub dbr: u8,
    pub dp: u16,
    pub block_end: bool,
    pub assumption: bool,
}

fn push_token(tokens: &mut Vec<Token>, kind: TokenKind, start: usize, len: usize) {
    tokens.push(Token {
        kind,
        start: start as u16,
        len: len as u16,
    });
}

fn data_directive(kind: RegionKind, len: u32) -> (&'static str, u32) {
    match kind {
        RegionKind::Data(DataKind::Word | DataKind::Palette | DataKind::Tilemap) => ("dw", 2),
        RegionKind::Data(DataKind::Long) => ("dl", 3),
        RegionKind::Data(DataKind::Pointer { .. }) => {
            if len.is_multiple_of(3) && !len.is_multiple_of(2) {
                ("dl", 3)
            } else {
                ("dw", 2)
            }
        }
        _ => ("db", 1),
    }
}

fn entry_kind(kind: RegionKind) -> Option<(u32, BankRule, bool)> {
    match kind {
        RegionKind::Data(d) => d.entry_rule(),
        _ => None,
    }
}

/// A table of addresses, rendered as the labels it points at.
///
/// This is the most legible single result of Phase 2: `dw CODE_808423` instead
/// of `dw $8423`. The reader can see what a dispatch table dispatches to
/// without leaving the line, and the label carries the xref that makes the
/// jump navigable both ways.
fn entry_text(
    rom: &RomImage,
    symbols: &Symbols<'_>,
    offset: u32,
    bytes: &[u8],
    width: u32,
    bank: BankRule,
    out: &mut LineText,
) {
    let table_bank = rom
        .snes_address_for(FileOffset(offset))
        .map_or(0, |a| a.bank());
    let mut i = 0usize;
    let mut first = true;
    while i + width as usize <= bytes.len() {
        if !first {
            out.text.push(',');
        }
        first = false;
        let start = out.text.len();
        let target = bank.target(&bytes[i..], width, table_bank);
        match target.and_then(|t| symbols.label_at(t).map(|l| (t, l))) {
            Some((_, label)) => {
                out.text.push_str(&label.name);
                push_token(
                    &mut out.tokens,
                    if label.source == LabelSource::Auto {
                        TokenKind::AutoLabel
                    } else {
                        TokenKind::UserLabel
                    },
                    start,
                    label.name.len(),
                );
            }
            None => {
                // No label: the number, so an entry that points nowhere
                // readable still shows what it holds.
                let mut v = 0u32;
                for k in (0..width as usize).rev() {
                    v = (v << 8) | bytes[i + k] as u32;
                }
                let _ = write!(out.text, "${:0w$X}", v, w = width as usize * 2);
                push_token(
                    &mut out.tokens,
                    TokenKind::DataValue,
                    start,
                    out.text.len() - start,
                );
            }
        }
        i += width as usize;
    }
    // Any tail too short for an entry falls back to bytes.
    for byte in &bytes[i..] {
        if !first {
            out.text.push(',');
        }
        first = false;
        let start = out.text.len();
        let _ = write!(out.text, "${byte:02X}");
        push_token(
            &mut out.tokens,
            TokenKind::DataValue,
            start,
            out.text.len() - start,
        );
    }
}

fn data_text(
    rom: &RomImage,
    symbols: &Symbols<'_>,
    offset: u32,
    bytes: &[u8],
    region: &Region,
    out: &mut LineText,
) {
    if let Some((width, bank, _)) = entry_kind(region.kind)
        && (2..=4).contains(&width)
    {
        // The directive follows the entry width, not the region kind: a table
        // of two-byte addresses is `dw` whatever else the region is called.
        let directive = match width {
            2 => "dw",
            3 => "dl",
            _ => "dd",
        };
        out.text.push_str(directive);
        push_token(&mut out.tokens, TokenKind::Directive, 0, directive.len());
        out.text.push(' ');
        entry_text(rom, symbols, offset, bytes, width, bank, out);
        return;
    }
    let (directive, width) = data_directive(region.kind, bytes.len() as u32);
    out.text.push_str(directive);
    push_token(&mut out.tokens, TokenKind::Directive, 0, directive.len());
    out.text.push(' ');
    let start = out.text.len();
    if matches!(region.kind, RegionKind::Data(DataKind::String)) {
        let mut in_quote = false;
        let mut first = true;
        for &b in bytes {
            let printable = (0x20..0x7F).contains(&b) && b != b'"';
            if printable {
                if !in_quote {
                    if !first {
                        out.text.push(',');
                    }
                    out.text.push('"');
                    in_quote = true;
                }
                out.text.push(b as char);
            } else {
                if in_quote {
                    out.text.push('"');
                    in_quote = false;
                }
                if !first {
                    out.text.push(',');
                }
                let _ = write!(out.text, "${b:02X}");
            }
            first = false;
        }
        if in_quote {
            out.text.push('"');
        }
    } else {
        let mut i = 0;
        let mut first = true;
        while i < bytes.len() {
            if !first {
                out.text.push(',');
            }
            first = false;
            let w = (width as usize).min(bytes.len() - i);
            if w == width as usize && width > 1 {
                let mut v = 0u32;
                for k in (0..w).rev() {
                    v = (v << 8) | bytes[i + k] as u32;
                }
                let _ = write!(out.text, "${:0width$X}", v, width = w * 2);
            } else {
                for k in 0..w {
                    if k > 0 {
                        out.text.push(',');
                    }
                    let _ = write!(out.text, "${:02X}", bytes[i + k]);
                }
            }
            i += w;
        }
    }
    push_token(
        &mut out.tokens,
        TokenKind::DataValue,
        start,
        out.text.len() - start,
    );
}

/// Format one line of the index.
pub fn line_text(
    rom: &RomImage,
    snap: &AnalysisSnapshot,
    project: &Project,
    symbols: &Symbols<'_>,
    explain: Option<&Explanations>,
    line: LineRef,
) -> LineText {
    let mut out = LineText::default();
    let canonical = rom.snes_address_for(FileOffset(line.offset));
    match line.kind {
        LineKind::Blank => {}
        LineKind::Section => {
            let region = snap.region_at(FileOffset(line.offset));
            if line.sub == 1
                && let Some(a) = canonical
            {
                let _ = write!(out.text, "; ==== bank ${:02X} ====", a.bank());
            } else if let Some(r) = region {
                let _ = write!(
                    out.text,
                    "; ---- {} ({}%) ----",
                    r.kind.name(),
                    (r.confidence * 100.0).round() as u32
                );
            } else {
                out.text.push_str("; ----");
            }
            push_token(&mut out.tokens, TokenKind::Section, 0, out.text.len());
        }
        LineKind::Label => {
            if let Some(a) = canonical
                && let Some(l) = symbols.label_at(a)
            {
                out.text.push_str(&l.name);
                out.text.push(':');
                let kind = if l.source.is_user_or_imported() {
                    TokenKind::UserLabelDef
                } else {
                    TokenKind::AutoLabelDef
                };
                push_token(&mut out.tokens, kind, 0, l.name.len());
                push_token(&mut out.tokens, TokenKind::Punct, l.name.len(), 1);
            }
        }
        LineKind::Comment => {
            if let Some(a) = canonical
                && let Some(c) = project.comment_at(a, CommentKind::Block)
                && let Some(text) = c.text.lines().nth(line.sub as usize)
            {
                out.text.push_str("; ");
                out.text.push_str(text);
                push_token(&mut out.tokens, TokenKind::Comment, 0, out.text.len());
            }
        }
        LineKind::Note => {
            if let Some(i) = explain.and_then(|x| {
                x.idioms_starting_at(FileOffset(line.offset))
                    .get(line.sub as usize)
                    .copied()
            }) {
                let _ = write!(out.text, "; ▸ {}: {}", i.title, i.summary);
                push_token(&mut out.tokens, TokenKind::Note, 0, out.text.len());
            }
        }
        LineKind::Instruction => {
            let explained = explain
                .and_then(|x| x.write_at(FileOffset(line.offset)))
                .map(|e| e.short());
            let auto_comment = if let Some(rec) = snap.instruction_at(FileOffset(line.offset))
                && let Some(insn) = snap.decode_at(rom, rec)
            {
                let f = format_instruction(&insn, symbols);
                out.text = f.text;
                out.tokens = f.tokens;
                out.target = insn.target.map(|t| t.address);
                out.target_offset = insn.target.and_then(|t| rom.file_offset_for(t.address));
                out.flags_packed = insn.flags_before.packed();
                out.dbr = insn.flags_before.dbr.unwrap_or(0);
                out.dp = insn.flags_before.dp.unwrap_or(0);
                out.block_end = insn.mnemonic.is_block_end();
                out.assumption = insn.assumptions != 0;
                explained.or_else(|| f.register.map(|r| r.name.to_owned()))
            } else {
                None
            };
            let user = canonical.and_then(|a| project.comment_at(a, CommentKind::Line));
            let (comment, kind) = match (user, auto_comment) {
                (Some(c), _) => (Some(c.text.clone()), TokenKind::Comment),
                (None, Some(r)) => (Some(r), TokenKind::AutoComment),
                (None, None) => (None, TokenKind::Comment),
            };
            if let Some(c) = comment {
                let start = out.text.len() + 2;
                let _ = write!(out.text, "  ; {c}");
                push_token(&mut out.tokens, kind, start, out.text.len() - start);
                out.comment = Some(c);
            }
        }
        LineKind::Data => {
            let bytes =
                &rom.bytes()[line.offset as usize..(line.offset + line.sub as u32) as usize];
            if let Some(region) = snap.region_at(FileOffset(line.offset)) {
                data_text(rom, symbols, line.offset, bytes, region, &mut out);
            }
            if let Some(a) = canonical
                && let Some(c) = project.comment_at(a, CommentKind::Line)
            {
                let start = out.text.len() + 2;
                let _ = write!(out.text, "  ; {}", c.text);
                push_token(
                    &mut out.tokens,
                    TokenKind::Comment,
                    start,
                    out.text.len() - start,
                );
                out.comment = Some(c.text.clone());
            }
        }
    }
    out
}

/// Encode lines `[start_line, start_line + count)` as a flat batch.
pub fn encode_lines(
    rom: &RomImage,
    snap: &AnalysisSnapshot,
    project: &Project,
    idx: &LineIndex,
    start_line: u32,
    count: u32,
) -> Vec<u8> {
    let symbols = Symbols::new(rom, project, &snap.auto_labels);
    let total = idx.len() as u32;
    let n = count.min(total.saturating_sub(start_line));
    let stride = ASM_LINE_STRIDE as usize;
    let text_area_off = ASM_BATCH_HEADER_LEN + n as usize * stride;
    let mut out = vec![0u8; text_area_off];
    out[0..2].copy_from_slice(&ASM_LINE_VERSION.to_le_bytes());
    out[2..4].copy_from_slice(&ASM_LINE_STRIDE.to_le_bytes());
    out[4..8].copy_from_slice(&n.to_le_bytes());
    out[8..12].copy_from_slice(&(text_area_off as u32).to_le_bytes());
    let mut text_area: Vec<u8> = Vec::with_capacity(n as usize * 24);
    for i in 0..n as usize {
        let line = idx.lines[start_line as usize + i];
        let lt = line_text(rom, snap, project, &symbols, idx.explain.as_deref(), line);
        let rec =
            &mut out[ASM_BATCH_HEADER_LEN + i * stride..ASM_BATCH_HEADER_LEN + (i + 1) * stride];
        let canonical = rom.snes_address_for(FileOffset(line.offset));
        rec[0..4].copy_from_slice(&line.offset.to_le_bytes());
        rec[4..8].copy_from_slice(
            &canonical
                .map_or(NONE_ADDRESS, SnesAddress::as_u24)
                .to_le_bytes(),
        );
        rec[8] = line.kind as u8;
        let byte_count = if line.kind.is_content() { line.sub } else { 0 };
        rec[9] = byte_count;
        let region = snap.region_at(FileOffset(line.offset));
        rec[10] = region.map_or(0, |r| r.kind.code());
        rec[11] = region.map_or(0, |r| (r.confidence * 100.0).round() as u8);
        let mut fb = lt.flags_packed;
        if project
            .flag_overrides
            .contains_key(&FileOffset(line.offset))
            && line.kind == LineKind::Instruction
        {
            fb |= 0x20;
        }
        if line.kind.is_content() && !snap.warnings_at(FileOffset(line.offset)).is_empty() {
            fb |= 0x40;
        }
        if lt.assumption {
            fb |= 0x80;
        }
        rec[12] = fb;
        let mut lf = 0u8;
        if line.kind.is_content()
            && let Some(a) = canonical
        {
            if project.comment_at(a, CommentKind::Line).is_some() {
                lf |= 0x01;
            }
            if project.comment_at(a, CommentKind::Block).is_some() {
                lf |= 0x02;
            }
            if symbols.label_at(a).is_some() {
                lf |= 0x04;
            }
            let xin = snap.xrefs_to(a);
            if !xin.is_empty() {
                lf |= 0x10;
            }
            if xin.iter().any(|x| x.kind == XRefKind::Call) {
                lf |= 0x20;
            }
            rec[32..34].copy_from_slice(&(xin.len().min(u16::MAX as usize) as u16).to_le_bytes());
        }
        if lt.block_end {
            lf |= 0x08;
        }
        if region.is_some_and(|r| r.confidence < 0.5) {
            lf |= 0x40;
        }
        if project
            .region_override_at(FileOffset(line.offset))
            .is_some()
        {
            lf |= 0x80;
        }
        rec[13] = lf;
        rec[14] = lt.tokens.len().min(MAX_TOKENS) as u8;
        rec[15] = lt.dbr;
        rec[16..18].copy_from_slice(&lt.dp.to_le_bytes());
        let text = lt.text.as_bytes();
        rec[18..20].copy_from_slice(&(text.len().min(u16::MAX as usize) as u16).to_le_bytes());
        rec[20..24].copy_from_slice(&((text_area_off + text_area.len()) as u32).to_le_bytes());
        text_area.extend_from_slice(text);
        rec[24..28].copy_from_slice(
            &lt.target
                .map_or(NONE_ADDRESS, SnesAddress::as_u24)
                .to_le_bytes(),
        );
        rec[28..32].copy_from_slice(
            &lt.target_offset
                .map_or(NONE_ADDRESS, FileOffset::value)
                .to_le_bytes(),
        );
        if byte_count > 0 {
            let src =
                &rom.bytes()[line.offset as usize..(line.offset + byte_count as u32) as usize];
            rec[36..36 + src.len()].copy_from_slice(src);
        }
        for (t, tok) in lt.tokens.iter().take(MAX_TOKENS).enumerate() {
            let at = 52 + t * 6;
            rec[at] = tok.kind as u8;
            rec[at + 2..at + 4].copy_from_slice(&tok.start.to_le_bytes());
            rec[at + 4..at + 6].copy_from_slice(&tok.len.to_le_bytes());
        }
    }
    out[12..16].copy_from_slice(&(text_area.len() as u32).to_le_bytes());
    out.extend_from_slice(&text_area);
    out
}

/// Text column where a comment starts when the instruction text is shorter.
pub const COMMENT_COLUMN: usize = 44;

/// Lines for the CLI and goldens:
///
/// ```text
/// 0x00041C  $80:841C  78            SEI
/// ```
///
/// Label, section and blank lines have no address columns. `verbose` adds
/// a `m1x1e1 $00:$0000` flags column before the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextOptions {
    pub style: AddressStyle,
    pub verbose: bool,
}

pub fn format_lines_text(
    rom: &RomImage,
    snap: &AnalysisSnapshot,
    project: &Project,
    idx: &LineIndex,
    start_line: u32,
    count: u32,
    options: TextOptions,
) -> String {
    let TextOptions { style, verbose } = options;
    let symbols = Symbols::new(rom, project, &snap.auto_labels);
    let total = idx.len() as u32;
    let n = count.min(total.saturating_sub(start_line));
    let mut s = String::with_capacity(n as usize * 64);
    for i in 0..n as usize {
        let line = idx.lines[start_line as usize + i];
        let lt = line_text(rom, snap, project, &symbols, idx.explain.as_deref(), line);
        match line.kind {
            LineKind::Blank => {}
            LineKind::Label | LineKind::Section => s.push_str(&lt.text),
            LineKind::Comment | LineKind::Note => {
                let indent = address_width(style) + 13 + if verbose { 18 } else { 0 };
                for _ in 0..indent {
                    s.push(' ');
                }
                s.push_str(&lt.text);
            }
            LineKind::Instruction | LineKind::Data => {
                let canonical = rom.snes_address_for(FileOffset(line.offset));
                if style != AddressStyle::Snes {
                    let _ = write!(s, "{}  ", FileOffset(line.offset));
                }
                if style != AddressStyle::File {
                    match canonical {
                        Some(a) => {
                            let _ = write!(s, "{a}  ");
                        }
                        None => s.push_str("--:----  "),
                    }
                }
                let bytes =
                    &rom.bytes()[line.offset as usize..(line.offset + line.sub as u32) as usize];
                let hex = format_bytes(&bytes[..bytes.len().min(4)]);
                let _ = write!(s, "{hex:<12} ");
                if verbose {
                    if line.kind == LineKind::Instruction {
                        let f = snap
                            .instruction_at(FileOffset(line.offset))
                            .map(|r| r.flags_before());
                        match f {
                            Some(f) => {
                                let _ = write!(
                                    s,
                                    "{} {}:{}  ",
                                    f.short(),
                                    f.dbr.map_or("$??".to_owned(), |b| format!("${b:02X}")),
                                    f.dp.map_or("$????".to_owned(), |d| format!("${d:04X}"))
                                );
                            }
                            None => s.push_str("                  "),
                        }
                    } else {
                        s.push_str("                  ");
                    }
                }
                // Text and comment: the comment sits at a fixed column.
                match &lt.comment {
                    Some(c) => {
                        let body_len = lt.text.len() - c.len() - 4;
                        let body = &lt.text[..body_len];
                        let _ = write!(s, "{body:<width$}; {c}", width = COMMENT_COLUMN);
                    }
                    None => s.push_str(&lt.text),
                }
            }
        }
        let trimmed = s.trim_end_matches(' ').len();
        s.truncate(trimmed);
        s.push('\n');
    }
    s
}

fn address_width(style: AddressStyle) -> usize {
    match style {
        AddressStyle::Both => 20,
        AddressStyle::Snes | AddressStyle::File => 10,
    }
}

/// True when a label at `address` is the analyzer's, for shells that dim it.
pub fn is_auto_label(symbols: &Symbols<'_>, address: SnesAddress) -> bool {
    symbols
        .label_at(address)
        .is_some_and(|l| l.source == LabelSource::Auto)
}
