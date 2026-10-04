//! The hex canvas: one drawing area that paints only the visible rows from
//! cached 256-row batches. Mirrors the macOS `HexTableView` and
//! `HexRowPainter`.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{cairo, gdk};

use crate::actions;
use crate::canvas::{self, Metrics, VScroll, set_source, with_alpha};
use crate::hex::{BYTES_PER_ROW, HexLayout, HexRow};
use crate::model::{Change, Document, EditorSource};
use crate::palette;

struct State {
    doc: Rc<Document>,
    area: gtk::DrawingArea,
    scroll: VScroll,
    metrics: Cell<Metrics>,
}

/// Build the hex view for a document: the canvas and its scrollbar.
/// The hex canvas with its scrollbar, and what the Both tab needs to line it
/// up with the listing.
pub struct HexPane {
    pub widget: gtk::Widget,
    pub area: gtk::DrawingArea,
    pub scroll: VScroll,
    state: Rc<State>,
}

impl HexPane {
    pub fn row_height(&self) -> f64 {
        self.state.metrics.get().row_height
    }

    /// The x, in the canvas, of the right edge of the last hex byte.
    pub fn hex_right_edge(&self) -> f64 {
        let m = self.state.metrics.get();
        let layout = self.state.layout();
        HexLayout::LEFT_PADDING
            + (layout.hex_column(BYTES_PER_ROW - 1) + 2) as f64 * m.char_width
            + 1.0
    }
}

pub fn build(doc: &Rc<Document>) -> HexPane {
    let area = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .focusable(true)
        .build();
    area.add_css_class("romlens-hex");
    let metrics = Metrics::measure(&area);
    let scroll = VScroll::new(doc.row_count(), metrics.row_height);
    let state = Rc::new(State {
        doc: Rc::clone(doc),
        area: area.clone(),
        scroll: scroll.clone(),
        metrics: Cell::new(metrics),
    });

    let s = Rc::clone(&state);
    area.set_draw_func(move |_, cr, w, h| s.draw(cr, f64::from(w), f64::from(h)));
    let s = Rc::clone(&state);
    area.connect_resize(move |_, _, h| {
        s.scroll.set_rows(s.doc.row_count(), f64::from(h));
        s.follow_scroll();
    });
    let s = Rc::clone(&state);
    area.connect_map(move |_| s.follow_scroll());
    scroll.adjustment.connect_value_changed({
        let area = area.clone();
        move |_| area.queue_draw()
    });
    scroll.attach_wheel(&area, None);

    let click = gtk::GestureClick::builder()
        .button(gdk::BUTTON_PRIMARY)
        .build();
    let s = Rc::clone(&state);
    click.connect_pressed(move |g, _, x, y| {
        s.area.grab_focus();
        let extend = g
            .current_event_state()
            .contains(gdk::ModifierType::SHIFT_MASK);
        s.click_at(x, y, extend);
    });
    area.add_controller(click);

    let s = Rc::clone(&state);
    actions::attach_context_menu(&area, doc, move |x, y| s.select_for_menu(x, y));

    let s = Rc::clone(&state);
    canvas::attach_keys(&area, move |command| {
        s.doc
            .perform(command, EditorSource::Hex, s.scroll.visible_rows());
    });

    let s = Rc::clone(&state);
    doc.subscribe(move |change| match change {
        Change::Rows | Change::AddressStyle | Change::Selection => s.area.queue_draw(),
        Change::Scroll => s.follow_scroll(),
        _ => {}
    });

    // Re-measure when the system font or scale changes.
    let s = Rc::clone(&state);
    area.connect_notify_local(Some("scale-factor"), move |_, _| {
        let m = Metrics::measure(&s.area);
        s.metrics.set(m);
        s.scroll.set_row_height(m.row_height);
        s.area.queue_draw();
    });

    let bar = gtk::Scrollbar::new(gtk::Orientation::Vertical, Some(&scroll.adjustment));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.append(&area);
    row.append(&bar);
    HexPane {
        widget: row.upcast(),
        area,
        scroll,
        state,
    }
}

impl State {
    fn layout(&self) -> HexLayout {
        HexLayout {
            style: self.doc.address_style(),
        }
    }

    fn follow_scroll(&self) {
        self.scroll
            .follow(self.doc.scroll_request(), None, &self.area, |offset| {
                Some(offset / BYTES_PER_ROW as u32)
            });
    }

    fn row(&self, row: u32) -> Option<HexRow> {
        self.doc.hex_cache.batch(row)?.row(row).cloned()
    }

