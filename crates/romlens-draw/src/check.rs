//! The checks every picture passes before it is drawn, Romlens's own and
//! the tutor's SVG alike (docs/26):
//!
//! 1. Size and structure: at most 64 KB of well-formed XML with a `viewBox`
//!    of at most 1,600 × 1,200 and an aspect between 1:3 and 3:1.
//! 2. Nothing unsafe: no scripts, embedded documents, images, animation,
//!    event attributes, imports or links but to a local `#fragment`.
//! 3. On the canvas: every element inside the `viewBox`.
//! 4. Text: no two texts overlapping, none running out of the shape it sits
//!    on, at least 11 points at display size, at least 3:1 contrast against
//!    what is behind it, and every character in the fonts.
//! 5. Not empty: something drawn, and some text.

use std::fmt;

use resvg::usvg::{self, Node, Paint, Rect, roxmltree};

use crate::fonts;
use crate::svg::palette;

pub const MAX_BYTES: usize = 64 * 1024;
pub const MAX_WIDTH: f64 = 1600.0;
pub const MAX_HEIGHT: f64 = 1200.0;
/// The smallest text, in points at the size the picture is shown.
pub const MIN_TEXT: f64 = 11.0;
pub const MIN_CONTRAST: f64 = 3.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub what: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.what)
    }
}

fn problem(what: impl Into<String>) -> Problem {
    Problem { what: what.into() }
}

/// The problems as one message, a line each.
pub fn describe(problems: &[Problem]) -> String {
    problems
        .iter()
        .map(|p| format!("- {p}"))
        .collect::<Vec<_>>()
        .join("\n")
}

const REFUSED: &[&str] = &[
    "script",
    "foreignObject",
    "image",
    "iframe",
    "object",
    "embed",
    "audio",
    "video",
    "canvas",
    "animate",
    "animateMotion",
    "animateTransform",
    "animateColor",
    "set",
    "discard",
    "a",
];

/// Checks 1 and 2, on the XML as written. Returns the `viewBox`'s size.
pub fn structure(svg: &str) -> Result<(f64, f64), Vec<Problem>> {
    if svg.len() > MAX_BYTES {
        return Err(vec![problem(format!(
            "the SVG is {} KB; at most {} KB",
            svg.len() / 1024,
            MAX_BYTES / 1024
        ))]);
    }
    if svg.contains("<!DOCTYPE") || svg.contains("<!ENTITY") {
        return Err(vec![problem("no DOCTYPE or entities")]);
    }
    let doc = roxmltree::Document::parse(svg)
        .map_err(|e| vec![problem(format!("not well-formed XML: {e}"))])?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err(vec![problem("the root element must be <svg>")]);
    }
    let mut out = Vec::new();
    for node in doc.descendants().filter(|n| n.is_element()) {
        let tag = node.tag_name().name();
        if REFUSED.contains(&tag) {
            out.push(problem(format!("<{tag}> is not allowed")));
        }
        if tag == "style" {
            let css = node.text().unwrap_or("");
            if css.contains("@import") || css.contains("url(") && !css.contains("url(#") {
                out.push(problem("a <style> may not import or link anything"));
            }
        }
        for a in node.attributes() {
            let name = a.name();
            if name.to_ascii_lowercase().starts_with("on") {
                out.push(problem(format!(
                    "the event attribute {name} is not allowed"
                )));
            }
            if name == "href" && !a.value().starts_with('#') {
                out.push(problem(format!(
                    "a link must be to a local #fragment, not {}",
                    a.value()
                )));
            }
            if a.value().contains("url(") && !a.value().replace(' ', "").contains("url(#") {
                out.push(problem(format!(
                    "{name} may only refer to a local #fragment"
                )));
            }
        }
    }
    let size = match root.attribute("viewBox").map(|v| {
        v.split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<f64>())
            .collect::<Result<Vec<_>, _>>()
    }) {
        Some(Ok(v)) if v.len() == 4 => {
            let (w, h) = (v[2], v[3]);
            if !(w >= 50.0 && h >= 50.0 && w <= MAX_WIDTH && h <= MAX_HEIGHT) {
                out.push(problem(format!(
                    "the viewBox is {w} × {h}; it must be between 50 × 50 and {MAX_WIDTH} × {MAX_HEIGHT}"
                )));
            } else if w / h > 3.0 || h / w > 3.0 {
                out.push(problem(format!(
                    "the viewBox is {w} × {h}; its aspect must be between 1:3 and 3:1"
                )));
            }
            (w, h)
        }
        _ => {
            out.push(problem(
                "the <svg> needs a viewBox of four numbers, such as viewBox=\"0 0 720 400\"",
            ));
            (0.0, 0.0)
        }
    };
    if out.is_empty() { Ok(size) } else { Err(out) }
}

