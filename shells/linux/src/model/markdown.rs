//! An answer's text (docs/24, "The transcript"): prose, fenced code, tables
//! and rules apart; prose as Pango markup, with each citation (`$BB:AAAA`,
//! `frame N`) a link into the main window and each glossary term linked the
//! first time a message uses it (docs/27). Pure text in, text out. The macOS
//! twin is `MessageText` and `Glossary`.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use romlens_ffi::GlossaryEntryInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Prose(String),
    Code {
        language: String,
        body: String,
    },
    /// A Markdown pipe table; each row has as many cells as the header.
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// `---` on its own line.
    Rule,
}

/// Prose, fenced code, tables and rules, apart. An unclosed fence (still
/// streaming) is code to the end.
pub fn segments(text: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut prose: Vec<&str> = Vec::new();
    let mut code: Option<Vec<&str>> = None;
    let mut lang = String::new();
    let lines: Vec<&str> = text.split('\n').collect();
    let flush = |prose: &mut Vec<&str>, out: &mut Vec<Segment>| {
        let p = prose.join("\n");
        let p = p.trim_matches('\n');
        if !p.is_empty() {
            out.push(Segment::Prose(p.to_owned()));
        }
        prose.clear();
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim();
        i += 1;
        if code.is_none() && t.starts_with('|') && i < lines.len() && is_table_rule(lines[i]) {
            let header = cells(t);
            let mut rows = Vec::new();
            i += 1;
            while i < lines.len() {
                let r = lines[i].trim();
                if !r.starts_with('|') {
                    break;
                }
                let mut c = cells(r);
                c.resize(header.len(), String::new());
                rows.push(c);
                i += 1;
            }
            flush(&mut prose, &mut out);
            out.push(Segment::Table { header, rows });
            continue;
        }
        if code.is_none() && is_rule(t) {
            flush(&mut prose, &mut out);
            out.push(Segment::Rule);
            continue;
        }
        if let Some(info) = t.strip_prefix("```") {
            match code.take() {
                Some(mut c) => {
                    if c.last().is_some_and(|l| l.is_empty()) {
                        c.pop();
                    }
                    out.push(Segment::Code {
                        language: lang.clone(),
                        body: c.join("\n"),
                    });
                }
                None => {
                    flush(&mut prose, &mut out);
                    lang = info.to_lowercase();
                    code = Some(Vec::new());
                }
            }
        } else if let Some(c) = &mut code {
            c.push(line);
        } else {
            prose.push(line);
        }
    }
    if let Some(c) = code {
        out.push(Segment::Code {
            language: lang,
            body: c.join("\n"),
        });
    }
    let rest = prose.join("\n");
    let rest = rest.trim_matches('\n');
    if !rest.is_empty() {
        out.push(Segment::Prose(rest.to_owned()));
    }
    out.into_iter()
        .filter(|s| !matches!(s, Segment::Prose(p) if p.trim().is_empty()))
        .collect()
}

/// `---`, `***` or `___` on its own line, with optional spaces between.
fn is_rule(t: &str) -> bool {
    let chars: Vec<char> = t.chars().filter(|c| *c != ' ').collect();
    chars.len() >= 3
        && matches!(chars[0], '-' | '*' | '_')
        && chars.iter().all(|c| *c == chars[0])
        // The spaces only separate: "- - -" but not "-  x".
        && t.chars().all(|c| c == chars[0] || c == ' ')
}

/// `|---|:--:|`: the line under a table's header.
fn is_table_rule(line: &str) -> bool {
    let t = line.trim();
    t.contains('-')
        && t.contains('|')
        && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
        && t.split('|').filter(|c| !c.trim().is_empty()).all(|c| {
            let c = c.trim().trim_matches(':');
            !c.is_empty() && c.chars().all(|x| x == '-')
        })
}

