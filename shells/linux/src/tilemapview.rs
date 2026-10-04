//! A BG tilemap (checklist 2.24): each cell's `vhopppcc cccccccc` decoded,
//! shown as tile numbers over ROM bytes. The macOS twin is `TilemapView` and
//! `CellDetail`.
//!
//! The rendered layer a recording supplies (2.26) arrives with the recording
//! work later in L3.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use romlens_ffi::TilemapCellInfo;

use crate::canvas::with_alpha;
use crate::gfxdraw::*;
use crate::model::graphics::{self as gfx, SCREEN_SIZES, Source};
use crate::model::{Change, Document};
use crate::pixels;
use romlens_ffi::StateRegion;

struct View {
    doc: Rc<Document>,
    map: gtk::DrawingArea,
    scroll: gtk::ScrolledWindow,
    detail: gtk::Box,
    size: gtk::DropDown,
    scale: gtk::DropDown,
    cells: RefCell<Vec<TilemapCellInfo>>,
    image: RefCell<Option<gtk::cairo::ImageSurface>>,
    /// Map pixels a cell: 8, or 16 for 16x16 tiles.
    unit: std::cell::Cell<f64>,
    layers: Vec<gtk::ToggleButton>,
    layer_box: gtk::Box,
    info: gtk::Label,
    hint: gtk::Label,
    syncing: std::cell::Cell<bool>,
}

/// The size choices: 0 fits the window.
const SCALES: [(i32, &str); 6] = [
    (0, "Fit"),
    (1, "1:1"),
    (2, "2×"),
    (3, "3×"),
    (4, "4×"),
    (8, "8×"),
];

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let (root, view) = View::build(doc);
    root.connect_destroy(move |_| {
        let _ = &view;
    });
    root
}

