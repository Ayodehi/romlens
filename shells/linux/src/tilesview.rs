//! The Tile Decoder (checklist 2.20, 2.21): one tile as bytes, planes,
//! indices and pixels side by side, and the sheet it sits in. Hovering a
//! pixel lights its bit in every plane and the bytes those bits live in; the
//! lookup is a table fetched once per format, so moving the pointer never
//! crosses into the core. The macOS twin is `TileDecoderView`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::cairo;
use romlens_ffi::{TileFormat, TileInfo};

use crate::canvas::with_alpha;
use crate::gfxdraw::*;
use crate::model::graphics::{self as gfx, PaletteChoice, Source};
use crate::model::{Change, Document};
use crate::pixels;
use romlens_ffi::StateRegion;

const CELL: f64 = 24.0;
const SHEET_SCALE: f64 = 3.0;

struct View {
    doc: Rc<Document>,
    zoom: gtk::DrawingArea,
    zoom_note: gtk::Label,
    indices: gtk::DrawingArea,
    planes: gtk::DrawingArea,
    bytes: gtk::DrawingArea,
    sheet: gtk::DrawingArea,
    sheet_note: gtk::Label,
    history: gtk::Box,
    palette_items: Cell<bool>,
    previous: gtk::Button,
    next: gtk::Button,
    formats: Vec<(TileFormat, gtk::ToggleButton)>,
    palette: gtk::DropDown,
    columns: gtk::SpinButton,
    tile: RefCell<Option<TileInfo>>,
    colours: RefCell<Vec<u32>>,
    surface: RefCell<Option<cairo::ImageSurface>>,
    hover: Cell<Option<(usize, usize)>>,
    syncing: Cell<bool>,
}

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let (root, view) = View::build(doc);
    root.connect_destroy(move |_| {
        let _ = &view;
    });
    root
}

fn area(w: i32, h: i32) -> gtk::DrawingArea {
    gtk::DrawingArea::builder()
        .content_width(w)
        .content_height(h)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .build()
}

fn titled(title: &str, child: &impl IsA<gtk::Widget>, note: Option<&gtk::Label>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    b.append(&caption(title));
    b.append(child);
    if let Some(n) = note {
        b.append(n);
    }
    b
}

impl View {
    fn build(doc: &Rc<Document>) -> (gtk::Widget, Rc<Self>) {
        let zoom = area((CELL * 8.0) as i32, (CELL * 8.0) as i32);
        let zoom_note = gtk::Label::builder().xalign(0.0).build();
        zoom_note.add_css_class("monospace");
        zoom_note.add_css_class("caption");
        let indices = area(8 * 22, 8 * 20);
        let planes = area(8 * 12 * 4, 8 * 12 * 2);
        let bytes = area(320, 200);
        let sheet = area(16 * 8 * SHEET_SCALE as i32, 8 * 8 * SHEET_SCALE as i32);
        let sheet_note = caption("");
        sheet_note.set_hexpand(true);
        let previous = gtk::Button::with_label("Previous");
        let next = gtk::Button::with_label("Next");

        let format_group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        format_group.add_css_class("linked");
        let formats: Vec<_> = [
            TileFormat::Bpp2,
            TileFormat::Bpp4,
            TileFormat::Bpp8,
            TileFormat::Mode7,
        ]
        .into_iter()
        .map(|f| {
            let b = gtk::ToggleButton::with_label(gfx::format_title(f));
            format_group.append(&b);
            (f, b)
        })
        .collect();
        let palette = gtk::DropDown::from_strings(&["Grayscale", "Colours at Tile Start in ROM"]);
        let history = gtk::Box::new(gtk::Orientation::Vertical, 0);
        palette.set_tooltip_text(Some("Where the colours come from"));
        let columns = gtk::SpinButton::with_range(1.0, 32.0, 1.0);
        columns.set_tooltip_text(Some("Tiles across the sheet"));
        let across = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        across.append(&columns);
        across.append(&gtk::Label::new(Some("across")));

        let controls = gtk::Box::builder().spacing(16).build();
        controls.append(&format_group);
        controls.append(&palette);
        controls.append(&across);

        let tile_row = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .column_spacing(20)
            .row_spacing(12)
            .homogeneous(false)
            .max_children_per_line(4)
            .build();
        tile_row.insert(&titled("Pixels", &zoom, Some(&zoom_note)), -1);
        tile_row.insert(&titled("Indices", &indices, None), -1);
        tile_row.insert(&titled("Bitplanes", &planes, None), -1);
        tile_row.insert(&titled("Bytes", &bytes, None), -1);

        let sheet_head = gtk::Box::builder().spacing(6).build();
        sheet_head.append(&sheet_note);
        sheet_head.append(&previous);
        sheet_head.append(&next);

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .margin_start(16)
            .margin_end(16)
            .margin_top(12)
            .margin_bottom(16)
            .build();
        content.append(&controls);
        content.append(&tile_row);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content.append(&sheet_head);
        content.append(&history);
        content.append(&sheet);
        let scroll = gtk::ScrolledWindow::builder().child(&content).build();

        let view = Rc::new(Self {
            doc: Rc::clone(doc),
            zoom,
            zoom_note,
            indices,
            planes,
            bytes,
            sheet,
            sheet_note,
            history,
            palette_items: Cell::new(false),
            previous,
            next,
            formats,
            palette,
            columns,
            tile: RefCell::new(None),
            colours: RefCell::new(Vec::new()),
            surface: RefCell::new(None),
            hover: Cell::new(None),
            syncing: Cell::new(false),
        });
        view.wire();
        view.refresh();
        (scroll.upcast(), view)
    }

