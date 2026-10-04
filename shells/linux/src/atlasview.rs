//! The Atlas tab (docs/22, A2): the whole ROM as a map, one bank a row,
//! zoomed continuously from all of it down to single instructions. Columns
//! come from the core a window at a time: the visible bytes of each visible
//! row, a column a pixel. The macOS twin is `AtlasView`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{cairo, gdk, pango};
use romlens_ffi::{ArcEndInfo, AtlasItemsInfo, CallArcInfo};

use crate::canvas::{self, set_source, with_alpha};
use crate::model::atlas::{AtlasState, Geometry, ITEM_ZOOM, Overlay};
use crate::model::strip::{self, StripColumn};
use crate::model::{Change, Document, Tab, Zoom};
use crate::palette;

const KEY_LIMIT_COLUMNS: usize = 600;
const KEY_LIMIT_ITEMS: usize = 200;
const ZOOM_STEP: f64 = 1.5;

type ColumnKey = (u32, u32, u32);

struct View {
    doc: Rc<Document>,
    area: gtk::DrawingArea,
    status: gtk::Label,
    overlays: Vec<gtk::ToggleButton>,
    geo: RefCell<Geometry>,
    state: Cell<AtlasState>,
    hover: Cell<Option<(f64, f64)>>,
    /// A click on the map moved the selection: it is in view already.
    selecting: Cell<bool>,
    selection: Cell<Option<u32>>,
    generation: Cell<u64>,
    columns: RefCell<HashMap<ColumnKey, Vec<StripColumn>>>,
    items: RefCell<HashMap<(u32, u32), AtlasItemsInfo>>,
    /// Every call in the ROM between columns of `bytes` bytes.
    arcs: RefCell<Option<(u32, u32, Vec<CallArcInfo>)>>,
    zoom_id: Cell<u64>,
}

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let area = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .focusable(true)
        .build();
    let title = gtk::Label::new(Some("Atlas"));
    title.add_css_class("heading");
    let status = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    status.add_css_class("dim-label");
    status.add_css_class("caption");

    let overlay_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    overlay_box.add_css_class("linked");
    let overlays: Vec<_> = Overlay::ALL
        .iter()
        .map(|o| {
            let b = gtk::ToggleButton::with_label(o.title());
            b.set_tooltip_text(Some(o.help()));
            overlay_box.append(&b);
            b
        })
        .collect();
    let zoom_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    zoom_box.add_css_class("linked");
    for (label, action, tip) in [
        ("−", "win.zoom-out", "Zoom out (Ctrl+−)"),
        ("Fit", "win.zoom-fit", "The whole ROM, one bank a row"),
        (
            "+",
            "win.zoom-in",
            "Zoom in (Ctrl+=); pinch or Ctrl+scroll to zoom where the pointer is",
        ),
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
        overlay_box.upcast_ref(),
        zoom_box.upcast_ref(),
    ] {
        header.append(w);
    }
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&area);

    let bank = if doc.info.mapping == romlens_ffi::Mapping::LoRom {
        0x8000
    } else {
        0x1_0000
    };
    let view = Rc::new(View {
        doc: Rc::clone(doc),
        area,
        status,
        overlays,
        geo: RefCell::new(Geometry::new(doc.byte_count(), bank)),
        state: Cell::new(AtlasState::default()),
        hover: Cell::new(None),
        selecting: Cell::new(false),
        selection: Cell::new(None),
        generation: Cell::new(doc.generation()),
        columns: RefCell::new(HashMap::new()),
        items: RefCell::new(HashMap::new()),
        arcs: RefCell::new(None),
        zoom_id: Cell::new(doc.zoom_request().map_or(0, |(_, id)| id)),
    });
    view.wire();
    view.sync_overlays();
    view.show_status();
    let keep = Rc::clone(&view);
    root.connect_destroy(move |_| {
        let _ = &keep;
    });
    root.upcast()
}

