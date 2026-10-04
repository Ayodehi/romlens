//! The screen at a frame of a recording, drawn from the PPU state (checklist
//! 3.12): hovering names what drew the pixel, clicking keeps it and offers the
//! views that show where it came from, and the chain of writes behind its
//! bytes (3.14). The macOS twin is `FrameView`, `PixelDetail` and
//! `ProvenancePartView`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::cairo;
use romlens_ffi::{PixelWinnerInfo, ProvenanceInfo, ProvenancePartInfo};

use crate::canvas::with_alpha;
use crate::gfxdraw::*;
use crate::model::graphics::{self as gfx, Reveal, Source, winner_colour, winner_summary};
use crate::model::{Change, Document, Tab};
use crate::pixels;

struct View {
    doc: Rc<Document>,
    area: gtk::DrawingArea,
    zoom: Vec<gtk::ToggleButton>,
    mode: gtk::Label,
    unsupported: gtk::Label,
    status: gtk::Label,
    detail: gtk::Box,
    surface: RefCell<Option<cairo::ImageSurface>>,
    shown_frame: Cell<u64>,
    /// The pixel and frame the detail is for.
    detail_key: RefCell<Option<(usize, usize, u64)>>,
    ticket: Cell<u64>,
    syncing: Cell<bool>,
}

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let (root, view) = View::build(doc);
    root.connect_destroy(move |_| {
        let _ = &view;
    });
    root
}