    fn draw(&self, cr: &cairo::Context, _width: f64, height: f64) {
        let m = self.metrics.get();
        let fg = self.area.color();
        let accent = adw::StyleManager::default().accent_color_rgba();
        let focused = self.area.has_focus();
        let layout = self.layout();
        let (cw, rh) = (m.char_width, m.row_height);
        let pad = HexLayout::LEFT_PADDING;
        let selected = self.doc.selected();
        let highlighted = self.doc.highlighted_range();

        let top = self.scroll.top();
        let first = top.floor().max(0.0) as u32;
        let mut y = -(top - top.floor()) * rh;
        let pango_layout = self.area.create_pango_layout(None);
        pango_layout.set_font_description(Some(&canvas::font()));

        let hex_x = |byte: usize| pad + layout.hex_column(byte) as f64 * cw - 1.0;
        let ascii_x = |byte: usize| pad + layout.ascii_column(byte) as f64 * cw;
        // A rectangle over bytes `from..=to` in both the hex and ASCII areas.
        let fill_bytes = |from: usize, to: usize, y: f64, inset: f64| {
            cr.rectangle(
                hex_x(from),
                y + inset,
                hex_x(to) + cw * 2.0 + 2.0 - hex_x(from),
                rh - 2.0 * inset,
            );
            let _ = cr.fill();
            cr.rectangle(
                ascii_x(from),
                y + inset,
                ascii_x(to) + cw - ascii_x(from),
                rh - 2.0 * inset,
            );
            let _ = cr.fill();
        };

        let mut row_index = first;
        while y < height && row_index < self.doc.row_count() {
            let Some(row) = self.row(row_index) else {
                break;
            };

            // Region lane and header spans share the span-id lane.
            let mut i = 0;
            while i < row.byte_count {
                let id = row.span_ids[i];
                let mut j = i + 1;
                while j < row.byte_count && row.span_ids[j] == id {
                    j += 1;
                }
                if id != 0
                    && let Some(tint) = palette::span_tint(&self.doc, id)
                {
                    set_source(cr, &tint);
                    fill_bytes(i, j - 1, y, 2.0);
                }
                i = j;
            }

            // The highlighted range: the instruction's bytes, or the
            // shift-extended range.
            if let Some(range) = &highlighted {
                let lo = range.start.max(row.file_offset);
                let hi = range.end.min(row.file_offset + row.byte_count as u32);
                if lo < hi {
                    set_source(cr, &with_alpha(&accent, if focused { 0.3 } else { 0.15 }));
                    fill_bytes(
                        (lo - row.file_offset) as usize,
                        (hi - 1 - row.file_offset) as usize,
                        y,
                        1.0,
                    );
                }
            }

            if let Some(sel) = selected
                && sel >= row.file_offset
                && ((sel - row.file_offset) as usize) < row.byte_count
            {
                let b = (sel - row.file_offset) as usize;
                set_source(cr, &accent);
                cr.set_line_width(1.0);
                cr.rectangle(hex_x(b) + 0.5, y + 1.5, cw * 2.0 + 1.0, rh - 3.0);
                let _ = cr.stroke();
            }

            pango_layout.set_text(&layout.text(&row));
            set_source(cr, &fg);
            cr.move_to(pad, m.text_top(y));
            pangocairo::functions::show_layout(cr, &pango_layout);

            y += rh;
            row_index += 1;
            if row.is_last_row() {
                break;
            }
        }
    }

    /// The byte under a point, with its row.
    fn byte_at(&self, x: f64, y: f64) -> Option<u32> {
        let m = self.metrics.get();
        let row = (self.scroll.top() + y / m.row_height).floor();
        let col = ((x - HexLayout::LEFT_PADDING) / m.char_width).floor();
        if row < 0.0 || col < 0.0 {
            return None;
        }
        let rec = self.row(row as u32)?;
        let byte = self.layout().byte_at_column(col as usize)?;
        (byte < rec.byte_count).then(|| rec.file_offset + byte as u32)
    }

    fn click_at(&self, x: f64, y: f64, extend: bool) {
        if let Some(offset) = self.byte_at(x, y) {
            if extend {
                self.doc.extend_selection(offset);
            } else {
                self.doc.select(Some(offset));
            }
        }
    }

    /// Right-click: select what is under the pointer unless it is already in
    /// the highlighted range, so the menu acts on what was clicked.
    fn select_for_menu(&self, x: f64, y: f64) {
        if let Some(offset) = self.byte_at(x, y)
            && !self
                .doc
                .highlighted_range()
                .is_some_and(|r| r.contains(&offset))
        {
            self.doc.select(Some(offset));
        }
    }
}
