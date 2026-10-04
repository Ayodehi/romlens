//! Audio RAM's 64 KB coloured by what each byte is (docs/23), its parts
//! listed, and the part selected: the SPC700's code listed and explained, or
//! its bytes, with the upload blocks that filled it from the ROM. The macOS
//! twin is `AramView`, `AramMapImage`, `AramLegend`, `AramPartDetail` and
//! `SpcLineRow`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::cairo;
use romlens_ffi::{AramKindInfo, AramRegionInfo, BitmapInfo, SpcLineInfo};

use crate::audioview::{aram_colour, dollar, heading_label, show_in_rom};
use crate::gfxdraw::*;
use crate::inspector::explain::register_part;
use crate::model::audio::Tab;
use crate::model::{Change, Document};
use crate::pixels;

struct View {
    doc: Rc<Document>,
    map: gtk::DrawingArea,
    hover: gtk::Label,
    legend: gtk::FlowBox,
    parts: gtk::ListBox,
    detail: gtk::Box,
    surface: RefCell<Option<cairo::ImageSurface>>,
    map_key: RefCell<Vec<u64>>,
    rows: RefCell<Vec<(u16, gtk::ListBoxRow)>>,
    selected_line: Cell<Option<u16>>,
    detail_key: RefCell<String>,
    syncing: Cell<bool>,
}

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let map = gtk::DrawingArea::builder()
        .content_width(512)
        .content_height(512)
        .halign(gtk::Align::Start)
        .build();
    map.set_tooltip_text(Some(
        "Audio RAM a byte a pixel, 256 bytes a row: click a part to open it",
    ));
    let hover = caption("");
    hover.add_css_class("monospace");
    hover.set_height_request(16);
    let legend = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(3)
        .column_spacing(12)
        .row_spacing(4)
        .build();
    let left = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    left.append(&map);
    left.append(&hover);
    left.append(&legend);
    let left_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .width_request(300)
        .child(&left)
        .build();

    let parts = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .build();
    parts.add_css_class("navigation-sidebar");
    let parts_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(160)
        .child(&parts)
        .build();
    let detail = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_start(8)
        .margin_end(8)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    let detail_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(200)
        .child(&detail)
        .build();
    let right = gtk::Paned::builder()
        .orientation(gtk::Orientation::Vertical)
        .start_child(&parts_scroll)
        .end_child(&detail_scroll)
        .resize_start_child(false)
        .resize_end_child(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .build();
    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&left_scroll)
        .end_child(&right)
        .resize_start_child(false)
        .resize_end_child(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .position(560)
        .build();

    let view = Rc::new(View {
        doc: Rc::clone(doc),
        map,
        hover,
        legend,
        parts,
        detail,
        surface: RefCell::new(None),
        map_key: RefCell::new(Vec::new()),
        rows: RefCell::new(Vec::new()),
        selected_line: Cell::new(None),
        detail_key: RefCell::new(String::new()),
        syncing: Cell::new(false),
    });
    view.wire();
    view.refresh();
    let weak = Rc::downgrade(&view);
    doc.subscribe(move |c| {
        if matches!(c, Change::Audio | Change::Graphics | Change::Layout)
            && let Some(v) = weak.upgrade()
        {
            v.refresh();
        }
    });
    let weak = Rc::downgrade(&view);
    adw::StyleManager::default().connect_accent_color_notify(move |_| {
        if let Some(v) = weak.upgrade() {
            v.map.queue_draw();
        }
    });
    paned.connect_destroy(move |_| {
        let _ = &view;
    });
    paned.upcast()
}

/// The address under a point of the map, a byte a pixel at `scale`.
fn address(x: f64, y: f64, scale: f64) -> Option<u16> {
    let (px, py) = ((x / scale).floor() as i32, (y / scale).floor() as i32);
    ((0..256).contains(&px) && (0..256).contains(&py)).then(|| (py * 256 + px) as u16)
}

/// The rows a part covers, as rectangles `(x, y, width)` in bytes.
fn outline_rects(start: u32, len: u32) -> Vec<(u32, u32, u32)> {
    let end = start + len.max(1);
    let mut at = start;
    let mut out = Vec::new();
    while at < end {
        let row = at / 256;
        let row_end = end.min((row + 1) * 256);
        out.push((at % 256, row, row_end - at));
        at = row_end;
    }
    out
}

