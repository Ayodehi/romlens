//! CGRAM as sixteen rows of sixteen (checklist 2.22). Clicking a swatch shows
//! its bit fields and, reading ROM, selects its two bytes. The macOS twin is
//! `PaletteView` and `ColourDetail`.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use romlens_ffi::PaletteEntryInfo;

use crate::canvas::with_alpha;
use crate::gfxdraw::*;
use crate::model::graphics as gfx;
use crate::model::{Change, Document};
use romlens_ffi::StateRegion;

const SWATCH: f64 = 22.0;
const GAP: f64 = 2.0;
const LABEL: f64 = 52.0;

struct View {
    doc: Rc<Document>,
    swatches: gtk::DrawingArea,
    detail: gtk::Box,
    entries: RefCell<Vec<PaletteEntryInfo>>,
}

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let (root, view) = View::build(doc);
    root.connect_destroy(move |_| {
        let _ = &view;
    });
    root
}

fn label(row: usize) -> String {
    if row < 8 {
        format!("BG {row}")
    } else {
        format!("OBJ {}", row - 8)
    }
}

impl View {
    fn build(doc: &Rc<Document>) -> (gtk::Widget, Rc<Self>) {
        let swatches = gtk::DrawingArea::builder()
            .content_width((LABEL + 16.0 * (SWATCH + GAP)) as i32)
            .content_height((16.0 * (SWATCH + GAP)) as i32)
            .valign(gtk::Align::Start)
            .build();
        let detail = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .valign(gtk::Align::Start)
            .hexpand(true)
            .build();
        let row = gtk::Box::builder()
            .spacing(24)
            .margin_start(16)
            .margin_end(16)
            .margin_top(12)
            .margin_bottom(16)
            .build();
        row.append(&swatches);
        row.append(&detail);
        let scroll = gtk::ScrolledWindow::builder().child(&row).build();
        let view = Rc::new(Self {
            doc: Rc::clone(doc),
            swatches,
            detail,
            entries: RefCell::new(Vec::new()),
        });
        view.wire();
        view.refresh();
        (scroll.upcast(), view)
    }

    fn wire(self: &Rc<Self>) {
        let this = Rc::downgrade(self);
        self.swatches.set_draw_func(move |a, cr, _, _| {
            if let Some(v) = this.upgrade() {
                v.draw(a, cr);
            }
        });
        let click = gtk::GestureClick::new();
        let this = Rc::downgrade(self);
        click.connect_pressed(move |_, _, x, y| {
            let Some(v) = this.upgrade() else { return };
            let (col, row) = (
                ((x - LABEL) / (SWATCH + GAP)).floor(),
                (y / (SWATCH + GAP)).floor(),
            );
            if (0.0..16.0).contains(&col) && (0.0..16.0).contains(&row) {
                let i = row as usize * 16 + col as usize;
                v.doc.edit_graphics(|g| g.select_colour(i));
            }
        });
        self.swatches.add_controller(click);
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
                v.swatches.queue_draw();
            }
        });
    }

    fn refresh(&self) {
        if self.doc.graphics_tab() != Some(gfx::Tab::Palette) {
            return;
        }
        *self.entries.borrow_mut() = self.doc.graphics().colours();
        self.swatches.queue_draw();
        while let Some(c) = self.detail.first_child() {
            self.detail.remove(&c);
        }
        let selected = self.doc.graphics().selected_colour;
        match selected.and_then(|i| self.entries.borrow().get(i).cloned()) {
            Some(e) => {
                self.detail.append(&colour_detail(&e));
                if let Some(row) =
                    history_row(&self.doc, StateRegion::Cgram, (e.index * 2).into(), 2)
                {
                    self.detail.append(&row);
                }
            }
            None => {
                let l = gtk::Label::builder()
                    .label("Click a swatch")
                    .xalign(0.0)
                    .build();
                l.add_css_class("dim-label");
                self.detail.append(&l);
            }
        }
    }

    fn draw(&self, a: &gtk::DrawingArea, cr: &gtk::cairo::Context) {
        let fg = a.color();
        let g = self.doc.graphics();
        let selected = g.selected_colour;
        let changed = g
            .change(StateRegion::Cgram)
            .map(|c| c.bytes)
            .unwrap_or_default();
        let entries = self.entries.borrow();
        for row in 0..16usize {
            let y = row as f64 * (SWATCH + GAP);
            let l = mono_layout(a, 8.5, &label(row));
            let (w, _) = l.pixel_size();
            text(
                cr,
                &l,
                LABEL - 6.0 - f64::from(w),
                y + 5.0,
                &with_alpha(&fg, 0.6),
            );
            for col in 0..16usize {
                let i = row * 16 + col;
                let x = LABEL + col as f64 * (SWATCH + GAP);
                if let Some(e) = entries.get(i) {
                    rgb(cr, e.rgb);
                    cr.rectangle(x, y, SWATCH, SWATCH);
                    let _ = cr.fill();
                    if e.unused_bit {
                        let l = mono_layout(a, 9.0, "!");
                        text(cr, &l, x + 8.0, y + 4.0, &gdk_red());
                    }
                }
                if changed.contains(&(i * 2)) || changed.contains(&(i * 2 + 1)) {
                    cr.set_source_rgb(0.996, 0.584, 0.0);
                    cr.arc(x + SWATCH - 2.0, y + 2.0, 3.0, 0.0, std::f64::consts::TAU);
                    let _ = cr.fill();
                }
                if selected == Some(i) {
                    outline(cr, x, y, SWATCH, SWATCH, &accent(), 2.0);
                } else {
                    outline(cr, x, y, SWATCH, SWATCH, &with_alpha(&fg, 0.3), 0.5);
                }
            }
        }
    }
}

