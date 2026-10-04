//! Ctrl+F. Bytes with `??` wildcards, or text.

use std::rc::Rc;

use adw::prelude::*;

use super::{Frame, frame, mono_entry, set_status, status};
use crate::model::search::SearchMode;
use crate::model::{Document, ResultsKind};

pub fn build(doc: &Rc<Document>) -> Frame {
    let f = frame("Find", 460, "Find");
    let (query, mode, ignore) = {
        let s = doc.search();
        (s.query.clone(), s.mode, s.ignore_case)
    };

    let entry = mono_entry(placeholder(mode));
    entry.set_text(&query);
    entry.set_hexpand(true);

    let bytes = gtk::ToggleButton::with_label("Bytes");
    let text = gtk::ToggleButton::with_label("Text");
    text.set_group(Some(&bytes));
    bytes.set_active(mode == SearchMode::Bytes);
    text.set_active(mode == SearchMode::Text);
    let modes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    modes.add_css_class("linked");
    modes.append(&bytes);
    modes.append(&text);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&entry);
    row.append(&modes);
    f.body.append(&row);

    let case = gtk::CheckButton::with_label("Ignore case");
    case.set_active(ignore);
    case.set_sensitive(mode == SearchMode::Text);
    f.body.append(&case);

    let message = status();
    f.body.append(&message);
    let show = gtk::Button::with_label("Show Results");
    show.set_halign(gtk::Align::Start);
    f.body.append(&show);

    let refresh = {
        let (doc, entry, case, message, show, primary) = (
            Rc::clone(doc),
            entry.clone(),
            case.clone(),
            message.clone(),
            show.clone(),
            f.primary.clone(),
        );
        move || {
            let s = doc.search();
            entry.set_placeholder_text(Some(placeholder(s.mode)));
            case.set_sensitive(s.mode == SearchMode::Text);
            primary.set_sensitive(!s.query.trim().is_empty());
            show.set_sensitive(s.has_results());
            let summary = s.summary();
            if summary.is_empty() {
                set_status(&message, hint(s.mode), false);
            } else {
                set_status(&message, &summary, s.error.is_some());
            }
        }
    };
    refresh();

    entry.connect_changed({
        let (doc, refresh) = (Rc::clone(doc), refresh.clone());
        move |e| {
            doc.edit_search(|s| s.query = e.text().to_string());
            refresh();
        }
    });
    for (button, m) in [(&bytes, SearchMode::Bytes), (&text, SearchMode::Text)] {
        button.connect_toggled({
            let (doc, refresh) = (Rc::clone(doc), refresh.clone());
            move |b| {
                if b.is_active() {
                    doc.edit_search(|s| s.mode = m);
                    refresh();
                }
            }
        });
    }
    case.connect_toggled({
        let doc = Rc::clone(doc);
        move |c| doc.edit_search(|s| s.ignore_case = c.is_active())
    });
    show.connect_clicked({
        let (doc, dialog) = (Rc::clone(doc), f.dialog.clone());
        move |_| {
            doc.show_results(ResultsKind::Find);
            dialog.close();
        }
    });
    f.primary.connect_clicked({
        let (doc, dialog, refresh) = (Rc::clone(doc), f.dialog.clone(), refresh);
        move |_| {
            doc.run_search();
            if doc.search().has_results() {
                doc.show_results(ResultsKind::Find);
                dialog.close();
            } else {
                refresh();
            }
        }
    });
    f.dialog.set_focus(Some(&entry));
    f
}

fn placeholder(mode: SearchMode) -> &'static str {
    match mode {
        SearchMode::Bytes => "78 18 ?? 5C",
        SearchMode::Text => "Super Metroid",
    }
}

fn hint(mode: SearchMode) -> &'static str {
    match mode {
        SearchMode::Bytes => "Hex byte pairs; ?? matches any byte.",
        SearchMode::Text => "Matched as bytes of text, not as a tile encoding.",
    }
}
