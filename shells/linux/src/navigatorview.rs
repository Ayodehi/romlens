//! Labels, variables, regions and banks; selecting a row jumps (one way).
//! The macOS twin is `NavigatorView`.

use std::rc::Rc;

use gtk::prelude::*;
use romlens_ffi::{LabelInfo, LabelSource, RegionInfo, RegionKind, VariableInfo};

use crate::lists::{ValueList, child_at, row_box, spacer};
use crate::model::navigator::{Bank, NavTab};
use crate::model::workspace::{CodeRep, TabDrop};
use crate::model::{Change, Document};
use crate::style::chip;

struct Lists {
    labels: ValueList<LabelInfo>,
    variables: ValueList<VariableInfo>,
    regions: ValueList<RegionInfo>,
    banks: ValueList<Bank>,
}

pub fn build(doc: &Rc<Document>) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

    // The tabs as icons: the narrowest of the macOS fallbacks, since a
    // sidebar this wide cannot spell four words.
    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tabs.add_css_class("linked");
    tabs.set_halign(gtk::Align::Center);
    tabs.set_margin_top(8);
    tabs.set_margin_bottom(8);
    let mut buttons = Vec::new();
    for tab in NavTab::ALL {
        let b = gtk::ToggleButton::builder()
            .icon_name(tab.icon())
            .tooltip_text(tab.title())
            .build();
        let d = Rc::clone(doc);
        b.connect_clicked(move |_| d.set_nav_tab(tab));
        tabs.append(&b);
        buttons.push((tab, b));
    }
    root.append(&tabs);

    let lists = Rc::new(Lists {
        labels: label_list(doc),
        variables: variable_list(doc),
        regions: region_list(doc),
        banks: bank_list(doc),
    });
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.add_named(&lists.labels.widget, Some("labels"));
    stack.add_named(&variables_page(&lists.variables, doc), Some("variables"));
    stack.add_named(&regions_page(&lists.regions, doc), Some("regions"));
    stack.add_named(&lists.banks.widget, Some("banks"));

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&stack));
    let spinner = gtk::Spinner::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    overlay.add_overlay(&spinner);
    root.append(&overlay);

    let filter = gtk::SearchEntry::builder()
        .placeholder_text("Filter")
        .margin_start(8)
        .margin_end(8)
        .margin_top(4)
        .margin_bottom(8)
        .build();
    let d = Rc::clone(doc);
    filter.connect_search_changed(move |e| d.set_nav_filter(&e.text()));
    root.append(&filter);

    let refresh = {
        let (doc, lists) = (Rc::clone(doc), Rc::clone(&lists));
        move || {
            let nav = doc.navigator();
            let tab = nav.tab();
            for (t, b) in &buttons {
                b.set_active(*t == tab);
            }
            stack.set_visible_child_name(match tab {
                NavTab::Labels => "labels",
                NavTab::Variables => "variables",
                NavTab::Regions => "regions",
                NavTab::Banks => "banks",
            });
            filter.set_visible(tab != NavTab::Banks);
            // The lists are the filtered ones; banks have no filter.
            lists.labels.set(nav.filtered_labels.clone());
            lists.variables.set(nav.filtered_variables.clone());
            lists.regions.set(nav.filtered_regions.clone());
            lists.banks.set(nav.data.banks.clone());
            let busy = nav.loading && nav.data.labels.is_empty();
            spinner.set_spinning(busy);
            spinner.set_visible(busy);
        }
    };
    refresh();
    doc.subscribe(move |c| {
        if c == Change::Navigator {
            refresh();
        }
    });
    root
}

fn mono(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("monospace");
    l.set_xalign(0.0);
    l
}

fn dim_mono(text: &str) -> gtk::Label {
    let l = mono(text);
    l.add_css_class("dim-label");
    l.add_css_class("caption");
    l
}

fn set_text(w: &gtk::Widget, text: &str) {
    if let Some(l) = w.downcast_ref::<gtk::Label>() {
        l.set_text(text);
    }
}

fn label_list(doc: &Rc<Document>) -> ValueList<LabelInfo> {
    let d = Rc::clone(doc);
    ValueList::new(
        false,
        || {
            let r = row_box();
            let name = mono("");
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            r.append(&name);
            r.append(&spacer());
            r.append(&chip("", "chip-accent"));
            r.append(&dim_mono(""));
            // Dragged into the editor area: a new Disassembly tab at the
            // label, where it is dropped (docs/29). The row is reused, so it
            // carries its label's address in its name.
            let row = r.clone();
            r.add_controller(crate::tabgrid::drag_source(move || {
                let address = u32::from_str_radix(&row.widget_name(), 16).ok()?;
                Some(TabDrop::OpenAt(CodeRep::Assembly, address))
            }));
            r.upcast()
        },
        |row: &gtk::Widget, l: &LabelInfo| {
            row.set_widget_name(&format!("{:06X}", l.address));
            let name = child_at(row, 0);
            set_text(&name, &l.name);
            if l.source == LabelSource::Auto {
                name.add_css_class("dim-label");
            } else {
                name.remove_css_class("dim-label");
            }
            let tag = child_at(row, 2);
            match l.source {
                LabelSource::User => set_text(&tag, "user"),
                LabelSource::Imported => set_text(&tag, &l.origin),
                _ => {}
            }
            tag.set_visible(matches!(
                l.source,
                LabelSource::User | LabelSource::Imported
            ));
            set_text(
                &child_at(row, 3),
                &romlens_ffi::format_snes_address(l.address),
            );
        },
        move |_, l: &LabelInfo| d.jump_to_snes(l.address),
    )
}