fn gdk_red() -> gtk::gdk::RGBA {
    gtk::gdk::RGBA::new(1.0, 0.13, 0.12, 1.0)
}

/// One entry decoded: the raw word, the three five-bit fields, and the
/// eight-bit colour they expand to.
fn colour_detail(e: &PaletteEntryInfo) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let chip = gtk::DrawingArea::builder()
        .content_width(64)
        .content_height(64)
        .halign(gtk::Align::Start)
        .build();
    let c = e.rgb;
    chip.set_draw_func(move |a, cr, w, h| {
        rgb(cr, c);
        cr.rectangle(0.0, 0.0, f64::from(w), f64::from(h));
        let _ = cr.fill();
        outline(
            cr,
            0.0,
            0.0,
            f64::from(w),
            f64::from(h),
            &with_alpha(&a.color(), 0.4),
            1.0,
        );
    });
    b.append(&chip);
    let grid = gtk::Grid::builder()
        .column_spacing(12)
        .row_spacing(4)
        .build();
    let bits = {
        let s = format!("{:016b}", e.raw);
        format!("{} {} {} {}", &s[0..1], &s[1..6], &s[6..11], &s[11..16])
    };
    let mut rows = vec![
        (
            "Entry",
            format!(
                "{} (row {}, column {})",
                e.index,
                e.index / 16,
                e.index % 16
            ),
        ),
        ("Raw", format!("${}", hex(e.raw, 4))),
        ("Bits", bits),
        ("Blue", format!("{} of 31", e.blue5)),
        ("Green", format!("{} of 31", e.green5)),
        ("Red", format!("{} of 31", e.red5)),
        ("RGB", format!("#{}", hex(e.rgb, 6))),
    ];
    if e.unused_bit {
        rows.push((
            "Bit 15",
            "set: the PPU ignores it, so real palettes rarely have it".to_owned(),
        ));
    }
    for (i, (k, v)) in rows.iter().enumerate() {
        let kl = gtk::Label::builder().label(*k).xalign(0.0).build();
        kl.add_css_class("dim-label");
        let vl = gtk::Label::builder()
            .label(v)
            .xalign(0.0)
            .selectable(true)
            .build();
        vl.add_css_class("monospace");
        grid.attach(&kl, 0, i as i32, 1, 1);
        grid.attach(&vl, 1, i as i32, 1, 1);
    }
    b.append(&grid);
    b.append(&caption(
        "0bbbbbgggggrrrrr; each field expands as (c << 3) | (c >> 2).",
    ));
    b.upcast()
}
