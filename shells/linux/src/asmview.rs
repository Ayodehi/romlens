//! The disassembly canvas: one drawing area that paints the visible lines
//! from cached batches. Mirrors the macOS `AsmTableView` and
//! `AsmLinePainter`.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{cairo, gdk, pango};

use crate::actions;
use crate::asm::{AsmLayout, AsmLine, LineKind};
use crate::canvas::{self, Metrics, VScroll, set_source, with_alpha};
use crate::model::workspace::Id;
use crate::model::{Change, Document, EditorSource};
use crate::palette;

struct State {
    doc: Rc<Document>,
    /// The tab the view is in, for the scroll requests meant for it.
    item: Option<Id>,
    area: gtk::DrawingArea,
    scroll: VScroll,
    hscroll: gtk::Adjustment,
    metrics: Cell<Metrics>,
}

/// Build the disassembly view for a document: the canvas and its scrollbars.
/// The disassembly canvas with its scrollbars, and what the Both tab needs
/// to line it up with the hex.
pub struct AsmPane {
    pub widget: gtk::Widget,
    pub area: gtk::DrawingArea,
    pub scroll: VScroll,
    state: Rc<State>,
}

impl AsmPane {
    pub fn row_height(&self) -> f64 {
        self.state.metrics.get().row_height
    }
}

pub fn build(doc: &Rc<Document>, item: Option<Id>) -> AsmPane {
    let area = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .focusable(true)
        .build();
    area.add_css_class("romlens-asm");
    let metrics = Metrics::measure(&area);
    let scroll = VScroll::new(doc.asm_line_count(), metrics.row_height);
    let hscroll = gtk::Adjustment::new(0.0, 0.0, 1.0, 16.0, 120.0, 1.0);
    let state = Rc::new(State {
        doc: Rc::clone(doc),
        item,
        area: area.clone(),
        scroll: scroll.clone(),
        hscroll: hscroll.clone(),
        metrics: Cell::new(metrics),
    });

    let s = Rc::clone(&state);
    area.set_draw_func(move |_, cr, w, h| s.draw(cr, f64::from(w), f64::from(h)));
    let s = Rc::clone(&state);
    area.connect_resize(move |_, w, h| {
        s.resized(f64::from(w), f64::from(h));
        s.follow_scroll();
    });
    let s = Rc::clone(&state);
    area.connect_map(move |_| s.follow_scroll());
    for adj in [&scroll.adjustment, &hscroll] {
        let area = area.clone();
        adj.connect_value_changed(move |_| area.queue_draw());
    }
    scroll.attach_wheel(&area, Some(&hscroll));

    let click = gtk::GestureClick::builder()
        .button(gdk::BUTTON_PRIMARY)
        .build();
    let s = Rc::clone(&state);
    click.connect_pressed(move |g, n_press, x, y| {
        s.area.grab_focus();
        let extend = g
            .current_event_state()
            .contains(gdk::ModifierType::SHIFT_MASK);
        s.click_at(x, y, n_press, extend);
    });
    area.add_controller(click);

    let s = Rc::clone(&state);
    actions::attach_context_menu(&area, doc, move |x, y| s.select_for_menu(x, y));

    let s = Rc::clone(&state);
    canvas::attach_keys(&area, move |command| {
        s.doc
            .perform(command, EditorSource::Asm, s.scroll.visible_rows());
    });

    let s = Rc::clone(&state);
    doc.subscribe(move |change| match change {
        Change::Rows => {
            // The line count changes with an analysis.
            s.resized(f64::from(s.area.width()), f64::from(s.area.height()));
            s.area.queue_draw();
        }
        Change::Citations => s.area.queue_draw(),
        Change::AddressStyle | Change::Selection => {
            s.resized(f64::from(s.area.width()), f64::from(s.area.height()));
            s.area.queue_draw();
        }
        Change::Scroll => s.follow_scroll(),
        _ => {}
    });

    let s = Rc::clone(&state);
    area.connect_notify_local(Some("scale-factor"), move |_, _| {
        let m = Metrics::measure(&s.area);
        s.metrics.set(m);
        s.scroll.set_row_height(m.row_height);
        s.area.queue_draw();
    });
    // A new colour scheme changes the token colours.
    let a = area.clone();
    adw::StyleManager::default().connect_dark_notify(move |_| a.queue_draw());

    let vbar = gtk::Scrollbar::new(gtk::Orientation::Vertical, Some(&scroll.adjustment));
    let hbar = gtk::Scrollbar::new(gtk::Orientation::Horizontal, Some(&hscroll));
    // Like the macOS scroller, only there when the line is wider than the view.
    let sync_hbar = {
        let hbar = hbar.clone();
        move |adj: &gtk::Adjustment| hbar.set_visible(adj.upper() - adj.page_size() > 1.0)
    };
    sync_hbar(&hscroll);
    hscroll.connect_changed(sync_hbar);
    let grid = gtk::Grid::new();
    grid.attach(&area, 0, 0, 1, 1);
    grid.attach(&vbar, 1, 0, 1, 1);
    grid.attach(&hbar, 0, 1, 1, 1);
    AsmPane {
        widget: grid.upcast(),
        area,
        scroll,
        state,
    }
}