/// How much smaller than its viewBox a picture is shown: never larger than
/// 720 × 540 points.
pub fn display_scale(width: f64, height: f64) -> f64 {
    (720.0 / width).min(540.0 / height).min(1.0)
}

struct Filled {
    rect: Rect,
    colour: usvg::Color,
}

struct Label {
    text: String,
    rect: Rect,
    colour: Option<usvg::Color>,
    size: f64,
    /// The last filled shape drawn under the text.
    behind: Option<usize>,
}

fn scale_of(t: usvg::Transform) -> f64 {
    ((t.sx * t.sy - t.kx * t.ky).abs() as f64).sqrt()
}

fn walk(g: &usvg::Group, shapes: &mut Vec<Filled>, labels: &mut Vec<Label>, drawn: &mut usize) {
    for node in g.children() {
        match node {
            Node::Group(g) => walk(g, shapes, labels, drawn),
            Node::Path(p) => {
                if !p.is_visible() {
                    continue;
                }
                *drawn += 1;
                if let Some(f) = p.fill()
                    && let Paint::Color(c) = f.paint()
                    && f.opacity().get() >= 0.5
                {
                    let r = p.abs_bounding_box();
                    if r.width() > 2.0 && r.height() > 2.0 {
                        shapes.push(Filled {
                            rect: r,
                            colour: *c,
                        });
                    }
                }
            }
            Node::Image(_) => *drawn += 1,
            Node::Text(t) => {
                let text: String = t.chunks().iter().map(|c| c.text()).collect();
                if text.trim().is_empty() {
                    continue;
                }
                let span = t.chunks().iter().flat_map(|c| c.spans()).next();
                let colour = span.and_then(|s| s.fill()).and_then(|f| match f.paint() {
                    Paint::Color(c) => Some(*c),
                    _ => None,
                });
                let size = span.map(|s| s.font_size().get() as f64).unwrap_or(12.0)
                    * scale_of(t.abs_transform());
                for c in text.chars().filter(|c| !c.is_whitespace()) {
                    let mono = t.chunks().iter().flat_map(|c| c.spans()).any(|s| {
                        s.font()
                            .families()
                            .iter()
                            .any(|f| f.to_string().to_lowercase().contains("mono"))
                    });
                    if !fonts::has_char(c, mono) && !fonts::has_char(c, !mono) {
                        labels.push(Label {
                            text: format!("\u{0}{c}"),
                            rect: t.abs_bounding_box(),
                            colour,
                            size,
                            behind: None,
                        });
                    }
                }
                // The glyphs' own box, tighter than the line box.
                let rect = t.flattened().abs_bounding_box();
                // What it sits on: the last filled shape under a good part
                // of it (a quarter), so a label that runs out of its box is
                // still that box's.
                let area = (rect.width() * rect.height()) as f64;
                let behind = shapes
                    .iter()
                    .rposition(|s| overlap(&s.rect, &rect) >= area * 0.25);
                *drawn += 1;
                labels.push(Label {
                    text,
                    rect,
                    colour,
                    size,
                    behind,
                });
            }
        }
    }
}

