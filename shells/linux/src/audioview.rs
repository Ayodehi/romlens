//! A sound view's tab (docs/23): what the views read and the transport, then
//! the view. The macOS twin is `AudioEditorView`,
//! `AudioSourceBar` and `TransportControls`.

use std::rc::Rc;

use adw::prelude::*;

use crate::gfxdraw::*;
use crate::graphicsview::FrameStepper;
use crate::model::audio::{Source, Tab};
use crate::model::{Change, Document};
use crate::{aramview, echoview, portsview, samplesview, scopeview, timelineview, voicesview};

pub fn build(doc: &Rc<Document>, tab: Tab) -> gtk::Widget {
    let view = match tab {
        Tab::Voices => voicesview::build(doc),
        Tab::Samples => samplesview::build(doc),
        Tab::Aram => aramview::build(doc),
        Tab::Timeline => timelineview::build(doc),
        Tab::Ports => portsview::build(doc),
        Tab::Echo => echoview::build(doc),
        Tab::Scope => scopeview::build(doc),
    };

    let unavailable = adw::StatusPage::builder()
        .icon_name("audio-volume-muted-symbolic")
        .build();
    let body = gtk::Stack::new();
    body.add_named(&view, Some("content"));
    body.add_named(&unavailable, Some("none"));
    body.set_vexpand(true);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&source_bar(doc));
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&body);

    let show = {
        let (doc, body, unavailable) = (Rc::clone(doc), body, unavailable);
        move || {
            let a = doc.audio();
            if a.state().is_some() {
                body.set_visible_child_name("content");
            } else {
                let (title, message) = unavailable_text(&a);
                unavailable.set_title(&title);
                unavailable.set_description(Some(&message));
                body.set_visible_child_name("none");
            }
        }
    };
    show();
    doc.subscribe(move |c| {
        if matches!(c, Change::Audio | Change::Graphics | Change::Layout) {
            show();
        }
    });

    // Space plays and pauses, unless a text field wants it.
    let keys = gtk::ShortcutController::new();
    keys.set_scope(gtk::ShortcutScope::Local);
    let d = Rc::clone(doc);
    keys.add_shortcut(gtk::Shortcut::new(
        gtk::ShortcutTrigger::parse_string("space"),
        Some(gtk::CallbackAction::new(move |_, _| {
            d.edit_audio(|a| a.toggle_play());
            true.into()
        })),
    ));
    root.add_controller(keys);
    root.upcast()
}

/// Why there is nothing to show yet.
fn unavailable_text(a: &crate::model::audio::AudioModel) -> (String, String) {
    match a.source {
        Source::Recording => (
            "No Sound in This Recording".to_owned(),
            "Recordings made before the sound layer have only the picture side. Record again \
             with this Romlens's recorder script (File › Save Mesen Recorder Script…)."
                .to_owned(),
        ),
        Source::Rom => {
            if a.upload_loading() {
                (
                    "Tracing the Upload…".to_owned(),
                    "Looking for the code that sends the sound driver through the four ports."
                        .to_owned(),
                )
            } else {
                (
                    "No Sound Driver Found".to_owned(),
                    a.rom_problem().map(str::to_owned).unwrap_or_else(|| {
                        "The analysis found no routine that waits for the sound CPU's $BBAA and \
                         sends it a block list. A game with another way of uploading needs a \
                         recording (with the sound layer) to show its sound."
                            .to_owned()
                    }),
                )
            }
        }
    }
}