/// A table row's cells, without the outer pipes; `\|` is a pipe in a cell,
/// and so is one inside backticks.
pub fn cells(row: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cell = String::new();
    let (mut in_code, mut escaped) = (false, false);
    for ch in row.chars() {
        if escaped {
            if ch != '|' {
                cell.push('\\');
            }
            cell.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '`' => {
                in_code = !in_code;
                cell.push(ch);
            }
            '|' if !in_code => out.push(std::mem::take(&mut cell)),
            _ => cell.push(ch),
        }
    }
    out.push(cell);
    if row.starts_with('|') {
        out.remove(0);
    }
    if row.ends_with('|') && !out.is_empty() {
        out.pop();
    }
    out.into_iter().map(|c| c.trim().to_owned()).collect()
}

// MARK: Prose

/// Markdown for a run of prose as Pango markup, citations and glossary terms
/// made links. Headings become bold lines and list items bullets, since the
/// text keeps its line breaks. `seen` holds the glossary terms already linked
/// in this message.
pub fn prose_markup(text: &str, seen: &mut HashSet<String>) -> String {
    let mut lines = Vec::new();
    for line in text.split('\n') {
        let mut l = line.to_owned();
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            l = format!("**{}**", line[hashes..].trim_start());
        } else {
            let indent = line.len() - line.trim_start_matches(' ').len();
            let rest = &line[indent..];
            if (rest.starts_with("- ") || rest.starts_with("* ")) && rest.len() > 2 {
                l = format!("{}• {}", " ".repeat(indent), rest[2..].trim_start());
            }
        }
        lines.push(inline(&link_citations(&link_glossary(&l, seen))));
    }
    lines.join("\n")
}

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            _ => o.push(c),
        }
    }
    o
}

