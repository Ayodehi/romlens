//! A glossary entry in a bubble at the link (docs/27): the term, the words it
//! stands for, and a sentence or two. The macOS twin is `GlossaryPopover`.

use adw::prelude::*;
use romlens_ffi::{GlossaryEntryInfo, GlossaryKindInfo};

use crate::gfxdraw::caption;
use crate::model::markdown;

/// Shows `entry` pointing at `anchor`. `ask` puts a question about the term in
/// the composer.
pub fn show(anchor: &gtk::Label, entry: &GlossaryEntryInfo, ask: Box<dyn Fn(&str)>) {
    let popover = gtk::Popover::new();
    popover.set_parent(anchor);
    popover.set_autohide(true);
    // The bubble belongs to the click, not the whole label.
    popover.connect_closed(|p| {
        let p = p.clone();
        gtk::glib::idle_add_local_once(move || p.unparent());
    });
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    b.set_size_request(300, -1);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let term = gtk::Label::builder()
        .label(&entry.term)
        .xalign(0.0)
        .hexpand(true)
        .build();
    term.add_css_class("heading");
    head.append(&term);
    head.append(&caption(kind(entry.kind)));
    b.append(&head);
    let words = gtk::Label::builder()
        .label(&entry.words)
        .xalign(0.0)
        .wrap(true)
        .build();
    words.add_css_class("dim-label");
    b.append(&words);
    let about = gtk::Label::builder()
        .use_markup(true)
        .label(markdown::inline(&entry.about))
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(40)
        .build();
    b.append(&about);
    if !entry.also.is_empty() {
        let also = caption(&format!("Also written {}", entry.also.join(", ")));
        also.set_wrap(true);
        b.append(&also);
    }
    let button = gtk::Button::with_label(&format!("Ask the tutor about {}", entry.term));
    button.add_css_class("flat");
    button.set_halign(gtk::Align::Start);
    let term = entry.term.clone();
    let p = popover.clone();
    button.connect_clicked(move |_| {
        ask(&term);
        p.popdown();
    });
    b.append(&button);
    popover.set_child(Some(&b));
    popover.popup();
}

fn kind(k: GlossaryKindInfo) -> &'static str {
    match k {
        GlossaryKindInfo::Term => "Glossary",
        GlossaryKindInfo::Register => "Register",
        GlossaryKindInfo::DspRegister => "S-DSP register",
    }
}