/// The recording at its frame, or the ROM's upload and which lists are laid
/// over the driver; and the transport.
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
    recording.set_group(Some(&rom));
    let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    group.add_css_class("linked");
    group.append(&rom);
    group.append(&recording);
    let description = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    description.add_css_class("monospace");
    description.add_css_class("dim-label");

    let transport = Transport::build(doc);
    let stepper = FrameStepper::build(doc);
    let uploads = gtk::MenuButton::builder()
        .label("Uploads")
        .tooltip_text("The songs and samples a game sends the running driver: which to lay over it")
        .build();
    let uploads_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    uploads_box.set_margin_top(8);
    uploads_box.set_margin_bottom(8);
    uploads_box.set_margin_start(8);
    uploads_box.set_margin_end(8);
    let popover = gtk::Popover::new();
    popover.set_child(Some(&uploads_box));
    uploads.set_popover(Some(&popover));

    bar.append(&group);
    bar.append(&description);
    bar.append(&transport.widget);
    bar.append(&stepper.widget);
    bar.append(&uploads);

    for (b, source) in [(&rom, Source::Rom), (&recording, Source::Recording)] {
        let doc = Rc::clone(doc);
        b.connect_clicked(move |b| {
            if b.is_active() && doc.audio().source != source {
                let trace = doc.edit_audio(|a| a.pick(source));
                if trace {
                    doc.load_upload();
                }
            }
        });
    }

    let built_for: std::cell::RefCell<Vec<u32>> = std::cell::RefCell::new(Vec::new());
    let update = {
        let doc = Rc::clone(doc);
        move || {
            let a = doc.audio();
            description.set_text(&a.source_description());
            recording.set_visible(a.has_recording_sound());
            match a.source {
                Source::Rom => rom.set_active(true),
                Source::Recording => recording.set_active(true),
            }
            stepper.widget.set_visible(a.source == Source::Recording);
            let others: Vec<(u32, String)> = a
                .other_uploads()
                .iter()
                .map(|u| {
                    (
                        u.list,
                        format!(
                            "{}: {} blocks, {} bytes",
                            romlens_ffi::format_snes_address(u.list),
                            u.blocks.len(),
                            u.bytes
                        ),
                    )
                })
                .collect();
            uploads.set_visible(a.source == Source::Rom && !others.is_empty());
            let lists: Vec<u32> = others.iter().map(|(l, _)| *l).collect();
            if *built_for.borrow() != lists {
                *built_for.borrow_mut() = lists;
                while let Some(c) = uploads_box.first_child() {
                    uploads_box.remove(&c);
                }
                for (list, text) in &others {
                    let check = gtk::CheckButton::with_label(text);
                    let (doc, list) = (Rc::clone(&doc), *list);
                    check.connect_toggled(move |c| {
                        let on = c.is_active();
                        if doc.audio().included_lists().contains(&list) != on {
                            doc.edit_audio(|a| a.set_included(list, on));
                        }
                    });
                    uploads_box.append(&check);
                }
            }
            let mut child = uploads_box.first_child();
            for (list, _) in &others {
                if let Some(c) = child.clone().and_downcast::<gtk::CheckButton>() {
                    c.set_active(a.included_lists().contains(list));
                }
                child = child.and_then(|c| c.next_sibling());
            }
            drop(a);
            stepper.update(&doc);
            transport.update(&doc);
        }
    };
    update();
    doc.subscribe(move |c| {
        if matches!(c, Change::Audio | Change::Graphics | Change::Layout) {
            update();
        }
    });
    bar.upcast()
}

/// Play and pause, following the recording, the time heard, and the port
/// console (docs/23, A12).
struct Transport {
    widget: gtk::Box,
    play: gtk::Button,
    time: gtk::Label,
    follow: gtk::ToggleButton,
    problem: gtk::Image,
    syncing: Rc<std::cell::Cell<bool>>,
}