impl View {
    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.map.set_draw_func(move |_, cr, w, h| {
            if let Some(v) = weak.upgrade() {
                v.draw(cr, f64::from(w.min(h)));
            }
        });
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            let Some(v) = weak.upgrade() else { return };
            let scale = f64::from(v.map.content_width()) / 256.0;
            let text = address(x, y, scale)
                .and_then(|at| {
                    v.doc
                        .audio()
                        .part_containing(at)
                        .map(|p| format!("{}: {}", dollar(at, 4), p.label))
                })
                .unwrap_or_default();
            v.hover.set_text(&text);
        });
        let weak = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(v) = weak.upgrade() {
                v.hover.set_text("");
            }
        });
        self.map.add_controller(motion);
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |_, _, x, y| {
            let Some(v) = weak.upgrade() else { return };
            let scale = f64::from(v.map.content_width()) / 256.0;
            if let Some(at) = address(x, y, scale) {
                let part = v.doc.audio().part_containing(at).map(|p| p.start);
                v.doc.edit_audio(|a| a.selected_part = part);
            }
        });
        self.map.add_controller(click);
        let weak = Rc::downgrade(self);
        self.parts.connect_row_selected(move |_, row| {
            let Some(v) = weak.upgrade() else { return };
            if v.syncing.get() {
                return;
            }
            let start = row.and_then(|r| {
                v.rows
                    .borrow()
                    .iter()
                    .find(|(_, x)| x == r)
                    .map(|(s, _)| *s)
            });
            v.doc.edit_audio(|a| a.selected_part = start);
        });
    }

    fn refresh(self: &Rc<Self>) {
        if self.doc.audio_tab() != Some(Tab::Aram) {
            return;
        }
        let a = self.doc.audio();
        let Some(state) = a.state() else { return };
        let (selected, target) = (a.selected_part, a.listing_target);
        let key: Vec<u64> = state
            .map
            .iter()
            .map(|p| u64::from(p.start) << 40 | u64::from(p.len) << 8 | kind_id(p.kind))
            .collect();
        drop(a);
        if *self.map_key.borrow() != key {
            *self.map_key.borrow_mut() = key;
            *self.surface.borrow_mut() = map_surface(&state.map);
            self.fill_parts(&state.map);
            self.fill_legend(&state.map);
        }
        self.syncing.set(true);
        match selected.and_then(|s| {
            self.rows
                .borrow()
                .iter()
                .find(|(x, _)| *x == s)
                .map(|(_, r)| r.clone())
        }) {
            Some(r) => self.parts.select_row(Some(&r)),
            None => self.parts.unselect_all(),
        }
        self.syncing.set(false);
        self.map.queue_draw();
        let detail_key = format!(
            "{selected:?}|{target:?}|{}",
            self.doc.audio().source_description()
        );
        if *self.detail_key.borrow() != detail_key {
            *self.detail_key.borrow_mut() = detail_key;
            let part = selected.and_then(|s| state.map.iter().find(|p| p.start == s).cloned());
            self.show_detail(part.as_ref(), state.pc);
        }
    }

    fn fill_parts(&self, map: &[AramRegionInfo]) {
        while let Some(c) = self.parts.first_child() {
            self.parts.remove(&c);
        }
        let mut rows = self.rows.borrow_mut();
        rows.clear();
        for p in map {
            let line = gtk::Box::builder()
                .spacing(10)
                .margin_start(6)
                .margin_end(6)
                .margin_top(3)
                .margin_bottom(3)
                .build();
            let range = gtk::Label::builder()
                .label(format!(
                    "{}-{}",
                    dollar(p.start, 4),
                    dollar(u32::from(p.start) + p.len - 1, 4)
                ))
                .width_chars(12)
                .xalign(0.0)
                .build();
            range.add_css_class("monospace");
            let bytes = gtk::Label::builder()
                .label(p.len.to_string())
                .width_chars(6)
                .xalign(1.0)
                .build();
            bytes.add_css_class("monospace");
            let (r, g, b) = aram_colour(p.kind);
            let swatch = gtk::DrawingArea::builder()
                .content_width(8)
                .content_height(8)
                .valign(gtk::Align::Center)
                .build();
            swatch.set_draw_func(move |_, cr, w, h| {
                cr.set_source_rgb(r, g, b);
                cr.arc(
                    f64::from(w) / 2.0,
                    f64::from(h) / 2.0,
                    4.0,
                    0.0,
                    std::f64::consts::TAU,
                );
                let _ = cr.fill();
            });
            let what = gtk::Label::builder()
                .label(&p.label)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .hexpand(true)
                .build();
            what.set_tooltip_text(Some(&p.label));
            for w in [
                range.upcast_ref::<gtk::Widget>(),
                bytes.upcast_ref(),
                swatch.upcast_ref(),
                what.upcast_ref(),
            ] {
                line.append(w);
            }
            let row = gtk::ListBoxRow::builder().child(&line).build();
            self.parts.append(&row);
            rows.push((p.start, row));
        }
    }

    fn fill_legend(&self, map: &[AramRegionInfo]) {
        while let Some(c) = self.legend.first_child() {
            self.legend.remove(&c);
        }
        use AramKindInfo::*;
        for k in [
            DirectPage, Io, Stack, Code, DriverData, Directory, Sample, DspData, Echo, Boot, Other,
        ] {
            let parts: Vec<_> = map.iter().filter(|p| p.kind == k).collect();
            let Some(first) = parts.first() else { continue };
            let total: u32 = parts.iter().map(|p| p.len).sum();
            let (r, g, b) = aram_colour(k);
            let swatch = gtk::DrawingArea::builder()
                .content_width(10)
                .content_height(10)
                .valign(gtk::Align::Center)
                .build();
            swatch.set_draw_func(move |_, cr, w, h| {
                cr.set_source_rgb(r, g, b);
                cr.rectangle(0.0, 0.0, f64::from(w), f64::from(h));
                let _ = cr.fill();
            });
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            row.append(&swatch);
            row.append(&caption(&first.kind_name));
            row.append(&caption(&format!("{total} B")));
            self.legend.insert(&row, -1);
        }
    }

    fn draw(&self, cr: &cairo::Context, size: f64) {
        let scale = size / 256.0;
        if let Some(s) = self.surface.borrow().as_ref() {
            paint_surface(cr, s, scale);
        }
        let a = self.doc.audio();
        let white = gtk::gdk::RGBA::WHITE;
        if let Some(sel) = a.selected_part
            && let Some(p) = a
                .state()
                .and_then(|s| s.map.into_iter().find(|p| p.start == sel))
        {
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(1.5);
            for (x, y, w) in outline_rects(u32::from(p.start), p.len) {
                cr.rectangle(
                    f64::from(x) * scale,
                    f64::from(y) * scale,
                    f64::from(w) * scale,
                    scale,
                );
            }
            let _ = cr.stroke();
        }
        if let Some(pc) = a.state().map(|s| s.pc) {
            crate::canvas::set_source(cr, &white);
            cr.set_line_width(1.5);
            cr.arc(
                f64::from(pc & 0xFF) * scale + scale / 2.0,
                f64::from(pc >> 8) * scale + scale / 2.0,
                4.0,
                0.0,
                std::f64::consts::TAU,
            );
            let _ = cr.stroke();
        }
    }

    // MARK: The part selected

    fn show_detail(self: &Rc<Self>, part: Option<&AramRegionInfo>, pc: u16) {
        while let Some(c) = self.detail.first_child() {
            self.detail.remove(&c);
        }
        let Some(part) = part else {
            let page = adw::StatusPage::builder()
                .icon_name("drive-harddisk-symbolic")
                .title("Choose a Part")
                .description(
                    "Audio RAM is the sound CPU's only memory: its driver's code and data, the \
                     sample directory, the samples, and the echo buffer all share these 64 KB.",
                )
                .vexpand(true)
                .build();
            self.detail.append(&page);
            return;
        };
        let t = heading_label(&part.label);
        t.set_wrap(true);
        self.detail.append(&t);
        for (upload, block) in self.doc.audio().blocks_filling(part) {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.append(&caption(&format!(
                "sent by the upload from {}: ROM {}, {} bytes to {}",
                upload.source,
                romlens_ffi::format_file_offset(block.rom_offset),
                block.len,
                dollar(block.aram, 4)
            )));
            let b = gtk::Button::with_label("Show in ROM");
            b.add_css_class("flat");
            b.add_css_class("caption");
            let (doc, offset) = (
                Rc::clone(&self.doc),
                block.rom_offset + (i32::from(part.start) - i32::from(block.aram)).max(0) as u32,
            );
            b.connect_clicked(move |_| show_in_rom(&doc, offset));
            row.append(&b);
            self.detail.append(&row);
        }
        self.detail
            .append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        if matches!(part.kind, AramKindInfo::Code | AramKindInfo::Boot) {
            self.listing(part, pc);
        } else {
            self.hex(part);
        }
    }

    fn listing(self: &Rc<Self>, part: &AramRegionInfo, pc: u16) {
        let lines: Vec<SpcLineInfo> = self
            .doc
            .audio()
            .listing(part.start, (part.len / 2).clamp(16, 600))
            .into_iter()
            .filter(|l| u32::from(l.address) < u32::from(part.start) + part.len)
            .collect();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        let mut rows = Vec::new();
        for l in &lines {
            let row = gtk::ListBoxRow::builder()
                .child(&spc_line_row(l, pc))
                .build();
            list.append(&row);
            rows.push((l.address, row));
        }
        let info = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_start(8)
            .build();
        let info_scroll = gtk::ScrolledWindow::builder()
            .width_request(240)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&info)
            .build();
        let show = {
            let lines = lines.clone();
            let info = info.clone();
            move |at: Option<u16>| {
                while let Some(c) = info.first_child() {
                    info.remove(&c);
                }
                let Some(l) = at.and_then(|a| lines.iter().find(|l| l.address == a)) else {
                    return;
                };
                let text = gtk::Label::builder()
                    .label(&l.text)
                    .xalign(0.0)
                    .wrap(true)
                    .build();
                text.add_css_class("monospace");
                text.add_css_class("heading");
                info.append(&text);
                if let Some(i) = &l.idiom {
                    info.append(&heading_label(&i.title));
                    for (t, dim) in [(&i.summary, false), (&i.why, true)] {
                        let c = caption(t);
                        c.set_wrap(true);
                        if !dim {
                            c.remove_css_class("dim-label");
                        }
                        info.append(&c);
                    }
                }
                if !l.write.is_empty() {
                    info.append(&caption(if l.write_to_dsp {
                        "Writes the DSP"
                    } else {
                        "Writes an I/O register"
                    }));
                    for p in &l.write {
                        info.append(&register_part(p));
                    }
                } else if let Some(c) = &l.comment {
                    let c = caption(c);
                    c.set_wrap(true);
                    info.append(&c);
                }
                if !l.code {
                    let w = caption(
                        "Not reached from the program counter or the execution log: these bytes \
                         may be data read as code.",
                    );
                    w.add_css_class("warning");
                    w.set_wrap(true);
                    info.append(&w);
                }
            }
        };
        let (view, rows_for_select) = (Rc::downgrade(self), rows.clone());
        let show2 = show.clone();
        list.connect_row_selected(move |_, row| {
            let at = row.and_then(|r| {
                rows_for_select
                    .iter()
                    .find(|(_, x)| x == r)
                    .map(|(a, _)| *a)
            });
            if let Some(v) = view.upgrade() {
                v.selected_line.set(at);
            }
            show2(at);
        });
        let scroll = gtk::ScrolledWindow::builder()
            .min_content_width(300)
            .hexpand(true)
            .vexpand(true)
            .child(&list)
            .build();
        let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        split.set_vexpand(true);
        split.append(&scroll);
        split.append(&info_scroll);
        self.detail.append(&split);
        // Another view asked for a line: select and scroll to it.
        if let Some(target) = self.doc.audio().listing_target
            && let Some((_, row)) = rows.iter().find(|(a, _)| *a == target)
        {
            list.select_row(Some(row));
            let row = row.clone();
            gtk::glib::idle_add_local_once(move || {
                row.grab_focus();
            });
        }
    }

    fn hex(&self, part: &AramRegionInfo) {
        let len = part.len.min(4096);
        let bytes = self.doc.audio().aram(part.start, len);
        let mut text = String::new();
        for (r, chunk) in bytes.chunks(16).enumerate() {
            text += &format!(
                "{}  {}\n",
                dollar(u32::from(part.start) + (r * 16) as u32, 4),
                chunk
                    .iter()
                    .map(|b| hex(*b, 2))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
        if part.len > len {
            text += &format!("… {} more bytes\n", part.len - len);
        }
        let l = gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .yalign(0.0)
            .selectable(true)
            .build();
        l.add_css_class("monospace");
        l.add_css_class("caption");
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&l)
            .build();
        self.detail.append(&scroll);
    }
}

