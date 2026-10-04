//! An answer's text as widgets (docs/24, "The transcript"): prose as markup
//! with each citation a link into the main window, code blocks coloured as
//! Romlens colours its own, tables and rules. The macOS twin is `MessageText`,
//! `TableBlock` and `CodeBlock`.

use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use romlens_ffi::{CTokenInfo, CTokenKind};

use crate::gfxdraw::caption;
use crate::model::Document;
use crate::model::markdown::{self, Segment};
use crate::model::tutor;
use crate::palette;

/// What a click on a link does: the window opens the glossary bubble or shows
/// a citation in the main window.
pub type OnLink = Rc<dyn Fn(&gtk::Label, &str) -> bool>;

/// The widgets for `text`: one per segment.
pub fn message(doc: &Rc<Document>, text: &str, on_link: &OnLink) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let mut seen: HashSet<String> = HashSet::new();
    for s in markdown::segments(text) {
        match s {
            Segment::Prose(p) => b.append(&prose(&markdown::prose_markup(&p, &mut seen), on_link)),
            Segment::Code { language, body } => b.append(&code_block(doc, &language, &body)),
            Segment::Table { header, rows } => b.append(&table(&header, &rows, &mut seen, on_link)),
            Segment::Rule => b.append(&gtk::Separator::new(gtk::Orientation::Horizontal)),
        }
    }
    b
}

fn prose(markup: &str, on_link: &OnLink) -> gtk::Label {
    let l = gtk::Label::builder()
        .use_markup(true)
        .label(markup)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .build();
    let on_link = Rc::clone(on_link);
    l.connect_activate_link(move |label, uri| {
        if on_link(label, uri) {
            gtk::glib::Propagation::Stop
        } else {
            gtk::glib::Propagation::Proceed
        }
    });
    l
}

/// A Markdown table: a bold header, rows between rules, each cell inline
/// Markdown with its citations as links.
fn table(
    header: &[String],
    rows: &[Vec<String>],
    seen: &mut HashSet<String>,
    on_link: &OnLink,
) -> gtk::Widget {
    let grid = gtk::Grid::builder()
        .column_spacing(12)
        .row_spacing(6)
        .build();
    for (c, h) in header.iter().enumerate() {
        let l = prose(
            &format!("<b>{}</b>", markdown::prose_markup(h, seen)),
            on_link,
        );
        l.set_hexpand(true);
        grid.attach(&l, c as i32, 0, 1, 1);
    }
    grid.attach(
        &gtk::Separator::new(gtk::Orientation::Horizontal),
        0,
        1,
        header.len().max(1) as i32,
        1,
    );
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let l = prose(&markdown::prose_markup(cell, seen), on_link);
            l.set_hexpand(true);
            grid.attach(&l, c as i32, r as i32 + 2, 1, 1);
        }
    }
    let frame = gtk::Frame::new(None);
    frame.set_child(Some(&grid));
    grid.set_margin_top(8);
    grid.set_margin_bottom(8);
    grid.set_margin_start(8);
    grid.set_margin_end(8);
    // Cells wrap to the width there is: a scroller would measure them at their
    // narrowest and be tall.
    frame.set_valign(gtk::Align::Start);
    frame.upcast()
}

// MARK: Code

/// Code in an answer: C coloured by the core's lexer with the C view's
/// palette, 65816 and SPC700 assembly by a light pass.
fn code_block(doc: &Rc<Document>, language: &str, code: &str) -> gtk::Widget {
    let l = gtk::Label::builder()
        .use_markup(true)
        .label(coloured(doc, language, code))
        .xalign(0.0)
        .selectable(true)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .build();
    l.add_css_class("monospace");
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .hexpand(true)
        .valign(gtk::Align::Start)
        .child(&l)
        .build();
    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.add_css_class("flat");
    copy.set_tooltip_text(Some("Copy"));
    copy.set_halign(gtk::Align::End);
    copy.set_valign(gtk::Align::Start);
    let text = code.to_owned();
    copy.connect_clicked(move |b| b.clipboard().set_text(&text));
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroll));
    overlay.add_overlay(&copy);
    let frame = gtk::Frame::new(None);
    frame.set_child(Some(&overlay));
    frame.set_valign(gtk::Align::Start);
    frame.upcast()
}

fn hex(c: &gdk::RGBA) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (c.red() * 255.0) as u8,
        (c.green() * 255.0) as u8,
        (c.blue() * 255.0) as u8
    )
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Pango markup for `code` with `spans` (byte ranges and colours) painted.
fn paint(code: &str, mut spans: Vec<(usize, usize, String)>) -> String {
    spans.sort_by_key(|s| s.0);
    let mut out = String::new();
    let mut at = 0;
    for (a, b, colour) in spans {
        if a < at || b > code.len() || !code.is_char_boundary(a) || !code.is_char_boundary(b) {
            continue;
        }
        out += &esc(&code[at..a]);
        out += &format!("<span foreground=\"{colour}\">{}</span>", esc(&code[a..b]));
        at = b;
    }
    out + &esc(&code[at..])
}

pub fn coloured(doc: &Rc<Document>, language: &str, code: &str) -> String {
    let is_c = ["c", "h", "cpp"].contains(&language)
        || (language.is_empty() && code.contains(';') && code.contains('('));
    if is_c {
        let style = adw::StyleManager::default();
        let (dark, accent) = (style.is_dark(), style.accent_color_rgba());
        let fg = if dark {
            gdk::RGBA::new(0.9, 0.9, 0.9, 1.0)
        } else {
            gdk::RGBA::new(0.1, 0.1, 0.1, 1.0)
        };
        let tokens = doc.workbench().lex_c(code.to_owned());
        let spans = utf16_spans(code, &tokens)
            .into_iter()
            .map(|(a, b, kind)| (a, b, hex(&palette::c_token_color(kind, dark, &fg, &accent))))
            .collect();
        paint(code, spans)
    } else {
        paint(code, asm_spans(code))
    }
}

