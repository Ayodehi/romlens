//! The whole-ROM overview: one column per pixel, the map at a glance. It
//! never scrolls: scrolling an overview would defeat the point of having
//! one. The macOS twin is `RegionStripNSView`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::cairo;
use gtk::prelude::*;

use crate::canvas::{set_source, with_alpha};
use crate::model::strip::{self, StripColumn};
use crate::model::{Change, Document};
use crate::palette;

struct State {
    doc: Rc<Document>,
    area: gtk::DrawingArea,
    columns: RefCell<Vec<StripColumn>>,
    /// The generation and width the columns were reduced for.
    reduced_for: Cell<(u64, i32)>,
}

pub fn build(doc: &Rc<Document>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .hexpand(true)
        .height_request(20)
        .has_tooltip(true)
        .build();
    area.add_css_class("romlens-strip");
    let state = Rc::new(State {
        doc: Rc::clone(doc),
        area: area.clone(),
        columns: RefCell::new(Vec::new()),
        reduced_for: Cell::new((u64::MAX, 0)),
    });

    let s = Rc::clone(&state);
    area.set_draw_func(move |_, cr, w, h| s.draw(cr, f64::from(w), f64::from(h)));

    // Click and drag to scrub.
    let drag = gtk::GestureDrag::new();
    let start_x = Rc::new(Cell::new(0.0));
    let s = Rc::clone(&state);
    drag.connect_drag_begin({
        let start_x = Rc::clone(&start_x);
        move |_, x, _| {
            start_x.set(x);
            s.scrub(x);
        }
    });
    let s = Rc::clone(&state);
    drag.connect_drag_update(move |_, dx, _| s.scrub(start_x.get() + dx));
    area.add_controller(drag);

    let s = Rc::clone(&state);
    area.connect_query_tooltip(move |_, x, _, _, tip| match s.describe(f64::from(x)) {
        Some(text) => {
            tip.set_text(Some(&text));
            true
        }
        None => false,
    });

    let s = Rc::clone(&state);
    doc.subscribe(move |change| {
        if matches!(change, Change::Rows | Change::Selection | Change::Layout) {
            s.area.queue_draw();
        }
    });
    let a = area.clone();
    adw::StyleManager::default().connect_accent_color_rgba_notify(move |_| a.queue_draw());
    area
}

impl State {
    /// Re-reduce when the width or the analysis changes, so one column
    /// really is one pixel.
    fn ensure_columns(&self, width: i32) {
        let key = (self.doc.generation(), width);
        if self.reduced_for.get() == key {
            return;
        }
        self.reduced_for.set(key);
        let buckets = width.max(64) as u32;
        *self.columns.borrow_mut() = strip::decode(&self.doc.workbench().region_map(buckets));
    }

    fn draw(&self, cr: &cairo::Context, width: f64, height: f64) {
        self.ensure_columns(width.round() as i32);
        let fg = self.area.color();
        let accent = adw::StyleManager::default().accent_color_rgba();
        set_source(cr, &with_alpha(&fg, 0.05));
        cr.rectangle(0.0, 0.0, width, height);
        let _ = cr.fill();

        let columns = self.columns.borrow();
        if !columns.is_empty() {
            let w = width / columns.len() as f64;
            for (i, c) in columns.iter().enumerate() {
                let x = i as f64 * w;
                let base = palette::strip_color(c.kind_code, &accent, &fg);
                // Confidence is the alpha: a guess looks like a guess.
                let alpha = if c.kind_code == 0 {
                    base.alpha()
                } else {
                    (0.35 + 0.65 * c.confidence) as f32
                };
                set_source(cr, &with_alpha(&base, alpha));
                cr.rectangle(x, 0.0, w.max(1.0), height);
                let _ = cr.fill();
                if c.mixed() {
                    hatch(cr, x, w.max(1.0), height);
                }
                if c.executed > 0.0 {
                    set_source(cr, &with_alpha(&accent, 0.9));
                    cr.rectangle(x, height - 3.0, w.max(1.0), 3.0 * c.executed);
                    let _ = cr.fill();
                }
            }
        }
        if let Some(sel) = self.doc.selected()
            && self.doc.byte_count() > 0
        {
            let x = width * f64::from(sel) / f64::from(self.doc.byte_count());
            set_source(cr, &fg);
            cr.rectangle(x - 0.5, 0.0, 1.5, height);
            let _ = cr.fill();
        }
        // A thin border, as the macOS strip has.
        set_source(cr, &with_alpha(&fg, 0.2));
        cr.set_line_width(1.0);
        cr.rectangle(0.5, 0.5, width - 1.0, height - 1.0);
        let _ = cr.stroke();
    }

    fn scrub(&self, x: f64) {
        let width = f64::from(self.area.width());
        if let Some(offset) = strip::offset_at(x, width, self.doc.byte_count()) {
            self.doc.jump_to(offset);
        }
    }

    fn describe(&self, x: f64) -> Option<String> {
        let columns = self.columns.borrow();
        let i = strip::column_at(x, f64::from(self.area.width()), columns.len())?;
        let c = columns[i];
        let share = if c.mixed() {
            format!(" ({:.0}%)", c.share * 100.0)
        } else {
            String::new()
        };
        Some(format!(
            "{}  {}{share}  {:.0}% confident  {:.1} bits/byte",
            romlens_ffi::format_file_offset(c.start),
            strip::kind_name(c.kind_code),
            c.confidence * 100.0,
            c.entropy
        ))
    }
}

/// Diagonal lines over a column that is not all one kind.
fn hatch(cr: &cairo::Context, x: f64, w: f64, h: f64) {
    let _ = cr.save();
    cr.rectangle(x, 0.0, w, h);
    cr.clip();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
    cr.set_line_width(1.0);
    let mut lx = x - h;
    while lx < x + w {
        cr.move_to(lx, h);
        cr.line_to(lx + h, 0.0);
        lx += 4.0;
    }
    let _ = cr.stroke();
    let _ = cr.restore();
}