impl View {
    fn build(doc: &Rc<Document>) -> (gtk::Widget, Rc<Self>) {
        let area = gtk::DrawingArea::builder()
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
            .child(&area)
            .build();

        let zoom_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        zoom_box.add_css_class("linked");
        let zoom: Vec<gtk::ToggleButton> = (1..=4)
            .map(|n| {
                let b = gtk::ToggleButton::with_label(&format!("{n}×"));
                zoom_box.append(&b);
                b
            })
            .collect();
        let mode = caption("");
        mode.set_hexpand(true);
        mode.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let unsupported = gtk::Label::new(None);
        unsupported.add_css_class("caption");
        unsupported.add_css_class("warning");
        unsupported.set_tooltip_text(Some(
            "The game turns these on here; the drawing leaves them out, so these \
             pixels can differ from the real screen",
        ));
        let controls = gtk::Box::builder()
            .spacing(12)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        controls.append(&zoom_box);
        controls.append(&mode);
        controls.append(&unsupported);

        let detail = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .margin_start(12)
            .margin_end(12)
            .margin_top(10)
            .margin_bottom(10)
            .build();
        let detail_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(280)
            .child(&detail)
            .build();
        let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        split.append(&scroll);
        let sep = gtk::Separator::new(gtk::Orientation::Vertical);
        split.append(&sep);
        split.append(&detail_scroll);

        let status = gtk::Label::builder()
            .xalign(0.0)
            .margin_start(12)
            .margin_end(12)
            .margin_top(4)
            .margin_bottom(4)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        status.add_css_class("monospace");
        status.add_css_class("caption");

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&controls);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&split);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&status);

        let view = Rc::new(Self {
            doc: Rc::clone(doc),
            area,
            zoom,
            mode,
            unsupported,
            status,
            detail,
            surface: RefCell::new(None),
            shown_frame: Cell::new(u64::MAX),
            detail_key: RefCell::new(None),
            ticket: Cell::new(0),
            syncing: Cell::new(false),
        });
        // The detail pane exists only while a pixel is kept.
        detail_scroll.set_visible(false);
        sep.set_visible(false);
        view.wire(detail_scroll, sep);
        view.refresh();
        (root.upcast(), view)
    }

    fn wire(self: &Rc<Self>, detail_scroll: gtk::ScrolledWindow, sep: gtk::Separator) {
        let this = Rc::downgrade(self);
        self.area.set_draw_func(move |a, cr, _, _| {
            if let Some(v) = this.upgrade() {
                v.draw(a, cr);
            }
        });
        for (i, b) in self.zoom.iter().enumerate() {
            let this = Rc::downgrade(self);
            b.connect_clicked(move |b| {
                b.set_active(true);
                if let Some(v) = this.upgrade()
                    && !v.syncing.get()
                {
                    v.doc.edit_graphics(|g| {
                        g.frame_scale = i as i32 + 1;
                        None
                    });
                }
            });
        }
        let motion = gtk::EventControllerMotion::new();
        let this = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            if let Some(v) = this.upgrade() {
                v.hover(x, y);
            }
        });
        let this = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(v) = this.upgrade() {
                v.status
                    .set_text("Point at a pixel to see what drew it; click to keep it");
            }
        });
        self.area.add_controller(motion);
        let click = gtk::GestureClick::new();
        let this = Rc::downgrade(self);
        click.connect_pressed(move |_, _, x, y| {
            let Some(v) = this.upgrade() else { return };
            let s = f64::from(v.doc.graphics().frame_scale);
            let (px, py) = ((x / s) as usize, (y / s) as usize);
            let inside = v
                .doc
                .graphics()
                .frame_image()
                .is_some_and(|f| px < f.image.width as usize && py < f.image.height as usize);
            if inside {
                v.doc.edit_graphics(|g| {
                    g.selected_pixel = Some((px, py));
                    None
                });
            }
        });
        self.area.add_controller(click);

        let this = Rc::downgrade(self);
        let (scroll, sep) = (detail_scroll, sep);
        self.doc.subscribe(move |c| {
            if matches!(c, Change::Graphics | Change::Layout)
                && let Some(v) = this.upgrade()
            {
                v.refresh();
                let kept = v.doc.graphics().selected_pixel.is_some();
                scroll.set_visible(kept);
                sep.set_visible(kept);
            }
        });
        let this = Rc::downgrade(self);
        adw::StyleManager::default().connect_accent_color_notify(move |_| {
            if let Some(v) = this.upgrade() {
                v.area.queue_draw();
            }
        });
    }

    fn hover(&self, x: f64, y: f64) {
        let g = self.doc.graphics();
        let s = f64::from(g.frame_scale);
        let (px, py) = ((x / s) as usize, (y / s) as usize);
        match g.pixel(px, py) {
            Some(w) => self
                .status
                .set_text(&format!("({px}, {py}): {}", winner_summary(&w))),
            None => self
                .status
                .set_text("Point at a pixel to see what drew it; click to keep it"),
        }
    }

    fn refresh(self: &Rc<Self>) {
        if self.doc.graphics_tab() != Some(gfx::Tab::Frame) {
            return;
        }
        let (frame, scale, image, selected) = {
            let g = self.doc.graphics();
            (g.frame(), g.frame_scale, g.frame_image(), g.selected_pixel)
        };
        self.syncing.set(true);
        for (i, b) in self.zoom.iter().enumerate() {
            b.set_active(i as i32 + 1 == scale);
        }
        self.syncing.set(false);
        match &image {
            Some(f) => {
                *self.surface.borrow_mut() = pixels::surface(&f.image);
                self.area.set_content_width((f.image.width as i32) * scale);
                self.area
                    .set_content_height((f.image.height as i32) * scale);
                self.mode.set_text(&format!(
                    "Mode {}, drawn from the PPU state {}",
                    f.bg_mode,
                    if f.per_line {
                        "line by line"
                    } else {
                        "as the frame ended"
                    }
                ));
                self.mode.set_tooltip_text(Some(if f.per_line {
                    "The recording logged every register and memory write while the frame was drawn, so each line has the registers it had"
                } else {
                    "This recording has no line-by-line writes: every line is drawn with the registers as the frame ended, so a split screen (a status bar, a gradient) shows the bottom's settings throughout. Record with the current recorder script to get them."
                }));
                self.unsupported.set_visible(!f.unsupported.is_empty());
                self.unsupported
                    .set_text(&format!("not drawn: {}", f.unsupported.join(", ")));
            }
            None => *self.surface.borrow_mut() = None,
        }
        self.shown_frame.set(frame);
        self.area.queue_draw();
        self.show_detail(selected, frame);
    }

    fn draw(&self, a: &gtk::DrawingArea, cr: &cairo::Context) {
        let g = self.doc.graphics();
        let scale = f64::from(g.frame_scale);
        let (w, h) = (f64::from(a.content_width()), f64::from(a.content_height()));
        checker(cr, w, h, &a.color());
        if let Some(s) = self.surface.borrow().as_ref() {
            paint_surface(cr, s, scale);
        }
        if let Some((x, y)) = g.selected_pixel {
            outline(
                cr,
                x as f64 * scale - 2.0,
                y as f64 * scale - 2.0,
                scale + 4.0,
                scale + 4.0,
                &accent(),
                2.0,
            );
        }
        let _ = with_alpha;
    }

    // MARK: The kept pixel

    fn show_detail(self: &Rc<Self>, selected: Option<(usize, usize)>, frame: u64) {
        let key = selected.map(|(x, y)| (x, y, frame));
        if *self.detail_key.borrow() == key {
            return;
        }
        *self.detail_key.borrow_mut() = key;
        self.ticket.set(self.ticket.get() + 1);
        while let Some(c) = self.detail.first_child() {
            self.detail.remove(&c);
        }
        let Some((x, y)) = selected else { return };
        let Some(winner) = self.doc.graphics().pixel(x, y) else {
            return;
        };
        let title = gtk::Label::builder()
            .label(format!("Pixel ({x}, {y})"))
            .xalign(0.0)
            .build();
        title.add_css_class("heading");
        self.detail.append(&title);
        let summary = gtk::Label::builder()
            .label(winner_summary(&winner))
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .build();
        self.detail.append(&summary);

        let reveal_button = |text: &str, what: Reveal| {
            let b = gtk::Button::with_label(text);
            b.add_css_class("flat");
            b.set_halign(gtk::Align::Start);
            let (doc, winner) = (Rc::clone(&self.doc), winner.clone());
            b.connect_clicked(move |_| {
                let mut tab = None;
                doc.edit_graphics(|g| {
                    tab = g.reveal(what, &winner);
                    None
                });
                if let Some(t) = tab {
                    doc.open_graphics(t);
                }
            });
            b
        };
        match winner {
            PixelWinnerInfo::Sprite { .. } => {
                self.detail
                    .append(&reveal_button("Show Its OAM Entry", Reveal::Sprite));
                self.detail
                    .append(&reveal_button("Show Its Tile", Reveal::Tile));
            }
            PixelWinnerInfo::Background { .. } => {
                self.detail
                    .append(&reveal_button("Show Its Tilemap Cell", Reveal::Cell));
                self.detail
                    .append(&reveal_button("Show Its Tile", Reveal::Tile));
            }
            _ => {}
        }
        if winner_colour(&winner).is_some() {
            self.detail
                .append(&reveal_button("Show Its Colour", Reveal::Colour));
        }
        self.detail
            .append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let head = gtk::Label::builder()
            .label("Where it came from")
            .xalign(0.0)
            .build();
        head.add_css_class("heading");
        self.detail.append(&head);
        let spinner = adw::Spinner::new();
        spinner.set_halign(gtk::Align::Start);
        spinner.set_size_request(16, 16);
        self.detail.append(&spinner);

        let (this, ticket) = (Rc::downgrade(self), self.ticket.get());
        self.doc.pixel_provenance(x, y, move |chain| {
            let Some(v) = this.upgrade() else { return };
            if v.ticket.get() != ticket {
                return;
            }
            v.detail.remove(&spinner);
            match chain {
                Some(chain) => v.show_chain(&chain),
                None => {
                    let l = caption("The recording cannot say.");
                    v.detail.append(&l);
                }
            }
        });
    }

    fn show_chain(&self, chain: &ProvenanceInfo) {
        for part in &chain.parts {
            self.detail.append(&part_view(&self.doc, part));
        }
    }
}