/// The lexer's tokens (UTF-16 offsets) as byte ranges.
fn utf16_spans(code: &str, tokens: &[CTokenInfo]) -> Vec<(usize, usize, CTokenKind)> {
    // Byte offset of each UTF-16 unit boundary.
    let mut at16 = Vec::with_capacity(code.len() + 1);
    for (b, c) in code.char_indices() {
        for _ in 0..c.len_utf16() {
            at16.push(b);
        }
    }
    at16.push(code.len());
    tokens
        .iter()
        .filter_map(|t| {
            let (a, b) = (
                *at16.get(t.start as usize)?,
                *at16.get((t.start + t.len) as usize)?,
            );
            Some((a, b, t.kind))
        })
        .collect()
}

/// A mnemonic after the address and bytes, `$` numbers, `;` comments.
pub fn asm_spans(code: &str) -> Vec<(usize, usize, String)> {
    let mut spans = Vec::new();
    let mut line_start = 0;
    for line in code.split_inclusive('\n') {
        let body = line.trim_end_matches('\n');
        // A comment takes the rest of the line.
        let comment_at = body.find(';');
        let scan = &body[..comment_at.unwrap_or(body.len())];
        let b = scan.as_bytes();
        let mut i = 0;
        while i < b.len() {
            // An operand: #$FF, $2100, $80:8000.
            if b[i] == b'$' || (b[i] == b'#' && b.get(i + 1) == Some(&b'$')) {
                let start = i;
                i += if b[i] == b'#' { 2 } else { 1 };
                while i < b.len() && (b[i].is_ascii_hexdigit() || b[i] == b':') {
                    i += 1;
                }
                spans.push((line_start + start, line_start + i, "#2aa198".to_owned()));
                continue;
            }
            // A mnemonic: three capitals standing alone, or with a dotted size.
            let word_start = i == 0 || !b[i - 1].is_ascii_alphanumeric();
            if word_start
                && i + 3 <= b.len()
                && b[i..i + 3].iter().all(u8::is_ascii_uppercase)
                && b.get(i + 3).is_none_or(|c| !c.is_ascii_alphanumeric())
            {
                spans.push((line_start + i, line_start + i + 3, "#a626a4".to_owned()));
                i += 3;
                continue;
            }
            i += 1;
        }
        if let Some(c) = comment_at {
            spans.push((
                line_start + c,
                line_start + body.len(),
                "#50a14f".to_owned(),
            ));
        }
        line_start += line.len();
    }
    spans
}

/// The hint line under a message footer: the model and what the reply cost.
pub fn footer(model: Option<&str>, cost: f64) -> Option<gtk::Label> {
    if model.is_none() && cost <= 0.0 {
        return None;
    }
    let mut parts = Vec::new();
    if let Some(m) = model {
        parts.push(m.to_owned());
    }
    if cost > 0.0 {
        parts.push(format!("${cost:.4}"));
    }
    Some(caption(&parts.join("  ")))
}

/// Show a citation in the window's tabs: beside the tutor's tab when that
/// has focus (docs/29). The tutor is in the window, so nothing is raised.
pub fn follow(doc: &Rc<Document>, uri: &str) -> bool {
    tutor::parse_citation(uri).is_some_and(|c| doc.follow_citation(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembly_gets_its_mnemonics_numbers_and_comments_painted() {
        let code = "$80:8000  SEI      ; stop IRQs\n  LDA #$8F\n  sta $2100";
        let spans = asm_spans(code);
        let text = |(a, b, _): &(usize, usize, String)| &code[*a..*b];
        let found: Vec<&str> = spans.iter().map(text).collect();
        assert!(found.contains(&"SEI") && found.contains(&"LDA"));
        assert!(found.contains(&"#$8F") && found.contains(&"$2100") && found.contains(&"$80:8000"));
        assert!(found.contains(&"; stop IRQs"));
        assert!(!found.contains(&"sta"), "lower case is not a mnemonic here");
        // A mnemonic-looking word inside the comment is not painted twice.
        let c = "NOP ; RTS";
        let s = asm_spans(c);
        assert_eq!(s.iter().filter(|s| &c[s.0..s.1] == "RTS").count(), 0);
    }

    #[test]
    fn the_lexers_utf16_offsets_become_byte_ranges_around_wide_characters() {
        let code = "é = 1; // 😀 x";
        let tokens = [
            CTokenInfo {
                start: 0,
                len: 1,
                kind: CTokenKind::Variable,
                address: None,
            },
            // The comment starts after the emoji's two UTF-16 units.
            CTokenInfo {
                start: 7,
                len: 7,
                kind: CTokenKind::Comment,
                address: None,
            },
        ];
        let s = utf16_spans(code, &tokens);
        assert_eq!(&code[s[0].0..s[0].1], "é");
        assert_eq!(&code[s[1].0..s[1].1], "// 😀 x");
    }

    #[test]
    fn painted_code_is_escaped_markup() {
        let m = paint("a<b && c", vec![(0, 1, "#fff".into())]);
        assert_eq!(m, "<span foreground=\"#fff\">a</span>&lt;b &amp;&amp; c");
        // A span outside the text, or inside a character, is skipped.
        assert_eq!(paint("é", vec![(1, 2, "#fff".into())]), "é");
        assert_eq!(paint("ab", vec![(0, 9, "#fff".into())]), "ab");
    }
}
