//! The explanations in the C (docs/20): what each hardware store's value
//! means, after the statement, and each idiom's note above the code that
//! does it. Comments only, so the code is exactly what it was.

use std::collections::BTreeMap;

use crate::decompile::emit::{CToken, CTokenKind, Writer, safe_comment};
use crate::decompile::function::Function;
use crate::explain::{Explained, Idiom};

/// `body` with the comments added.
pub fn annotate(body: Writer, f: &Function, writes: &[Explained], idioms: &[Idiom]) -> Writer {
    let n = body.lines.len();
    if n == 0 {
        return body;
    }
    let step_of = |off| f.index_of(off);
    // The first line each step is printed on.
    let mut line_of: BTreeMap<usize, usize> = BTreeMap::new();
    for (l, steps) in body.lines.iter().enumerate() {
        for &s in steps {
            line_of.entry(s).or_insert(l);
        }
    }
    // Each line's text runs from its start to its newline.
    let ends: Vec<usize> = body.text.match_indices('\n').map(|(i, _)| i).collect();
    if ends.len() != n {
        return body;
    }
    let starts: Vec<usize> = std::iter::once(0)
        .chain(ends.iter().map(|e| e + 1))
        .take(n)
        .collect();
    let text_of = |l: usize| &body.text[starts[l]..ends[l]];

    // After a store: what its value means.
    let mut after: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for e in writes {
        let summaries: Vec<&str> = e
            .write
            .parts
            .iter()
            .filter_map(|p| p.summary.as_deref())
            .collect();
        if summaries.is_empty() {
            continue;
        }
        if let Some(&l) = step_of(e.offset).and_then(|s| line_of.get(&s)) {
            after.entry(l).or_default().push(summaries.join("; "));
        }
    }
    // Above an idiom: its note, before the first line any of its
    // instructions print on, and above the loop that line opens into.
    let mut before: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for i in idioms {
        let Some(mut l) = i
            .offsets
            .iter()
            .filter_map(|&o| step_of(o).and_then(|s| line_of.get(&s)).copied())
            .min()
        else {
            continue;
        };
        while l > 0 && body.lines[l - 1].is_empty() && text_of(l - 1).trim_end().ends_with('{') {
            l -= 1;
        }
        before
            .entry(l)
            .or_default()
            .push(format!("▸ {}: {}", i.title, i.summary));
    }
    if after.is_empty() && before.is_empty() {
        return body;
    }

    let mut out = Writer::default();
    out.indent = body.indent;
    let mut tok = body.tokens.into_iter().peekable();
    for l in 0..n {
        let line = text_of(l);
        let indent: String = line.chars().take_while(|c| *c == ' ').collect();
        for note in before.get(&l).into_iter().flatten() {
            out.text.push_str(&indent);
            push_comment(&mut out, note);
            out.text.push('\n');
            out.lines.push(Vec::new());
        }
        let shift = out.text.len() as i64 - starts[l] as i64;
        while let Some(t) = tok.next_if(|t| (t.start as usize) < ends[l]) {
            out.tokens.push(CToken {
                start: (t.start as i64 + shift) as u32,
                ..t
            });
        }
        out.text.push_str(line);
        if let Some(notes) = after.get(&l) {
            out.text.push(' ');
            push_comment(&mut out, &notes.join("; "));
        }
        out.text.push('\n');
        out.lines.push(body.lines[l].clone());
    }
    out
}

fn push_comment(w: &mut Writer, text: &str) {
    let c = format!("/* {} */", safe_comment(text));
    w.tokens.push(CToken {
        start: w.text.len() as u32,
        len: c.len() as u32,
        kind: CTokenKind::Comment,
        address: None,
    });
    w.text.push_str(&c);
}
