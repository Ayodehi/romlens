//! The selected item's sections, or the header summary when nothing is
//! selected. The macOS twin is `InspectorView`. Rebuilt whenever the
//! selection or the analysis changes: what it shows is derived from the core,
//! and a section's own edits go back through the document.
//!
//! Not here yet: Play This Command (L4).

mod explain;
mod header;
mod screen;
mod sections;
mod ui;

use std::cell::RefCell;
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
    // The Screen section's result arrives later and updates in place, so the
    // inspector does not jump back to the top.
    let screen_body = Rc::new(RefCell::new(None));
    rebuild(&content, doc, &screen_body);
    doc.subscribe({
        let (content, doc) = (content.clone(), Rc::clone(doc));
        move |c| match c {
            Change::Selection | Change::Rows => rebuild(&content, &doc, &screen_body),
            Change::Screen => {
                if let Some(section) = screen_body.borrow().as_ref() {
                    section.fill(&doc);
                }
            }
            _ => {}
        }
    });
    scroll.upcast()
}

fn rebuild(content: &gtk::Box, doc: &Rc<Document>, screen_body: &RefCell<Option<screen::Section>>) {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    *screen_body.borrow_mut() = None;
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
    if d.instruction.is_some() {
        let section = screen::section(doc);
        content.append(&section.widget());
        *screen_body.borrow_mut() = Some(section);
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
