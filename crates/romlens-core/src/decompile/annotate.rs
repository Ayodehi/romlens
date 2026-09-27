//! The student's (and the tutor's) words applied to the generated C
//! (docs/24, decision 6): a routine's locals by the names they were given,
//! a note above the routine, and comments before the statements their
//! instructions make. It works on the finished text, so every level gets
//! it and the stages before stay as they are; a name for a local the
//! routine no longer has is simply not used.

use crate::decompile::Decompiled;
use crate::decompile::emit::{CToken, CTokenKind};
use crate::memory::address::FileOffset;
use crate::model::project::Project;
use crate::rom::image::RomImage;

struct Line {
    text: String,
    /// Tokens, with `start` from the line's first byte.
    toks: Vec<CToken>,
    offs: Vec<FileOffset>,
}

pub fn annotate(d: &mut Decompiled, project: &Project, rom: &RomImage) {
    let renames: Vec<(&str, &str)> = project
        .local_names
        .range((d.entry, String::new())..)
        .take_while(|((r, _), _)| *r == d.entry)
        .map(|((_, l), n)| (l.as_str(), n.as_str()))
        .collect();
    let note = project.routine_notes.get(&d.entry);
    let own: std::collections::BTreeSet<FileOffset> = d.lines.iter().flatten().copied().collect();
    let comments: Vec<(FileOffset, &str)> = project
        .c_comments
        .iter()
        .filter_map(|(a, t)| {
            let off = rom.file_offset_for(*a)?;
            own.contains(&off).then_some((off, t.as_str()))
        })
        .collect();
    if renames.is_empty() && note.is_none() && comments.is_empty() {
        return;
    }

    let mut lines = split(d);
    let def = definition_line(&lines, d);
    if let Some(def) = def {
        for l in &mut lines[def..] {
            rename(l, &renames);
        }
    }
    // Comments, from the bottom up so the indices hold.
    let mut at: Vec<(usize, FileOffset, &str)> = comments
        .iter()
        .filter_map(|(off, t)| {
            let i = lines.iter().position(|l| l.offs.contains(off))?;
            Some((i, *off, *t))
        })
        .collect();
    at.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    for (i, off, text) in at {
        let indent: String = lines[i]
            .text
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect();
        lines.insert(i, comment_line(&indent, text, vec![off]));
    }
    if let (Some(note), Some(def)) = (note, definition_line(&lines, d).or(def)) {
        // Above the routine's own summary comment, when it has one.
        let mut i = def;
        if i > 0 && lines[i - 1].text.trim_start().starts_with("/*") {
            i -= 1;
        }
        lines.insert(i, comment_line("", note, Vec::new()));
    }
    join(d, lines);
}

fn comment_line(indent: &str, text: &str, offs: Vec<FileOffset>) -> Line {
    let body = text.replace('\n', &format!("\n{indent}   "));
    let t = format!("{indent}/* {body} */");
    let start = indent.len() as u32;
    Line {
        toks: vec![CToken {
            start,
            len: t.len() as u32 - start,
            kind: CTokenKind::Comment,
            address: None,
        }],
        text: t,
        offs,
    }
}

fn split(d: &Decompiled) -> Vec<Line> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, text) in d.text.split('\n').enumerate() {
        let end = start + text.len();
        let toks = d
            .tokens
            .iter()
            .filter(|t| (start..end.max(start + 1)).contains(&(t.start as usize)))
            .map(|t| CToken {
                start: t.start - start as u32,
                ..t.clone()
            })
            .collect();
        out.push(Line {
            text: text.to_owned(),
            toks,
            offs: d.lines.get(i).cloned().unwrap_or_default(),
        });
        start = end + 1;
    }
    out
}

fn join(d: &mut Decompiled, lines: Vec<Line>) {
    let mut text = String::new();
    let mut tokens = Vec::new();
    let mut offs = Vec::with_capacity(lines.len());
    let n = lines.len();
    for (i, l) in lines.into_iter().enumerate() {
        let base = text.len() as u32;
        tokens.extend(l.toks.into_iter().map(|t| CToken {
            start: t.start + base,
            ..t
        }));
        text.push_str(&l.text);
        if i + 1 < n {
            text.push('\n');
        }
        offs.push(l.offs);
    }
    // `lines` had one entry per line of the text, not counting a last
    // empty one after the final newline.
    if text.ends_with('\n') {
        offs.pop();
    }
    d.text = text;
    d.tokens = tokens;
    d.lines = offs;
}

/// The line that opens the routine's definition: its own name, called,
/// not ending in `;`.
fn definition_line(lines: &[Line], d: &Decompiled) -> Option<usize> {
    lines.iter().position(|l| {
        let t = l.text.trim_end();
        !t.ends_with(';')
            && l.toks.iter().any(|k| {
                k.kind == CTokenKind::Function
                    && l.text.get(k.start as usize..(k.start + k.len) as usize)
                        == Some(d.name.as_str())
            })
    })
}

fn rename(l: &mut Line, renames: &[(&str, &str)]) {
    let comments: Vec<(u32, u32)> = l
        .toks
        .iter()
        .filter(|t| t.kind == CTokenKind::Comment)
        .map(|t| (t.start, t.start + t.len))
        .collect();
    let in_comment = |i: usize| {
        comments
            .iter()
            .any(|(s, e)| (*s as usize..*e as usize).contains(&i))
    };
    let b = l.text.as_bytes();
    let mut edits: Vec<(usize, usize, &str)> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if (b[i].is_ascii_alphabetic() || b[i] == b'_') && (i == 0 || !is_word(b[i - 1])) {
            let mut j = i + 1;
            while j < b.len() && is_word(b[j]) {
                j += 1;
            }
            if !in_comment(i)
                && let Some((_, to)) = renames.iter().find(|(from, _)| *from == &l.text[i..j])
            {
                edits.push((i, j, to));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    for (s, e, to) in edits.into_iter().rev() {
        let delta = to.len() as i64 - (e - s) as i64;
        l.text.replace_range(s..e, to);
        for t in &mut l.toks {
            if t.start as usize == s && t.len as usize == e - s {
                t.len = to.len() as u32;
            } else if t.start as usize >= e {
                t.start = (t.start as i64 + delta) as u32;
            }
        }
    }
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}
