//! The header bar's centre (docs/29): the jump bar, and Open Quickly.
//!
//! The jump bar names where the focused tab is: its chip, its view, and for
//! code the bank, the routine and the instruction at the selection. The chip,
//! view, bank and routine are menus, built when opened. Open Quickly finds an
//! address, a view, a label or a variable by typing. The macOS twins are
//! `JumpBar` and `OpenQuicklySheet`.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::model::quick::{self, Hit};
use crate::model::sidebar::{Entry, Section};
use crate::model::workspace::EditorContent;
use crate::model::{Change, Document};

/// The labels a routine menu lists, from the bank's start.
const ROUTINE_MENU_LIMIT: usize = 300;

/// The section whose views include `content`.
fn section_of(content: EditorContent) -> Option<Section> {
    Section::ALL
        .into_iter()
        .find(|s| s.entries().iter().any(|e| e.content == Some(content)))
}

/// The action that opens an entry's view, for a menu item.
fn open_action(entry: &Entry) -> Option<String> {
    entry
        .content
        .map(|c| format!("win.open-view::{}", c.singleton_key()))
}

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    bar.set_halign(gtk::Align::Center);
    bar.set_size_request(140, -1);
    bar.add_css_class("romlens-jumpbar");

    let chip = menu_button();
    let view = menu_button();
    let bank = menu_button();
    let routine = menu_button();
    let instruction = gtk::Label::builder()
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(36)
        .build();
    instruction.add_css_class("dim-label");
    instruction.add_css_class("monospace");
    let parts: [&gtk::Widget; 5] = [
        chip.upcast_ref(),
        view.upcast_ref(),
        bank.upcast_ref(),
        routine.upcast_ref(),
        instruction.upcast_ref(),
    ];
    for (i, w) in parts.into_iter().enumerate() {
        if i > 0 {
            // Each part after the first brings its own separator, shown
            // only while the part is.
            let sep = gtk::Image::from_icon_name("go-next-symbolic");
            sep.add_css_class("dim-label");
            w.bind_property("visible", &sep, "visible")
                .sync_create()
                .build();
            bar.append(&sep);
        }
        bar.append(w);
    }

    // The menus are built as they open, from the document as it is then.
    let d = Rc::downgrade(doc);
    chip.set_create_popup_func(move |b| {
        let menu = gio::Menu::new();
        if let Some(doc) = d.upgrade() {
            for s in Section::ALL {
                // A chip opens its first view that can open.
                let first = s.entries().into_iter().find(|e| {
                    e.content
                        .is_some_and(|c| doc.unavailable_reason(c).is_none())
                });
                if let Some(a) = first.as_ref().and_then(open_action) {
                    menu.append(Some(s.title()), Some(&a));
                }
            }
        }
        b.set_menu_model(Some(&menu));
    });
    let d = Rc::downgrade(doc);
    view.set_create_popup_func(move |b| {
        let menu = gio::Menu::new();
        if let Some(doc) = d.upgrade()
            && let Some(section) = doc.focused_content().and_then(section_of)
        {
            for e in section.entries() {
                let item = gio::MenuItem::new(Some(e.title), None);
                // A view that cannot open yet is listed, without an action.
                if e.content
                    .is_some_and(|c| doc.unavailable_reason(c).is_none())
                    && let Some(a) = open_action(&e)
                {
                    item.set_detailed_action(&a);
                }
                menu.append_item(&item);
            }
        }
        b.set_menu_model(Some(&menu));
    });
    let d = Rc::downgrade(doc);
    bank.set_create_popup_func(move |b| {
        let menu = gio::Menu::new();
        if let Some(doc) = d.upgrade() {
            for bank in &doc.navigator().data.banks {
                menu.append(
                    Some(&format!("Bank ${:02X}", bank.bank)),
                    Some(&format!("win.jump-offset::{:X}", bank.file_offset)),
                );
            }
        }
        b.set_menu_model(Some(&menu));
    });
    let d = Rc::downgrade(doc);
    routine.set_create_popup_func(move |b| {
        let menu = gio::Menu::new();
        if let Some(doc) = d.upgrade()
            && let Some(address) = doc.selected().and_then(|o| doc.rom.snes_address_for(o))
        {
            let mut labels: Vec<_> = doc
                .navigator()
                .data
                .labels
                .iter()
                .filter(|l| l.address >> 16 == address >> 16)
                .map(|l| (l.address, l.name.clone()))
                .collect();
            labels.sort();
            for (a, name) in labels.into_iter().take(ROUTINE_MENU_LIMIT) {
                menu.append(Some(&name), Some(&format!("win.jump-snes::{a:06X}")));
            }
        }
        b.set_menu_model(Some(&menu));
    });

    let update = {
        let doc = Rc::downgrade(doc);
        move || {
            let Some(doc) = doc.upgrade() else { return };
            let content = doc.focused_content();
            let section = content.and_then(section_of);
            chip.set_label(
                section
                    .map(|s| s.title().split(' ').next().unwrap_or(""))
                    .unwrap_or("Romlens"),
            );
            view.set_label(content.map_or("No Tab", EditorContent::title));
            view.set_visible(content.is_some());
            let code = matches!(content, Some(EditorContent::Code(_)));
            let address = doc.selected().and_then(|o| doc.rom.snes_address_for(o));
            let shown = code.then_some(address).flatten();
            bank.set_visible(shown.is_some());
            if let Some(a) = shown {
                bank.set_label(&format!("Bank ${:02X}", a >> 16));
            }
            let name = shown.and_then(|a| doc.routine_name(a));
            routine.set_visible(name.is_some());
            routine.set_label(name.as_deref().unwrap_or(""));
            let text = shown.and_then(|a| {
                let i = doc.details().instruction.clone()?;
                Some(format!(
                    "{} {}",
                    romlens_ffi::format_snes_address(a),
                    i.text
                ))
            });
            instruction.set_visible(text.is_some());
            instruction.set_text(text.as_deref().unwrap_or(""));
        }
    };
    update();
    doc.subscribe(move |c| {
        if matches!(c, Change::Layout | Change::Selection | Change::Navigator) {
            update();
        }
    });
    bar.upcast()
}