impl Transport {
    fn build(doc: &Rc<Document>) -> Rc<Self> {
        let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
        let time = gtk::Label::new(None);
        time.add_css_class("caption");
        time.add_css_class("dim-label");
        time.set_width_chars(6);
        let follow = gtk::ToggleButton::new();
        follow.set_icon_name("media-seek-forward-symbolic");
        let ports = gtk::MenuButton::builder()
            .label("Ports")
            .tooltip_text(
                "Write the four ports as the S-CPU does: how a game asks its driver for a song or a sound effect",
            )
            .build();
        let popover = gtk::Popover::new();
        popover.set_child(Some(&port_console(doc)));
        ports.set_popover(Some(&popover));
        let problem = gtk::Image::from_icon_name("audio-volume-muted-symbolic");
        problem.add_css_class("warning");
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for w in [
            play.upcast_ref::<gtk::Widget>(),
            time.upcast_ref(),
            follow.upcast_ref(),
            ports.upcast_ref(),
            problem.upcast_ref(),
        ] {
            widget.append(w);
        }
        let this = Rc::new(Self {
            widget,
            play,
            time,
            follow,
            problem,
            syncing: Rc::new(std::cell::Cell::new(false)),
        });
        let d = Rc::clone(doc);
        this.play
            .connect_clicked(move |_| d.edit_audio(|a| a.toggle_play()));
        let (d, syncing) = (Rc::clone(doc), Rc::clone(&this.syncing));
        this.follow.connect_toggled(move |b| {
            if !syncing.get() {
                let on = b.is_active();
                d.edit_audio(|a| a.set_follow_recording(on));
            }
        });
        this
    }

    fn update(&self, doc: &Rc<Document>) {
        let a = doc.audio();
        let playing = a.is_playing();
        self.play.set_icon_name(if playing {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        });
        self.play.set_sensitive(a.state().is_some());
        self.play.set_tooltip_text(Some(if playing {
            "Pause (Space)"
        } else if a.source == Source::Rom {
            "Play the ROM's driver, run by Romlens (Space)"
        } else {
            "Play from this frame, run by Romlens from the recording's snapshot (Space)"
        }));
        self.time.set_visible(playing);
        self.time.set_text(&format!("{:.1} s", a.played_seconds()));
        self.follow.set_visible(a.source == Source::Recording);
        self.syncing.set(true);
        self.follow.set_active(a.follow_recording());
        self.syncing.set(false);
        self.follow.set_tooltip_text(Some(if a.follow_recording() {
            "Following the recording: its port writes arrive as they did, and the frame moves with the sound"
        } else {
            "The driver on its own from this frame, as if the game sent nothing more"
        }));
        self.problem.set_visible(a.output.problem.is_some());
        self.problem.set_tooltip_text(a.output.problem.as_deref());
    }
}