/// Inline Markdown (bold, italics, code, links) as Pango markup.
pub fn inline(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let find = |from: usize, pat: &[char]| -> Option<usize> {
        (from..c.len().saturating_sub(pat.len() - 1)).find(|&j| c[j..j + pat.len()] == *pat)
    };
    while i < c.len() {
        match c[i] {
            '\\' if i + 1 < c.len() => {
                out.push_str(&escape(&c[i + 1].to_string()));
                i += 2;
            }
            '`' => match find(i + 1, &['`']) {
                Some(j) => {
                    let body: String = c[i + 1..j].iter().collect();
                    out.push_str(&format!("<tt>{}</tt>", escape(&body)));
                    i = j + 1;
                }
                None => {
                    out.push('`');
                    i += 1;
                }
            },
            '[' => {
                // A link: [text](url).
                let link =
                    find(i + 1, &[']', '(']).and_then(|j| find(j + 2, &[')']).map(|k| (j, k)));
                match link {
                    Some((j, k)) => {
                        let text: String = c[i + 1..j].iter().collect();
                        let url: String = c[j + 2..k].iter().collect();
                        out.push_str(&format!(
                            "<a href=\"{}\">{}</a>",
                            escape(&url),
                            inline(&text)
                        ));
                        i = k + 1;
                    }
                    None => {
                        out.push('[');
                        i += 1;
                    }
                }
            }
            '*' if c.get(i + 1) == Some(&'*') => match find(i + 2, &['*', '*']) {
                Some(j) if j > i + 2 => {
                    let body: String = c[i + 2..j].iter().collect();
                    out.push_str(&format!("<b>{}</b>", inline(&body)));
                    i = j + 2;
                }
                _ => {
                    out.push_str("**");
                    i += 2;
                }
            },
            m @ ('*' | '_') => {
                // An emphasis opens before a word and closes after one.
                let opens = c.get(i + 1).is_some_and(|n| !n.is_whitespace())
                    && (m == '*' || i == 0 || !c[i - 1].is_alphanumeric());
                let close = opens.then(|| find(i + 1, &[m])).flatten().filter(|&j| {
                    j > i + 1
                        && !c[j - 1].is_whitespace()
                        && (m == '*' || c.get(j + 1).is_none_or(|n| !n.is_alphanumeric()))
                });
                match close {
                    Some(j) => {
                        let body: String = c[i + 1..j].iter().collect();
                        out.push_str(&format!("<i>{}</i>", inline(&body)));
                        i = j + 1;
                    }
                    None => {
                        out.push(m);
                        i += 1;
                    }
                }
            }
            ch => {
                out.push_str(&escape(&ch.to_string()));
                i += 1;
            }
        }
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `$80:8000` (bare or in backticks) and `frame 12` as Markdown links.
pub fn link_citations(line: &str) -> String {
    let c: Vec<char> = line.chars().collect();
    let hex = |s: &[char]| s.iter().all(char::is_ascii_hexdigit);
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        // `$BB:AAAA` in backticks.
        if c[i] == '`'
            && i + 9 < c.len()
            && c[i + 1] == '$'
            && hex(&c[i + 2..i + 4])
            && c[i + 4] == ':'
            && hex(&c[i + 5..i + 9])
            && c[i + 9] == '`'
        {
            let (bb, aaaa): (String, String) = (
                c[i + 2..i + 4].iter().collect(),
                c[i + 5..i + 9].iter().collect(),
            );
            out.push_str(&format!("[`${bb}:{aaaa}`](romlens://a/{bb}{aaaa})"));
            i += 10;
            continue;
        }
        // A bare $BB:AAAA, not inside a link, code or a longer word.
        if c[i] == '$'
            && i + 8 <= c.len()
            && hex(&c[i + 1..i + 3])
            && c[i + 3] == ':'
            && hex(&c[i + 4..i + 8])
        {
            let before_ok = i == 0 || !(is_word(c[i - 1]) || matches!(c[i - 1], '[' | '`'));
            let after_ok = c
                .get(i + 8)
                .is_none_or(|n| !(is_word(*n) || matches!(n, '`' | ']')));
            if before_ok && after_ok {
                let (bb, aaaa): (String, String) = (
                    c[i + 1..i + 3].iter().collect(),
                    c[i + 4..i + 8].iter().collect(),
                );
                out.push_str(&format!("[${bb}:{aaaa}](romlens://a/{bb}{aaaa})"));
                i += 8;
                continue;
            }
        }
        // frame N
        if matches!(c[i], 'F' | 'f')
            && c[i..]
                .iter()
                .take(6)
                .collect::<String>()
                .eq_ignore_ascii_case("frame ")
            && (i == 0 || !(is_word(c[i - 1]) || c[i - 1] == '['))
        {
            let digits: String = c[i + 6..]
                .iter()
                .take_while(|d| d.is_ascii_digit())
                .collect();
            let end = i + 6 + digits.len();
            if !digits.is_empty() && c.get(end).is_none_or(|n| !is_word(*n)) {
                let word: String = c[i..i + 5].iter().collect();
                out.push_str(&format!("[{word} {digits}](romlens://f/{digits})"));
                i = end;
                continue;
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

// MARK: Glossary

/// Every spelling, a term's own and its others, to its entry.
fn entries() -> &'static HashMap<String, GlossaryEntryInfo> {
    static E: OnceLock<HashMap<String, GlossaryEntryInfo>> = OnceLock::new();
    E.get_or_init(|| {
        let mut d = HashMap::new();
        for e in romlens_ffi::glossary() {
            for s in std::iter::once(e.term.clone()).chain(e.also.iter().cloned()) {
                d.entry(s).or_insert_with(|| e.clone());
            }
        }
        d
    })
}

/// The spellings, longest first, so `DSP-1` wins over `DSP`.
fn spellings() -> &'static Vec<String> {
    static S: OnceLock<Vec<String>> = OnceLock::new();
    S.get_or_init(|| {
        let mut v: Vec<String> = entries().keys().cloned().collect();
        v.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then(a.cmp(b)));
        v
    })
}

pub fn entry(spelling: &str) -> Option<&'static GlossaryEntryInfo> {
    entries().get(spelling)
}

/// The link a term's first appearance carries.
pub fn glossary_url(term: &str) -> String {
    format!(
        "romlens://g/?t={}",
        term.replace('%', "%25")
            .replace(' ', "%20")
            .replace('&', "%26")
    )
}

/// The entry a glossary link names, or `None` for any other link.
pub fn entry_for(url: &str) -> Option<&'static GlossaryEntryInfo> {
    let rest = url.strip_prefix("romlens://g/?t=")?;
    let term = rest
        .replace("%20", " ")
        .replace("%26", "&")
        .replace("%25", "%");
    entry(&term)
}

