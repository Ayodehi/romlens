//! The editor area when a graphics view is open: a bar naming what the views
//! read, then the view. The macOS twin is `GraphicsEditorView` and
//! `GraphicsSourceBar`.

use std::rc::Rc;

use adw::prelude::*;

use crate::model::graphics::{Source, Tab};
use crate::model::{Change, Document};
use crate::{frameview, layersview, oamview, paletteview, tilemapview, tilesview};

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let stack = gtk::Stack::new();
    stack.add_named(&tilesview::build(doc), Some(Tab::Tiles.id()));
    stack.add_named(&paletteview::build(doc), Some(Tab::Palette.id()));
    stack.add_named(&oamview::build(doc), Some(Tab::Oam.id()));
    stack.add_named(&tilemapview::build(doc), Some(Tab::Tilemap.id()));
    // Frame and Layers draw the screen, which exists only in a recording:
    // without one, a note says how to get one.
    for (tab, view) in [
        (Tab::Frame, frameview::build(doc)),
        (Tab::Layers, layersview::build(doc)),
    ] {
        let inner = gtk::Stack::new();
        inner.add_named(&view, Some("view"));
        inner.add_named(&needs_recording(tab), Some("none"));
        let doc_for = Rc::clone(doc);
        let update = {
            let inner = inner.clone();
            move || {
                let has = doc_for.graphics().has_recording();
                inner.set_visible_child_name(if has { "view" } else { "none" });
            }
        };
        update();
        doc.subscribe(move |c| {
            if c == Change::Graphics {
                update();
            }
        });
        stack.add_named(&inner, Some(tab.id()));
    }

    let bar = source_bar(doc);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&bar);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&stack);
    stack.set_vexpand(true);

    let show = {
        let (stack, doc) = (stack.clone(), Rc::clone(doc));
        move || {
            if let Some(tab) = doc.graphics_tab() {
                stack.set_visible_child_name(tab.id());
            }
        }
    };
    show();
    doc.subscribe(move |c| {
        if matches!(c, Change::Layout | Change::Graphics) {
            show();
        }
    });
    root.upcast()
}

/// What the Frame and Layers views say before there is a recording: the
/// screen exists only in one.
fn needs_recording(tab: Tab) -> gtk::Widget {
    adw::StatusPage::builder()
        .icon_name("image-x-generic-symbolic")
        .title("No Recording")
        .description(format!(
            "The {} view draws the screen from a recording's PPU memories and \
             registers. Open one with File › Open Recording…, or start a live \
             session with File › Start Live Session and run the recorder script \
             in Mesen.",
            tab.title()
        ))
        .build()
        .upcast()
}

/// Where the bytes come from: ROM at an offset, the attached recording at a
/// frame, or a decompressed block.
fn source_bar(doc: &Rc<Document>) -> gtk::Widget {
    let bar = gtk::Box::builder()
        .spacing(12)
        .margin_start(12)
        .margin_end(12)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    let rom = gtk::ToggleButton::with_label("ROM");
    let recording = gtk::ToggleButton::with_label("Recording");
    let bytes = gtk::ToggleButton::with_label("Decompressed");
    recording.set_group(Some(&rom));
    bytes.set_group(Some(&rom));
    let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    group.add_css_class("linked");
    group.append(&rom);
    group.append(&recording);
    group.append(&bytes);
    let description = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    description.add_css_class("monospace");
    description.add_css_class("dim-label");
    let packing = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let spinner = adw::Spinner::new();
    spinner.set_size_request(14, 14);
    let packing_label = gtk::Label::new(None);
    packing_label.add_css_class("dim-label");
    packing.append(&spinner);
    packing.append(&packing_label);
    packing.set_tooltip_text(Some(
        "Turning the recorder's stream into a recording; it opens when done",
    ));
    let note = gtk::Label::new(None);
    note.add_css_class("warning");
    note.set_tooltip_text(Some(
        "INIDISP ($2100) at this frame: what the views show was not what the screen showed",
    ));
    let read = gtk::Button::with_label("Read From Selection");
    read.set_tooltip_text(Some("Decode the bytes at the editor's selection"));
    let stepper = FrameStepper::build(doc);
    bar.append(&group);
    bar.append(&description);
    bar.append(&packing);
    bar.append(&note);
    bar.append(&read);
    bar.append(&stepper.widget);

    let doc_for_read = Rc::clone(doc);
    read.connect_clicked(move |_| doc_for_read.read_graphics_from_selection());
    for (button, source) in [(&rom, Source::Rom), (&recording, Source::Recording)] {
        let doc = Rc::clone(doc);
        button.connect_clicked(move |b| {
            if b.is_active() {
                let source = source.clone();
                doc.edit_graphics(|g| {
                    if source == Source::Rom || g.has_recording() {
                        g.source = source;
                    }
                    None
                });
            }
        });
    }

    let update = {
        let doc = Rc::clone(doc);
        move || {
            let g = doc.graphics();
            description.set_text(&g.source_description());
            read.set_visible(g.source == Source::Rom);
            read.set_sensitive(doc.selected().is_some());
            recording.set_visible(g.has_recording());
            stepper.widget.set_visible(g.source == Source::Recording);
            packing.set_visible(g.packing.is_some());
            packing_label.set_text(&format!(
                "Packing {}…",
                g.packing.as_deref().unwrap_or_default()
            ));
            let screen_note = g.screen_note();
            note.set_visible(screen_note.is_some());
            note.set_text(screen_note.as_deref().unwrap_or(""));
            match &g.source {
                Source::Bytes { label, .. } => {
                    bytes.set_label(label);
                    bytes.set_visible(true);
                    bytes.set_active(true);
                }
                Source::Recording => {
                    bytes.set_visible(false);
                    recording.set_active(true);
                }
                Source::Rom => {
                    bytes.set_visible(false);
                    rom.set_active(true);
                }
            }
            drop(g);
            stepper.update(&doc);
        }
    };
    update();
    doc.subscribe(move |c| {
        if matches!(c, Change::Graphics | Change::Selection | Change::Layout) {
            update();
        }
    });
    bar.upcast()
}

