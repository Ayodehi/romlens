//! What the screen is set up to be at the selected instruction (docs/21): the
//! PPU registers' meaning from the code before it on every path, calls
//! included. A disclosure that is closed until asked, because it walks the
//! routine. The macOS twin is `ScreenSection` and `ScreenRowView`.

use std::rc::Rc;

use gtk::prelude::*;
use romlens_ffi::ScreenRowInfo;

use super::ui::*;
use crate::model::Document;
use crate::model::screen::link_title;

/// The disclosure and the box inside it, which `fill` keeps up to date as
/// the result arrives. (A collapsed `GtkExpander` does not parent its child,
/// so the box cannot be asked what it is in.)
#[derive(Clone)]
pub struct Section {
    expander: gtk::Expander,
    body: gtk::Box,
}

impl Section {
    pub fn widget(&self) -> gtk::Widget {
        self.expander.clone().upcast()
    }

    /// Show the current result, or why there is none, and follow the model's
    /// open state (it can be opened from elsewhere).
    pub fn fill(&self, doc: &Rc<Document>) {
        // Not while the model is borrowed: opening notifies back into it.
        let shown = doc.screen().shown;
        self.expander.set_expanded(shown);
        fill(&self.body, doc, &doc.screen());
    }
}

pub fn section(doc: &Rc<Document>) -> Section {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_top(6);
    let expander = gtk::Expander::builder()
        .label_widget(&heading("Screen at this point"))
        .child(&body)
        .tooltip_text(
            "What the PPU registers hold when this instruction runs, from the code \
             before it on every path, calls included.",
        )
        .build();
    let section = Section { expander, body };
    section.fill(doc);
    let d = Rc::clone(doc);
    section
        .expander
        .connect_expanded_notify(move |e| d.set_show_screen(e.is_expanded()));
    section
}

fn fill(body: &gtk::Box, doc: &Rc<Document>, screen: &crate::model::screen::ScreenModel) {
    while let Some(child) = body.first_child() {
        body.remove(&child);
    }
    if let Some(setup) = &screen.setup {
        for section in &setup.sections {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let title = gtk::Label::builder()
                .label(&section.title)
                .xalign(0.0)
                .build();
            title.add_css_class("heading");
            b.append(&title);
            for row in &section.rows {
                b.append(&row_view(doc, row));
            }
            body.append(&b);
        }
    } else if screen.loading {
        let s = adw::Spinner::new();
        s.set_halign(gtk::Align::Start);
        s.set_size_request(16, 16);
        body.append(&s);
    } else if screen.shown {
        body.append(&note(
            "The selection is not inside a routine the analysis found.",
        ));
    }
}

fn jump_button(doc: &Rc<Document>, label: &str, tooltip: &str, offset: u32) -> gtk::Button {
    let b = small_button(label);
    b.set_tooltip_text(Some(tooltip));
    let d = Rc::clone(doc);
    b.connect_clicked(move |_| d.jump_to(offset));
    b
}

fn row_view(doc: &Rc<Document>, row: &ScreenRowInfo) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let line = hbox(8);
    let label = gtk::Label::builder()
        .label(&row.label)
        .xalign(0.0)
        .valign(gtk::Align::Start)
        .width_request(72)
        .build();
    label.add_css_class("dim-label");
    line.append(&label);
    let text = gtk::Label::builder()
        .label(&row.text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .hexpand(true)
        .selectable(true)
        .build();
    line.append(&text);
    if let Some(&at) = row.set_at.last() {
        let go = gtk::Button::builder()
            .icon_name("go-next-symbolic")
            .valign(gtk::Align::Start)
            .tooltip_text("Go to the store that set it")
            .build();
        go.add_css_class("flat");
        let d = Rc::clone(doc);
        go.connect_clicked(move |_| d.jump_to(at));
        line.append(&go);
    }
    b.append(&line);

    // The DMA that wrote the memory it describes, and the graphics view of it.
    let detail = hbox(6);
    detail.set_margin_start(80);
    if let Some(source) = &row.source {
        detail.append(&note(source));
    }
    if let Some(at) = row.source_at {
        detail.append(&jump_button(doc, "Go", "Go to the DMA", at));
    }
    if let Some(link) = row.link {
        // The graphics views arrive with L3; the link is shown, not live.
        let open = small_button(link_title(link));
        open.set_sensitive(false);
        open.set_tooltip_text(Some("The graphics views are not built yet"));
        detail.append(&open);
    }
    if detail.first_child().is_some() {
        b.append(&detail);
    }
    b.upcast()
}
