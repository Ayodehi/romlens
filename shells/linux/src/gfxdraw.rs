//! Drawing helpers the graphics views share.

use gtk::cairo;
use gtk::prelude::*;
use gtk::{gdk, pango};

use crate::canvas::{font, set_source, with_alpha};

pub fn hex(v: impl Into<u64>, digits: usize) -> String {
    format!("{:0digits$X}", v.into())
}

/// `0xRRGGBB` as the cairo source.
pub fn rgb(cr: &cairo::Context, c: u32) {
    cr.set_source_rgb(
        f64::from(c >> 16 & 0xFF) / 255.0,
        f64::from(c >> 8 & 0xFF) / 255.0,
        f64::from(c & 0xFF) / 255.0,
    );
}

pub fn accent() -> gdk::RGBA {
    adw::StyleManager::default().accent_color_rgba()
}

/// A layout in the monospaced font at `size` points.
pub fn mono_layout(area: &impl IsA<gtk::Widget>, size: f64, text: &str) -> pango::Layout {
    let layout = area.create_pango_layout(Some(text));
    let mut f = font();
    f.set_size((size * f64::from(pango::SCALE)) as i32);
    layout.set_font_description(Some(&f));
    layout
}

/// Text at a point, top left, in `colour`.
pub fn text(cr: &cairo::Context, layout: &pango::Layout, x: f64, y: f64, colour: &gdk::RGBA) {
    set_source(cr, colour);
    cr.move_to(x, y);
    pangocairo::functions::show_layout(cr, layout);
}

/// A rectangle outline inside `(x, y, w, h)`.
pub fn outline(
    cr: &cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    colour: &gdk::RGBA,
    width: f64,
) {
    set_source(cr, colour);
    cr.set_line_width(width);
    cr.rectangle(x + width / 2.0, y + width / 2.0, w - width, h - width);
    let _ = cr.stroke();
}

pub fn fill_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, colour: &gdk::RGBA) {
    set_source(cr, colour);
    cr.rectangle(x, y, w, h);
    let _ = cr.fill();
}

/// What transparent looks like.
pub fn checker(cr: &cairo::Context, w: f64, h: f64, fg: &gdk::RGBA) {
    let s: f64 = 8.0;
    set_source(cr, &with_alpha(fg, 0.12));
    let mut y = 0.0;
    let mut row = 0;
    while y < h {
        let mut x = 0.0;
        let mut col = 0;
        while x < w {
            if (row + col) % 2 == 0 {
                cr.rectangle(x, y, s.min(w - x), s.min(h - y));
            }
            x += s;
            col += 1;
        }
        y += s;
        row += 1;
    }
    let _ = cr.fill();
}

/// A bitmap at an integer scale, with square pixels.
pub fn paint_surface(cr: &cairo::Context, surface: &cairo::ImageSurface, scale: f64) {
    cr.save().ok();
    cr.scale(scale, scale);
    let pattern = cairo::SurfacePattern::create(surface);
    pattern.set_filter(cairo::Filter::Nearest);
    let _ = cr.set_source(&pattern);
    let _ = cr.paint();
    cr.restore().ok();
}

/// A dim, small caption.
pub fn caption(text: &str) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).build();
    l.add_css_class("caption");
    l.add_css_class("dim-label");
    l
}

/// "Changed at frame N, next at M", each a button that goes there: when the
/// selected bytes last changed at or before this frame, and next change.
pub fn history_row(
    doc: &std::rc::Rc<crate::model::Document>,
    region: romlens_ffi::StateRegion,
    offset: u32,
    len: u32,
) -> Option<gtk::Widget> {
    let h = doc.graphics().history(region, offset, len)?;
    let frame = doc.graphics().frame();
    if !h.indexed {
        let l = caption("Kept in keyframes only, so not frame by frame");
        l.set_wrap(true);
        return Some(l.upcast());
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let link = |text: &str, to: u64, enabled: bool| {
        let b = gtk::Button::with_label(text);
        b.add_css_class("flat");
        b.add_css_class("caption");
        b.set_sensitive(enabled);
        let d = std::rc::Rc::clone(doc);
        b.connect_clicked(move |_| d.set_frame(to));
        b
    };
    if let Some(last) = h.last {
        let text = if last == 0 {
            "Unchanged since frame 0".to_owned()
        } else {
            format!("Changed at frame {last}")
        };
        row.append(&link(&text, last, last != frame));
    }
    match h.next {
        Some(next) => row.append(&link(&format!("Next at frame {next}"), next, true)),
        None => row.append(&caption("No later change")),
    }
    Some(row.upcast())
}

/// The orange mark on an entry that changed since the previous frame.
pub fn change_colour() -> gtk::gdk::RGBA {
    gtk::gdk::RGBA::new(0.996, 0.584, 0.0, 1.0)
}
