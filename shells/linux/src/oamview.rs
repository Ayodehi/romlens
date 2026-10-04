//! The sprite table (checklist 2.23): 128 entries decoded from the low table
//! and the high table's two bits. The macOS twin is `OamTableView` and
//! `SpriteDetail`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use romlens_ffi::OamEntryInfo;

use crate::gfxdraw::{caption, hex, history_row};
use crate::model::graphics::Source;
use crate::model::graphics::{self as gfx, OamSortChoice};
use crate::model::{Change, Document};
use crate::pixels;
use romlens_ffi::StateRegion;

/// Column titles and widths in characters.
const COLUMNS: [(&str, i32); 10] = [
    ("#", 4),
    ("X", 5),
    ("Y", 4),
    ("Tile", 5),
    ("Pal", 4),
    ("Pri", 4),
    ("Flip", 5),
    ("Size", 6),
    ("Table", 6),
    ("VRAM", 6),
];

struct View {
    doc: Rc<Document>,
    list: gtk::ListBox,
    detail: gtk::Box,
    order: gtk::DropDown,
    on_screen: gtk::CheckButton,
    obsel: gtk::Entry,
    obsel_note: gtk::Label,
    obsel_box: gtk::Box,
    rows: RefCell<Vec<(u8, gtk::ListBoxRow)>>,
    /// What the rows were built from; unchanged, a selection only moves the
    /// highlight, so the list keeps its scroll position.
    key: RefCell<String>,
    syncing: Cell<bool>,
}

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let (root, view) = View::build(doc);
    root.connect_destroy(move |_| {
        let _ = &view;
    });
    root
}

fn cell(text: &str, chars: i32) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .xalign(1.0)
        .width_chars(chars)
        .build();
    l.add_css_class("monospace");
    l
}

fn flip(e: &OamEntryInfo) -> &'static str {
    match (e.hflip, e.vflip) {
        (false, false) => "-",
        (true, false) => "h",
        (false, true) => "v",
        (true, true) => "hv",
    }
}