    fn wire(self: &Rc<Self>) {
        for (f, button) in &self.formats {
            let (this, f) = (Rc::downgrade(self), *f);
            button.connect_clicked(move |b| {
                // Radio behaviour without a group: a click always selects.
                b.set_active(true);
                if let Some(v) = this.upgrade()
                    && !v.syncing.get()
                {
                    v.doc.edit_graphics(|g| {
                        g.format = f;
                        g.select_tile(g.selected_tile)
                    });
                }
            });
        }
        let this = Rc::downgrade(self);
        self.palette.connect_selected_notify(move |d| {
            if let Some(v) = this.upgrade()
                && !v.syncing.get()
            {
                let sel = d.selected();
                v.doc.edit_graphics(|g| {
                    g.palette = match sel {
                        0 => PaletteChoice::Grayscale,
                        1 => PaletteChoice::Rom(g.rom_offset),
                        n => PaletteChoice::Cgram((n - 2) as u8),
                    };
                    None
                });
            }
        });
        let this = Rc::downgrade(self);
        self.columns.connect_value_changed(move |s| {
            if let Some(v) = this.upgrade()
                && !v.syncing.get()
            {
                v.doc.edit_graphics(|g| {
                    g.columns = s.value() as usize;
                    None
                });
            }
        });
        for (button, delta) in [(&self.previous, -1), (&self.next, 1)] {
            let this = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(v) = this.upgrade() {
                    v.doc.edit_graphics(|g| g.page(delta));
                }
            });
        }

        // Draw functions.
        for (area, which) in [
            (&self.zoom, 0),
            (&self.indices, 1),
            (&self.planes, 2),
            (&self.bytes, 3),
            (&self.sheet, 4),
        ] {
            let this = Rc::downgrade(self);
            area.set_draw_func(move |a, cr, w, h| {
                if let Some(v) = this.upgrade() {
                    match which {
                        0 => v.draw_zoom(cr),
                        1 => v.draw_indices(a, cr),
                        2 => v.draw_planes(a, cr),
                        3 => v.draw_bytes(a, cr),
                        _ => v.draw_sheet(cr, f64::from(w), f64::from(h)),
                    }
                }
            });
        }

        // Hover a pixel: the other panes light its bits.
        let motion = gtk::EventControllerMotion::new();
        let this = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            if let Some(v) = this.upgrade() {
                let (px, py) = ((x / CELL) as usize, (y / CELL) as usize);
                v.set_hover((px < 8 && py < 8).then_some((px, py)));
            }
        });
        let this = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(v) = this.upgrade() {
                v.set_hover(None);
            }
        });
        self.zoom.add_controller(motion);

        // Click a tile of the sheet.
        let click = gtk::GestureClick::new();
        let this = Rc::downgrade(self);
        click.connect_pressed(move |_, _, x, y| {
            let Some(v) = this.upgrade() else { return };
            let s = 8.0 * SHEET_SCALE;
            let (col, row) = ((x / s) as usize, (y / s) as usize);
            let (columns, tiles) = {
                let g = v.doc.graphics();
                (g.columns, g.sheet_tiles)
            };
            let index = row * columns + col;
            if col < columns && index < tiles {
                v.doc.edit_graphics(|g| g.select_tile(index));
            }
        });
        self.sheet.add_controller(click);

        let this = Rc::downgrade(self);
        self.doc.subscribe(move |c| {
            if matches!(c, Change::Graphics | Change::Rows)
                && let Some(v) = this.upgrade()
            {
                v.refresh();
            }
        });
        let this = Rc::downgrade(self);
        adw::StyleManager::default().connect_accent_color_notify(move |_| {
            if let Some(v) = this.upgrade() {
                v.redraw();
            }
        });
    }

    fn set_hover(&self, h: Option<(usize, usize)>) {
        if self.hover.get() == h {
            return;
        }
        self.hover.set(h);
        match (h, self.tile.borrow().as_ref()) {
            (Some((x, y)), Some(t)) => {
                let index = t.indices[y * 8 + x];
                let n = gfx::bits_per_pixel(t.format);
                let bits: Vec<String> = (0..n)
                    .rev()
                    .map(|p| format!("p{p}={}", index >> p & 1))
                    .collect();
                self.zoom_note
                    .set_text(&format!("({x}, {y}) = index {index}, {}", bits.join(" ")));
            }
            _ => self.zoom_note.set_text("Hover a pixel"),
        }
        self.redraw();
    }

    fn redraw(&self) {
        for a in [
            &self.zoom,
            &self.indices,
            &self.planes,
            &self.bytes,
            &self.sheet,
        ] {
            a.queue_draw();
        }
    }

    // MARK: Updating

    fn refresh(self: &Rc<Self>) {
        if !self.doc.shows_graphics(gfx::Tab::Tiles) {
            return;
        }
        let g = self.doc.graphics();
        *self.tile.borrow_mut() = Some(g.selected_tile_info());
        *self.colours.borrow_mut() = g.palette_rgb(gfx::colours(g.format));
        *self.surface.borrow_mut() = pixels::surface(&g.sheet());

        self.syncing.set(true);
        for (f, b) in &self.formats {
            b.set_active(*f == g.format);
        }
        // A recording adds its sixteen CGRAM rows to the palette choices.
        if self.palette_items.get() != g.has_recording() {
            self.palette_items.set(g.has_recording());
            let mut items = vec![
                "Grayscale".to_owned(),
                "Colours at Tile Start in ROM".to_owned(),
            ];
            if g.has_recording() {
                items.extend((0..16).map(|r| {
                    if r < 8 {
                        format!("CGRAM: BG palette {r}")
                    } else {
                        format!("CGRAM: OBJ palette {}", r - 8)
                    }
                }));
            }
            let refs: Vec<&str> = items.iter().map(String::as_str).collect();
            self.palette.set_model(Some(&gtk::StringList::new(&refs)));
        }
        self.palette.set_selected(match g.palette {
            PaletteChoice::Grayscale => 0,
            PaletteChoice::Rom(_) => 1,
            PaletteChoice::Cgram(row) => 2 + u32::from(row),
        });
        self.columns.set_value(g.columns as f64);
        self.syncing.set(false);

        let rom = g.source == Source::Rom;
        self.previous.set_visible(rom);
        self.next.set_visible(rom);
        self.previous.set_sensitive(g.rom_offset > 0);
        self.sheet_note.set_text(&format!(
            "Tile {} of {}, {} bytes each",
            g.selected_tile,
            g.sheet_tiles,
            g.tile_len()
        ));
        let tile_len = g.tile_len();
        let bpp = gfx::bits_per_pixel(g.format);
        let (columns, tiles) = (g.columns, g.sheet_tiles);
        let history = (g.source == Source::Recording).then(|| {
            (
                g.vram_offset + (g.selected_tile * tile_len) as u32,
                tile_len as u32,
            )
        });
        drop(g);
        while let Some(c) = self.history.first_child() {
            self.history.remove(&c);
        }
        if let Some((offset, len)) = history
            && let Some(row) = history_row(&self.doc, StateRegion::Vram, offset, len)
        {
            self.history.append(&row);
        }

        // The bytes and planes areas size to the format.
        let per_row = if self.doc.graphics().format == TileFormat::Mode7 {
            8
        } else {
            2
        };
        self.bytes
            .set_content_height(((tile_len / per_row) as i32 + 2) * 20);
        self.planes
            .set_content_width((bpp.min(4) as i32) * (8 * 9 + 16));
        self.planes
            .set_content_height(bpp.div_ceil(4) as i32 * (8 * 9 + 22));
        self.sheet
            .set_content_width((columns as f64 * 8.0 * SHEET_SCALE) as i32);
        self.sheet
            .set_content_height((tiles.div_ceil(columns.max(1)) as f64 * 8.0 * SHEET_SCALE) as i32);
        self.hover.set(None);
        self.zoom_note.set_text("Hover a pixel");
        self.redraw();
    }

    // MARK: Drawing

    fn draw_zoom(&self, cr: &cairo::Context) {
        let tile = self.tile.borrow();
        let colours = self.colours.borrow();
        let Some(t) = tile.as_ref() else { return };
        if colours.is_empty() {
            return;
        }
        for y in 0..8 {
            for x in 0..8 {
                let i = usize::from(t.indices[y * 8 + x]);
                rgb(cr, colours[i % colours.len()]);
                cr.rectangle(x as f64 * CELL, y as f64 * CELL, CELL, CELL);
                let _ = cr.fill();
            }
        }
        if let Some((x, y)) = self.hover.get() {
            outline(
                cr,
                x as f64 * CELL,
                y as f64 * CELL,
                CELL,
                CELL,
                &accent(),
                2.0,
            );
        }
        outline(
            cr,
            0.0,
            0.0,
            CELL * 8.0,
            CELL * 8.0,
            &with_alpha(&self.zoom.color(), 0.4),
            1.0,
        );
    }

    fn draw_indices(&self, a: &gtk::DrawingArea, cr: &cairo::Context) {
        let tile = self.tile.borrow();
        let Some(t) = tile.as_ref() else { return };
        let fg = a.color();
        let wide = gfx::bits_per_pixel(t.format) == 8;
        for y in 0..8 {
            for x in 0..8 {
                let v = t.indices[y * 8 + x];
                let (cx, cy) = (x as f64 * 22.0, y as f64 * 20.0);
                if self.hover.get() == Some((x, y)) {
                    fill_rect(cr, cx, cy, 22.0, 20.0, &with_alpha(&accent(), 0.3));
                }
                let l = mono_layout(a, 10.5, &hex(v, if wide { 2 } else { 1 }));
                let alpha = if v == 0 { 0.4 } else { 1.0 };
                text(cr, &l, cx + 4.0, cy + 1.0, &with_alpha(&fg, alpha));
            }
        }
    }

    fn draw_planes(&self, a: &gtk::DrawingArea, cr: &cairo::Context) {
        let tile = self.tile.borrow();
        let Some(t) = tile.as_ref() else { return };
        let fg = a.color();
        let n = gfx::bits_per_pixel(t.format);
        let (bw, bh) = (8.0 * 9.0 + 16.0, 8.0 * 9.0 + 22.0);
        for plane in 0..n {
            let (ox, oy) = ((plane % 4) as f64 * bw, (plane / 4) as f64 * bh);
            let l = mono_layout(a, 8.5, &format!("plane {plane}"));
            text(cr, &l, ox, oy, &with_alpha(&fg, 0.6));
            for y in 0..8 {
                for x in 0..8 {
                    let on = t.indices[y * 8 + x] >> plane & 1 == 1;
                    let (px, py) = (ox + x as f64 * 9.0, oy + 16.0 + y as f64 * 9.0);
                    fill_rect(
                        cr,
                        px,
                        py,
                        8.0,
                        8.0,
                        &with_alpha(&fg, if on { 0.9 } else { 0.12 }),
                    );
                    if self.hover.get() == Some((x, y)) {
                        outline(cr, px - 1.0, py - 1.0, 10.0, 10.0, &accent(), 2.0);
                    }
                }
            }
        }
    }

    fn draw_bytes(&self, a: &gtk::DrawingArea, cr: &cairo::Context) {
        let tile = self.tile.borrow();
        let Some(t) = tile.as_ref() else { return };
        let fg = a.color();
        let hot: Vec<usize> = match self.hover.get() {
            Some((x, y)) => {
                let g = self.doc.graphics();
                (0..gfx::bits_per_pixel(t.format))
                    .map(|p| g.bit_source(x, y, p).0)
                    .collect()
            }
            None => Vec::new(),
        };
        let per_row = if t.format == TileFormat::Mode7 { 8 } else { 2 };
        for row in 0..t.bytes.len() / per_row {
            let y = row as f64 * 20.0;
            let l = mono_layout(a, 9.5, &hex(row as u64 * per_row as u64, 2));
            text(cr, &l, 0.0, y + 2.0, &with_alpha(&fg, 0.45));
            for col in 0..per_row {
                let i = row * per_row + col;
                let x = 28.0 + col as f64 * 22.0;
                if hot.contains(&i) {
                    fill_rect(cr, x - 2.0, y, 22.0, 19.0, &with_alpha(&accent(), 0.35));
                }
                let l = mono_layout(a, 10.5, &hex(t.bytes[i], 2));
                text(cr, &l, x, y + 1.0, &fg);
            }
        }
        if t.format != TileFormat::Mode7 {
            let l = mono_layout(a, 8.0, "Rows 0-7 of planes 0 and 1, then 2 and 3...");
            text(
                cr,
                &l,
                0.0,
                (t.bytes.len() / per_row) as f64 * 20.0 + 4.0,
                &with_alpha(&fg, 0.45),
            );
        }
    }

    fn draw_sheet(&self, cr: &cairo::Context, w: f64, h: f64) {
        checker(cr, w, h, &self.sheet.color());
        if let Some(s) = self.surface.borrow().as_ref() {
            paint_surface(cr, s, SHEET_SCALE);
        }
        let g = self.doc.graphics();
        let s = 8.0 * SHEET_SCALE;
        let columns = g.columns.max(1);
        // Tiles whose bytes changed since the previous frame, outlined.
        if g.source == Source::Recording {
            let (len, base) = (g.tile_len(), g.vram_offset as usize);
            for i in
                (0..g.sheet_tiles).filter(|i| g.changed(StateRegion::Vram, base + i * len, len))
            {
                outline(
                    cr,
                    (i % columns) as f64 * s + 1.0,
                    (i / columns) as f64 * s + 1.0,
                    s - 2.0,
                    s - 2.0,
                    &change_colour(),
                    1.5,
                );
            }
        }
        let (col, row) = (g.selected_tile % columns, g.selected_tile / columns);
        outline(cr, col as f64 * s, row as f64 * s, s, s, &accent(), 2.0);
    }
}
