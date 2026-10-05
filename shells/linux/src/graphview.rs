//! The Graph tab (docs/19): the routine at the cursor as a control-flow graph
//! of the listing's own lines, or with its callers and callees. The macOS twin
//! is `GraphView` and `GraphCanvasView`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{cairo, gdk, glib, pango};

use crate::actions;
use crate::asm::TokenKind;
use crate::canvas::{self, Metrics, set_source, with_alpha};
use crate::model::Zoom;
use crate::model::graph::{GraphMode, GraphState};
use crate::model::graphscene::{self, Box_, Cells, Edge, EdgeColor, Line, Scene, SegmentStyle};
use crate::model::workspace::Id;
use crate::model::{Change, Document};
use crate::palette;

const MIN_ZOOM: f64 = 0.1;
const MAX_ZOOM: f64 = 3.0;
const STEP: f64 = 1.25;

struct View {
    doc: Rc<Document>,
    /// The tab the view is in: its own graph, and the zoom when it has focus.
    item: Option<Id>,
    area: gtk::DrawingArea,
    scroll: gtk::ScrolledWindow,
    title: gtk::Label,
    status: gtk::Label,
    modes: Vec<gtk::ToggleButton>,
    scene: RefCell<Scene>,
    zoom: Cell<f64>,
    metrics: Cell<Metrics>,
    shown_generation: Cell<u64>,
    shown_entry: Cell<Option<u32>>,
    shown_mode: Cell<Option<GraphMode>>,
    selected: Cell<Option<u32>>,
    /// A click in the canvas moved the selection: it is in view already.
    selecting_from_canvas: Cell<bool>,
    zoom_id: Cell<u64>,
}

pub fn build(doc: &Rc<Document>, item: Option<Id>) -> gtk::Widget {
    let area = gtk::DrawingArea::builder()
        .focusable(true)
        .has_tooltip(true)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .child(&area)
        .build();
    scroll.add_css_class("romlens-graph");
    let title = gtk::Label::builder().xalign(0.0).build();
    title.add_css_class("heading");
    let status = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    status.add_css_class("dim-label");
    status.add_css_class("caption");

    let mode_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    mode_box.add_css_class("linked");
    mode_box.set_tooltip_text(Some(
        "Blocks: the routine's control flow. Calls: who calls it and what it calls.",
    ));
    let modes: Vec<_> = GraphMode::ALL
        .iter()
        .map(|m| {
            let b = gtk::ToggleButton::with_label(m.title());
            mode_box.append(&b);
            b
        })
        .collect();
    let zoom_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    zoom_box.add_css_class("linked");
    for (label, action, tip) in [
        ("−", "win.zoom-out", "Zoom out (Ctrl+−)"),
        ("Fit", "win.zoom-fit", "Fit the graph in the window"),
        ("+", "win.zoom-in", "Zoom in (Ctrl+=)"),
    ] {
        zoom_box.append(
            &gtk::Button::builder()
                .label(label)
                .action_name(action)
                .tooltip_text(tip)
                .build(),
        );
    }
    let header = gtk::Box::builder()
        .spacing(8)
        .margin_start(8)
        .margin_end(8)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    for w in [
        title.upcast_ref::<gtk::Widget>(),
        status.upcast_ref(),
        mode_box.upcast_ref(),
        zoom_box.upcast_ref(),
    ] {
        header.append(w);
    }
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&scroll);

    let view = Rc::new(View {
        doc: Rc::clone(doc),
        item,
        area,
        scroll,
        title,
        status,
        modes,
        scene: RefCell::new(Scene::empty()),
        zoom: Cell::new(1.0),
        metrics: Cell::new(Metrics::default()),
        shown_generation: Cell::new(u64::MAX),
        shown_entry: Cell::new(None),
        shown_mode: Cell::new(None),
        selected: Cell::new(None),
        selecting_from_canvas: Cell::new(false),
        zoom_id: Cell::new(0),
    });
    view.metrics.set(Metrics::measure(&view.area));
    view.wire();
    view.update();
    // The closures hold the view weakly; the widget keeps it alive.
    let keep = Rc::clone(&view);
    root.connect_destroy(move |_| {
        let _ = &keep;
    });
    root.upcast()
}