impl View {
    fn build(doc: &Rc<Document>) -> (gtk::Widget, Rc<Self>) {
        let size = gtk::DropDown::from_strings(&SCREEN_SIZES.map(gfx::screen_title));
        let scale = gtk::DropDown::from_strings(&SCALES.map(|(_, t)| t));
        scale.set_tooltip_text(Some(
            "Fit the whole map in the window, or draw each map pixel as 1 to 8 screen pixels",
        ));
        let hint = caption(
            "Tile numbers; set a tile address on the marked range to see it drawn in the inspector",
        );
        hint.set_hexpand(true);
        let controls = gtk::Box::builder()
            .spacing(12)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        let layer_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        layer_box.add_css_class("linked");
        let layers: Vec<gtk::ToggleButton> = (1..=4)
            .map(|n| {
                let b = gtk::ToggleButton::with_label(&format!("BG{n}"));
                layer_box.append(&b);
                b
            })
            .collect();
        let info = caption("");
        info.set_hexpand(true);
        controls.append(&layer_box);
        controls.append(&size);
        controls.append(&info);
        controls.append(&hint);
        controls.append(&scale);

        let map = gtk::DrawingArea::builder()
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .margin_bottom(12)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&map)
            .build();
        let detail = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_start(12)
            .margin_end(12)
            .margin_top(10)
            .width_request(240)
            .build();
        let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        split.append(&scroll);
        split.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        split.append(&detail);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&controls);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&split);

        let view = Rc::new(Self {
            doc: Rc::clone(doc),
            map,
            scroll,
            detail,
            size,
            scale,
            cells: RefCell::new(Vec::new()),
            image: RefCell::new(None),
            unit: std::cell::Cell::new(8.0),
            layers,
            layer_box,
            info,
            hint,
            syncing: std::cell::Cell::new(false),
        });
        view.wire();
        view.refresh();
        (root.upcast(), view)
    }

    /// Screen pixels a cell takes: 8 map pixels times the scale, or fitted.
    fn cell_size(&self) -> f64 {
        let scale = self.doc.graphics().tilemap_scale;
        let (cols, rows) = self.doc.graphics().map_cells();
        if scale > 0 {
            return self.unit.get() * f64::from(scale);
        }
        let (w, h) = (
            f64::from(self.scroll.width() - 24),
            f64::from(self.scroll.height() - 24),
        );
        (w / cols as f64)
            .min(h / rows as f64)
            .clamp(2.0, 64.0)
            .floor()
    }

    fn wire(self: &Rc<Self>) {
        let this = Rc::downgrade(self);
        self.map.set_draw_func(move |a, cr, _, _| {
            if let Some(v) = this.upgrade() {
                v.draw(a, cr);
            }
        });
        let this = Rc::downgrade(self);
        self.size.connect_selected_notify(move |d| {
            if let Some(v) = this.upgrade()
                && !v.syncing.get()
            {
                let s = SCREEN_SIZES[d.selected() as usize];
                v.doc.edit_graphics(|g| {
                    g.screen_size = s;
                    g.selected_cell = None;
                    None
                });
            }
        });
        let this = Rc::downgrade(self);
        self.scale.connect_selected_notify(move |d| {
            if let Some(v) = this.upgrade()
                && !v.syncing.get()
            {
                let s = SCALES[d.selected() as usize].0;
                v.doc.edit_graphics(|g| {
                    g.tilemap_scale = s;
                    None
                });
            }
        });
        for (i, b) in self.layers.iter().enumerate() {
            let this = Rc::downgrade(self);
            b.connect_clicked(move |b| {
                b.set_active(true);
                if let Some(v) = this.upgrade()
                    && !v.syncing.get()
                {
                    v.doc.edit_graphics(|g| {
                        g.background_layer = i as u8 + 1;
                        g.selected_cell = None;
                        None
                    });
                }
            });
        }
        let click = gtk::GestureClick::new();
        let this = Rc::downgrade(self);
        click.connect_pressed(move |_, _, x, y| {
            let Some(v) = this.upgrade() else { return };
            let cell = v.cell_size();
            let (col, row) = ((x / cell) as u32, (y / cell) as u32);
            let found = v
                .cells
                .borrow()
                .iter()
                .position(|c| c.col == col && c.row == row);
            if let Some(i) = found {
                v.doc.edit_graphics(|g| g.select_cell(i));
            }
        });
        self.map.add_controller(click);
        let this = Rc::downgrade(self);
        self.scroll
            .connect_notify_local(Some("width"), move |_, _| {
                if let Some(v) = this.upgrade()
                    && v.doc.graphics().tilemap_scale == 0
                {
                    v.resize();
                }
            });
        let this = Rc::downgrade(self);
        self.doc.subscribe(move |c| {
            if matches!(c, Change::Graphics | Change::Rows | Change::Layout)
                && let Some(v) = this.upgrade()
            {
                v.refresh();
            }
        });
        let this = Rc::downgrade(self);
        adw::StyleManager::default().connect_accent_color_notify(move |_| {
            if let Some(v) = this.upgrade() {
                v.map.queue_draw();
            }
        });
    }

    fn resize(&self) {
        let cell = self.cell_size();
        let (cols, rows) = self.doc.graphics().map_cells();
        self.map.set_content_width((cols as f64 * cell) as i32);
        self.map.set_content_height((rows as f64 * cell) as i32);
        self.map.queue_draw();
    }

    fn refresh(&self) {
        if self.doc.graphics_tab() != Some(gfx::Tab::Tilemap) {
            return;
        }
        let (recording, size, scale, layer, info, image) = {
            let g = self.doc.graphics();
            let recording = g.source == Source::Recording;
            let info = if recording {
                match (g.current_layer(), g.ppu()) {
                    (Some(_), _) if g.is_mode7() => "Mode 7: the 128×128 plane at $0000, 8 bpp, drawn untransformed (the M7A-M7D rotation and scaling are not applied)".to_owned(),
                    (Some(l), Some(ppu)) => match l.format {
                        Some(f) => format!(
                            "Mode {}: {}, {}, map ${}, tiles ${}{}",
                            ppu.bg_mode,
                            gfx::format_title(f),
                            gfx::screen_title(l.size),
                            hex(l.map_word, 4),
                            hex(l.char_word, 4),
                            if l.tile16 { ", 16×16" } else { "" }
                        ),
                        None => format!("Mode {} has no BG{}", ppu.bg_mode, g.background_layer),
                    },
                    _ => String::new(),
                }
            } else {
                String::new()
            };
            (
                recording,
                g.screen_size,
                g.tilemap_scale,
                g.background_layer,
                info,
                g.layer_image(),
            )
        };
        *self.cells.borrow_mut() = self.doc.graphics().cells();
        // Map pixels a cell: 8, or 16 for 16x16 tiles.
        let (cols, _) = self.doc.graphics().map_cells();
        self.unit.set(
            image
                .as_ref()
                .map_or(8.0, |i| f64::from(i.width) / cols.max(1) as f64),
        );
        *self.image.borrow_mut() = image.as_ref().and_then(pixels::surface);
        self.syncing.set(true);
        self.layer_box.set_visible(recording);
        self.size.set_visible(!recording);
        self.hint.set_visible(!recording);
        self.info.set_visible(recording);
        self.info.set_text(&info);
        for (i, b) in self.layers.iter().enumerate() {
            b.set_active(i as u8 + 1 == layer);
        }
        self.size
            .set_selected(SCREEN_SIZES.iter().position(|s| *s == size).unwrap_or(0) as u32);
        self.scale
            .set_selected(SCALES.iter().position(|(s, _)| *s == scale).unwrap_or(2) as u32);
        self.syncing.set(false);
        self.resize();
        self.show_detail();
    }

    fn draw(&self, a: &gtk::DrawingArea, cr: &gtk::cairo::Context) {
        let cell = self.cell_size();
        let fg = a.color();
        let (cols, rows) = self.doc.graphics().map_cells();
        let cells = self.cells.borrow();
        let image = self.image.borrow();
        if let Some(s) = image.as_ref() {
            // The rendered layer, a map pixel to `cell / unit` screen pixels.
            paint_surface(cr, s, cell / self.unit.get());
        }
        for c in cells.iter().filter(|_| image.is_none()) {
            let (x, y) = (f64::from(c.col) * cell, f64::from(c.row) * cell);
            // A hue per palette, so a map's regions show at a glance.
            let hue = f64::from(c.palette) / 8.0;
            let (r, g, b) = hsv(hue, 0.25, 0.95);
            cr.set_source_rgba(r, g, b, 0.5);
            cr.rectangle(x, y, cell, cell);
            let _ = cr.fill();
            // Too small to read below 12 pixels a cell.
            if cell >= 12.0 {
                let l = mono_layout(a, cell * 0.28, &hex(c.tile, 3));
                let (w, h) = l.pixel_size();
                text(
                    cr,
                    &l,
                    x + (cell - f64::from(w)) / 2.0,
                    y + (cell - f64::from(h)) / 2.0,
                    &fg,
                );
            }
        }
        if cell >= 12.0 {
            cr.set_source_rgba(
                f64::from(fg.red()),
                f64::from(fg.green()),
                f64::from(fg.blue()),
                0.25,
            );
            cr.set_line_width(0.5);
            for c in 0..=cols {
                cr.move_to(c as f64 * cell, 0.0);
                cr.line_to(c as f64 * cell, rows as f64 * cell);
            }
            for r in 0..=rows {
                cr.move_to(0.0, r as f64 * cell);
                cr.line_to(cols as f64 * cell, r as f64 * cell);
            }
            let _ = cr.stroke();
        }
        {
            let g = self.doc.graphics();
            for c in cells.iter() {
                if let Some((offset, len)) = g.cell_vram(c)
                    && g.changed(StateRegion::Vram, offset as usize, len as usize)
                {
                    outline(
                        cr,
                        f64::from(c.col) * cell + 1.0,
                        f64::from(c.row) * cell + 1.0,
                        cell - 2.0,
                        cell - 2.0,
                        &change_colour(),
                        1.5,
                    );
                }
            }
        }
        if let Some(i) = self.doc.graphics().selected_cell
            && let Some(c) = cells.get(i)
        {
            outline(
                cr,
                f64::from(c.col) * cell,
                f64::from(c.row) * cell,
                cell,
                cell,
                &accent(),
                2.0,
            );
        }
        let _ = with_alpha;
    }

    fn show_detail(&self) {
        while let Some(c) = self.detail.first_child() {
            self.detail.remove(&c);
        }
        let g = self.doc.graphics();
        let cells = self.cells.borrow();
        let Some(c) = g.selected_cell.and_then(|i| cells.get(i)) else {
            self.detail.append(&caption("Click a cell"));
            return;
        };
        let title = gtk::Label::builder()
            .label(format!("Cell ({}, {})", c.col, c.row))
            .xalign(0.0)
            .build();
        title.add_css_class("heading");
        self.detail.append(&title);
        let mode7 = g.is_mode7();
        let rows: Vec<(&str, String)> = if mode7 {
            // A Mode 7 entry is one byte: the tile number and nothing else.
            vec![
                ("Tile", format!("${}", hex(c.tile, 2))),
                ("VRAM", format!("${}", hex(c.byte_offset, 4))),
            ]
        } else {
            let flips = [c.hflip.then_some("h"), c.vflip.then_some("v")]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            vec![
                ("Entry", format!("${}", hex(c.raw, 4))),
                ("Tile", format!("${}", hex(c.tile, 3))),
                ("Palette", c.palette.to_string()),
                (
                    "Priority",
                    if c.priority { "high" } else { "low" }.to_owned(),
                ),
                (
                    "Flips",
                    if flips.is_empty() {
                        "none".to_owned()
                    } else {
                        flips
                    },
                ),
                ("Bytes", format!("+${}", hex(c.byte_offset, 4))),
            ]
        };
        let grid = gtk::Grid::builder()
            .column_spacing(10)
            .row_spacing(4)
            .build();
        for (n, (k, v)) in rows.iter().enumerate() {
            let kl = gtk::Label::builder().label(*k).xalign(0.0).build();
            kl.add_css_class("dim-label");
            let vl = gtk::Label::builder().label(v).xalign(0.0).build();
            vl.add_css_class("monospace");
            grid.attach(&kl, 0, n as i32, 1, 1);
            grid.attach(&vl, 1, n as i32, 1, 1);
        }
        self.detail.append(&grid);
        if mode7 {
            let note = caption(
                "Low byte of word row × 128 + column; the tile's pixels are the high bytes of words 64 × tile onward",
            );
            note.set_wrap(true);
            self.detail.append(&note);
        } else {
            self.detail.append(&caption("vhopppcc cccccccc"));
            if g.source == Source::Recording
                && let Some(format) = g.current_layer().and_then(|l| l.format)
                && let Some(layer) = g.current_layer()
            {
                let button = gtk::Button::with_label("Show Tile in Decoder");
                button.set_halign(gtk::Align::Start);
                let (doc, tile) = (Rc::clone(&self.doc), c.tile);
                button.connect_clicked(move |_| {
                    doc.edit_graphics(|g| {
                        g.format = format;
                        g.vram_offset = u32::from(layer.char_word) * 2
                            + u32::from(tile) * romlens_ffi::tile_byte_len(format);
                        g.select_tile(0);
                        None
                    });
                    doc.open_graphics(gfx::Tab::Tiles);
                });
                self.detail.append(&button);
            }
        }
        if let Some((offset, len)) = g.cell_vram(c) {
            drop(g);
            if let Some(row) = history_row(&self.doc, StateRegion::Vram, offset, len) {
                self.detail.append(&row);
            }
        }
    }
}

/// HSV to RGB, each in 0 to 1.
fn hsv(h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    match (i as i32).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}