impl State {
    fn layout(&self) -> AsmLayout {
        AsmLayout {
            style: self.doc.address_style(),
        }
    }

    fn follow_scroll(&self) {
        let doc = &self.doc;
        self.scroll
            .follow(doc.scroll_request(), self.item, &self.area, |offset| {
                doc.line_for_offset(offset)
            });
    }

    fn resized(&self, width: f64, height: f64) {
        let m = self.metrics.get();
        self.scroll.set_rows(self.doc.asm_line_count(), height);
        let total = self.layout().total_width(m.char_width);
        self.hscroll.configure(
            self.hscroll.value().min((total - width).max(0.0)),
            0.0,
            total.max(width),
            16.0,
            (width - 16.0).max(16.0),
            width,
        );
    }

    fn line(&self, line: u32) -> Option<AsmLine> {
        self.doc.asm_cache.batch(line)?.line(line).cloned()
    }

    fn draw(&self, cr: &cairo::Context, width: f64, height: f64) {
        let m = self.metrics.get();
        let style = adw::StyleManager::default();
        let (dark, accent) = (style.is_dark(), style.accent_color_rgba());
        let fg = self.area.color();
        let focused = self.area.has_focus();
        let layout = self.layout();
        let (cw, rh) = (m.char_width, m.row_height);
        let hx = self.hscroll.value();
        let count = self.doc.asm_line_count();
        let selected_line = self
            .doc
            .selected()
            .and_then(|o| self.doc.line_for_offset(o));
        let cited = self.doc.citation_highlight();

        let top = self.scroll.top();
        let first = top.floor().max(0.0) as u32;
        let mut y = -(top - top.floor()) * rh;
        let pango_layout = self.area.create_pango_layout(None);
        pango_layout.set_font_description(Some(&canvas::font()));

        let mut index = first;
        while y < height && index < count {
            let Some(line) = self.line(index) else { break };
            let confidence = f32::from(line.confidence) / 100.0;

            if let Some(color) = palette::line_region_color(line.region, confidence) {
                set_source(cr, &with_alpha(&color, 0.05 + 0.10 * confidence));
                cr.rectangle(0.0, y, width, rh);
                let _ = cr.fill();
                set_source(cr, &color);
                cr.rectangle(AsmLayout::LEFT_PADDING - hx, y, AsmLayout::GUTTER_WIDTH, rh);
                let _ = cr.fill();
            }
            if line.kind == LineKind::Label {
                set_source(cr, &with_alpha(&fg, 0.15));
                cr.rectangle(0.0, y, width, 1.0);
                let _ = cr.fill();
            }
            if selected_line == Some(index) {
                set_source(cr, &with_alpha(&accent, if focused { 0.25 } else { 0.12 }));
                cr.rectangle(0.0, y, width, rh);
                let _ = cr.fill();
                set_source(cr, &accent);
                cr.set_line_width(1.0);
                cr.rectangle(0.5, y + 0.5, width - 1.0, rh - 1.0);
                let _ = cr.stroke();
            }
            if line.has_warning() {
                warning_mark(cr, AsmLayout::mark_start() - hx, y, rh);
            }
            // A line the paragraph pointed at in an answer cites (docs/29).
            if cited.iter().any(|r| r.contains(&line.file_offset)) {
                cited_outline(cr, &accent, width, y, rh);
            }

            pango_layout.set_text(&layout.text(&line));
            pango_layout.set_attributes(Some(&self.attributes(&layout, &line, dark, &fg, &accent)));
            set_source(cr, &fg);
            cr.move_to(AsmLayout::text_start() - hx, m.text_top(y));
            pangocairo::functions::show_layout(cr, &pango_layout);
            let _ = cw;

            y += rh;
            index += 1;
        }
    }

