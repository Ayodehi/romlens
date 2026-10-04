//! The two CPUs' conversation (docs/23, A13): every port write both ways
//! around the frame, the S-CPU's matched to the 65816 code that sends that
//! value, and the uploads sent so far, block by block. The macOS twin is
//! `PortsView` and `PortEventRow`.

use std::rc::Rc;

use adw::prelude::*;

use crate::audioview::{dollar, heading_label, port_console, show_in_rom};
use crate::gfxdraw::*;
use crate::model::audio::{Source, Tab};
use crate::model::{Change, Document};

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let stack = gtk::Stack::new();
    let events = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let uploads = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let rom = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    let pad = |b: &gtk::Box| {
        b.set_margin_start(12);
        b.set_margin_end(12);
        b.set_margin_top(10);
        b.set_margin_bottom(10);
    };
    pad(&events);
    pad(&uploads);
    let left = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .child(&events)
        .build();
    let right = gtk::ScrolledWindow::builder()
        .width_request(320)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&uploads)
        .build();
    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&left)
        .end_child(&right)
        .resize_start_child(true)
        .resize_end_child(false)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .build();
    stack.add_named(&paned, Some("recording"));
    stack.add_named(
        &gtk::ScrolledWindow::builder().child(&rom).build(),
        Some("rom"),
    );

    let console = port_console(doc);
    let refresh = {
        let (doc, stack) = (Rc::clone(doc), stack.clone());
        move || {
            if doc.audio_tab() != Some(Tab::Ports) {
                return;
            }
            match doc.audio().source {
                Source::Recording => {
                    stack.set_visible_child_name("recording");
                    fill_recording(&doc, &events, &uploads);
                }
                Source::Rom => {
                    stack.set_visible_child_name("rom");
                    fill_rom(&doc, &rom, &console);
                }
            }
        }
    };
    refresh();
    doc.subscribe(move |c| {
        if matches!(c, Change::Audio | Change::Graphics | Change::Layout) {
            refresh();
        }
    });
    stack.upcast()
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn link(text: &str, tip: &str, f: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class("flat");
    b.add_css_class("caption");
    b.set_tooltip_text(Some(tip));
    b.connect_clicked(move |_| f());
    b
}

fn fill_recording(doc: &Rc<Document>, events: &gtk::Box, uploads: &gtk::Box) {
    clear(events);
    clear(uploads);
    let a = doc.audio();
    let frame = a.frame();
    let list = a.port_events(frame, 30);
    events.append(&heading_label(&format!(
        "Frames {} to {}: {} writes",
        frame.saturating_sub(30),
        frame + 30,
        list.len()
    )));
    for e in &list {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let n = gtk::Label::builder()
            .label(e.frame.to_string())
            .width_chars(6)
            .xalign(1.0)
            .build();
        n.add_css_class("caption");
        if e.frame != frame {
            n.add_css_class("dim-label");
        }
        let who = gtk::Label::builder()
            .label(if e.from_cpu {
                "S-CPU →"
            } else {
                "← SPC700"
            })
            .width_chars(9)
            .xalign(if e.from_cpu { 0.0 } else { 1.0 })
            .build();
        who.add_css_class("monospace");
        who.add_css_class("caption");
        who.add_css_class(if e.from_cpu { "accent" } else { "success" });
        let what = gtk::Label::new(Some(&format!("port {}  {}", e.port, dollar(e.value, 2))));
        what.add_css_class("monospace");
        row.append(&n);
        row.append(&who);
        row.append(&what);
        if e.from_cpu && e.value != 0 {
            for site in a.command_sites(e.port, e.value).into_iter().take(2) {
                let (doc, at) = (Rc::clone(doc), site.at);
                row.append(&link(
                    &format!("from {}", romlens_ffi::format_snes_address(site.routine)),
                    "The 65816 code that sends this value, found by tracing it",
                    move || show_in_rom(&doc, at),
                ));
            }
        }
        let cycle = caption(&format!("cycle {}", e.spc_cycle));
        cycle.set_hexpand(true);
        cycle.set_xalign(1.0);
        row.append(&cycle);
        events.append(&row);
    }
    let sent = a.sent_uploads();
    uploads.append(&heading_label(&format!("Uploads sent by frame {frame}")));
    if sent.is_empty() {
        let l = caption(
            "None yet. At power-on the SPC700's boot program waits for one: the S-CPU writes $CC \
             and then each byte with its index, and the boot program echoes each index back.",
        );
        l.set_wrap(true);
        uploads.append(&l);
    }
    for (i, u) in sent.iter().enumerate() {
        let bytes: u32 = u.blocks.iter().map(|b| b.len).sum();
        let tail = u.entry.map_or_else(
            || ", not finished".to_owned(),
            |e| format!(", then {}", dollar(e, 4)),
        );
        let t = gtk::Label::builder()
            .label(format!(
                "Upload {i}: {} blocks, {bytes} bytes{tail}",
                u.blocks.len()
            ))
            .xalign(0.0)
            .wrap(true)
            .build();
        uploads.append(&t);
        for b in &u.blocks {
            let end = u32::from(b.aram) + b.len.saturating_sub(1);
            let l = caption(&format!(
                "  {}-{}  {} bytes",
                dollar(b.aram, 4),
                dollar(end, 4),
                b.len
            ));
            l.add_css_class("monospace");
            uploads.append(&l);
        }
    }
}

fn fill_rom(doc: &Rc<Document>, rom: &gtk::Box, console: &gtk::Widget) {
    // The console is one widget; take it out before the rest is rebuilt.
    if console.parent().is_some() {
        rom.remove(console);
    }
    clear(rom);
    rom.append(&heading_label("The ROM's driver, run by Romlens"));
    let about = caption(
        "A recording holds the game's own port writes. Here, write them yourself: the driver \
         reads a port, acts, and answers on it.",
    );
    about.set_wrap(true);
    rom.append(&about);
    rom.append(console);
    let a = doc.audio();
    let Some(upload) = a.upload() else { return };
    if upload.commands.is_empty() {
        return;
    }
    rom.append(&heading_label("Where the game's code sends sound commands"));
    for c in &upload.commands {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let what = gtk::Label::new(Some(&format!(
            "port {} = {}",
            c.port,
            dollar(c.value, if c.width == 2 { 4 } else { 2 })
        )));
        what.add_css_class("monospace");
        let at = caption(&format!(
            "in {}{}",
            romlens_ffi::format_snes_address(c.routine),
            c.via.map_or_else(String::new, |v| format!(
                ", set in {} and copied to the port",
                romlens_ffi::format_snes_address(v)
            ))
        ));
        at.set_hexpand(true);
        row.append(&what);
        row.append(&at);
        let (d, port, value) = (Rc::clone(doc), c.port, (c.value & 0xFF) as u8);
        let send = gtk::Button::with_label("Send");
        send.connect_clicked(move |_| d.edit_audio(|a| a.send(port, value)));
        row.append(&send);
        let (d, at) = (Rc::clone(doc), c.at);
        row.append(&link("Show in ROM", "", move || show_in_rom(&d, at)));
        rom.append(&row);
    }
}
