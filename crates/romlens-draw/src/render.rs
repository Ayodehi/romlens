//! Drawing an SVG: the checks, then `resvg` onto the paper, as a PNG at
//! twice the size it is shown so it is sharp on a Retina screen.

use resvg::tiny_skia;
use resvg::usvg;

use crate::check::{self, Problem};
use crate::fonts;
use crate::svg::palette;

/// A 64-bit FNV-1a hash, for naming pictures.
pub fn hash(bytes: &str) -> u64 {
    bytes.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A drawn picture. Its PNG is twice its display size in points.
#[derive(Clone, Debug)]
pub struct Picture {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub svg: String,
}

/// Checks the SVG and draws it, or says everything that is wrong with it.
pub fn render(svg: &str) -> Result<Picture, Vec<Problem>> {
    check::structure(svg)?;
    let tree = usvg::Tree::from_str(svg, &fonts::options()).map_err(|e| {
        vec![Problem {
            what: format!("the SVG could not be read: {e}"),
        }]
    })?;
    check::layout(&tree)?;
    let size = tree.size();
    let scale = 2.0 * check::display_scale(size.width() as f64) as f32;
    let (w, h) = (
        (size.width() * scale).ceil() as u32,
        (size.height() * scale).ceil() as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| {
        vec![Problem {
            what: "the picture has no area".to_string(),
        }]
    })?;
    let paper = check::hex(palette::PAPER);
    pixmap.fill(tiny_skia::Color::from_rgba8(
        paper.red,
        paper.green,
        paper.blue,
        255,
    ));
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let png = pixmap.encode_png().map_err(|e| {
        vec![Problem {
            what: format!("the picture could not be encoded: {e}"),
        }]
    })?;
    Ok(Picture {
        png,
        width: w,
        height: h,
        svg: svg.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_a_png_twice_its_size() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 200"><text x="20" y="60" font-size="14" fill="#1d2433">Hello</text></svg>"##;
        let p = render(svg).unwrap();
        assert_eq!((p.width, p.height), (800, 400));
        assert_eq!(&p.png[..8], b"\x89PNG\r\n\x1a\n");
        // Real deflate: a mostly empty picture is small.
        assert!(p.png.len() < 40_000, "{}", p.png.len());
        // The same picture every time.
        assert_eq!(render(svg).unwrap().png, p.png);
    }

    #[test]
    fn a_large_picture_is_shown_smaller() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1440 540"><text x="20" y="60" font-size="28" fill="#1d2433">Hello</text></svg>"##;
        let p = render(svg).unwrap();
        assert_eq!((p.width, p.height), (1440, 540));
    }
}