fn spc_line_row(l: &SpcLineInfo, pc: u16) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.set_margin_top(1);
    b.set_margin_bottom(1);
    let mono = |t: &str| {
        let l = gtk::Label::builder().label(t).xalign(0.0).build();
        l.add_css_class("monospace");
        l.add_css_class("caption");
        l
    };
    if let Some(label) = &l.label {
        let m = mono(&format!("{label}:"));
        m.add_css_class("accent");
        b.append(&m);
    }
    if let Some(i) = &l.idiom {
        let m = mono(&format!("; ▸ {}: {}", i.title, i.summary));
        m.add_css_class("dim-label");
        m.set_ellipsize(gtk::pango::EllipsizeMode::End);
        b.append(&m);
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let arrow = mono(if pc == l.address { "▶" } else { " " });
    arrow.add_css_class("success");
    row.append(&arrow);
    let addr = mono(&dollar(l.address, 4));
    addr.add_css_class("dim-label");
    row.append(&addr);
    let bytes = mono(
        &l.bytes
            .iter()
            .map(|b| hex(*b, 2))
            .collect::<Vec<_>>()
            .join(" "),
    );
    bytes.add_css_class("dim-label");
    bytes.set_width_chars(9);
    row.append(&bytes);
    let text = mono(&l.text);
    if !l.code {
        text.add_css_class("dim-label");
    }
    row.append(&text);
    if let Some(c) = &l.comment {
        let m = mono(&format!("; {c}"));
        m.add_css_class("success");
        row.append(&m);
    }
    b.append(&row);
    b.upcast()
}

