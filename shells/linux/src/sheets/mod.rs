//! The sheets: small dialogs over the window, each editing one thing through
//! the document. The macOS twins are the SwiftUI sheets in `Views/Sheets`.
//! They are `AdwDialog`s: Cancel at the start of the header, the action at
//! the end, Enter runs the action and Escape cancels.

mod datatype;
mod find;
mod flags;
mod jump;
mod label;
mod variable;

use std::rc::Rc;

use adw::prelude::*;

use crate::model::{Document, Sheet};

/// The pieces every sheet is built from.
pub struct Frame {
    pub dialog: adw::Dialog,
    pub body: gtk::Box,
    pub primary: gtk::Button,
}

/// A dialog with the standard header and an empty body to fill.
pub fn frame(title: &str, width: i32, primary_label: &str) -> Frame {
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(width)
        .follows_content_size(true)
        .build();
    let cancel = gtk::Button::with_label("Cancel");
    let primary = gtk::Button::with_label(primary_label);
    primary.add_css_class("suggested-action");
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&primary);
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_start(18)
        .margin_end(18)
        .margin_top(12)
        .margin_bottom(18)
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&body));
    dialog.set_child(Some(&view));
    dialog.set_default_widget(Some(&primary));
    let d = dialog.clone();
    cancel.connect_clicked(move |_| {
        d.close();
    });
    Frame {
        dialog,
        body,
        primary,
    }
}

/// A line of feedback under a field: secondary text, or red for a problem.
pub fn status() -> gtk::Label {
    let l = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build();
    l.add_css_class("caption");
    l.set_height_request(20);
    l
}

pub fn set_status(l: &gtk::Label, text: &str, error: bool) {
    l.set_text(text);
    if error {
        l.remove_css_class("dim-label");
        l.add_css_class("error");
    } else {
        l.remove_css_class("error");
        l.add_css_class("dim-label");
    }
}

/// A caption line, such as the address a sheet is about.
pub fn caption(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("monospace");
    l.add_css_class("dim-label");
    l.set_xalign(0.0);
    l
}

pub fn mono_entry(placeholder: &str) -> gtk::Entry {
    let e = gtk::Entry::builder()
        .placeholder_text(placeholder)
        .activates_default(true)
        .build();
    e.add_css_class("monospace");
    e
}

/// A destructive button for the start of the action row.
pub fn destructive(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("destructive-action");
    b
}

/// Show the sheet the document asked for.
pub fn present(window: &adw::ApplicationWindow, doc: &Rc<Document>, sheet: Sheet) {
    let frame = match sheet {
        Sheet::Jump => jump::build(doc),
        Sheet::RenameLabel => label::rename(doc),
        Sheet::Comment => label::comment(doc),
        Sheet::Flags => flags::build(doc),
        Sheet::Find => find::build(doc),
        Sheet::DataType => datatype::build(doc),
        Sheet::Variable => variable::build(doc),
    };
    // Whoever dismisses it, the document forgets the request.
    let d = Rc::clone(doc);
    frame.dialog.connect_closed(move |_| d.show_sheet(None));
    frame.dialog.present(Some(window));
}
