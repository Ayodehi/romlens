//! What the hex and disassembly canvases share: font metrics, a scrollbar
//! driven by an adjustment (so a quarter of a million rows never becomes a
//! widget that tall), wheel and touchpad handling, and the keyboard
//! controller. Each canvas keeps only its own drawing and hit-testing.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib, pango};

use crate::editor_keys;
use crate::model::workspace::Id;
use crate::model::{EditorCommand, ScrollRequest};

/// The editor font. Adwaita Mono where installed, else Source Code Pro, else
/// whatever the system calls monospace.
pub fn font() -> pango::FontDescription {
    pango::FontDescription::from_string("Adwaita Mono, Source Code Pro, Monospace 11")
}

/// Glyph metrics of the monospaced font, shared by both canvases so they use
/// the same row height.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub char_width: f64,
    pub row_height: f64,
    pub ascent: f64,
    pub descent: f64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            char_width: 8.0,
            row_height: 18.0,
            ascent: 14.0,
            descent: 4.0,
        }
    }
}

impl Metrics {
    pub fn measure(widget: &impl IsA<gtk::Widget>) -> Self {
        let ctx = widget.pango_context();
        let font = font();
        ctx.set_font_description(Some(&font));
        let metrics = ctx.metrics(Some(&font), None);
        // The advance of a run of cells, not the font's "approximate" digit
        // width, which differs from the real advance by a fraction of a
        // pixel per cell and drifts across a 78-column row.
        let probe = pango::Layout::new(&ctx);
        probe.set_text(&"0".repeat(100));
        let scale = f64::from(pango::SCALE);
        let ascent = f64::from(metrics.ascent()) / scale;
        let descent = f64::from(metrics.descent()) / scale;
        Self {
            char_width: (f64::from(probe.size().0) / scale / 100.0).max(1.0),
            row_height: (ascent + descent).ceil() + 4.0,
            ascent,
            descent,
        }
    }

    /// Top of the text for a row of this height, centring the glyphs.
    pub fn text_top(&self, row_top: f64) -> f64 {
        row_top + (self.row_height - (self.ascent + self.descent)) / 2.0
    }
}

/// Vertical scroll position in rows (fractional), shared plumbing.
#[derive(Clone)]
pub struct VScroll {
    pub adjustment: gtk::Adjustment,
    row_height: Rc<Cell<f64>>,
    /// The last scroll request this canvas acted on.
    handled: Rc<Cell<u64>>,
}

impl VScroll {
    pub fn new(rows: u32, row_height: f64) -> Self {
        Self {
            adjustment: gtk::Adjustment::new(0.0, 0.0, f64::from(rows), 1.0, 16.0, 0.0),
            row_height: Rc::new(Cell::new(row_height)),
            handled: Rc::new(Cell::new(0)),
        }
    }

    pub fn set_row_height(&self, h: f64) {
        self.row_height.set(h);
    }

    pub fn set_rows(&self, rows: u32, viewport_height: f64) {
        let page = self.page(viewport_height);
        let max = (f64::from(rows) - page).max(0.0);
        self.adjustment.configure(
            self.adjustment.value().min(max),
            0.0,
            f64::from(rows),
            1.0,
            page - 1.0,
            page,
        );
    }

    fn page(&self, viewport_height: f64) -> f64 {
        (viewport_height / self.row_height.get()).max(1.0)
    }

    pub fn top(&self) -> f64 {
        self.adjustment.value()
    }

    pub fn visible_rows(&self) -> usize {
        (self.adjustment.page_size() as usize)
            .saturating_sub(1)
            .max(1)
    }

    pub fn by(&self, rows: f64) {
        let adj = &self.adjustment;
        adj.set_value((adj.value() + rows).clamp(0.0, (adj.upper() - adj.page_size()).max(0.0)));
    }

    /// Scroll so the row sits in the middle of the visible area, as the macOS
    /// canvases do for every jump and key move.
    pub fn centre_on(&self, row: u32) {
        let adj = &self.adjustment;
        let target = f64::from(row) + 0.5 - adj.page_size() / 2.0;
        adj.set_value(target.clamp(0.0, (adj.upper() - adj.page_size()).max(0.0)));
    }

    /// Act on a request to scroll to an offset, once there is room to do it
    /// in. A canvas that is hidden or not yet sized (the Disassembly tab
    /// before its first analysis, or while another tab shows) leaves the
    /// request pending and takes it up when it appears, so a jump made
    /// elsewhere is not lost.
    ///
    /// `item` is the tab the view is in: a request for other tabs (one that
    /// does not follow the selection) leaves it where it is, except that a
    /// view never scrolled yet opens on the selection.
    pub fn follow(
        &self,
        request: Option<ScrollRequest>,
        item: Option<Id>,
        area: &gtk::DrawingArea,
        row_of: impl FnOnce(u32) -> Option<u32>,
    ) {
        let Some(request) = request else { return };
        if request.id == self.handled.get() || !area.is_mapped() || area.height() <= 0 {
            return;
        }
        if self.handled.get() != 0 && !request.applies(item) {
            self.handled.set(request.id);
            return;
        }
        if let Some(row) = row_of(request.offset) {
            self.handled.set(request.id);
            self.centre_on(row);
        }
    }

    /// Wheel, touchpad and (optionally) horizontal scrolling for `area`.
    pub fn attach_wheel(&self, area: &gtk::DrawingArea, horizontal: Option<&gtk::Adjustment>) {
        let wheel = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::BOTH_AXES | gtk::EventControllerScrollFlags::DISCRETE,
        );
        let (v, h) = (self.clone(), horizontal.cloned());
        wheel.connect_scroll(move |_, dx, dy| {
            if dy != 0.0 {
                v.by(dy * 3.0);
            }
            if let (Some(h), true) = (&h, dx != 0.0) {
                h.set_value(
                    (h.value() + dx * 48.0).clamp(0.0, (h.upper() - h.page_size()).max(0.0)),
                );
            }
            glib::Propagation::Stop
        });
        area.add_controller(wheel);

        // Smooth (touchpad) scrolling arrives as pixel deltas.
        let smooth = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let (v, h) = (self.clone(), horizontal.cloned());
        smooth.connect_scroll(move |c, dx, dy| {
            if c.unit() != gdk::ScrollUnit::Surface {
                return glib::Propagation::Proceed;
            }
            v.by(dy / v.row_height.get());
            if let Some(h) = &h {
                h.set_value((h.value() + dx).clamp(0.0, (h.upper() - h.page_size()).max(0.0)));
            }
            glib::Propagation::Stop
        });
        area.add_controller(smooth);
    }
}

/// Route unmodified navigation and mark keys to `on_command`.
pub fn attach_keys(area: &gtk::DrawingArea, on_command: impl Fn(EditorCommand) + 'static) {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(
        move |_, key, _, state| match editor_keys::command_for(key, state) {
            Some(c) => {
                on_command(c);
                glib::Propagation::Stop
            }
            None => glib::Propagation::Proceed,
        },
    );
    area.add_controller(keys);
}

/// Draw a gdk colour into a cairo context.
pub fn set_source(cr: &gtk::cairo::Context, c: &gdk::RGBA) {
    cr.set_source_rgba(
        f64::from(c.red()),
        f64::from(c.green()),
        f64::from(c.blue()),
        f64::from(c.alpha()),
    );
}

pub fn with_alpha(c: &gdk::RGBA, alpha: f32) -> gdk::RGBA {
    gdk::RGBA::new(c.red(), c.green(), c.blue(), alpha)
}
