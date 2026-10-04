//! The sidebar (docs/29): every view, grouped by the chip that owns it, then
//! the symbols, with one filter at the bottom. Which rows show is
//! `model::sidebar`'s; this draws them in one list, so a thousand labels
//! scroll like ten. Choosing a view opens its tab (or brings it forward); one
//! that cannot open yet is dimmed, says why, and choosing it does what it
//! needs. Views and labels drag into the editor area. Replaces the
//! navigator; the macOS twin is `SidebarView`.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use romlens_ffi::{LabelSource, RegionKind};

use crate::lists::{ValueList, child_at, row_box, spacer};
use crate::model::sidebar::{self, Row};
use crate::model::workspace::{CodeRep, EditorContent, TabDrop};
use crate::model::{Change, Document};

/// Children of a row, in order (see `make_row`).
const ICON: usize = 0;
const TITLE: usize = 1;
const DETAIL: usize = 3;
const BUTTON: usize = 4;

/// What choosing a row needs: the document, the closed sections, and the
/// list itself, to redraw it and to reach the window's actions.
struct Ctx {
    doc: Weak<Document>,
    collapsed: RefCell<BTreeSet<String>>,
    list: RefCell<Option<Rc<ValueList<Row>>>>,
}

impl Ctx {
    fn refresh(&self) {
        if let (Some(doc), Some(list)) = (self.doc.upgrade(), self.list.borrow().as_ref()) {
            list.set(sidebar::rows(&doc, &self.collapsed.borrow()));
        }
    }

    /// Run a window action, found through the list's ancestors.
    fn run(&self, action: &str) {
        if let Some(list) = self.list.borrow().as_ref() {
            let _ = list.widget.activate_action(action, None);
        }
    }
}

pub fn build(doc: &Rc<Document>) -> gtk::Box {
    let ctx = Rc::new(Ctx {
        doc: Rc::downgrade(doc),
        collapsed: RefCell::new(
            crate::config::Settings::load()
                .sidebar_collapsed
                .into_iter()
                .collect(),
        ),
        list: RefCell::new(None),
    });
    let list: Rc<ValueList<Row>> = Rc::new(ValueList::new(false, make_row, bind_row, {
        let ctx = Rc::downgrade(&ctx);
        move |_, row: &Row| {
            if let Some(ctx) = ctx.upgrade() {
                choose(&ctx, row);
            }
        }
    }));
    *ctx.list.borrow_mut() = Some(Rc::clone(&list));

    let filter = gtk::SearchEntry::builder()
        .placeholder_text("Filter views and symbols")
        .margin_start(8)
        .margin_end(8)
        .margin_top(4)
        .margin_bottom(8)
        .build();
    let d = Rc::clone(doc);
    filter.connect_search_changed(move |e| d.set_nav_filter(&e.text()));

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&list.widget);
    root.append(&filter);

    ctx.refresh();
    // Rebuilt when what it lists or what is selected or available changes;
    // a section opened or closed rebuilds it directly (`choose`).
    let weak = Rc::downgrade(&ctx);
    // The list lives as long as its widget.
    root.connect_destroy(move |_| {
        let _ = &ctx;
    });
    doc.subscribe(move |c| {
        if matches!(
            c,
            Change::Navigator
                | Change::Layout
                | Change::Rows
                | Change::Status
                | Change::Source
                | Change::Compare
                | Change::Graphics
        ) && let Some(ctx) = weak.upgrade()
        {
            ctx.refresh();
        }
    });
    root
}

/// A row: an icon, a title, a spacer, the detail (a reason, a shortcut, an
/// address or a count) and a button for the rows that have one.
fn make_row() -> gtk::Widget {
    let r = row_box();
    r.append(&gtk::Image::new());
    let title = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    r.append(&title);
    r.append(&spacer());
    let detail = gtk::Label::builder().xalign(1.0).build();
    detail.add_css_class("dim-label");
    detail.add_css_class("caption");
    r.append(&detail);
    let button = gtk::Button::with_label("Define Variable…");
    button.add_css_class("flat");
    button.set_action_name(Some("win.define-variable"));
    r.append(&button);
    // Dragged into the editor area: what the row's name says to open.
    let row = r.clone();
    r.add_controller(crate::tabgrid::drag_source(move || {
        TabDrop::from_json(&row.widget_name())
    }));
    r.upcast()
}

fn label(row: &gtk::Widget, n: usize) -> gtk::Label {
    child_at(row, n).downcast().expect("a label")
}