fn luminance(c: usvg::Color) -> f64 {
    let ch = |v: u8| {
        let s = v as f64 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(c.red) + 0.7152 * ch(c.green) + 0.0722 * ch(c.blue)
}

pub fn contrast(a: usvg::Color, b: usvg::Color) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

pub fn hex(c: &str) -> usvg::Color {
    let v = u32::from_str_radix(c.trim_start_matches('#'), 16).unwrap_or(0);
    usvg::Color::new_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn short(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 30 {
        format!("“{}…”", s.chars().take(28).collect::<String>())
    } else {
        format!("“{s}”")
    }
}

fn overlap(a: &Rect, b: &Rect) -> f64 {
    let w = (a.right().min(b.right()) - a.left().max(b.left())).max(0.0);
    let h = (a.bottom().min(b.bottom()) - a.top().max(b.top())).max(0.0);
    (w * h) as f64
}

/// Checks 3 to 5, on the tree as the renderer will draw it.
pub fn layout(tree: &usvg::Tree) -> Result<(), Vec<Problem>> {
    let size = tree.size();
    let (w, h) = (size.width() as f64, size.height() as f64);
    let shown = display_scale(w, h);
    let mut shapes = Vec::new();
    let mut labels = Vec::new();
    let mut drawn = 0;
    walk(tree.root(), &mut shapes, &mut labels, &mut drawn);
    let mut out = Vec::new();

    let outside = |r: &Rect| {
        (r.left() as f64) < -1.0
            || (r.top() as f64) < -1.0
            || (r.right() as f64) > w + 1.0
            || (r.bottom() as f64) > h + 1.0
    };
    for node in tree.root().children() {
        let r = node.abs_stroke_bounding_box();
        // Texts are checked with the rest of the text below.
        if matches!(node, Node::Text(_)) {
            continue;
        }
        if (r.width() > 0.0 || r.height() > 0.0) && outside(&r) {
            let what = if node.id().is_empty() {
                "an element".to_string()
            } else {
                format!("#{}", node.id())
            };
            out.push(problem(format!(
                "{what} runs off the canvas (it spans {:.0},{:.0} to {:.0},{:.0}; the canvas is {w:.0} × {h:.0})",
                r.left(),
                r.top(),
                r.right(),
                r.bottom()
            )));
        }
    }

    let paper = hex(palette::PAPER);
    let (missing, labels): (Vec<_>, Vec<_>) = labels
        .into_iter()
        .partition(|l| l.text.starts_with('\u{0}'));
    for m in missing {
        out.push(problem(format!(
            "the fonts have no “{}”; use another character",
            m.text.trim_start_matches('\u{0}')
        )));
    }
    for (i, l) in labels.iter().enumerate() {
        if outside(&l.rect) {
            out.push(problem(format!("{} runs off the canvas", short(&l.text))));
        }
        let pts = l.size * shown;
        if pts < MIN_TEXT - 0.01 {
            out.push(problem(format!(
                "{} is {pts:.1} points when shown; at least {MIN_TEXT}",
                short(&l.text)
            )));
        }
        let back = l.behind.map(|b| shapes[b].colour).unwrap_or(paper);
        if let Some(c) = l.colour {
            let k = contrast(c, back);
            if k < MIN_CONTRAST {
                out.push(problem(format!(
                    "{} has a contrast of {k:.1}:1 against what is behind it; at least {MIN_CONTRAST}:1",
                    short(&l.text)
                )));
            }
        }
        if let Some(b) = l.behind {
            let s = &shapes[b].rect;
            if l.rect.left() < s.left() - 1.0
                || l.rect.right() > s.right() + 1.0
                || l.rect.top() < s.top() - 1.0
                || l.rect.bottom() > s.bottom() + 1.0
            {
                out.push(problem(format!(
                    "{} runs out of the shape it sits on",
                    short(&l.text)
                )));
            }
        }
        for m in &labels[i + 1..] {
            let o = overlap(&l.rect, &m.rect);
            let smaller =
                ((l.rect.width() * l.rect.height()).min(m.rect.width() * m.rect.height())) as f64;
            if o > 1.0 && o > smaller * 0.02 {
                out.push(problem(format!(
                    "{} and {} overlap",
                    short(&l.text),
                    short(&m.text)
                )));
            }
        }
    }
    if drawn == 0 {
        out.push(problem("nothing is drawn"));
    } else if labels.is_empty() {
        out.push(problem("a diagram needs some text"));
    }
    if out.is_empty() { Ok(()) } else { Err(out) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render;

    fn svg(body: &str) -> String {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 200">{body}</svg>"#)
    }

    fn problems(s: &str) -> Vec<String> {
        match render::render(s) {
            Ok(_) => vec![],
            Err(p) => p.into_iter().map(|p| p.what).collect(),
        }
    }

    fn refused(s: &str, word: &str) {
        let p = problems(s);
        assert!(
            p.iter().any(|p| p.contains(word)),
            "expected “{word}” in {p:?}"
        );
    }

    const GOOD: &str = r##"<rect x="20" y="20" width="160" height="60" rx="4" fill="#e8edf5" stroke="#8a92a3"/>
        <text x="100" y="56" font-size="14" text-anchor="middle" fill="#1d2433">S-CPU</text>
        <text x="220" y="56" font-size="14" font-family="Menlo" fill="#1d2433">$2100</text>"##;

    #[test]
    fn a_good_picture_passes() {
        assert_eq!(problems(&svg(GOOD)), Vec::<String>::new());
    }

    #[test]
    fn size_and_structure() {
        refused(&svg(&" ".repeat(MAX_BYTES)), "KB");
        refused(
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><text",
            "well-formed",
        );
        refused(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>x</text></svg>"#,
            "viewBox",
        );
        refused(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4000 200"><text>x</text></svg>"#,
            "between 50",
        );
        refused(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 100"><text>x</text></svg>"#,
            "aspect",
        );
        refused(
            r#"<!DOCTYPE svg [<!ENTITY a "b">]><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 200"/>"#,
            "DOCTYPE",
        );
    }

    #[test]
    fn nothing_unsafe() {
        refused(&svg("<script>alert(1)</script>"), "<script>");
        refused(
            &svg("<foreignObject><div/></foreignObject>"),
            "<foreignObject>",
        );
        refused(
            &svg(r#"<image href="https://example.com/a.png"/>"#),
            "<image>",
        );
        refused(
            &svg(r#"<rect width="9" height="9" onclick="x()"/>"#),
            "onclick",
        );
        refused(
            &svg(r#"<use href="https://example.com/a.svg#x"/>"#),
            "local",
        );
        refused(
            &svg(r#"<rect width="9" height="9" fill="url(https://e.com/p)"/>"#),
            "local",
        );
        refused(
            &svg(r#"<style>@import url(https://e.com/a.css);</style>"#),
            "import",
        );
        refused(&svg(r#"<animate attributeName="x"/>"#), "<animate>");
    }

    #[test]
    fn on_the_canvas() {
        refused(
            &svg(&format!(
                r##"{GOOD}<rect x="300" y="150" width="200" height="30" fill="#e8edf5"/>"##
            )),
            "off the canvas",
        );
        refused(
            &svg(r##"<text x="360" y="60" font-size="14" fill="#1d2433">Off the edge</text>"##),
            "off the canvas",
        );
    }

    #[test]
    fn text_is_readable() {
        refused(
            &svg(
                r##"<text x="20" y="60" font-size="14" fill="#1d2433">One label</text><text x="40" y="62" font-size="14" fill="#1d2433">Another</text>"##,
            ),
            "overlap",
        );
        refused(
            &svg(r##"<text x="20" y="60" font-size="8" fill="#1d2433">Tiny</text>"##),
            "points",
        );
        refused(
            &svg(r##"<text x="20" y="60" font-size="14" fill="#eeeeee">Faint</text>"##),
            "contrast",
        );
        refused(
            &svg(
                r##"<rect x="20" y="40" width="200" height="40" fill="#1d2433"/><text x="30" y="66" font-size="14" fill="#1d2433">Ink on ink</text>"##,
            ),
            "contrast",
        );
        refused(
            &svg(
                r##"<rect x="20" y="40" width="60" height="40" fill="#e8edf5"/><text x="24" y="66" font-size="14" fill="#1d2433">Much too long for it</text>"##,
            ),
            "runs out",
        );
        refused(
            &svg(r##"<text x="20" y="60" font-size="14" fill="#1d2433">Snowman ☃</text>"##),
            "no “☃”",
        );
        // A wide picture is shown smaller, so its text must be larger.
        let wide = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1440 600"><text x="20" y="60" font-size="14" fill="#1d2433">Shown at half</text></svg>"##;
        refused(wide, "points");
    }

    #[test]
    fn not_empty() {
        refused(&svg(""), "nothing is drawn");
        refused(
            &svg(r##"<rect x="9" y="9" width="90" height="40" fill="#e8edf5"/>"##),
            "needs some text",
        );
    }

    #[test]
    fn contrast_is_measured_as_the_web_does() {
        assert!((contrast(hex("#000000"), hex("#ffffff")) - 21.0).abs() < 0.01);
        assert!(contrast(hex(palette::INK), hex(palette::PAPER)) > 12.0);
        assert!(contrast(hex(palette::MUTED), hex(palette::PAPER)) > 4.5);
        assert!(contrast(hex(palette::ACCENT), hex(palette::ACCENT_FILL)) > 4.5);
        for f in palette::FILLS {
            assert!(contrast(hex(palette::INK), hex(f)) > 10.0, "{f}");
            assert!(contrast(hex(palette::MUTED), hex(f)) > 4.0, "{f}");
        }
    }
}
