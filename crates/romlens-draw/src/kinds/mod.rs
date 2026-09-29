//! The kinds of diagram Romlens draws from a spec (docs/26). Each kind that
//! shows the hardware takes its facts from the core; the spec only says
//! which register, bank or frame, and what to point at.

mod blocks;
mod chain;
mod fields;
mod memory_map;
mod timeline;

use serde_json::Value;

use romlens_core::RomImage;

use crate::fonts::{Font, measure};

pub const KINDS: &[&str] = &["fields", "memory_map", "blocks", "timeline", "chain"];

/// Where a diagram's facts come from beyond the core's tables: the ROM,
/// the project's names, and the recording.
pub trait Source {
    fn rom(&self) -> Option<&RomImage>;
    /// A 24-bit address for text the student or the tutor wrote: `$00:FFFC`,
    /// `RESET`, `SUB_0080E8`.
    fn resolve(&self, text: &str) -> Result<u32, String>;
    /// The label at an address, if it has one.
    fn name_at(&self, address: u32) -> Option<String>;
    /// A recording frame's DMA starts and register writes.
    fn frame(&self, _frame: u64) -> Result<FrameEvents, String> {
        Err("no recording is open, so there is no frame to draw; leave frame null".to_string())
    }
    /// How the pixel at `x`, `y` of a recording frame got to the screen.
    fn chain(&self, _frame: u64, _x: u32, _y: u32) -> Result<Vec<ChainStep>, String> {
        Err("no recording is open, so no pixel can be traced; give steps instead".to_string())
    }
}

/// What happened during a recording frame, in the frame's own numbering:
/// line 0 starts the picture and the vertical blank before it is negative.
#[derive(Debug, Clone, Default)]
pub struct FrameEvents {
    pub frame: u64,
    /// Lines in the frame: 262, or 312 on a PAL machine.
    pub lines: u16,
    pub events: Vec<FrameEvent>,
    /// Events left out to keep the picture readable.
    pub more: usize,
}

#[derive(Debug, Clone)]
pub struct FrameEvent {
    /// First and last line, the same for one moment.
    pub from: i32,
    pub to: i32,
    pub label: String,
    pub dma: bool,
}

#[derive(Debug, Clone)]
pub struct ChainStep {
    pub label: String,
    pub detail: Option<String>,
}

/// A drawn diagram before its checks: the SVG, a title, and what it shows
/// in words for a model that cannot see the picture.
#[derive(Debug, Clone)]
pub struct Drawing {
    pub svg: String,
    pub title: String,
    pub description: String,
}

pub fn draw(kind: &str, spec: &Value, src: &dyn Source) -> Result<Drawing, String> {
    let spec = match spec {
        Value::String(s) => {
            serde_json::from_str(s).map_err(|e| format!("the spec is not JSON: {e}"))?
        }
        v => v.clone(),
    };
    if !spec.is_object() {
        return Err("the spec must be a JSON object".to_string());
    }
    match kind {
        "fields" => fields::draw(&spec),
        "memory_map" => memory_map::draw(&spec, src),
        "blocks" => blocks::draw(&spec),
        "timeline" => timeline::draw(&spec, src),
        "chain" => chain::draw(&spec, src),
        _ => Err(format!(
            "no kind named {kind}; the kinds are {}",
            KINDS.join(", ")
        )),
    }
}

// Helpers the kinds share.

pub(crate) fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

pub(crate) fn list<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

pub(crate) fn number(v: &Value, k: &str) -> Option<i64> {
    match v.get(k)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => parse_number(s),
        _ => None,
    }
}

/// `$8F`, `0x8f`, `143`, `%10001111`.
pub(crate) fn parse_number(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('$').or_else(|| s.strip_prefix("0x")) {
        i64::from_str_radix(&h.replace('_', ""), 16).ok()
    } else if let Some(b) = s.strip_prefix('%').or_else(|| s.strip_prefix("0b")) {
        i64::from_str_radix(&b.replace('_', ""), 2).ok()
    } else {
        s.parse().ok()
    }
}

pub(crate) fn addr(a: u32) -> String {
    format!("${:02X}:{:04X}", (a >> 16) & 0xFF, a & 0xFFFF)
}

