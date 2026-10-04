//! Each voice's wave and the two outputs, while something plays (docs/23).
//! The macOS twin is `ScopeView` and `ScopeTrace`.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::cairo;

use crate::canvas::with_alpha;
use crate::gfxdraw::*;
use crate::model::Document;

const TRACE_HEIGHT: f64 = 90.0;
const GAP: f64 = 8.0;
const MIN_WIDTH: f64 = 260.0;

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let none = adw::StatusPage::builder()
        .icon_name("audio-volume-muted-symbolic")
        .title("Nothing Playing")
        .description(
            "Press Play (Space) to hear the driver: each voice's wave after its envelope, and \
             the mix left and right, are drawn here as they play.",
        )
        .build();
    let area = gtk::DrawingArea::builder().hexpand(true).build();
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&area)
        .build();
    let stack = gtk::Stack::new();
    stack.add_named(&scroll, Some("scope"));
    stack.add_named(&none, Some("none"));

    area.set_draw_func({
        let doc = Rc::clone(doc);
        move |a, cr, w, _| draw(&doc, a, cr, f64::from(w))
    });
    area.connect_resize(|a, w, _| {
        let (_, h) = layout(f64::from(w));
        a.set_content_height(h as i32);
    });
    // Thirty times a second while it is on screen, and only then.
    let last = Cell::new(0i64);
    area.add_tick_callback({
        let (doc, stack) = (Rc::clone(doc), stack.clone());
        move |a, clock| {
            let playing = doc.audio().playing().is_some();
            stack.set_visible_child_name(if playing { "scope" } else { "none" });
            let now = clock.frame_time();
            if playing && a.is_mapped() && now - last.get() >= 33_000 {
                last.set(now);
                a.queue_draw();
            }
            gtk::glib::ControlFlow::Continue
        }
    });
    stack.upcast()
}

/// Columns, and the height of the whole: the two outputs on top, then the
/// eight voices in a grid of at least `MIN_WIDTH` columns.
fn layout(width: f64) -> (usize, f64) {
    let cols = (((width - GAP) / (MIN_WIDTH + GAP)).floor() as usize).clamp(1, 8);
    let rows = 8usize.div_ceil(cols);
    (
        cols,
        GAP + TRACE_HEIGHT + GAP + rows as f64 * (TRACE_HEIGHT + GAP),
    )
}

fn draw(doc: &Rc<Document>, a: &gtk::DrawingArea, cr: &cairo::Context, width: f64) {
    let audio = doc.audio();
    let muted = audio.muted();
    let half = (width - GAP * 3.0) / 2.0;
    trace(a, cr, GAP, GAP, half, "Left", &audio.scope(8, 1024));
    trace(
        a,
        cr,
        GAP * 2.0 + half,
        GAP,
        half,
        "Right",
        &audio.scope(9, 1024),
    );
    let (cols, _) = layout(width);
    let w = (width - GAP * (cols as f64 + 1.0)) / cols as f64;
    for v in 0..8usize {
        let (col, row) = (v % cols, v / cols);
        let title = if muted & (1 << v) != 0 {
            format!("Voice {v} (muted)")
        } else {
            format!("Voice {v}")
        };
        trace(
            a,
            cr,
            GAP + col as f64 * (w + GAP),
            GAP + TRACE_HEIGHT + GAP + row as f64 * (TRACE_HEIGHT + GAP),
            w,
            &title,
            &audio.scope(v as u8, 512),
        );
    }
}

fn trace(
    a: &gtk::DrawingArea,
    cr: &cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    title: &str,
    samples: &[i16],
) {
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.85);
    cr.rectangle(x, y, w, TRACE_HEIGHT);
    let _ = cr.fill();
    let mid = y + TRACE_HEIGHT / 2.0;
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.25);
    cr.set_line_width(0.5);
    cr.move_to(x, mid);
    cr.line_to(x + w, mid);
    let _ = cr.stroke();
    if samples.len() > 1 {
        cr.set_source_rgb(0.3, 0.9, 0.4);
        cr.set_line_width(1.0);
        let n = (samples.len() - 1) as f64;
        for (i, v) in samples.iter().enumerate() {
            let (px, py) = (
                x + i as f64 / n * w,
                mid - f64::from(*v) / 32768.0 * (TRACE_HEIGHT / 2.0),
            );
            if i == 0 {
                cr.move_to(px, py);
            } else {
                cr.line_to(px, py);
            }
        }
        let _ = cr.stroke();
    }
    let l = mono_layout(a, 8.5, title);
    text(
        cr,
        &l,
        x + 4.0,
        y + 3.0,
        &with_alpha(&gtk::gdk::RGBA::WHITE, 0.7),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_voices_wrap_into_columns_as_the_width_allows() {
        assert_eq!(layout(300.0).0, 1);
        assert_eq!(layout(600.0).0, 2);
        assert_eq!(layout(1100.0).0, 4);
        assert_eq!(layout(5000.0).0, 8, "never more than the eight voices");
        let (_, narrow) = layout(300.0);
        let (_, wide) = layout(1100.0);
        assert!(narrow > wide, "fewer columns, more rows");
    }
}
