//! The selected item's sections, or the header summary when nothing is
//! selected. The macOS twin is `InspectorView`. Rebuilt whenever the
//! selection or the analysis changes: what it shows is derived from the core,
//! and a section's own edits go back through the document.
//!
//! Not here yet: the Screen section (L2) and Play This Command (L4).

mod explain;
mod header;
mod sections;
mod ui;

use std::rc::Rc;

use gtk::prelude::*;

use crate::model::{Change, Document};

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content)
        .build();

    // Sections edit through the document, which re-reads and reports a Rows
    // change; rebuilding then shows the new value in place.
    rebuild(&content, doc);
    doc.subscribe({
        let (content, doc) = (content.clone(), Rc::clone(doc));
        move |c| {
            if matches!(c, Change::Selection | Change::Rows) {
                rebuild(&content, &doc);
            }
        }
    });
    scroll.upcast()
}

fn rebuild(content: &gtk::Box, doc: &Rc<Document>) {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    if doc.selected().is_none() {
        content.append(&header::build(doc));
        return;
    }
    let d = doc.details();
    content.append(&sections::selection_header(doc, &d));
    if let Some(insn) = &d.instruction {
        content.append(&sections::instruction(doc, &d, insn));
    }
    if let Some(x) = d.explanation.as_ref().filter(|x| explain::has_content(x)) {
        content.append(&explain::section(
            doc,
            x,
            d.instruction.as_ref().map(|i| i.file_offset),
        ));
    }
    content.append(&sections::region(doc, &d));
    if let Some(p) = &d.preview {
        content.append(&sections::preview(doc, p));
    }
    content.append(&sections::label(doc, &d));
    content.append(&sections::comments(doc, &d));
    content.append(&sections::xrefs(doc, &d));
    if let Some(byte) = &d.inspection {
        // Collapsed when an instruction explains the bytes already.
        content.append(&sections::byte_readings(doc, byte, d.instruction.is_none()));
    }
}
