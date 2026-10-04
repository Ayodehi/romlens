//! What the inspector shows with nothing selected: the image's facts and the
//! header fields and vectors, each of which jumps to its bytes. The macOS
//! twin is `HeaderSummaryView`.

use std::rc::Rc;

use adw::prelude::*;

use super::ui::mono;
use crate::model::Document;
use crate::palette;

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 18);
    page.append(&image_group(doc));
    page.append(&spans_group(doc));
    page.upcast()
}

fn value(text: &str) -> gtk::Label {
    let l = mono(text);
    l.set_xalign(1.0);
    l.set_wrap(false);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l
}

fn image_group(doc: &Document) -> adw::PreferencesGroup {
    let i = &doc.info;
    let group = adw::PreferencesGroup::builder().title("Image").build();
    let mut rows = vec![
        ("File", i.file_name.clone()),
        (
            "Size",
            format!("{} bytes, {} rows", i.byte_len, i.row_count),
        ),
        (
            "Mapping",
            format!(
                "{}, {}",
                i.mapping_name,
                if i.fast_rom { "FastROM" } else { "SlowROM" }
            ),
        ),
        (
            "Header at",
            romlens_ffi::format_file_offset(i.header_offset),
        ),
        (
            "Checksum",
            if i.checksum_ok {
                "valid (mirrored sum)"
            } else {
                "mismatch"
            }
            .to_owned(),
        ),
        (
            "SHA-256",
            format!("{}…", &i.sha256[..16.min(i.sha256.len())]),
        ),
    ];
    if i.has_copier_header {
        rows.push(("Copier header", "512 bytes stripped".to_owned()));
    }
    for (label, text) in rows {
        let row = adw::ActionRow::builder().title(label).build();
        row.add_suffix(&value(&text));
        group.add(&row);
    }
    group
}

fn spans_group(doc: &Rc<Document>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Header and vectors")
        .build();
    for span in doc.rom.spans() {
        let row = adw::ActionRow::builder()
            .title(&span.name)
            .subtitle(&span.value_text)
            .activatable(true)
            .build();
        let kind = span.kind;
        let dot = gtk::DrawingArea::builder()
            .content_width(9)
            .content_height(9)
            .valign(gtk::Align::Center)
            .build();
        dot.set_draw_func(move |_, cr, w, h| {
            crate::canvas::set_source(cr, &palette::span_color(kind));
            cr.arc(
                f64::from(w) / 2.0,
                f64::from(h) / 2.0,
                f64::from(w.min(h)) / 2.0,
                0.0,
                std::f64::consts::TAU,
            );
            let _ = cr.fill();
        });
        row.add_prefix(&dot);
        let offset = gtk::Label::new(Some(&romlens_ffi::format_file_offset(span.start)));
        offset.add_css_class("monospace");
        offset.add_css_class("caption");
        offset.add_css_class("dim-label");
        row.add_suffix(&offset);
        let (d, start) = (Rc::clone(doc), span.start);
        row.connect_activated(move |_| d.jump_to(start));
        group.add(&row);
    }
    group
}