/// Variables are mostly RAM, which the editor cannot scroll to, so a row
/// lists what uses the variable.
fn variable_list(doc: &Rc<Document>) -> ValueList<VariableInfo> {
    let d = Rc::clone(doc);
    ValueList::new(
        false,
        || {
            let r = row_box();
            r.append(&mono(""));
            r.append(&chip("", ""));
            r.append(&spacer());
            r.append(&dim_mono(""));
            r.upcast()
        },
        |row: &gtk::Widget, v: &VariableInfo| {
            set_text(
                &child_at(row, 0),
                if v.name.is_empty() {
                    "(unnamed)"
                } else {
                    &v.name
                },
            );
            set_text(&child_at(row, 1), &v.description);
            set_text(
                &child_at(row, 3),
                &romlens_ffi::format_snes_address(v.address),
            );
            row.set_tooltip_text(Some(&format!(
                "{}, {} byte{}",
                v.memory,
                v.len,
                if v.len == 1 { "" } else { "s" }
            )));
        },
        move |_, v: &VariableInfo| d.find_references_to(v.address),
    )
}

fn variables_page(list: &ValueList<VariableInfo>, doc: &Rc<Document>) -> gtk::Box {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&list.widget));
    let hint = gtk::Label::builder()
        .label(
            "No variables. Name a RAM address with Define Variable…, or \
             right-click an instruction to name what it reads or writes.",
        )
        .wrap(true)
        .justify(gtk::Justification::Center)
        .margin_start(16)
        .margin_end(16)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .can_target(false)
        .build();
    hint.add_css_class("dim-label");
    overlay.add_overlay(&hint);
    // Hidden once there is a variable to list.
    doc.subscribe({
        let (doc, hint) = (Rc::clone(doc), hint.clone());
        move |c| {
            if c == Change::Navigator {
                hint.set_visible(doc.navigator().data.variables.is_empty());
            }
        }
    });
    overlay.set_vexpand(true);

    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.append(&overlay);
    let add = gtk::Button::builder()
        .label("Define Variable…")
        .icon_name("list-add-symbolic")
        .halign(gtk::Align::Start)
        .margin_start(8)
        .margin_top(4)
        .build();
    add.add_css_class("flat");
    let d = Rc::clone(doc);
    add.connect_clicked(move |_| d.begin_new_variable());
    page.append(&add);
    page
}

fn region_list(doc: &Rc<Document>) -> ValueList<RegionInfo> {
    let d = Rc::clone(doc);
    ValueList::new(
        false,
        || {
            let r = row_box();
            r.append(&chip("", ""));
            r.append(&dim_mono(""));
            r.append(&spacer());
            r.append(&dim_mono(""));
            r.upcast()
        },
        |row: &gtk::Widget, region: &RegionInfo| {
            let kind = child_at(row, 0);
            set_text(&kind, &region.name);
            for c in ["chip-blue", "chip-orange", "chip-gray"] {
                kind.remove_css_class(c);
            }
            kind.add_css_class(match region.kind {
                RegionKind::Code => "chip-blue",
                RegionKind::Data => "chip-orange",
                RegionKind::Unknown => "chip-gray",
            });
            set_text(
                &child_at(row, 1),
                &format!(
                    "{} · {} B",
                    romlens_ffi::format_file_offset(region.start),
                    region.len
                ),
            );
            set_text(
                &child_at(row, 3),
                &format!("{}%", (region.confidence * 100.0).round() as i32),
            );
        },
        move |_, region: &RegionInfo| d.jump_to(region.start),
    )
}

fn regions_page(list: &ValueList<RegionInfo>, doc: &Rc<Document>) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let note = gtk::Label::builder()
        .label(format!(
            "Largest {} of each kind",
            crate::model::navigator::REGION_LIMIT
        ))
        .xalign(0.0)
        .margin_start(12)
        .margin_top(4)
        .margin_bottom(4)
        .tooltip_text(
            "A classified ROM has far more regions than a list can usefully \
             hold; the overview strip shows all of them.",
        )
        .build();
    note.add_css_class("caption");
    note.add_css_class("dim-label");
    page.append(&note);
    page.append(&list.widget);
    let (doc, note) = (Rc::clone(doc), note.clone());
    doc.clone().subscribe(move |c| {
        if c == Change::Navigator {
            note.set_visible(doc.navigator().data.regions_truncated);
        }
    });
    page
}

fn bank_list(doc: &Rc<Document>) -> ValueList<Bank> {
    let d = Rc::clone(doc);
    ValueList::new(
        false,
        || {
            let r = row_box();
            r.append(&mono(""));
            r.append(&spacer());
            r.append(&dim_mono(""));
            r.upcast()
        },
        |row: &gtk::Widget, bank: &Bank| {
            set_text(&child_at(row, 0), &format!("Bank ${:02X}", bank.bank));
            set_text(
                &child_at(row, 2),
                &format!(
                    "{} · {} KB",
                    romlens_ffi::format_file_offset(bank.file_offset),
                    bank.length / 1024
                ),
            );
        },
        move |_, bank: &Bank| d.jump_to(bank.file_offset),
    )
}