fn button(text: &str, tip: &str, f: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class("flat");
    b.add_css_class("caption");
    b.set_tooltip_text(Some(tip));
    b.connect_clicked(move |_| f());
    b
}

fn wrapped(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build()
}

/// One part of a pixel's chain: the writes that put its bytes there, then the
/// hop before them, each with buttons to what it names.
fn part_view(doc: &Rc<Document>, part: &ProvenancePartInfo) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let mut what = part.what.clone();
    if let Some(first) = what.get(0..1) {
        what = first.to_uppercase() + &what[1..];
    }
    let t = gtk::Label::builder().label(&what).xalign(0.0).build();
    t.add_css_class("heading");
    b.append(&t);

    let show_code = {
        let doc = Rc::clone(doc);
        move |pc: u32| {
            doc.set_tab(Tab::Disassembly);
            doc.jump_to_snes(pc);
        }
    };
    let show_bytes = {
        let doc = Rc::clone(doc);
        move |start: u32, len: u32| {
            doc.set_tab(Tab::Hex);
            doc.jump_to(start);
            doc.select_range(start..start + len);
        }
    };
    for link in &part.links {
        let l = gtk::Box::new(gtk::Orientation::Vertical, 3);
        l.append(&wrapped(&link.summary));
        let n = link.bytes.len();
        let list = gtk::Label::builder()
            .label(link.bytes.join("\n"))
            .xalign(0.0)
            .selectable(true)
            .build();
        list.add_css_class("monospace");
        list.add_css_class("caption");
        let ex = gtk::Expander::builder()
            .label(format!("{n} byte{}", if n == 1 { "" } else { "s" }))
            .child(&list)
            .build();
        l.append(&ex);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        if let Some(pc) = link.started_at {
            let f = show_code.clone();
            row.append(&button(
                "Show the Code",
                "The instruction that started the DMA",
                move || f(pc),
            ));
        }
        if let Some(start) = link.rom_start {
            let (f, len) = (show_bytes.clone(), link.rom_len.max(1));
            row.append(&button(
                "Show the Bytes",
                "The DMA's source in the ROM",
                move || f(start, len),
            ));
        }
        l.append(&row);
        b.append(&l);
    }
    if let Some(hop) = &part.hop {
        b.append(&wrapped(&hop.code_summary));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        for c in &hop.code {
            let f = show_code.clone();
            let pc = c.pc;
            let tip = if c.clears {
                "Clears memory: writes this among many others".to_owned()
            } else {
                format!("Writes it {} times", c.count)
            };
            let btn = button(&romlens_ffi::format_snes_address(pc), &tip, move || f(pc));
            btn.add_css_class("monospace");
            row.append(&btn);
        }
        b.append(&row);
        if let Some(text) = &hop.placed_summary {
            b.append(&wrapped(text));
        }
        if let Some(p) = &hop.placed {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let (start, len) = if p.compressed {
                (p.start, p.len)
            } else {
                (p.focus, 32)
            };
            let f = show_bytes.clone();
            row.append(&button(
                if p.compressed {
                    "Show the Stream"
                } else {
                    "Show the Bytes"
                },
                "",
                move || f(start, len.max(1)),
            ));
            if p.compressed {
                let (doc_open, p_open) = (Rc::clone(doc), p.clone());
                row.append(&button(
                    "Open Decompressed",
                    "Decompress the stream into the Tile Decoder",
                    move || {
                        let bytes = doc_open.rom.bytes(p_open.start, p_open.len.max(1));
                        if let Ok(d) = romlens_ffi::decompress_sm(bytes) {
                            doc_open.edit_graphics(|g| {
                                g.source = Source::Bytes {
                                    label: "Decompressed".into(),
                                    data: d.output,
                                };
                                g.selected_tile = 0;
                                None
                            });
                            doc_open.open_graphics(gfx::Tab::Tiles);
                        }
                    },
                ));
                let (doc, p) = (Rc::clone(doc), p.clone());
                row.append(&button(
                    "Mark as Compressed Graphics",
                    "A proposal: mark the stream in the ROM map, with this recording as the evidence. Undo takes it back.",
                    move || {
                        doc.select_range(p.start..p.start + p.len.max(1));
                        let _ = doc.mark(romlens_ffi::OverrideKind::Data, romlens_ffi::DataKind::Compressed);
                    },
                ));
            }
            b.append(&row);
        }
    }
    b.upcast()
}
