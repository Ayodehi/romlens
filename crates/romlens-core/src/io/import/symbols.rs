//! Symbol files from other tools.
//!
//! Three formats, and only two line grammars between them. WLA-DX and
//! bsnes-plus write `[labels]` and `[comments]` sections of `bb:aaaa NAME`;
//! no$sns writes the same label lines with no sections at all; ld65's VICE
//! output writes `al ADDRESS .NAME`. That is why WLA comes first — it is also
//! what `io::symbol_export` writes, so the reader round-trips against our own
//! exporter and gets a correctness test for free.
//!
//! Nothing is ever silently dropped. Real symbol files carry dots, `@`, ca65
//! `::` scopes and names far longer than a label may be, so names are
//! rewritten and the count is reported; a caller that does not show the report
//! is the bug.

use std::collections::{BTreeMap, HashSet};

use crate::error::ProjectError;
use crate::memory::address::SnesAddress;
use crate::model::comment::CommentKind;

/// The longest a label may be (`model::label::validate_label_name`).
pub const MAX_NAME: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolFormat {
    /// WLA-DX / bsnes-plus: `[labels]` sections of `bb:aaaa NAME`.
    Wla,
    /// no$sns: bare `bb:aaaa NAME` lines.
    Nocash,
    /// VICE / ld65: `al ADDRESS .NAME`.
    Lbl,
}

impl SymbolFormat {
    pub const fn name(self) -> &'static str {
        match self {
            SymbolFormat::Wla => "wla",
            SymbolFormat::Nocash => "nocash",
            SymbolFormat::Lbl => "lbl",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "wla" | "sym" => Some(SymbolFormat::Wla),
            "nocash" | "no$sns" => Some(SymbolFormat::Nocash),
            "lbl" | "vice" | "ca65" => Some(SymbolFormat::Lbl),
            _ => None,
        }
    }
}

/// What an import found, before anything is applied.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SymbolFile {
    pub labels: BTreeMap<SnesAddress, String>,
    pub comments: BTreeMap<(SnesAddress, CommentKind), String>,
    /// The file's leading comment block, kept so a licence notice travels with
    /// what it covers (`12-content-policy.md` rule 7).
    pub notice: String,
    /// Names that had to be rewritten to be usable, `original -> rewritten`.
    /// Reported, never hidden.
    pub rewritten: Vec<(String, String)>,
    /// Lines that were not a symbol and not a comment.
    pub skipped: Vec<String>,
}

/// Rewrite a symbol name into something `validate_label_name` accepts.
///
/// Truncation is from the *end* of the leading part and keeps a hash of what
/// was dropped, so two long names that share a prefix — which ca65 scopes
/// routinely produce — do not collapse into one.
pub fn sanitize_imported_label(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        let ok = if i == 0 {
            c.is_ascii_alphabetic() || c == '_'
        } else {
            c.is_ascii_alphanumeric() || c == '_'
        };
        if ok {
            out.push(c);
        } else if i == 0 && c.is_ascii_digit() {
            // A name starting with a digit keeps the digit behind an underscore
            // rather than losing it.
            out.push('_');
            out.push(c);
        } else {
            out.push('_');
        }
    }
    // Collapse the runs of underscores that `a::b.c` turns into.
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    if out.is_empty() {
        out.push('_');
    }
    if out.len() > MAX_NAME {
        let hash = short_hash(name);
        out.truncate(MAX_NAME - hash.len() - 1);
        out.push('_');
        out.push_str(&hash);
    }
    out
}

/// Six hex digits of FNV-1a. Enough to keep truncated names apart, and it is
/// twenty lines rather than a dependency.
fn short_hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{:06X}", (h & 0xFF_FFFF) as u32)
}

/// Guess the format from the file's contents.
pub fn detect(text: &str) -> SymbolFormat {
    for line in text.lines() {
        let line = line.trim();
        if line.eq_ignore_ascii_case("[labels]") || line.eq_ignore_ascii_case("[comments]") {
            return SymbolFormat::Wla;
        }
        if line.starts_with("al ") || line.starts_with("al\t") {
            return SymbolFormat::Lbl;
        }
    }
    SymbolFormat::Nocash
}

