//! A small SVG writer for Romlens's own diagrams: boxes, text, lines and
//! arrows in one palette, chosen for contrast on the paper every diagram is
//! drawn on. One user unit is one point at display size.

use std::fmt::Write;

use crate::fonts::Font;

/// The colours of every diagram. The paper is filled in when the picture is
/// drawn (`render`), so an SVG never needs its own background.
pub mod palette {
    pub const PAPER: &str = "#fbfaf7";
    pub const INK: &str = "#1d2433";
    pub const MUTED: &str = "#5b6474";
    pub const LINE: &str = "#8a92a3";
    pub const ACCENT: &str = "#1f5fc4";
    pub const ACCENT_FILL: &str = "#dce8fb";
    /// Fills for regions and boxes, light enough for ink on top.
    pub const FILLS: [&str; 6] = [
        "#e8edf5", "#e6f2e8", "#fbeede", "#efe7f6", "#f6e3e3", "#eef0e0",
    ];
    pub const QUIET_FILL: &str = "#f0eee9";
    /// Text on the accent.
    pub const ON_ACCENT: &str = "#ffffff";
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

/// How a text is set.
#[derive(Clone, Copy, Debug)]
pub struct Text {
    pub size: f64,
    pub font: Font,
    pub colour: &'static str,
    pub anchor: Anchor,
}

impl Text {
    pub const fn new(size: f64, font: Font) -> Text {
        Text {
            size,
            font,
            colour: palette::INK,
            anchor: Anchor::Start,
        }
    }
    pub const fn colour(self, colour: &'static str) -> Text {
        Text { colour, ..self }
    }
    pub const fn anchor(self, anchor: Anchor) -> Text {
        Text { anchor, ..self }
    }
    pub const fn middle(self) -> Text {
        self.anchor(Anchor::Middle)
    }
    /// The baseline for text centred vertically on `y`.
    pub fn centred(&self, y: f64) -> f64 {
        y + self.size * 0.36
    }
}

/// A box's fill and outline.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub fill: &'static str,
    pub stroke: &'static str,
    pub width: f64,
    pub radius: f64,
}

impl Shape {
    pub const fn new(fill: &'static str) -> Shape {
        Shape {
            fill,
            stroke: palette::LINE,
            width: 1.0,
            radius: 4.0,
        }
    }
    pub const fn stroke(self, stroke: &'static str, width: f64) -> Shape {
        Shape {
            stroke,
            width,
            ..self
        }
    }
    pub const fn radius(self, radius: f64) -> Shape {
        Shape { radius, ..self }
    }
}

pub struct Svg {
    width: f64,
    height: f64,
    body: String,
}

fn n(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

impl Svg {
    pub fn new(width: f64, height: f64) -> Svg {
        Svg {
            width,
            height,
            body: String::new(),
        }
    }

    pub fn width(&self) -> f64 {
        self.width
    }

    pub fn height(&self) -> f64 {
        self.height
    }

    /// Makes the canvas taller, for something added at the bottom.
    pub fn grow(&mut self, by: f64) {
        self.height += by;
    }

    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, s: Shape) {
        let _ = writeln!(
            self.body,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="{}" stroke="{}" stroke-width="{}"/>"#,
            n(x),
            n(y),
            n(w),
            n(h),
            n(s.radius),
            s.fill,
            s.stroke,
            n(s.width)
        );
    }

    pub fn text(&mut self, x: f64, y: f64, s: &str, t: Text) {
        let anchor = match t.anchor {
            Anchor::Start => "",
            Anchor::Middle => r#" text-anchor="middle""#,
            Anchor::End => r#" text-anchor="end""#,
        };
        let _ = writeln!(
            self.body,
            r#"<text x="{}" y="{}" font-family="{}" font-weight="{}" font-size="{}" fill="{}"{anchor}>{}</text>"#,
            n(x),
            n(y),
            t.font.family(),
            t.font.weight(),
            n(t.size),
            t.colour,
            escape(s)
        );
    }

    pub fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, colour: &str, width: f64) {
        let _ = writeln!(
            self.body,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{colour}" stroke-width="{}"/>"#,
            n(x1),
            n(y1),
            n(x2),
            n(y2),
            n(width)
        );
    }

    pub fn dashed(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, colour: &str) {
        let _ = writeln!(
            self.body,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{colour}" stroke-width="1" stroke-dasharray="4 3"/>"#,
            n(x1),
            n(y1),
            n(x2),
            n(y2)
        );
    }

    pub fn circle(&mut self, cx: f64, cy: f64, r: f64, fill: &str) {
        let _ = writeln!(
            self.body,
            r#"<circle cx="{}" cy="{}" r="{}" fill="{fill}"/>"#,
            n(cx),
            n(cy),
            n(r)
        );
    }

    /// A polyline without an arrowhead.
    pub fn arrow_less(&mut self, points: &[(f64, f64)], colour: &str, width: f64) {
        if points.len() < 2 {
            return;
        }
        let d: Vec<String> = points
            .iter()
            .enumerate()
            .map(|(i, (x, y))| format!("{}{} {}", if i == 0 { "M" } else { "L" }, n(*x), n(*y)))
            .collect();
        let _ = writeln!(
            self.body,
            r#"<path d="{}" fill="none" stroke="{colour}" stroke-width="{}" stroke-linejoin="round"/>"#,
            d.join(" "),
            n(width)
        );
    }

    /// A polyline with an arrowhead at its end, and at its start too when
    /// `both`.
    pub fn arrow(&mut self, points: &[(f64, f64)], colour: &str, width: f64, both: bool) {
        if points.len() < 2 {
            return;
        }
        let d: Vec<String> = points
            .iter()
            .enumerate()
            .map(|(i, (x, y))| format!("{}{} {}", if i == 0 { "M" } else { "L" }, n(*x), n(*y)))
            .collect();
        let _ = writeln!(
            self.body,
            r#"<path d="{}" fill="none" stroke="{colour}" stroke-width="{}" stroke-linejoin="round"/>"#,
            d.join(" "),
            n(width)
        );
        let head = |body: &mut String, tip: (f64, f64), from: (f64, f64)| {
            let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
            let len = (dx * dx + dy * dy).sqrt().max(0.001);
            let (ux, uy) = (dx / len, dy / len);
            let size = 5.0 + width * 2.0;
            let back = (tip.0 - ux * size, tip.1 - uy * size);
            let (px, py) = (-uy * size * 0.5, ux * size * 0.5);
            let _ = writeln!(
                body,
                r#"<path d="M{} {} L{} {} L{} {} Z" fill="{colour}"/>"#,
                n(tip.0),
                n(tip.1),
                n(back.0 + px),
                n(back.1 + py),
                n(back.0 - px),
                n(back.1 - py)
            );
        };
        let k = points.len();
        head(&mut self.body, points[k - 1], points[k - 2]);
        if both {
            head(&mut self.body, points[0], points[1]);
        }
    }

    /// The SVG. A short, wide drawing gets room below it, so its aspect
    /// stays within the checks' 3:1.
    pub fn finish(self) -> String {
        let height = self.height.max(self.width / 3.0).ceil();
        let (width, body) = (self.width, self.body);
        Svg {
            width,
            height,
            body,
        }
        .write()
    }

    fn write(self) -> String {
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\">\n{}</svg>\n",
            self.body,
            w = n(self.width),
            h = n(self.height)
        )
    }
}
