//! The right-hand drawer (docs/29): the inspector, or the tutor, on two
//! plain tabs. With the tutor showing, the header names the conversation and
//! offers to open it as a tab, with more room. The tutor's view is made the
//! first time it shows, so a project that never asks costs nothing. The
//! macOS twin is `RightPaneView`.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;

use crate::model::{Change, Document};
use crate::{inspector, tutorview};

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let inspector_tab = gtk::ToggleButton::with_label("Inspector");
    let tutor_tab = gtk::ToggleButton::with_label("Tutor");
    tutor_tab.set_group(Some(&inspector_tab));
    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tabs.add_css_class("linked");
    tabs.append(&inspector_tab);
    tabs.append(&tutor_tab);
    let title = gtk::Label::builder()
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .hexpand(true)
        .xalign(0.0)
        .build();
    title.add_css_class("caption");
    title.add_css_class("dim-label");
    let as_tab = gtk::Button::builder()
        .icon_name("view-paged-symbolic")
        .action_name("win.show-tutor-tab")
        .tooltip_text("Open the tutor as a tab, with more room")
        .build();
    as_tab.add_css_class("flat");
    let header = gtk::Box::builder()
        .spacing(8)
        .margin_start(8)
        .margin_end(8)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    header.append(&tabs);
    header.append(&title);
    header.append(&as_tab);

    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.add_named(&inspector::build(doc), Some("inspector"));
    let made = Rc::new(Cell::new(false));

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&stack);
    // The tutor's status line and composer need this much room.
    root.set_size_request(280, -1);

    let syncing = Rc::new(Cell::new(false));
    let update = {
        let (doc, stack, made, syncing) = (
            Rc::downgrade(doc),
            stack.clone(),
            Rc::clone(&made),
            Rc::clone(&syncing),
        );
        let (inspector_tab, tutor_tab, title, as_tab) = (
            inspector_tab.clone(),
            tutor_tab.clone(),
            title.clone(),
            as_tab.clone(),
        );
        move || {
            let Some(doc) = doc.upgrade() else { return };
            let tutor = doc.tutor_in_drawer();
            if tutor && stack.child_by_name("tutor").is_none() {
                // Making the view tells the document, which calls this again
                // before the view is in the stack: that call waits for it.
                if made.replace(true) {
                    return;
                }
                let view = tutorview::build(&doc);
                stack.add_named(&view, Some("tutor"));
            }
            stack.set_visible_child_name(if tutor { "tutor" } else { "inspector" });
            syncing.set(true);
            if tutor {
                tutor_tab.set_active(true);
            } else {
                inspector_tab.set_active(true);
            }
            syncing.set(false);
            title.set_visible(tutor);
            as_tab.set_visible(tutor);
            let name = doc.tutor().title.clone();
            title.set_text(name.as_deref().unwrap_or("New conversation"));
        }
    };
    update();
    let u = update.clone();
    doc.subscribe(move |c| {
        if matches!(c, Change::Layout | Change::Tutor) {
            u();
        }
    });
    for (button, tutor) in [(&inspector_tab, false), (&tutor_tab, true)] {
        let (doc, syncing) = (Rc::downgrade(doc), Rc::clone(&syncing));
        button.connect_toggled(move |b| {
            if b.is_active()
                && !syncing.get()
                && let Some(doc) = doc.upgrade()
            {
                doc.set_tutor_in_drawer(tutor);
            }
        });
    }
    root.upcast()
}
