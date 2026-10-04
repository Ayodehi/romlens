//! Small builders the inspector's sections share.

use gtk::prelude::*;

pub fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("heading");
    l.set_xalign(0.0);
    l
}

/// Wrapped secondary text.
pub fn note(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("dim-label");
    l.add_css_class("caption");
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l
}

/// Wrapped body text, selectable.
pub fn body(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_selectable(true);
    l
}

pub fn mono(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("monospace");
    l.set_xalign(0.0);
    l.set_selectable(true);
    l.set_wrap(true);
    l
}

/// A titled vertical group.
pub fn section(title: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if !title.is_empty() {
        b.append(&heading(title));
    }
    b
}

/// Label and value rows in two columns: the label dim, the value monospaced
/// and selectable.
pub fn rows(items: &[(&str, String)]) -> gtk::Grid {
    let g = gtk::Grid::builder()
        .column_spacing(12)
        .row_spacing(4)
        .build();
    for (i, (label, value)) in items.iter().enumerate() {
        let l = gtk::Label::new(Some(label));
        l.add_css_class("dim-label");
        l.set_xalign(0.0);
        l.set_yalign(0.0);
        g.attach(&l, 0, i as i32, 1, 1);
        let v = mono(value);
        v.set_hexpand(true);
        g.attach(&v, 1, i as i32, 1, 1);
    }
    g
}

pub fn small_button(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("flat");
    b.set_valign(gtk::Align::Center);
    b
}

pub fn hex(v: u32, digits: usize) -> String {
    format!("{v:0digits$X}")
}

/// A horizontal row of `spacing` that wraps nothing but gives children room.
pub fn hbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, spacing)
}