fn kind_id(k: AramKindInfo) -> u64 {
    k as u64
}

/// A byte a pixel, 256 to a row.
fn map_surface(map: &[AramRegionInfo]) -> Option<cairo::ImageSurface> {
    let mut rgba = vec![0u8; 256 * 256 * 4];
    for p in map {
        let (r, g, b) = aram_colour(p.kind);
        // Alternate parts of one kind are shaded apart so a run of samples
        // reads as samples, not as one block.
        let shade = if p.kind == AramKindInfo::Sample && p.sample.unwrap_or(0) % 2 == 1 {
            0.8
        } else {
            1.0
        };
        let end = (u32::from(p.start) + p.len).min(0x10000);
        for i in u32::from(p.start)..end {
            let o = i as usize * 4;
            rgba[o] = (r * 255.0 * shade) as u8;
            rgba[o + 1] = (g * 255.0 * shade) as u8;
            rgba[o + 2] = (b * 255.0 * shade) as u8;
            rgba[o + 3] = 255;
        }
    }
    pixels::surface(&BitmapInfo {
        width: 256,
        height: 256,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_a_byte_a_pixel_256_to_a_row() {
        assert_eq!(address(0.0, 0.0, 2.0), Some(0));
        assert_eq!(address(5.0, 3.0, 1.0), Some(3 * 256 + 5));
        assert_eq!(address(511.0, 511.0, 2.0), Some(0xFFFF));
        assert_eq!(address(512.0, 0.0, 2.0), None);
        assert_eq!(address(-1.0, 0.0, 2.0), None);
    }

    #[test]
    fn a_part_is_outlined_a_row_at_a_time() {
        // A part inside one row is one rectangle.
        assert_eq!(outline_rects(0x0105, 4), vec![(5, 1, 4)]);
        // A part across rows is a partial first, whole middles, a partial last.
        assert_eq!(
            outline_rects(0x01F0, 0x120),
            vec![(0xF0, 1, 0x10), (0, 2, 0x100), (0, 3, 0x10)]
        );
        // An empty part still marks its first byte.
        assert_eq!(outline_rects(0x0200, 0), vec![(0, 2, 1)]);
    }
}