fn bind_row(row: &gtk::Widget, item: &Row) {
    let icon: gtk::Image = child_at(row, ICON).downcast().expect("an image");
    let title = label(row, TITLE);
    let detail = label(row, DETAIL);
    let button = child_at(row, BUTTON);
    for c in [
        "heading",
        "dim-label",
        "monospace",
        "caption",
        "romlens-sidebar-selected",
        "romlens-sidebar-group",
    ] {
        title.remove_css_class(c);
    }
    row.remove_css_class("romlens-sidebar-selected");
    icon.set_visible(true);
    detail.set_visible(true);
    button.set_visible(false);
    row.set_tooltip_text(None);
    let mut drag: Option<TabDrop> = None;
    let chevron = |open: bool| {
        if open {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        }
    };
    match item {
        Row::Section { section, open } => {
            icon.set_icon_name(Some(chevron(*open)));
            title.set_text(section.title());
            title.add_css_class("heading");
            detail.set_visible(false);
        }
        Row::View {
            entry,
            selected,
            reason,
        } => {
            icon.set_icon_name(Some(
                entry
                    .content
                    .map_or("view-list-symbolic", crate::tabgrid::icon_name),
            ));
            title.set_text(entry.title);
            match reason {
                Some(r) => {
                    title.add_css_class("dim-label");
                    detail.set_text(r);
                    row.set_tooltip_text(Some(&format!(
                        "{} {r}. Choose it to fix that.",
                        entry.title
                    )));
                }
                None => {
                    // The selected row shows its shortcut.
                    let keys = entry
                        .action()
                        .filter(|_| *selected)
                        .and_then(|a| crate::actions::accels_for(&a).first().copied())
                        .and_then(gtk::accelerator_parse)
                        .map(|(key, mods)| gtk::accelerator_get_label(key, mods).to_string());
                    detail.set_text(keys.as_deref().unwrap_or(""));
                    row.set_tooltip_text(Some(&format!("Show {}", entry.title)));
                }
            }
            if *selected {
                row.add_css_class("romlens-sidebar-selected");
            }
            drag = entry.content.map(TabDrop::Open);
        }
        Row::Group {
            symbols,
            count,
            open,
        } => {
            icon.set_icon_name(Some(chevron(*open)));
            title.set_text(symbols.title());
            detail.set_text(&count.to_string());
        }
        Row::Label(l) => {
            icon.set_icon_name(None);
            title.set_text(&l.name);
            title.add_css_class("monospace");
            if l.source == LabelSource::Auto {
                title.add_css_class("dim-label");
            }
            detail.set_text(&romlens_ffi::format_snes_address(l.address));
            row.set_tooltip_text(Some(&match l.source {
                LabelSource::Imported => format!("Imported from {}", l.origin),
                LabelSource::User => "Your label".to_owned(),
                _ => "Named by the analysis".to_owned(),
            }));
            drag = Some(TabDrop::OpenAt(CodeRep::Assembly, l.address));
        }
        Row::Variable(v) => {
            icon.set_icon_name(None);
            title.set_text(if v.name.is_empty() {
                "(unnamed)"
            } else {
                &v.name
            });
            title.add_css_class("monospace");
            detail.set_text(&romlens_ffi::format_snes_address(v.address));
            row.set_tooltip_text(Some(&format!("{} · Find References", v.description)));
        }
        Row::Region(r) => {
            icon.set_icon_name(Some(match r.kind {
                RegionKind::Code => "format-justify-left-symbolic",
                RegionKind::Data => "drive-harddisk-symbolic",
                RegionKind::Unknown => "dialog-question-symbolic",
            }));
            title.set_text(&r.name);
            detail.set_text(&format!("{} B", r.len));
        }
        Row::Bank(b) => {
            icon.set_icon_name(None);
            title.set_text(&format!("Bank ${:02X}", b.bank));
            title.add_css_class("monospace");
            detail.set_text(&format!("{} KB", b.length / 1024));
        }
        Row::Note(text) => {
            icon.set_icon_name(None);
            title.set_text(text);
            title.add_css_class("dim-label");
            title.add_css_class("caption");
            detail.set_visible(false);
        }
        Row::DefineVariable => {
            icon.set_visible(false);
            title.set_text("");
            detail.set_visible(false);
            button.set_visible(true);
        }
    }
    row.set_widget_name(&drag.map(TabDrop::to_json).unwrap_or_default());
}

/// A row chosen: a view opens (or does what it needs first), a section or
/// list opens or closes, a symbol goes where it is.
fn choose(ctx: &Ctx, row: &Row) {
    let Some(doc) = ctx.doc.upgrade() else { return };
    let toggle = |id: &str| {
        {
            let mut c = ctx.collapsed.borrow_mut();
            if !c.remove(id) {
                c.insert(id.to_owned());
            }
            let mut saved = crate::config::Settings::load();
            saved.sidebar_collapsed = c.iter().cloned().collect();
            saved.save();
        }
        ctx.refresh();
    };
    match row {
        Row::Section { section, .. } => toggle(section.id()),
        Row::Group { symbols, .. } => toggle(symbols.id()),
        Row::View { entry, reason, .. } => match (entry.content, reason) {
            (None, _) => ctx.run("win.show-lessons"),
            (Some(EditorContent::Compare), Some(_)) => ctx.run("win.compare-with"),
            (Some(EditorContent::Source), Some(_)) => ctx.run("win.import-dbg"),
            (Some(EditorContent::Graphics(_)), Some(_)) => ctx.run("win.open-recording"),
            // Waiting for the analysis: nothing to do but wait.
            (Some(_), Some(_)) => {}
            (Some(EditorContent::Graphics(t)), None) => doc.open_graphics(t),
            (Some(EditorContent::Audio(t)), None) => doc.open_audio(t),
            (Some(EditorContent::Tutor), None) => doc.show_tutor_tab(),
            (Some(content), None) => doc.show(content),
        },
        Row::Label(l) => doc.jump_to_snes(l.address),
        Row::Variable(v) => doc.find_references_to(v.address),
        Row::Region(r) => doc.jump_to(r.start),
        Row::Bank(b) => doc.jump_to(b.file_offset),
        Row::Note(_) | Row::DefineVariable => {}
    }
}