/// A line of Markdown with each term not yet in `seen` made a link, and added
/// to it. Links already there are left alone; inline code is linked only when
/// it is a term and nothing else.
pub fn link_glossary(line: &str, seen: &mut HashSet<String>) -> String {
    let c: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    'scan: while i < c.len() {
        // A Markdown link, whole.
        if c[i] == '['
            && let Some(close) =
                (i + 1..c.len().saturating_sub(1)).find(|&j| c[j] == ']' && c[j + 1] == '(')
            && let Some(end) = (close + 2..c.len()).find(|&j| c[j] == ')')
        {
            out.extend(&c[i..=end]);
            i = end + 1;
            continue;
        }
        // Inline code: a term and nothing else is linked.
        if c[i] == '`'
            && let Some(close) = (i + 1..c.len()).find(|&j| c[j] == '`')
        {
            let inner: String = c[i + 1..close].iter().collect();
            let found: String = c[i..=close].iter().collect();
            match entry(&inner).filter(|e| !seen.contains(&e.term)) {
                Some(e) => {
                    seen.insert(e.term.clone());
                    out.push_str(&format!("[{found}]({})", glossary_url(&e.term)));
                }
                None => out.push_str(&found),
            }
            i = close + 1;
            continue;
        }
        // A term standing on its own.
        let before_ok = i == 0 || !(is_word(c[i - 1]) || matches!(c[i - 1], '$' | ':' | '/' | '-'));
        if before_ok && c[i].is_alphanumeric() {
            for s in spellings() {
                let n = s.chars().count();
                if i + n <= c.len()
                    && c[i..i + n].iter().copied().eq(s.chars())
                    && c.get(i + n).is_none_or(|x| !(is_word(*x) || *x == '-'))
                    && let Some(e) = entry(s).filter(|e| !seen.contains(&e.term))
                {
                    seen.insert(e.term.clone());
                    let word: String = c[i..i + n].iter().collect();
                    out.push_str(&format!("[{word}]({})", glossary_url(&e.term)));
                    i += n;
                    continue 'scan;
                }
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_code_tables_and_rules_come_apart_and_an_open_fence_is_code_to_the_end() {
        let text = "Intro line\n\n```c\nint x;\n```\nthen\n---\n| a | b |\n|---|:-:|\n| 1 | 2 |\n| 3 |\nend\n```asm\nLDA #$00";
        let s = segments(text);
        assert_eq!(s[0], Segment::Prose("Intro line".into()));
        assert_eq!(
            s[1],
            Segment::Code {
                language: "c".into(),
                body: "int x;".into()
            }
        );
        assert_eq!(s[2], Segment::Prose("then".into()));
        assert_eq!(s[3], Segment::Rule);
        assert_eq!(
            s[4],
            Segment::Table {
                header: vec!["a".into(), "b".into()],
                rows: vec![vec!["1".into(), "2".into()], vec!["3".into(), "".into()]]
            }
        );
        assert_eq!(s[5], Segment::Prose("end".into()));
        assert_eq!(
            s[6],
            Segment::Code {
                language: "asm".into(),
                body: "LDA #$00".into()
            }
        );
        assert_eq!(s.len(), 7);
    }

    #[test]
    fn a_table_needs_its_rule_and_cells_keep_escaped_and_quoted_pipes() {
        assert!(
            segments("| a | b |\n| c | d |")
                .iter()
                .all(|s| matches!(s, Segment::Prose(_)))
        );
        assert_eq!(cells("| `a|b` | c \\| d | e |"), ["`a|b`", "c | d", "e"]);
        assert!(is_table_rule("|---|:--:|"));
        assert!(is_table_rule("--- | ---"));
        assert!(!is_table_rule("| a | b |"));
        assert!(is_rule("- - -") && is_rule("***") && !is_rule("--") && !is_rule("-x-"));
    }

    #[test]
    fn inline_markdown_becomes_pango_markup_and_text_is_escaped() {
        assert_eq!(
            inline("a **bold** and *it* and `c<d>`"),
            "a <b>bold</b> and <i>it</i> and <tt>c&lt;d&gt;</tt>"
        );
        assert_eq!(inline("snake_case_name stays"), "snake_case_name stays");
        assert_eq!(inline("_em_ here"), "<i>em</i> here");
        assert_eq!(
            inline("[text](http://x?a=1&b=2)"),
            "<a href=\"http://x?a=1&amp;b=2\">text</a>"
        );
        assert_eq!(
            inline("2 * 3 * 4"),
            "2 * 3 * 4",
            "spaced stars are not emphasis"
        );
        assert_eq!(inline("a \\* b"), "a * b");
        assert_eq!(inline("unclosed `code"), "unclosed `code");
        assert_eq!(inline("**"), "**");
    }

    #[test]
    fn headings_are_bold_and_bullets_are_bullets() {
        let mut seen = HashSet::new();
        assert_eq!(
            prose_markup("## Title here", &mut seen),
            "<b>Title here</b>"
        );
        assert_eq!(prose_markup("- one\n  * two", &mut seen), "• one\n  • two");
    }

    #[test]
    fn addresses_and_frames_are_links_into_the_main_window() {
        assert_eq!(
            link_citations("at $80:8000 and `$7E:0200` then Frame 12."),
            "at [$80:8000](romlens://a/808000) and [`$7E:0200`](romlens://a/7E0200) then [Frame 12](romlens://f/12)."
        );
        // Not inside a longer word or an existing link.
        assert_eq!(link_citations("x$80:8000"), "x$80:8000");
        assert_eq!(link_citations("[$80:8000](u)"), "[$80:8000](u)");
        assert_eq!(link_citations("$80:80001"), "$80:80001");
        assert_eq!(
            link_citations("preframe 3 and frames"),
            "preframe 3 and frames"
        );
        assert_eq!(link_citations("short $80:80"), "short $80:80");
    }

    #[test]
    fn a_glossary_term_is_linked_the_first_time_a_message_uses_it() {
        let mut seen = HashSet::new();
        let first = link_glossary("The DMA copies; DMA again.", &mut seen);
        assert_eq!(first.matches("romlens://g/").count(), 1, "{first}");
        assert!(first.starts_with("The [DMA](romlens://g/?t=DMA)"));
        assert!(seen.contains("DMA"));
        // A link or inline code already there is left alone, a lone term in
        // backticks is linked.
        let mut seen = HashSet::new();
        assert_eq!(
            link_glossary("[DMA](http://x)", &mut seen),
            "[DMA](http://x)"
        );
        assert!(link_glossary("see `VMAIN`", &mut HashSet::new()).contains("romlens://g/"));
        assert_eq!(
            link_glossary("`LDA #1` stays", &mut HashSet::new()),
            "`LDA #1` stays"
        );
        // Inside a longer word or a cited address it is not a term.
        assert_eq!(link_glossary("ADMAX", &mut HashSet::new()), "ADMAX");
        assert_eq!(link_glossary("$80:DMA0", &mut HashSet::new()), "$80:DMA0");
    }

    #[test]
    fn a_glossary_link_finds_its_entry_again() {
        let e = romlens_ffi::glossary()
            .into_iter()
            .next()
            .expect("a glossary");
        let url = glossary_url(&e.term);
        assert_eq!(entry_for(&url).map(|x| x.term.clone()), Some(e.term));
        assert!(entry_for("romlens://a/008000").is_none());
        assert!(entry_for("https://example.com").is_none());
    }
}