impl View {
    fn wire(self: &Rc<Self>) {
        for (i, b) in self.overlays.iter().enumerate() {
            let this = Rc::downgrade(self);
            b.connect_clicked(move |b| {
                b.set_active(true);
                if let Some(v) = this.upgrade() {
                    v.state.set(AtlasState {
                        overlay: Overlay::ALL[i],
                    });
                    v.sync_overlays();
                    v.area.queue_draw();
                }
            });
        }
        let this = Rc::downgrade(self);
        self.area.set_draw_func(move |_, cr, w, h| {
            if let Some(v) = this.upgrade() {
                v.draw(cr, f64::from(w), f64::from(h));
            }
        });
        let this = Rc::downgrade(self);
        self.area.connect_resize(move |_, w, h| {
            if let Some(v) = this.upgrade() {
                v.geo.borrow_mut().resize(f64::from(w), f64::from(h));
                *v.arcs.borrow_mut() = None;
                v.show_status();
            }
        });

        let motion = gtk::EventControllerMotion::new();
        let this = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            if let Some(v) = this.upgrade() {
                v.hover.set(Some((x, y)));
                v.area.queue_draw();
                v.show_status();
            }
        });
        let this = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(v) = this.upgrade() {
                v.hover.set(None);
                v.area.queue_draw();
                v.show_status();
            }
        });
        self.area.add_controller(motion);

        // A click selects the byte under it; a double-click opens the listing there.
        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        let this = Rc::downgrade(self);
        click.connect_pressed(move |_, n, x, y| {
            if let Some(v) = this.upgrade() {
                v.area.grab_focus();
                v.clicked(n, (x, y));
            }
        });
        self.area.add_controller(click);

        // Scroll moves, Ctrl+scroll zooms at the pointer; pinch zooms too.
        let scroll = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::BOTH_AXES | gtk::EventControllerScrollFlags::KINETIC,
        );
        let this = Rc::downgrade(self);
        scroll.connect_scroll(move |c, dx, dy| {
            let Some(v) = this.upgrade() else {
                return gtk::glib::Propagation::Proceed;
            };
            let state = c.current_event_state();
            let p = v.hover.get();
            if state.contains(gdk::ModifierType::CONTROL_MASK) {
                v.zoom_by((-dy * 0.1).exp(), p);
            } else {
                // A wheel notch is a step; a touchpad delta is already pixels.
                let scale = if c.unit() == gdk::ScrollUnit::Wheel {
                    40.0
                } else {
                    1.0
                };
                v.geo.borrow_mut().scroll_by(dx * scale, dy * scale);
                v.after_move();
            }
            gtk::glib::Propagation::Stop
        });
        self.area.add_controller(scroll);
        let pinch = gtk::GestureZoom::new();
        let base = Rc::new(Cell::new(0.0));
        let this = Rc::downgrade(self);
        pinch.connect_begin({
            let (this, base) = (this.clone(), Rc::clone(&base));
            move |_, _| {
                if let Some(v) = this.upgrade() {
                    base.set(v.geo.borrow().ppb);
                }
            }
        });
        pinch.connect_scale_changed(move |g, scale| {
            if let Some(v) = this.upgrade() {
                let current = v.geo.borrow().ppb;
                if current > 0.0 && base.get() > 0.0 {
                    let anchor = g.bounding_box_center();
                    v.zoom_by(base.get() * scale / current, anchor);
                }
            }
        });
        self.area.add_controller(pinch);

        let this = Rc::downgrade(self);
        self.doc.subscribe(move |c| {
            let Some(v) = this.upgrade() else { return };
            match c {
                Change::Zoom => v.handle_zoom(),
                Change::Selection => v.selection_changed(),
                Change::Rows => v.analysis_changed(),
                _ => {}
            }
        });
        let area = self.area.clone();
        adw::StyleManager::default().connect_dark_notify(move |_| area.queue_draw());
    }

    fn sync_overlays(&self) {
        for (i, b) in self.overlays.iter().enumerate() {
            b.set_active(Overlay::ALL[i] == self.state.get().overlay);
        }
    }

    fn handle_zoom(&self) {
        let Some((kind, id)) = self.doc.zoom_request() else {
            return;
        };
        if id == self.zoom_id.get()
            || self.doc.focused_content() != Some(crate::model::workspace::EditorContent::Atlas)
        {
            return;
        }
        self.zoom_id.set(id);
        match kind {
            Zoom::In => self.zoom_by(ZOOM_STEP, None),
            Zoom::Out => self.zoom_by(1.0 / ZOOM_STEP, None),
            Zoom::Fit => {
                self.geo.borrow_mut().fit();
                self.after_move();
                *self.arcs.borrow_mut() = None;
            }
        }
    }

    fn zoom_by(&self, factor: f64, anchor: Option<(f64, f64)>) {
        if self.geo.borrow_mut().zoom_by(factor, anchor) {
            *self.arcs.borrow_mut() = None;
            self.after_move();
        }
    }

    fn after_move(&self) {
        self.area.queue_draw();
        self.show_status();
    }

    /// A new analysis drops what was fetched.
    fn analysis_changed(&self) {
        if self.doc.generation() != self.generation.get() {
            self.generation.set(self.doc.generation());
            self.columns.borrow_mut().clear();
            self.items.borrow_mut().clear();
            *self.arcs.borrow_mut() = None;
            self.area.queue_draw();
        }
    }

    fn selection_changed(&self) {
        let sel = self.doc.selected();
        if sel == self.selection.get() {
            return;
        }
        self.selection.set(sel);
        if !self.selecting.get()
            && let Some(s) = sel
        {
            self.geo.borrow_mut().reveal(s);
        }
        self.selecting.set(false);
        self.area.queue_draw();
    }

    fn clicked(&self, n_press: i32, p: (f64, f64)) {
        let Some(off) = self.geo.borrow().offset_at(p) else {
            return;
        };
        if n_press >= 2 {
            self.doc.set_tab(Tab::Disassembly);
            self.doc.jump_to(off);
        } else {
            self.selecting.set(true);
            self.doc.select(Some(off));
        }
    }

    // MARK: Data

    fn fetch_columns(&self, start: u32, len: u32, buckets: u32) -> Vec<StripColumn> {
        let key = (start, len, buckets);
        if let Some(c) = self.columns.borrow().get(&key) {
            return c.clone();
        }
        let mut cache = self.columns.borrow_mut();
        if cache.len() > KEY_LIMIT_COLUMNS {
            cache.clear();
        }
        let c = strip::decode(&self.doc.workbench().region_map_window(start, len, buckets));
        cache.insert(key, c.clone());
        c
    }

    fn fetch_items(&self, start: u32, len: u32) -> AtlasItemsInfo {
        if let Some(i) = self.items.borrow().get(&(start, len)) {
            return i.clone();
        }
        let mut cache = self.items.borrow_mut();
        if cache.len() > KEY_LIMIT_ITEMS {
            cache.clear();
        }
        let i = self.doc.workbench().atlas_items(start, len, 8000);
        cache.insert((start, len), i.clone());
        i
    }

    /// Every call, bucketed to columns about a pixel wide at this zoom.
    fn all_arcs(&self) -> (u32, u32, Vec<CallArcInfo>) {
        let ppb = self.geo.borrow().ppb.max(1e-9);
        let bytes = ((1.0 / ppb) as u32).max(1);
        if let Some((b, buckets, list)) = self.arcs.borrow().as_ref()
            && *b == bytes
        {
            return (*b, *buckets, list.clone());
        }
        let len = self.geo.borrow().rom_length;
        let buckets = (len / bytes).max(1);
        let list = self.doc.workbench().atlas_call_arcs(0, len, buckets);
        *self.arcs.borrow_mut() = Some((bytes, buckets, list.clone()));
        (bytes, buckets, list)
    }

    // MARK: Drawing

    fn draw(&self, cr: &cairo::Context, width: f64, height: f64) {
        let style = adw::StyleManager::default();
        let (dark, accent) = (style.is_dark(), style.accent_color_rgba());
        let fg = self.area.color();
        let bg = if dark {
            (0.1, 0.1, 0.11)
        } else {
            (0.91, 0.91, 0.92)
        };
        cr.set_source_rgb(bg.0, bg.1, bg.2);
        let _ = cr.paint();
        let geo = self.geo.borrow().clone();
        if geo.ppb <= 0.0 {
            return;
        }
        let layout = self.area.create_pango_layout(None);
        let mut font = canvas::font();
        font.set_size(
            (11.0f64.min((geo.row_height() - 2.0).max(8.0)) * f64::from(pango::SCALE)) as i32,
        );
        layout.set_font_description(Some(&font));
        let overlay = self.state.get().overlay;
        let show_items = geo.ppb >= ITEM_ZOOM;
        for (row, row_start, b0, end) in geo.visible_rows() {
            let y = geo.row_y(row);
            // A label every row when there is room, else every fourth.
            if geo.row_height() >= 8.0 || row % 4 == 0 {
                let bank = self.doc.rom.snes_address_for(row_start).map_or_else(
                    || format!("{row_start:06X}"),
                    |a| format!("${:02X}", a >> 16),
                );
                layout.set_text(&bank);
                set_source(cr, &with_alpha(&fg, 0.6));
                cr.move_to(6.0, y + ((geo.row_height() - 13.0).max(0.0)) / 2.0);
                pangocairo::functions::show_layout(cr, &layout);
            }
            let _ = cr.save();
            cr.rectangle(
                crate::model::atlas::GUTTER,
                y,
                geo.map_width(),
                geo.row_height(),
            );
            cr.clip();
            let len = end - b0;
            let buckets = ((f64::from(len) * geo.ppb).ceil() as u32).clamp(1, len);
            for c in self.fetch_columns(row_start + b0, len, buckets) {
                let r = (
                    geo.x_of_byte(c.start - row_start),
                    y,
                    (f64::from(c.len) * geo.ppb).max(1.0),
                    geo.row_height(),
                );
                paint(cr, &c, overlay, r, &accent, &fg);
            }
            if show_items {
                self.draw_items(
                    cr,
                    &geo,
                    &self.fetch_items(row_start + b0, len),
                    row_start,
                    y,
                    &bg,
                );
            }
            let _ = cr.restore();
        }
        self.draw_selection(cr, &geo, &fg);
        self.draw_arcs(cr, &geo, &accent);
        let _ = (width, height);
    }

    /// Each instruction and data row outlined, so single items read as such.
    fn draw_items(
        &self,
        cr: &cairo::Context,
        geo: &Geometry,
        found: &AtlasItemsInfo,
        row_start: u32,
        y: f64,
        bg: &(f64, f64, f64),
    ) {
        cr.set_source_rgba(bg.0, bg.1, bg.2, 0.85);
        cr.set_line_width(1.0);
        for i in &found.items {
            let (x, w, h) = (
                geo.x_of_byte(i.offset - row_start) + 0.5,
                f64::from(i.len) * geo.ppb - 1.0,
                geo.row_height() - 1.0,
            );
            if i.instruction {
                let r = 3.0f64.min(w / 4.0).min(h / 4.0).max(0.0);
                cr.new_sub_path();
                cr.arc(x + w - r, y + 0.5 + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
                cr.arc(
                    x + w - r,
                    y + 0.5 + h - r,
                    r,
                    0.0,
                    std::f64::consts::FRAC_PI_2,
                );
                cr.arc(
                    x + r,
                    y + 0.5 + h - r,
                    r,
                    std::f64::consts::FRAC_PI_2,
                    std::f64::consts::PI,
                );
                cr.arc(
                    x + r,
                    y + 0.5 + r,
                    r,
                    std::f64::consts::PI,
                    1.5 * std::f64::consts::PI,
                );
                cr.close_path();
            } else {
                cr.rectangle(x, y + 0.5, w, h);
            }
            let _ = cr.stroke();
        }
    }

    fn draw_selection(&self, cr: &cairo::Context, geo: &Geometry, fg: &gdk::RGBA) {
        let Some(s) = self.selection.get().filter(|s| *s < geo.rom_length) else {
            return;
        };
        let p = geo.point_of(s);
        if p.0 < crate::model::atlas::GUTTER - 2.0 {
            return;
        }
        let y = geo.row_y((s / geo.bank_size) as usize);
        let w = geo.ppb.max(2.0);
        set_source(cr, fg);
        cr.set_line_width(1.5);
        cr.rectangle(p.0 - w / 2.0, y - 1.0, w, geo.row_height() + 2.0);
        let _ = cr.stroke();
    }

    /// The calls into and out of the columns near the pointer: out in the
    /// accent colour, in in orange, thicker for more calls.
    fn draw_arcs(&self, cr: &cairo::Context, geo: &Geometry, accent: &gdk::RGBA) {
        let Some(under) = self.hover.get().and_then(|h| geo.offset_at(h)) else {
            return;
        };
        let (bytes, buckets, list) = self.all_arcs();
        let here = under / bytes;
        let near = |e: ArcEndInfo| matches!(e, ArcEndInfo::Column { index } if index + 3 >= here && index <= here + 3);
        let offset_of = |e: ArcEndInfo| match e {
            ArcEndInfo::Column { index } => Some(
                (u64::from(index) * u64::from(geo.rom_length) / u64::from(buckets)) as u32
                    + bytes / 2,
            ),
            _ => None,
        };
        let orange = gdk::RGBA::new(0.9, 0.38, 0.0, 1.0);
        for arc in list.iter().take(20_000) {
            let (out, into) = (near(arc.from), near(arc.to));
            let (Some(from), Some(to)) = (offset_of(arc.from), offset_of(arc.to)) else {
                continue;
            };
            if !(out || into) {
                continue;
            }
            let p0 = geo.point_of(from.min(geo.rom_length - 1));
            let p1 = geo.point_of(to.min(geo.rom_length - 1));
            let lift = 24.0f64.max((p1.0 - p0.0).abs() / 4.0 + (p1.1 - p0.1).abs() / 4.0);
            // Over the rows, or under them where there is no room above.
            let up = p0.1.min(p1.1) - lift;
            let mid = (
                (p0.0 + p1.0) / 2.0,
                if up > 4.0 { up } else { p0.1.max(p1.1) + lift },
            );
            set_source(cr, &with_alpha(if out { accent } else { &orange }, 0.85));
            cr.set_line_width((1.0 + f64::from(arc.calls).log2()).min(4.0));
            cr.move_to(p0.0, p0.1);
            cr.curve_to(mid.0, mid.1, mid.0, mid.1, p1.0, p1.1);
            let _ = cr.stroke();
            let end = if out { p1 } else { p0 };
            set_source(cr, if out { &orange } else { accent });
            cr.arc(end.0, end.1, 2.5, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
    }

    // MARK: Status

    fn show_status(&self) {
        self.status.set_text(&self.status_text());
    }

    fn status_text(&self) -> String {
        let geo = self.geo.borrow().clone();
        let zoom = if geo.ppb >= 1.0 {
            format!("{:.0} px a byte", geo.ppb)
        } else {
            format!("{:.0} bytes a pixel", 1.0 / geo.ppb.max(1e-6))
        };
        let Some(off) = self.hover.get().and_then(|h| geo.offset_at(h)) else {
            return format!(
                "{} banks, {zoom}. Pinch or Ctrl+scroll to zoom, scroll to move, double-click to open the listing.",
                geo.rows()
            );
        };
        let mut parts = vec![romlens_ffi::format_file_offset(off)];
        if let Some(a) = self.doc.rom.snes_address_for(off) {
            parts.push(romlens_ffi::format_snes_address(a));
        }
        let row_start = off / geo.bank_size * geo.bank_size;
        let b0 = (geo.origin.0 / geo.ppb).floor().max(0.0) as u32;
        let end = geo
            .bank_size
            .min(geo.rom_length - row_start)
            .min(((geo.origin.0 + geo.map_width()) / geo.ppb).ceil() as u32);
        if b0 < end {
            if geo.ppb >= ITEM_ZOOM
                && let Some(i) = self
                    .fetch_items(row_start + b0, end - b0)
                    .items
                    .into_iter()
                    .rfind(|i| i.offset <= off && off < i.offset + i.len)
            {
                parts.push(format!(
                    "{}, {} byte{}",
                    if i.instruction {
                        "instruction"
                    } else {
                        "data row"
                    },
                    i.len,
                    if i.len == 1 { "" } else { "s" }
                ));
                parts.push(format!(
                    "{} {}%",
                    strip::kind_name(i.kind_code),
                    (i.confidence * 100.0).round() as i32
                ));
            } else {
                let len = end - b0;
                let buckets = ((f64::from(len) * geo.ppb).ceil() as u32).clamp(1, len);
                if let Some(c) = self
                    .fetch_columns(row_start + b0, len, buckets)
                    .into_iter()
                    .find(|c| c.start <= off && off < c.start + c.len)
                {
                    parts.push(format!(
                        "{}{} {}%",
                        if c.mixed() { "mostly " } else { "" },
                        strip::kind_name(c.kind_code),
                        (c.confidence * 100.0).round() as i32
                    ));
                    parts.push(format!("{:.1} bits/byte", c.entropy));
                    if c.executed > 0.0 {
                        parts.push(format!("{:.0}% run", c.executed * 100.0));
                    }
                }
            }
        }
        parts.push(zoom);
        parts.join("  ·  ")
    }
}

fn hsv(h: f64, s: f64, v: f64) -> gdk::RGBA {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match (i as i32).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    gdk::RGBA::new(r as f32, g as f32, b as f32, 1.0)
}

fn paint(
    cr: &cairo::Context,
    c: &StripColumn,
    overlay: Overlay,
    r: (f64, f64, f64, f64),
    accent: &gdk::RGBA,
    fg: &gdk::RGBA,
) {
    let fill = |color: &gdk::RGBA| {
        set_source(cr, color);
        cr.rectangle(r.0, r.1, r.2, r.3);
        let _ = cr.fill();
    };
    match overlay {
        Overlay::Kind => {
            let base = palette::strip_color(c.kind_code, accent, fg);
            let alpha = if c.kind_code == 0 {
                base.alpha()
            } else {
                (0.35 + 0.65 * c.confidence) as f32
            };
            fill(&with_alpha(&base, alpha));
            if c.mixed() {
                hatch(cr, r);
            }
        }
        Overlay::Confidence => {
            if c.kind_code != 0 {
                fill(&hsv(0.33 * c.confidence, 0.65, 0.85));
            }
        }
        Overlay::Entropy => {
            let e = (c.entropy / 8.0).min(1.0);
            fill(&hsv(0.66 - 0.5 * e, 0.8, 0.35 + 0.6 * e));
        }
        Overlay::Coverage => {
            if c.executed > 0.0 {
                fill(&with_alpha(accent, (0.3 + 0.7 * c.executed) as f32));
            } else {
                fill(&with_alpha(fg, if c.kind_code == 1 { 0.18 } else { 0.06 }));
            }
        }
    }
}

fn hatch(cr: &cairo::Context, r: (f64, f64, f64, f64)) {
    let _ = cr.save();
    cr.rectangle(r.0, r.1, r.2, r.3);
    cr.clip();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
    cr.set_line_width(1.0);
    let mut x = r.0 - r.3;
    while x < r.0 + r.2 {
        cr.move_to(x, r.1 + r.3);
        cr.line_to(x + r.3, r.1);
        x += 4.0;
    }
    let _ = cr.stroke();
    let _ = cr.restore();
}