/// The frame: previous and next, a scrubber and the number.
struct FrameStepper {
    widget: gtk::Box,
    previous: gtk::Button,
    next: gtk::Button,
    scrub: gtk::Scale,
    number: gtk::SpinButton,
    live: gtk::ToggleButton,
    syncing: Rc<std::cell::Cell<bool>>,
}

impl FrameStepper {
    fn build(doc: &Rc<Document>) -> Rc<Self> {
        let previous = gtk::Button::from_icon_name("go-previous-symbolic");
        previous.set_tooltip_text(Some("Previous frame"));
        let next = gtk::Button::from_icon_name("go-next-symbolic");
        next.set_tooltip_text(Some("Next frame"));
        let scrub = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
        scrub.set_draw_value(false);
        scrub.set_width_request(160);
        scrub.set_tooltip_text(Some("Scrub through the recording"));
        let number = gtk::SpinButton::with_range(0.0, 1.0, 1.0);
        number.set_width_chars(6);
        let live = gtk::ToggleButton::new();
        live.set_icon_name("network-wireless-symbolic");
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        for w in [
            previous.upcast_ref::<gtk::Widget>(),
            scrub.upcast_ref(),
            number.upcast_ref(),
            next.upcast_ref(),
            live.upcast_ref(),
        ] {
            widget.append(w);
        }
        let this = Rc::new(Self {
            widget,
            previous,
            next,
            scrub,
            number,
            live,
            syncing: Rc::new(std::cell::Cell::new(false)),
        });
        for (button, delta) in [(&this.previous, -1), (&this.next, 1)] {
            let doc = Rc::clone(doc);
            button.connect_clicked(move |_| doc.step_frame(delta));
        }
        let (d, syncing) = (Rc::clone(doc), Rc::clone(&this.syncing));
        this.live.connect_toggled(move |b| {
            if !syncing.get() {
                let on = b.is_active();
                d.edit_graphics(|g| {
                    g.follow_live = on;
                    None
                });
            }
        });
        let (d, syncing) = (Rc::clone(doc), Rc::clone(&this.syncing));
        this.scrub.connect_value_changed(move |s| {
            if !syncing.get() {
                d.set_frame(s.value() as u64);
            }
        });
        let (d, syncing) = (Rc::clone(doc), Rc::clone(&this.syncing));
        this.number.connect_value_changed(move |s| {
            if !syncing.get() {
                d.set_frame(s.value() as u64);
            }
        });
        this
    }

    fn update(&self, doc: &Rc<Document>) {
        let g = doc.graphics();
        let (first, count, frame) = (g.first_frame(), g.frame_count(), g.frame());
        self.syncing.set(true);
        let last = count.saturating_sub(1).max(first) as f64;
        self.scrub
            .set_range(first as f64, last.max(first as f64 + 1.0));
        self.scrub.set_value(frame as f64);
        self.scrub.set_visible(count > 1);
        self.number.set_range(first as f64, last);
        self.number.set_value(frame as f64);
        self.previous.set_sensitive(frame > first);
        self.next.set_sensitive(frame + 1 < count);
        self.live.set_visible(g.is_live());
        self.live.set_active(g.follow_live);
        self.live.set_tooltip_text(Some(if g.follow_live {
            "Following the game: each frame shows as it arrives"
        } else {
            "Paused on this frame; click to follow the game again"
        }));
        self.syncing.set(false);
    }
}