/// The four ports, the S-CPU's side: a byte to send on each, with the values
/// the game's own code sends offered, and what the driver answers.
pub fn port_console(doc: &Rc<Document>) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin_top(14);
    root.set_margin_bottom(14);
    root.set_margin_start(14);
    root.set_margin_end(14);
    let title = gtk::Label::builder()
        .label("Ports $2140-$2143")
        .xalign(0.0)
        .build();
    title.add_css_class("heading");
    root.append(&title);
    let about = caption(
        "The S-CPU and the SPC700 share only these four bytes each way. A game asks its \
         driver for a song or a sound by writing a number to a port; the driver reads it \
         and answers on the same port.",
    );
    about.set_wrap(true);
    about.set_max_width_chars(48);
    root.append(&about);
    let grid = gtk::Grid::builder()
        .column_spacing(8)
        .row_spacing(6)
        .build();
    let mut entries = Vec::new();
    let mut readbacks = Vec::new();
    let mut menus = Vec::new();
    for p in 0..4u8 {
        let entry = gtk::Entry::builder()
            .text("00")
            .width_chars(3)
            .max_width_chars(4)
            .build();
        entry.add_css_class("monospace");
        let send = gtk::Button::with_label("Send");
        let offered = gtk::MenuButton::builder().label("from the code").build();
        offered.set_tooltip_text(Some(
            "The values the game's code sends on this port, found by tracing it",
        ));
        let back = caption("");
        grid.attach(
            &gtk::Label::new(Some(&format!("Port {p}"))),
            0,
            i32::from(p),
            1,
            1,
        );
        grid.attach(&entry, 1, i32::from(p), 1, 1);
        grid.attach(&send, 2, i32::from(p), 1, 1);
        grid.attach(&offered, 3, i32::from(p), 1, 1);
        grid.attach(&back, 4, i32::from(p), 1, 1);
        let send_now = {
            let (doc, entry) = (Rc::clone(doc), entry.clone());
            move || {
                let text = entry.text();
                let text = text.trim().trim_start_matches('$');
                if let Ok(v) = u8::from_str_radix(text, 16) {
                    doc.edit_audio(|a| a.send(p, v));
                }
            }
        };
        let s = send_now.clone();
        send.connect_clicked(move |_| s());
        entry.connect_activate(move |_| send_now());
        entries.push(entry);
        readbacks.push(back);
        menus.push(offered);
    }
    root.append(&grid);
    let sent = caption("");
    root.append(&sent);

    let refresh = {
        let doc = Rc::clone(doc);
        move || {
            let a = doc.audio();
            for p in 0..4u8 {
                let i = usize::from(p);
                readbacks[i].set_text(
                    &a.read_port(p)
                        .map_or_else(String::new, |v| format!("reads back ${}", hex(v, 2))),
                );
                let values = a.command_values(p);
                menus[i].set_visible(!values.is_empty());
                menus[i].set_label(&format!("{} from the code", values.len()));
                let pop = gtk::Popover::new();
                let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
                for v in values {
                    let b = gtk::Button::with_label(&format!("${}", hex(v, 2)));
                    b.add_css_class("flat");
                    let (doc, entry, pop2) = (Rc::clone(&doc), entries[i].clone(), pop.clone());
                    b.connect_clicked(move |_| {
                        entry.set_text(&hex(v, 2));
                        pop2.popdown();
                        doc.edit_audio(|a| a.send(p, v as u8));
                    });
                    list.append(&b);
                }
                let scroll = gtk::ScrolledWindow::builder()
                    .max_content_height(240)
                    .propagate_natural_height(true)
                    .child(&list)
                    .build();
                pop.set_child(Some(&scroll));
                menus[i].set_popover(Some(&pop));
            }
            let tail: Vec<String> = a
                .sent()
                .iter()
                .rev()
                .take(8)
                .rev()
                .map(|(p, v)| format!("{p}=${}", hex(*v, 2)))
                .collect();
            sent.set_visible(!tail.is_empty());
            sent.set_text(&format!("Sent: {}", tail.join(" ")));
        }
    };
    refresh();
    doc.subscribe(move |c| {
        if c == Change::Audio {
            refresh();
        }
    });
    root.upcast()
}

/// The colour a part of audio RAM is drawn in.
pub fn aram_colour(kind: romlens_ffi::AramKindInfo) -> (f64, f64, f64) {
    use romlens_ffi::AramKindInfo::*;
    match kind {
        DirectPage => (0.55, 0.55, 0.85),
        Io => (0.85, 0.3, 0.3),
        Stack => (0.45, 0.45, 0.7),
        Code => (0.25, 0.55, 0.95),
        Directory => (0.95, 0.75, 0.2),
        Sample => (0.3, 0.75, 0.4),
        Echo => (0.7, 0.4, 0.85),
        DspData => (0.55, 0.8, 0.55),
        DriverData => (0.4, 0.75, 0.85),
        Boot => (0.6, 0.6, 0.6),
        Other => (0.3, 0.3, 0.3),
    }
}

/// Leave the sound view for the listing (or the hex view before the first
/// analysis lands) at a ROM offset.
pub fn show_in_rom(doc: &Rc<Document>, offset: u32) {
    doc.set_tab(if doc.has_disassembly() {
        crate::model::Tab::Disassembly
    } else {
        crate::model::Tab::Hex
    });
    doc.jump_to(offset);
}

/// A bold section heading.
pub fn heading_label(text: &str) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).build();
    l.add_css_class("heading");
    l
}

/// `$` and `digits` hex digits.
pub fn dollar(v: impl Into<u64>, digits: usize) -> String {
    format!("${}", hex(v, digits))
}
