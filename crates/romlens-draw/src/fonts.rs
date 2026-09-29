//! The fonts every diagram is measured and drawn with, shipped with Romlens
//! (IBM Plex Sans and Mono, SIL Open Font License 1.1, `fonts/OFL.txt`) so
//! the layout and the pixels are the same on every machine. Nothing is read
//! from the system: any family an SVG names becomes sans or mono.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use resvg::usvg::fontdb::{Database, Family, Query, Weight};
use resvg::usvg::{self, FontFamily, FontResolver};

static SANS: &[u8] = include_bytes!("../fonts/IBMPlexSans-Regular.ttf");
static SANS_BOLD: &[u8] = include_bytes!("../fonts/IBMPlexSans-SemiBold.ttf");
static MONO: &[u8] = include_bytes!("../fonts/IBMPlexMono-Regular.ttf");

pub const SANS_FAMILY: &str = "IBM Plex Sans";
pub const MONO_FAMILY: &str = "IBM Plex Mono";

/// The three faces a diagram uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    Sans,
    Bold,
    Mono,
}

impl Font {
    pub fn family(self) -> &'static str {
        match self {
            Font::Sans | Font::Bold => SANS_FAMILY,
            Font::Mono => MONO_FAMILY,
        }
    }

    pub fn weight(self) -> u16 {
        match self {
            Font::Bold => 600,
            _ => 400,
        }
    }
}

pub fn database() -> Arc<Database> {
    static DB: OnceLock<Arc<Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = Database::new();
        for data in [SANS, SANS_BOLD, MONO] {
            db.load_font_data(data.to_vec());
        }
        db.set_sans_serif_family(SANS_FAMILY);
        db.set_serif_family(SANS_FAMILY);
        db.set_monospace_family(MONO_FAMILY);
        Arc::new(db)
    })
    .clone()
}

/// Whether a family an SVG names means the mono face.
fn is_mono(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["mono", "courier", "consol", "menlo", "code"]
        .iter()
        .any(|w| n.contains(w))
}

/// Parsing options: our fonts only, every family mapped to one of them, and
/// no fallback to fonts that are not there.
pub fn options() -> usvg::Options<'static> {
    usvg::Options {
        font_family: SANS_FAMILY.to_string(),
        fontdb: database(),
        font_resolver: FontResolver {
            select_font: Box::new(|font, db| {
                let mono = font.families().iter().any(|f| match f {
                    FontFamily::Monospace => true,
                    FontFamily::Named(n) => is_mono(n),
                    _ => false,
                });
                let (family, weight) = if mono {
                    (MONO_FAMILY, 400)
                } else if font.weight() >= 550 {
                    (SANS_FAMILY, 600)
                } else {
                    (SANS_FAMILY, 400)
                };
                db.query(&Query {
                    families: &[Family::Name(family)],
                    weight: Weight(weight),
                    ..Default::default()
                })
            }),
            select_fallback: Box::new(|_, _, _| None),
        },
        ..Default::default()
    }
}

/// Whether the fonts can draw a character.
pub fn has_char(c: char, mono: bool) -> bool {
    use skrifa::MetadataProvider;
    let data = if mono { MONO } else { SANS };
    skrifa::FontRef::new(data)
        .map(|f| f.charmap().map(c).is_some_and(|g| g.to_u32() != 0))
        .unwrap_or(false)
}

/// The width and height of a line of text: its advance and the font's line
/// box, as the renderer lays it out.
pub fn measure(text: &str, font: Font, size: f64) -> (f64, f64) {
    type Cache = Mutex<HashMap<(String, Font, u64), (f64, f64)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let key = (text.to_string(), font, size.to_bits());
    let cache = CACHE.get_or_init(Default::default);
    if let Some(m) = cache.lock().ok().and_then(|c| c.get(&key).copied()) {
        return m;
    }
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4000 400"><text x="0" y="200" font-family="{}" font-weight="{}" font-size="{size}">{}</text></svg>"#,
        font.family(),
        font.weight(),
        crate::svg::escape(text)
    );
    let m = usvg::Tree::from_str(&svg, &options())
        .ok()
        .and_then(|t| {
            t.root().children().iter().find_map(|n| match n {
                usvg::Node::Text(t) => {
                    let b = t.bounding_box();
                    Some((b.width() as f64, b.height() as f64))
                }
                _ => None,
            })
        })
        .unwrap_or((0.0, size * 1.3));
    if let Ok(mut c) = cache.lock() {
        c.insert(key, m);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_faces_load() {
        let db = database();
        assert_eq!(db.faces().count(), 3);
        for f in [Font::Sans, Font::Bold, Font::Mono] {
            let q = Query {
                families: &[Family::Name(f.family())],
                weight: Weight(f.weight()),
                ..Default::default()
            };
            assert!(db.query(&q).is_some(), "{f:?}");
        }
    }

    #[test]
    fn text_measures_the_same_every_time_and_grows_with_its_size() {
        let a = measure("Forced blank", Font::Sans, 14.0);
        assert_eq!(a, measure("Forced blank", Font::Sans, 14.0));
        assert!(a.0 > 50.0 && a.0 < 120.0, "{a:?}");
        let b = measure("Forced blank", Font::Sans, 28.0);
        assert!((b.0 / a.0 - 2.0).abs() < 0.05);
        assert!(measure("Forced blank", Font::Bold, 14.0).0 > a.0);
        // Mono: every character the same width.
        let (m1, _) = measure("iiii", Font::Mono, 14.0);
        let (m2, _) = measure("WWWW", Font::Mono, 14.0);
        assert!((m1 - m2).abs() < 0.01);
    }

    #[test]
    fn the_characters_diagrams_use_are_there() {
        for c in "→←↔–—·×$%0123456789ABCDEFabcdef…".chars() {
            assert!(has_char(c, false), "{c}");
        }
        assert!(has_char('$', true));
    }
}