/// Words set in lines no wider than `width`.
pub(crate) fn wrap(text: &str, font: Font, size: f64, width: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && measure(&candidate, font, size).0 > width {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `text` cut to fit `width`, with an ellipsis.
pub(crate) fn fit(text: &str, font: Font, size: f64, width: f64) -> String {
    if measure(text, font, size).0 <= width {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let s: String = chars.iter().collect::<String>().trim_end().to_string() + "…";
        if measure(&s, font, size).0 <= width {
            return s;
        }
    }
    "…".to_string()
}

/// The first sentence of a register's or a region's text.
pub(crate) fn first_sentence(s: &str) -> &str {
    match s.find(". ") {
        Some(i) => &s[..=i],
        None => s,
    }
}

/// Places labels where nothing else is: each rectangle taken is kept, and
/// a new label goes to the first of its candidate spots that is free.
#[derive(Default)]
pub(crate) struct Placer {
    taken: Vec<(f64, f64, f64, f64)>,
}

impl Placer {
    pub fn take(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.taken.push((x, y, w, h));
    }

    pub fn free(&self, x: f64, y: f64, w: f64, h: f64) -> bool {
        self.taken.iter().all(|&(a, b, c, d)| {
            x + w <= a + 0.5 || a + c <= x + 0.5 || y + h <= b + 0.5 || b + d <= y + 0.5
        })
    }

    /// The first free spot, taken.
    pub fn place(&mut self, spots: &[(f64, f64)], w: f64, h: f64) -> Option<(f64, f64)> {
        let at = spots
            .iter()
            .copied()
            .find(|&(x, y)| self.free(x, y, w, h))?;
        self.take(at.0, at.1, w, h);
        Some(at)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// The fixture ROM, with a few names.
    pub struct Fixture {
        pub rom: RomImage,
    }

    impl Fixture {
        pub fn new() -> Fixture {
            let rom = RomImage::from_bytes(romlens_core::fixtures::explain_lorom(), "explain.sfc")
                .unwrap();
            Fixture { rom }
        }
    }

    impl Source for Fixture {
        fn rom(&self) -> Option<&RomImage> {
            Some(&self.rom)
        }
        fn resolve(&self, text: &str) -> Result<u32, String> {
            if text.eq_ignore_ascii_case("RESET") {
                return Ok(0x008000);
            }
            self.rom
                .resolve_any(text)
                .map(|r| r.snes_address.as_u24())
                .map_err(|e| e.to_string())
        }
        fn name_at(&self, address: u32) -> Option<String> {
            (address == 0x008000).then(|| "RESET".to_string())
        }
    }

    /// Draws, then checks: the drawing must pass the checks it holds the
    /// tutor's SVG to.
    pub fn drawn(kind: &str, spec: Value, src: &dyn Source) -> Drawing {
        let d = draw(kind, &spec, src).unwrap_or_else(|e| panic!("{kind}: {e}"));
        if let Some(dir) = std::env::var_os("ROMLENS_DRAW_OUT") {
            let name = format!("{kind}-{:08x}", crate::render::hash(&d.svg) as u32);
            let _ = std::fs::write(
                std::path::Path::new(&dir).join(format!("{name}.svg")),
                &d.svg,
            );
            if let Ok(p) = crate::render(&d.svg) {
                let _ = std::fs::write(
                    std::path::Path::new(&dir).join(format!("{name}.png")),
                    &p.png,
                );
            }
        }
        if let Err(p) = crate::render(&d.svg) {
            panic!(
                "{kind} fails its checks:\n{}\n{}",
                crate::check::describe(&p),
                d.svg
            );
        }
        d
    }

    #[test]
    fn numbers_read_as_written() {
        assert_eq!(parse_number("$8F"), Some(0x8F));
        assert_eq!(parse_number("0x8f"), Some(0x8F));
        assert_eq!(parse_number("%1000_1111"), Some(0x8F));
        assert_eq!(parse_number("143"), Some(143));
        assert_eq!(parse_number("x"), None);
    }

    #[test]
    fn a_kind_that_does_not_exist_names_those_that_do() {
        let e = draw("pie", &serde_json::json!({}), &Fixture::new()).unwrap_err();
        assert!(e.contains("memory_map"), "{e}");
        let e = draw("fields", &serde_json::json!("{nope"), &Fixture::new()).unwrap_err();
        assert!(e.contains("not JSON"), "{e}");
    }
}