pub fn read(text: &str, format: Option<SymbolFormat>) -> Result<SymbolFile, ProjectError> {
    let format = format.unwrap_or_else(|| detect(text));
    let mut file = SymbolFile::default();
    let mut section = Section::Labels;
    let mut in_notice = true;
    let mut notice = Vec::new();
    // Names are sanitized after every line is read, not as they arrive. A file
    // may name the same address twice, and de-duplicating as we go would give
    // the survivor a `_2` suffix disambiguating it from a name that is no
    // longer there.
    let mut named: BTreeMap<SnesAddress, String> = BTreeMap::new();

    for raw in text.lines() {
        let line = raw.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            in_notice = false;
            continue;
        }
        if let Some(comment) = trimmed
            .strip_prefix(';')
            .or_else(|| trimmed.strip_prefix('#'))
        {
            if in_notice {
                notice.push(comment.trim().to_owned());
            }
            continue;
        }
        in_notice = false;
        if trimmed.starts_with('[') {
            section = match trimmed.to_ascii_lowercase().as_str() {
                "[labels]" => Section::Labels,
                "[comments]" => Section::Comments,
                // WLA writes sections we have nothing to do with (`[source
                // files]`, `[rom checksum]`); skipping their bodies is what
                // keeps them out of `skipped`.
                _ => Section::Other,
            };
            continue;
        }
        match section {
            Section::Other => continue,
            Section::Labels => match parse_label(trimmed, format) {
                // Last mention of an address wins, as it does in an assembler.
                Some((address, name)) => {
                    named.insert(address, name);
                }
                None => file.skipped.push(line.to_owned()),
            },
            Section::Comments => match parse_comment(trimmed) {
                Some((address, text)) => {
                    file.comments.insert((address, CommentKind::Line), text);
                }
                None => file.skipped.push(line.to_owned()),
            },
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    for (address, name) in named {
        let clean = unique(sanitize_imported_label(&name), &mut seen);
        if clean != name {
            file.rewritten.push((name, clean.clone()));
        }
        file.labels.insert(address, clean);
    }
    file.notice = notice.join("\n").trim().to_owned();
    if file.labels.is_empty() && file.comments.is_empty() {
        return Err(ProjectError::BadFormat(format!(
            "no {} symbols found; {} lines were not understood",
            format.name(),
            file.skipped.len()
        )));
    }
    Ok(file)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Labels,
    Comments,
    Other,
}

/// Two names that sanitize to the same thing are kept apart, because two
/// labels at different addresses must not become one.
fn unique(name: String, seen: &mut HashSet<String>) -> String {
    if seen.insert(name.clone()) {
        return name;
    }
    for n in 2..1000u32 {
        let suffix = format!("_{n}");
        let mut candidate = name.clone();
        if candidate.len() + suffix.len() > MAX_NAME {
            candidate.truncate(MAX_NAME - suffix.len());
        }
        candidate.push_str(&suffix);
        if seen.insert(candidate.clone()) {
            return candidate;
        }
    }
    name
}

fn parse_label(line: &str, format: SymbolFormat) -> Option<(SnesAddress, String)> {
    // Strip a trailing comment: WLA files routinely carry `; from bank.asm`.
    let line = line.split(';').next()?.trim();
    match format {
        SymbolFormat::Lbl => {
            let mut fields = line.split_whitespace();
            if !fields.next()?.eq_ignore_ascii_case("al") {
                return None;
            }
            let address = parse_flat(fields.next()?)?;
            let name = fields.next()?.trim_start_matches('.');
            (!name.is_empty()).then(|| (address, name.to_owned()))
        }
        SymbolFormat::Wla | SymbolFormat::Nocash => {
            let (address, rest) = line.split_once([' ', '\t'])?;
            let address = parse_banked(address)?;
            let name = rest.trim();
            (!name.is_empty()).then(|| (address, name.to_owned()))
        }
    }
}

fn parse_comment(line: &str) -> Option<(SnesAddress, String)> {
    let (address, rest) = line.split_once([' ', '\t'])?;
    let address = parse_banked(address)?;
    let text = rest.trim();
    (!text.is_empty()).then(|| (address, text.to_owned()))
}

/// `bb:aaaa`.
fn parse_banked(text: &str) -> Option<SnesAddress> {
    let (bank, offset) = text.trim_start_matches('$').split_once(':')?;
    Some(SnesAddress::new(
        u8::from_str_radix(bank, 16).ok()?,
        u16::from_str_radix(offset, 16).ok()?,
    ))
}

/// A flat 24-bit address, as VICE writes it.
fn parse_flat(text: &str) -> Option<SnesAddress> {
    let text = text.trim_start_matches('$').trim_start_matches("0x");
    let value = u32::from_str_radix(text, 16).ok()?;
    (value <= 0xFF_FFFF).then(|| SnesAddress::from_u24(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_without_losing_names() {
        assert_eq!(sanitize_imported_label("Boot"), "Boot");
        assert_eq!(sanitize_imported_label("Player::Draw"), "Player_Draw");
        assert_eq!(sanitize_imported_label("main.loop@2"), "main_loop_2");
        assert_eq!(sanitize_imported_label("2ndPass"), "_2ndPass");
        assert_eq!(sanitize_imported_label(""), "_");
        // Long names keep a hash of the original so a shared prefix does not
        // collapse two labels into one.
        let a = sanitize_imported_label(&format!("{}A", "x".repeat(80)));
        let b = sanitize_imported_label(&format!("{}B", "x".repeat(80)));
        assert_eq!(a.len(), MAX_NAME);
        assert_ne!(a, b);
    }

    #[test]
    fn reads_a_wla_file_with_sections_and_a_notice() {
        let text = "; Symbols for Demo\n; Licence: 0BSD\n\n[labels]\n00:8000 Boot\n\
80:841C Player::Draw  ; from bank80.asm\n[rom checksum]\nF8DF\n\
[comments]\n00:8000 disable IRQ\n";
        let file = read(text, None).unwrap();
        assert_eq!(detect(text), SymbolFormat::Wla);
        assert_eq!(file.notice, "Symbols for Demo\nLicence: 0BSD");
        assert_eq!(file.labels[&SnesAddress::new(0x00, 0x8000)], "Boot");
        assert_eq!(file.labels[&SnesAddress::new(0x80, 0x841C)], "Player_Draw");
        assert_eq!(
            file.rewritten,
            vec![("Player::Draw".into(), "Player_Draw".into())]
        );
        assert_eq!(
            file.comments[&(SnesAddress::new(0x00, 0x8000), CommentKind::Line)],
            "disable IRQ"
        );
        assert!(file.skipped.is_empty(), "{:?}", file.skipped);
    }

    #[test]
    fn reads_bare_and_vice_files() {
        let bare = "00:8000 Boot\n80:841C Draw\n";
        assert_eq!(detect(bare), SymbolFormat::Nocash);
        assert_eq!(read(bare, None).unwrap().labels.len(), 2);

        let vice = "al 008000 .Boot\nal 80841C .Draw\n";
        assert_eq!(detect(vice), SymbolFormat::Lbl);
        let file = read(vice, None).unwrap();
        assert_eq!(file.labels[&SnesAddress::new(0x00, 0x8000)], "Boot");
        assert_eq!(file.labels[&SnesAddress::new(0x80, 0x841C)], "Draw");
    }

    #[test]
    fn keeps_two_names_that_sanitize_alike_apart() {
        let text = "00:8000 a.b\n00:8001 a@b\n";
        let file = read(text, None).unwrap();
        let names: Vec<&String> = file.labels.values().collect();
        assert_eq!(names, vec!["a_b", "a_b_2"]);
    }

    #[test]
    fn reports_what_it_could_not_read() {
        let text = "00:8000 Boot\nnonsense line\n";
        let file = read(text, None).unwrap();
        assert_eq!(file.labels.len(), 1);
        assert_eq!(file.skipped, vec!["nonsense line"]);
        // A file with nothing usable is an error, not an empty success.
        assert!(read("nothing here\n", None).is_err());
    }
}

/// What importing a symbol file would do, before it is done.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SymbolImport {
    /// Ready for `Project::apply_batch` with `Origin::Import`.
    pub commands: Vec<crate::model::command::Command>,
    /// Addresses left alone because the user had named them.
    pub kept_user: Vec<SnesAddress>,
    /// Labels this replaces. A comment is never replaced.
    pub replaced: usize,
    pub labels_added: usize,
    pub comments_added: usize,
}

/// Plan an import against a project.
///
/// The user's names are never overwritten. That is the whole collision rule:
/// a symbol file is somebody else's opinion, and somebody else's opinion does
/// not get to rename what this person named. Anything it does replace is
/// counted, so the result can say so.
pub fn plan(
    rom: &crate::rom::image::RomImage,
    project: &crate::model::project::Project,
    file: &SymbolFile,
) -> SymbolImport {
    use crate::model::command::Command;
    use crate::model::label::LabelSource;
    use crate::model::project::Project;

    let mut out = SymbolImport::default();
    for (address, name) in &file.labels {
        let address = Project::canonical(rom, *address);
        match project.labels.get(&address) {
            Some(existing) if existing.source == LabelSource::User => {
                out.kept_user.push(address);
                continue;
            }
            Some(existing) if existing.name == *name => continue,
            Some(_) => out.replaced += 1,
            None => out.labels_added += 1,
        }
        out.commands.push(Command::SetLabel {
            address,
            name: Some(name.clone()),
        });
    }
    for ((address, kind), text) in &file.comments {
        let address = Project::canonical(rom, *address);
        match project.comment_at(address, *kind) {
            // A comment the person wrote is theirs. There is no
            // `CommentSource`, so an existing comment is always treated as
            // the user's — the cautious reading, and the one that cannot lose
            // anything they typed.
            Some(existing) => {
                if existing.text != *text {
                    out.kept_user.push(address);
                }
                continue;
            }
            None => out.comments_added += 1,
        }
        out.commands.push(Command::SetComment {
            address,
            kind: *kind,
            text: Some(text.clone()),
        });
    }
    out
}