fn menu_button() -> gtk::MenuButton {
    let b = gtk::MenuButton::builder().always_show_arrow(false).build();
    b.add_css_class("flat");
    b
}

// MARK: Open Quickly

/// Go › Open Quickly… (Ctrl+P): type, ↑ and ↓ to choose, Return to go.
pub fn open_quickly(window: &adw::ApplicationWindow, doc: &Rc<Document>) {
    let dialog = adw::Dialog::builder()
        .title("Open Quickly")
        .content_width(560)
        .content_height(380)
        .build();
    let entry = gtk::SearchEntry::builder()
        .placeholder_text("A label, variable, view or address")
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(6)
        .build();
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    list.set_selection_mode(gtk::SelectionMode::Browse);
    let empty = adw::StatusPage::builder()
        .description("Labels, variables, views, and addresses such as $00:8000 or RESET")
        .build();
    empty.add_css_class("compact");
    let stack = gtk::Stack::new();
    stack.add_named(
        &gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .build(),
        Some("list"),
    );
    stack.add_named(&empty, Some("empty"));
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&entry);
    content.append(&stack);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&content));
    dialog.set_child(Some(&view));

    let hits: Rc<std::cell::RefCell<Vec<Hit>>> = Rc::default();
    let fill = {
        let (doc, list, stack, empty, hits) = (
            Rc::clone(doc),
            list.clone(),
            stack.clone(),
            empty.clone(),
            Rc::clone(&hits),
        );
        move |q: &str| {
            let found = quick::results(&doc, q);
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            for h in &found {
                let row = gtk::Box::builder()
                    .spacing(12)
                    .margin_start(6)
                    .margin_end(6)
                    .margin_top(4)
                    .margin_bottom(4)
                    .build();
                let title = gtk::Label::builder()
                    .label(h.title())
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build();
                let detail = gtk::Label::new(Some(&h.detail()));
                detail.add_css_class("dim-label");
                detail.add_css_class("caption");
                row.append(&title);
                row.append(&detail);
                list.append(&row);
            }
            if found.is_empty() {
                empty.set_description(Some(if q.trim().is_empty() {
                    "Labels, variables, views, and addresses such as $00:8000 or RESET"
                } else {
                    "Nothing matches"
                }));
                stack.set_visible_child_name("empty");
            } else {
                stack.set_visible_child_name("list");
                list.select_row(list.row_at_index(0).as_ref());
            }
            *hits.borrow_mut() = found;
        }
    };
    fill("");
    entry.connect_search_changed(move |e| fill(&e.text()));

    // Return or a click goes, after the dialog has closed.
    let go = {
        let (doc, dialog, hits, window) = (
            Rc::clone(doc),
            dialog.clone(),
            Rc::clone(&hits),
            window.clone(),
        );
        move |index: usize| {
            let Some(hit) = hits.borrow().get(index).cloned() else {
                return;
            };
            dialog.close();
            let (doc, window) = (Rc::clone(&doc), window.clone());
            glib::idle_add_local_once(move || match hit {
                Hit::Address { offset, .. } => doc.jump_to(offset),
                Hit::View { entry, .. } => {
                    if let Some(c) = entry.content {
                        gio::prelude::ActionGroupExt::activate_action(
                            &window,
                            "open-view",
                            Some(&c.singleton_key().to_variant()),
                        );
                    }
                }
                Hit::Label(l) => doc.jump_to_snes(l.address),
                Hit::Variable(v) => doc.find_references_to(v.address),
            });
        }
    };
    let g = go.clone();
    list.connect_row_activated(move |_, row| g(row.index().max(0) as usize));
    let l = list.clone();
    entry.connect_activate(move |_| {
        if let Some(row) = l.selected_row() {
            go(row.index().max(0) as usize);
        }
    });
    // ↑ and ↓ move through the results without leaving the field.
    let keys = gtk::EventControllerKey::new();
    let l = list.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        let delta = match key {
            gtk::gdk::Key::Down => 1,
            gtk::gdk::Key::Up => -1,
            _ => return glib::Propagation::Proceed,
        };
        let at = l.selected_row().map_or(-1, |r| r.index());
        if let Some(row) = l.row_at_index((at + delta).max(0)) {
            l.select_row(Some(&row));
        }
        glib::Propagation::Stop
    });
    entry.add_controller(keys);
    dialog.present(Some(window));
    entry.grab_focus();
}