impl View {
    fn wire(self: &Rc<Self>) {
        for (i, b) in self.modes.iter().enumerate() {
            let doc = Rc::clone(&self.doc);
            b.connect_clicked(move |b| {
                b.set_active(true);
                doc.set_graph_mode(GraphMode::ALL[i]);
            });
        }
        let this = Rc::downgrade(self);
        self.area.set_draw_func(move |_, cr, w, h| {
            if let Some(v) = this.upgrade() {
                v.draw(cr, f64::from(w), f64::from(h));
            }
        });

        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        let this = Rc::downgrade(self);
        click.connect_pressed(move |_, n, x, y| {
            if let Some(v) = this.upgrade() {
                v.area.grab_focus();
                v.clicked(n, x, y);
            }
        });
        self.area.add_controller(click);

        let this = Rc::downgrade(self);
        actions::attach_context_menu(&self.area, &self.doc, move |x, y| {
            if let Some(v) = this.upgrade() {
                v.select_at(x, y);
            }
        });

        // Pinch and Ctrl+wheel zoom.
        let pinch = gtk::GestureZoom::new();
        let (this, base) = (Rc::downgrade(self), Rc::new(Cell::new(1.0)));
        pinch.connect_begin({
            let (this, base) = (this.clone(), Rc::clone(&base));
            move |_, _| {
                if let Some(v) = this.upgrade() {
                    base.set(v.zoom.get());
                }
            }
        });
        pinch.connect_scale_changed(move |_, scale| {
            if let Some(v) = this.upgrade() {
                v.set_zoom(base.get() * scale);
            }
        });
        self.area.add_controller(pinch);
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let this = Rc::downgrade(self);
        wheel.connect_scroll(move |c, _, dy| {
            let ctrl = c
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK);
            match this.upgrade() {
                Some(v) if ctrl => {
                    v.set_zoom(v.zoom.get() * if dy < 0.0 { STEP } else { 1.0 / STEP });
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.area.add_controller(wheel);

        let this = Rc::downgrade(self);
        self.area.connect_query_tooltip(move |_, x, y, _, tip| {
            let Some(text) = this
                .upgrade()
                .and_then(|v| v.tooltip_at(f64::from(x), f64::from(y)))
            else {
                return false;
            };
            tip.set_text(Some(&text));
            true
        });

        let this = Rc::downgrade(self);
        self.doc.subscribe(move |c| {
            if matches!(
                c,
                Change::Graph | Change::Selection | Change::Rows | Change::Layout
            ) && let Some(v) = this.upgrade()
            {
                v.update();
            }
        });
        let area = self.area.clone();
        adw::StyleManager::default().connect_dark_notify(move |_| area.queue_draw());
    }

    fn cells(&self) -> Cells {
        let m = self.metrics.get();
        Cells {
            char_width: m.char_width,
            row_height: m.row_height,
        }
    }

    // MARK: Updating

    fn update(self: &Rc<Self>) {
        let (state, mode, generation, blocks, calls) = {
            let g = self.doc.graph_of(self.item);
            (
                g.state.clone(),
                g.mode,
                g.result_generation,
                g.blocks.clone(),
                g.calls.clone(),
            )
        };
        for (i, b) in self.modes.iter().enumerate() {
            b.set_active(GraphMode::ALL[i] == mode);
        }
        match &state {
            GraphState::Idle => {
                self.title.set_text("No routine");
                self.status
                    .set_text("Select an instruction to see its routine as a graph.");
            }
            GraphState::Loading => self.status.set_text(if mode == GraphMode::Blocks {
                "Building the graph…"
            } else {
                "Finding callers and callees…"
            }),
            GraphState::Ready => {
                if let Some(b) = &blocks {
                    self.title.set_text(&format!(
                        "{}  {}",
                        b.name,
                        romlens_ffi::format_snes_address(b.entry)
                    ));
                    self.status.set_text(&graphscene::summary_blocks(b));
                } else if let Some(c) = &calls {
                    self.title.set_text(&format!(
                        "{}  {}",
                        c.name,
                        romlens_ffi::format_snes_address(c.entry)
                    ));
                    self.status.set_text(&graphscene::summary_calls(c));
                }
            }
            GraphState::NotInRoutine => {
                self.title.set_text("No routine");
                self.status
                    .set_text("The selection is not inside a routine the analysis found.");
            }
            GraphState::Failed(m) => self.status.set_text(m),
        }

        let mut routine_changed = false;
        if self.shown_generation.get() != generation {
            self.shown_generation.set(generation);
            let entry = blocks
                .as_ref()
                .map(|b| b.entry)
                .or(calls.as_ref().map(|c| c.entry));
            routine_changed =
                entry != self.shown_entry.get() || Some(mode) != self.shown_mode.get();
            let scene = if let Some(b) = &blocks {
                let doc = Rc::clone(&self.doc);
                graphscene::blocks(b, self.cells(), move |n| {
                    doc.asm_cache.batch(n)?.line(n).cloned()
                })
            } else if let Some(c) = &calls {
                graphscene::calls(c, self.cells())
            } else {
                Scene::empty()
            };
            self.install(scene);
            self.shown_entry.set(entry);
            self.shown_mode.set(Some(mode));
            if routine_changed {
                self.show_top();
            }
        }

        let request = self.doc.zoom_request();
        if let Some((kind, id)) = request
            && id != self.zoom_id.get()
        {
            self.zoom_id.set(id);
            // Only the focused Graph tab answers; the atlas has its own zoom.
            if self.doc.focused_item_id().is_some() && self.doc.focused_item_id() == self.item {
                match kind {
                    Zoom::In => self.set_zoom(self.zoom.get() * STEP),
                    Zoom::Out => self.set_zoom(self.zoom.get() / STEP),
                    Zoom::Fit => self.fit(),
                }
            }
        }

        let sel = self
            .doc
            .details()
            .instruction
            .map(|i| i.file_offset)
            .or_else(|| self.doc.selected());
        if sel != self.selected.get() || routine_changed {
            self.selected.set(sel);
            self.area.queue_draw();
            if !self.selecting_from_canvas.get()
                && let Some(rect) = sel.and_then(|s| self.scene.borrow().rect_for_offset(s))
            {
                self.reveal(rect);
            }
        }
        self.selecting_from_canvas.set(false);
    }

    fn install(&self, scene: Scene) {
        *self.scene.borrow_mut() = scene;
        self.resize();
        self.area.queue_draw();
    }

    fn resize(&self) {
        let (w, h) = self.scene.borrow().size;
        let z = self.zoom.get();
        let margin = 24.0 * z;
        self.area.set_content_width((w * z + margin) as i32);
        self.area.set_content_height((h * z + margin) as i32);
    }

    // MARK: Zoom and scrolling

    fn set_zoom(&self, zoom: f64) {
        let z = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if (z - self.zoom.get()).abs() < 1e-6 {
            return;
        }
        self.zoom.set(z);
        self.resize();
        self.area.queue_draw();
    }

    fn fit(&self) {
        let (w, h) = self.scene.borrow().size;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let (vw, vh) = (
            f64::from(self.scroll.width()),
            f64::from(self.scroll.height()),
        );
        self.set_zoom((vw / w).min(vh / h).min(1.0));
        self.show_top();
    }

    /// The routine's first box at the top, centred across.
    fn show_top(&self) {
        let Some(first) = self.scene.borrow().boxes.first().map(|b| b.rect) else {
            return;
        };
        let z = self.zoom.get();
        let (h, v) = (self.scroll.hadjustment(), self.scroll.vadjustment());
        glib::idle_add_local_once(move || {
            h.set_value(
                (first.mid_x() * z - h.page_size() / 2.0)
                    .clamp(0.0, (h.upper() - h.page_size()).max(0.0)),
            );
            v.set_value(0.0);
        });
    }

    fn reveal(&self, rect: graphscene::Rect) {
        let z = self.zoom.get();
        let (h, v) = (self.scroll.hadjustment(), self.scroll.vadjustment());
        let (x0, y0, x1, y1) = (
            rect.x * z,
            rect.y * z,
            (rect.x + rect.w) * z,
            (rect.y + rect.h) * z,
        );
        let inside = x0 >= h.value()
            && x1 <= h.value() + h.page_size()
            && y0 >= v.value()
            && y1 <= v.value() + v.page_size();
        if inside {
            return;
        }
        let center = |mid: f64, adj: &gtk::Adjustment| {
            (mid - adj.page_size() / 2.0).clamp(0.0, (adj.upper() - adj.page_size()).max(0.0))
        };
        let (tx, ty) = (center(rect.mid_x() * z, &h), center(rect.mid_y() * z, &v));
        glib::idle_add_local_once(move || {
            h.set_value(tx);
            v.set_value(ty);
        });
    }

    // MARK: Mouse

    fn to_scene(&self, x: f64, y: f64) -> (f64, f64) {
        let z = self.zoom.get();
        (x / z, y / z)
    }

    fn clicked(&self, n_press: i32, x: f64, y: f64) {
        let p = self.to_scene(x, y);
        let scene = self.scene.borrow();
        let Some((b, l)) = scene.hit(p) else { return };
        let bx = &scene.boxes[b];
        if n_press == 2
            && let Some(target) = bx.target
        {
            drop(scene);
            self.doc.jump_to_snes(target);
            return;
        }
        let offset = l
            .and_then(|l| bx.lines[l].offset)
            .or_else(|| bx.lines.iter().find_map(|ln| ln.offset));
        drop(scene);
        if let Some(offset) = offset {
            self.select_offset(offset);
        }
    }

    fn select_offset(&self, offset: u32) {
        self.selecting_from_canvas.set(true);
        self.doc.select(Some(offset));
        self.doc.request_scroll(offset);
    }

    /// Right-click: select the line under the pointer, so the menu acts on it.
    fn select_at(&self, x: f64, y: f64) {
        let p = self.to_scene(x, y);
        let offset = {
            let scene = self.scene.borrow();
            scene
                .hit(p)
                .and_then(|(b, l)| l.and_then(|l| scene.boxes[b].lines[l].offset))
        };
        if let Some(o) = offset {
            self.select_offset(o);
        }
    }

    fn tooltip_at(&self, x: f64, y: f64) -> Option<String> {
        let p = self.to_scene(x, y);
        let scene = self.scene.borrow();
        let (b, _) = scene.hit(p)?;
        scene.boxes[b].tooltip.clone()
    }

    // MARK: Drawing

    fn draw(&self, cr: &cairo::Context, _w: f64, _h: f64) {
        let style = adw::StyleManager::default();
        let (dark, accent) = (style.is_dark(), style.accent_color_rgba());
        let fg = self.area.color();
        let z = self.zoom.get();
        let scene = self.scene.borrow();
        let colors = Colors::new(dark, &fg, &accent);
        cr.scale(z, z);
        let clip = cr.clip_extents().unwrap_or((0.0, 0.0, 1e9, 1e9));
        let selected = self.selected.get().and_then(|o| scene.line_for_offset(o));
        let pango_layout = self.area.create_pango_layout(None);
        pango_layout.set_font_description(Some(&canvas::font()));
        let m = self.metrics.get();
        for (i, bx) in scene.boxes.iter().enumerate() {
            let r = bx.rect;
            if r.x + r.w + 60.0 < clip.0
                || r.x - 60.0 > clip.2
                || r.y + r.h + 20.0 < clip.1
                || r.y - 20.0 > clip.3
            {
                continue;
            }
            let line = selected.filter(|(b, _)| *b == i).map(|(_, l)| l);
            draw_box(
                cr,
                &pango_layout,
                &scene,
                i,
                bx,
                line,
                &colors,
                dark,
                &fg,
                &accent,
                m,
            );
        }
        for e in &scene.edges {
            draw_edge(cr, &pango_layout, e, &colors);
        }
    }
}

struct Colors {
    box_fill: gdk::RGBA,
    border: gdk::RGBA,
    loop_tint: gdk::RGBA,
    accent: gdk::RGBA,
    fg: gdk::RGBA,
    dark: bool,
}

impl Colors {
    fn new(dark: bool, fg: &gdk::RGBA, accent: &gdk::RGBA) -> Self {
        let rgb = |r: f32, g: f32, b: f32| gdk::RGBA::new(r, g, b, 1.0);
        Self {
            box_fill: if dark {
                rgb(0.14, 0.14, 0.15)
            } else {
                rgb(1.0, 1.0, 1.0)
            },
            border: with_alpha(fg, 0.3),
            loop_tint: rgb(0.21, 0.52, 0.89),
            accent: *accent,
            fg: *fg,
            dark,
        }
    }

    fn edge(&self, c: EdgeColor) -> gdk::RGBA {
        let tone = |light: (f32, f32, f32), dk: (f32, f32, f32)| {
            let (r, g, b) = if self.dark { dk } else { light };
            gdk::RGBA::new(r, g, b, 1.0)
        };
        match c {
            EdgeColor::Back => tone((0.21, 0.52, 0.89), (0.47, 0.68, 0.96)),
            EdgeColor::Taken => tone((0.15, 0.64, 0.41), (0.56, 0.94, 0.64)),
            EdgeColor::NotTaken => tone((0.88, 0.11, 0.14), (1.0, 0.48, 0.39)),
            EdgeColor::Case | EdgeColor::Table => tone((0.9, 0.38, 0.0), (1.0, 0.75, 0.44)),
            EdgeColor::Tail => tone((0.57, 0.25, 0.67), (0.86, 0.54, 0.87)),
            EdgeColor::Observed => tone((0.1, 0.5, 0.56), (0.5, 0.84, 0.88)),
            EdgeColor::Plain => with_alpha(&self.fg, 0.6),
        }
    }
}

fn blend(a: &gdk::RGBA, b: &gdk::RGBA, f: f32) -> gdk::RGBA {
    let mix = |x: f32, y: f32| x + (y - x) * f;
    gdk::RGBA::new(
        mix(a.red(), b.red()),
        mix(a.green(), b.green()),
        mix(a.blue(), b.blue()),
        1.0,
    )
}

fn rounded_rect(cr: &cairo::Context, r: graphscene::Rect, radius: f64) {
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    cr.new_sub_path();
    cr.arc(
        x + w - radius,
        y + radius,
        radius,
        -std::f64::consts::FRAC_PI_2,
        0.0,
    );
    cr.arc(
        x + w - radius,
        y + h - radius,
        radius,
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    cr.arc(
        x + radius,
        y + h - radius,
        radius,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + radius,
        y + radius,
        radius,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    cr.close_path();
}

/// Pango text and colours for a line's segments.
fn line_layout(
    layout: &pango::Layout,
    line: &Line,
    dark: bool,
    fg: &gdk::RGBA,
    accent: &gdk::RGBA,
) {
    let text: String = line.text();
    layout.set_text(&text);
    let list = pango::AttrList::new();
    let to_u16 = |v: f32| (v.clamp(0.0, 1.0) * 65535.0) as u16;
    let mut at = 0usize;
    for s in &line.segments {
        let len = s.text.len();
        let base = match (s.kind, s.style) {
            (Some(k), _) => palette::token_color(k, dark, fg, accent),
            (None, SegmentStyle::Comment) => {
                palette::token_color(TokenKind::Comment, dark, fg, accent)
            }
            (None, SegmentStyle::Faint) => with_alpha(fg, 0.45),
            (None, SegmentStyle::Dim) => with_alpha(fg, 0.7),
            (None, _) => *fg,
        };
        let mut a = pango::AttrColor::new_foreground(
            to_u16(base.red()),
            to_u16(base.green()),
            to_u16(base.blue()),
        );
        a.set_start_index(at as u32);
        a.set_end_index((at + len) as u32);
        list.insert(a);
        if base.alpha() < 1.0 {
            let mut a = pango::AttrInt::new_foreground_alpha(to_u16(base.alpha()));
            a.set_start_index(at as u32);
            a.set_end_index((at + len) as u32);
            list.insert(a);
        }
        if s.style == SegmentStyle::Bold
            || matches!(s.kind, Some(TokenKind::UserLabel | TokenKind::UserLabelDef))
        {
            let mut w = pango::AttrInt::new_weight(pango::Weight::Bold);
            w.set_start_index(at as u32);
            w.set_end_index((at + len) as u32);
            list.insert(w);
        }
        at += len;
    }
    layout.set_attributes(Some(&list));
}

#[allow(clippy::too_many_arguments)]
fn draw_box(
    cr: &cairo::Context,
    layout: &pango::Layout,
    scene: &Scene,
    index: usize,
    bx: &Box_,
    selected_line: Option<usize>,
    colors: &Colors,
    dark: bool,
    fg: &gdk::RGBA,
    accent: &gdk::RGBA,
    m: Metrics,
) {
    let _ = cr.save();
    if bx.dim {
        cr.push_group();
    }
    let r = bx.rect;
    rounded_rect(cr, r, 4.0);
    let mut fill = colors.box_fill;
    if bx.loop_depth > 0 {
        fill = blend(
            &fill,
            &colors.loop_tint,
            (0.08 * bx.loop_depth as f32).min(0.24),
        );
    }
    if !bx.stub {
        set_source(cr, &fill);
        let _ = cr.fill_preserve();
    }
    cr.new_path();
    if let Some(l) = selected_line {
        let lr = scene.line_rect(index, l);
        set_source(cr, &with_alpha(&colors.accent, 0.25));
        cr.rectangle(lr.x, lr.y, lr.w, lr.h);
        let _ = cr.fill();
        rounded_rect(cr, r, 4.0);
    }
    let border = if bx.loop_header {
        colors.loop_tint
    } else if bx.emphasis {
        colors.fg
    } else {
        colors.border
    };
    set_source(cr, &border);
    cr.set_line_width(if bx.emphasis || bx.loop_header {
        1.5
    } else {
        1.0
    });
    if bx.stub {
        cr.set_dash(&[4.0, 3.0], 0.0);
    }
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);

    for (i, line) in bx.lines.iter().enumerate() {
        let top = r.y + graphscene::PADDING.1 + i as f64 * scene.row_height;
        line_layout(layout, line, dark, fg, accent);
        set_source(cr, fg);
        cr.move_to(r.x + graphscene::PADDING.0, m.text_top(top));
        pangocairo::functions::show_layout(cr, layout);
    }
    if let Some(badge) = &bx.badge {
        layout.set_attributes(None);
        layout.set_text(badge);
        let (w, h) = layout.pixel_size();
        set_source(cr, &with_alpha(fg, 0.6));
        cr.move_to(r.x + r.w - f64::from(w), r.y - f64::from(h) - 1.0);
        pangocairo::functions::show_layout(cr, layout);
    }
    if bx.dim {
        let _ = cr.pop_group_to_source();
        let _ = cr.paint_with_alpha(0.4);
    }
    let _ = cr.restore();
}

fn draw_edge(cr: &cairo::Context, layout: &pango::Layout, e: &Edge, colors: &Colors) {
    let p = &e.points;
    if p.len() < 2 {
        return;
    }
    let _ = cr.save();
    if e.dim {
        cr.push_group();
    }
    let color = colors.edge(e.color);
    set_source(cr, &color);
    cr.set_line_width(e.width);
    cr.set_line_join(cairo::LineJoin::Round);
    if e.dashed {
        cr.set_dash(&[5.0, 3.0], 0.0);
    }
    cr.move_to(p[0].0, p[0].1);
    for i in 1..p.len() - 1 {
        // Round each corner by up to 8 points.
        let (a, b, c) = (p[i - 1], p[i], p[i + 1]);
        let dist =
            |u: (f64, f64), v: (f64, f64)| ((v.0 - u.0).powi(2) + (v.1 - u.1).powi(2)).sqrt();
        let r = 8.0f64.min(dist(a, b) / 2.0).min(dist(b, c) / 2.0);
        let toward = |from: (f64, f64), to: (f64, f64), d: f64| {
            let len = dist(from, to).max(1e-6);
            (
                from.0 + (to.0 - from.0) / len * d,
                from.1 + (to.1 - from.1) / len * d,
            )
        };
        let before = toward(b, a, r);
        let after = toward(b, c, r);
        cr.line_to(before.0, before.1);
        cr.curve_to(b.0, b.1, b.0, b.1, after.0, after.1);
    }
    // Stop short of the tip so the line does not poke through it.
    let (end, before) = (p[p.len() - 1], p[p.len() - 2]);
    let len = ((end.0 - before.0).powi(2) + (end.1 - before.1).powi(2))
        .sqrt()
        .max(0.001);
    let (ux, uy) = ((end.0 - before.0) / len, (end.1 - before.1) / len);
    let arrow = 7.0;
    cr.line_to(end.0 - ux * arrow * 0.8, end.1 - uy * arrow * 0.8);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    cr.move_to(end.0, end.1);
    cr.line_to(
        end.0 - ux * arrow - uy * arrow * 0.5,
        end.1 - uy * arrow + ux * arrow * 0.5,
    );
    cr.line_to(
        end.0 - ux * arrow + uy * arrow * 0.5,
        end.1 - uy * arrow - ux * arrow * 0.5,
    );
    cr.close_path();
    let _ = cr.fill();
    if let Some(label) = &e.label {
        layout.set_attributes(None);
        layout.set_text(label);
        cr.move_to(p[0].0 + 4.0, p[0].1 + 1.0);
        pangocairo::functions::show_layout(cr, layout);
    }
    if e.dim {
        let _ = cr.pop_group_to_source();
        let _ = cr.paint_with_alpha(0.35);
    }
    let _ = cr.restore();
}