    /// Colours for one line: the address and bytes columns dim, then each
    /// token in its kind's colour. Token offsets are bytes within the line's
    /// own text, which is also what Pango indexes by.
    fn attributes(
        &self,
        layout: &AsmLayout,
        line: &AsmLine,
        dark: bool,
        fg: &gdk::RGBA,
        accent: &gdk::RGBA,
    ) -> pango::AttrList {
        let list = pango::AttrList::new();
        let colour = |list: &pango::AttrList, from: usize, to: usize, c: &gdk::RGBA| {
            let to_u16 = |v: f32| (v.clamp(0.0, 1.0) * 65535.0) as u16;
            let mut a = pango::AttrColor::new_foreground(
                to_u16(c.red()),
                to_u16(c.green()),
                to_u16(c.blue()),
            );
            a.set_start_index(from as u32);
            a.set_end_index(to as u32);
            list.insert(a);
            if c.alpha() < 1.0 {
                let mut a = pango::AttrInt::new_foreground_alpha(to_u16(c.alpha()));
                a.set_start_index(from as u32);
                a.set_end_index(to as u32);
                list.insert(a);
            }
        };
        if line.kind.is_content() {
            let address = layout.address_chars();
            colour(&list, 0, address, &with_alpha(fg, 0.45));
            colour(
                &list,
                address,
                address + layout.bytes_chars(),
                &with_alpha(fg, 0.7),
            );
        }
        let origin = layout.text_origin(line);
        let dim = line.kind == LineKind::Data;
        for token in &line.tokens {
            let mut c = palette::token_color(token.kind, dark, fg, accent);
            if dim && c == *fg {
                c = with_alpha(fg, 0.7);
            }
            let (from, to) = (origin + token.start, origin + token.start + token.len);
            colour(&list, from, to, &c);
            if matches!(
                token.kind,
                crate::asm::TokenKind::UserLabel | crate::asm::TokenKind::UserLabelDef
            ) {
                let mut w = pango::AttrInt::new_weight(pango::Weight::Bold);
                w.set_start_index(from as u32);
                w.set_end_index(to as u32);
                list.insert(w);
            }
        }
        list
    }

    fn line_at(&self, y: f64) -> Option<u32> {
        let line = (self.scroll.top() + y / self.metrics.get().row_height).floor();
        (line >= 0.0 && line < f64::from(self.doc.asm_line_count())).then_some(line as u32)
    }

    fn click_at(&self, _x: f64, y: f64, n_press: i32, extend: bool) {
        let Some(line) = self.line_at(y).and_then(|l| self.line(l)) else {
            return;
        };
        if n_press == 2 {
            match line.target_file_offset {
                Some(target) => self.doc.jump_to(target),
                None => self.doc.follow_reference(),
            }
        } else if extend {
            self.doc.extend_selection(line.file_offset);
        } else if line.kind == LineKind::Note {
            self.doc.select_idiom(line.file_offset);
        } else {
            self.doc.select(Some(line.file_offset));
        }
    }

    /// Right-click: select the line under the pointer unless it is already
    /// in the highlighted range, so the menu acts on what was clicked.
    fn select_for_menu(&self, _x: f64, y: f64) {
        if let Some(line) = self.line_at(y).and_then(|l| self.line(l))
            && !self
                .doc
                .highlighted_range()
                .is_some_and(|r| r.contains(&line.file_offset))
        {
            self.doc.select(Some(line.file_offset));
        }
    }
}

/// A warning triangle in its own column, the size of a capital letter, so it
/// reads at a glance without covering the text.
fn warning_mark(cr: &cairo::Context, x: f64, y: f64, h: f64) {
    let side = (AsmLayout::MARK_WIDTH - 4.0).min(h - 4.0);
    let x = x + (AsmLayout::MARK_WIDTH - side) / 2.0 - 1.0;
    let top = y + (h - side) / 2.0;
    set_source(cr, &palette::warning_color());
    cr.move_to(x + side / 2.0, top);
    cr.line_to(x + side, top + side);
    cr.line_to(x, top + side);
    cr.close_path();
    let _ = cr.fill();
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.8);
    let bar = (side / 7.0).max(1.5);
    cr.rectangle(x + (side - bar) / 2.0, top + side * 0.35, bar, side * 0.33);
    let _ = cr.fill();
    cr.rectangle(x + (side - bar) / 2.0, top + side * 0.76, bar, bar);
    let _ = cr.fill();
}

/// The outline of a line an answer's paragraph cites: 1.5 px of the accent,
/// inset by 1, as the macOS canvases draw it.
pub fn cited_outline(cr: &cairo::Context, accent: &gdk::RGBA, width: f64, y: f64, rh: f64) {
    set_source(cr, accent);
    cr.set_line_width(1.5);
    cr.rectangle(1.0, y + 1.0, width - 2.0, rh - 2.0);
    let _ = cr.stroke();
}