impl View {
    fn build(doc: &Rc<Document>) -> (gtk::Widget, Rc<Self>) {
        let order = gtk::DropDown::from_strings(&OamSortChoice::ALL.map(OamSortChoice::title));
        let on_screen = gtk::CheckButton::with_label("On screen only");
        let obsel = gtk::Entry::builder()
            .max_width_chars(3)
            .width_chars(3)
            .tooltip_text(
                "OBSEL ($2101) sizes the sprites and places their tiles; it is not in OAM",
            )
            .build();
        obsel.add_css_class("monospace");
        let obsel_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        obsel_box.append(&gtk::Label::new(Some("OBSEL $")));
        obsel_box.append(&obsel);
        let controls = gtk::Box::builder()
            .spacing(12)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        controls.append(&order);
        controls.append(&on_screen);
        controls.append(&obsel_box);
        let obsel_note = caption("");
        controls.append(&obsel_note);

        let head = gtk::Box::builder()
            .spacing(10)
            .margin_start(12)
            .margin_end(12)
            .build();
        for (title, chars) in COLUMNS {
            let l = cell(title, chars);
            l.add_css_class("dim-label");
            head.append(&l);
        }
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        list.add_css_class("navigation-sidebar");
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .child(&list)
            .build();
        let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
        table.append(&head);
        table.append(&scroll);

        let detail = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .margin_start(12)
            .margin_end(12)
            .margin_top(10)
            .width_request(240)
            .build();
        let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        split.append(&table);
        split.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        split.append(&detail);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&controls);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&split);

        let view = Rc::new(Self {
            doc: Rc::clone(doc),
            list,
            detail,
            order,
            on_screen,
            obsel,
            obsel_note,
            obsel_box,
            rows: RefCell::new(Vec::new()),
            key: RefCell::new(String::new()),
            syncing: Cell::new(false),
        });
        view.wire();
        view.refresh();
        (root.upcast(), view)
    }

    fn wire(self: &Rc<Self>) {
        let this = Rc::downgrade(self);
        self.order.connect_selected_notify(move |d| {
            if let Some(v) = this.upgrade()
                && !v.syncing.get()
            {
                let choice = OamSortChoice::ALL[d.selected() as usize];
                v.doc.edit_graphics(|g| {
                    g.oam_sort = choice;
                    None
                });
            }
        });
        let this = Rc::downgrade(self);
        self.on_screen.connect_toggled(move |b| {
            if let Some(v) = this.upgrade()
                && !v.syncing.get()
            {
                let on = b.is_active();
                v.doc.edit_graphics(|g| {
                    g.visible_sprites_only = on;
                    None
                });
            }
        });
        let this = Rc::downgrade(self);
        self.obsel.connect_activate(move |e| {
            if let Some(v) = this.upgrade() {
                let text = e.text();
                let text = text.trim().trim_start_matches('$');
                if let Ok(n) = u8::from_str_radix(text, 16) {
                    v.doc.edit_graphics(|g| {
                        g.obsel = n;
                        None
                    });
                } else {
                    v.refresh();
                }
            }
        });
        let this = Rc::downgrade(self);
        self.list.connect_row_selected(move |_, row| {
            let Some(v) = this.upgrade() else { return };
            if v.syncing.get() {
                return;
            }
            let index = row.and_then(|r| {
                v.rows
                    .borrow()
                    .iter()
                    .find(|(_, x)| x == r)
                    .map(|(i, _)| *i)
            });
            v.doc.edit_graphics(|g| g.select_sprite(index));
        });
        let this = Rc::downgrade(self);
        self.doc.subscribe(move |c| {
            if matches!(c, Change::Graphics | Change::Rows)
                && let Some(v) = this.upgrade()
            {
                v.refresh();
            }
        });
    }

    fn refresh(self: &Rc<Self>) {
        if self.doc.graphics_tab() != Some(gfx::Tab::Oam) {
            return;
        }
        let (key, sort, visible, obsel) = {
            let g = self.doc.graphics();
            (
                format!(
                    "{:?}|{}|{}|{}|{}",
                    g.oam_sort,
                    g.visible_sprites_only,
                    g.obsel,
                    g.rom_offset,
                    g.source_description()
                ),
                g.oam_sort,
                g.visible_sprites_only,
                g.obsel,
            )
        };
        self.syncing.set(true);
        self.order.set_selected(
            OamSortChoice::ALL
                .iter()
                .position(|s| *s == sort)
                .unwrap_or(0) as u32,
        );
        self.on_screen.set_active(visible);
        self.obsel.set_text(&hex(obsel, 2));
        let recording = self.doc.graphics().source == Source::Recording;
        self.obsel_box.set_visible(!recording);
        self.obsel_note.set_visible(recording);
        self.obsel_note
            .set_text(&format!("OBSEL ${} from the recording", hex(obsel, 2)));
        if *self.key.borrow() != key {
            *self.key.borrow_mut() = key;
            self.fill();
        }
        self.sync_selection();
        self.syncing.set(false);
        self.show_detail();
    }

    fn fill(&self) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let mut rows = self.rows.borrow_mut();
        rows.clear();
        for e in self.doc.graphics().sprites() {
            let line = gtk::Box::builder()
                .spacing(10)
                .margin_start(6)
                .margin_end(6)
                .margin_top(2)
                .margin_bottom(2)
                .build();
            let values = [
                e.index.to_string(),
                e.x.to_string(),
                e.y.to_string(),
                format!("${}", hex(e.tile, 3)),
                e.palette.to_string(),
                e.priority.to_string(),
                flip(&e).to_owned(),
                format!("{}×{}", e.width, e.height),
                e.name_table.to_string(),
                format!("${}", hex(e.tile_word, 4)),
            ];
            for (n, (v, (_, chars))) in values.iter().zip(COLUMNS).enumerate() {
                let c = cell(v, chars);
                if n == 0 && self.doc.graphics().sprite_changed(e.index) {
                    // Changed since the previous frame.
                    c.set_use_markup(true);
                    c.set_markup(&format!("<span foreground=\"#fe9500\">●</span> {v}"));
                    c.set_tooltip_text(Some("Changed since the previous frame"));
                }
                line.append(&c);
            }
            let row = gtk::ListBoxRow::builder().child(&line).build();
            self.list.append(&row);
            rows.push((e.index, row));
        }
    }

    fn sync_selection(&self) {
        let selected = self.doc.graphics().selected_sprite;
        let row = selected.and_then(|i| {
            self.rows
                .borrow()
                .iter()
                .find(|(x, _)| *x == i)
                .map(|(_, r)| r.clone())
        });
        match row {
            Some(r) => self.list.select_row(Some(&r)),
            None => self.list.unselect_all(),
        }
    }

    fn show_detail(&self) {
        while let Some(c) = self.detail.first_child() {
            self.detail.remove(&c);
        }
        let g = self.doc.graphics();
        let Some(i) = g.selected_sprite else {
            self.detail.append(&caption("Choose a sprite"));
            return;
        };
        // Selected but filtered out (not on screen): found in table order.
        let all = romlens_ffi::oam_entries(g.oam_bytes(), g.obsel, romlens_ffi::OamSort::Table);
        let Some(e) = all.into_iter().find(|e| e.index == i) else {
            return;
        };
        let title = gtk::Label::builder()
            .label(format!("Sprite {}", e.index))
            .xalign(0.0)
            .build();
        title.add_css_class("heading");
        self.detail.append(&title);
        let grid = gtk::Grid::builder()
            .column_spacing(10)
            .row_spacing(4)
            .build();
        let pal = 128 + usize::from(e.palette) * 16;
        let bit = usize::from(e.index % 4) * 2;
        let rows = [
            (
                "Low table",
                format!(
                    "bytes ${}-${}",
                    hex(e.low_offset, 3),
                    hex(e.low_offset + 3, 3)
                ),
            ),
            (
                "High table",
                format!("byte ${}, bits {}-{}", hex(e.high_offset, 3), bit, bit + 1),
            ),
            (
                "Palette",
                format!("OBJ {}, CGRAM {}-{}", e.palette, pal, pal + 15),
            ),
        ];
        for (n, (k, v)) in rows.iter().enumerate() {
            let kl = gtk::Label::builder().label(*k).xalign(0.0).build();
            kl.add_css_class("dim-label");
            let vl = gtk::Label::builder()
                .label(v)
                .xalign(0.0)
                .wrap(true)
                .build();
            vl.add_css_class("monospace");
            grid.attach(&kl, 0, n as i32, 1, 1);
            grid.attach(&vl, 1, n as i32, 1, 1);
        }
        if let Some(image) = g.sprite_image(e.index) {
            let scale = (96 / image.width.max(1)).max(1) as i32;
            self.detail.append(&pixels::pixel_image(&image, scale));
        }
        self.detail.append(&grid);
        let note =
            caption("Byte 3 is vhoopppn: flips, priority, palette, and the tile's ninth bit.");
        note.set_wrap(true);
        self.detail.append(&note);
        if let Some(row) = history_row(&self.doc, StateRegion::Oam, e.low_offset, 4) {
            self.detail.append(&row);
        }
    }
}
