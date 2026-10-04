//! The Tutor's sheets as dialogs: `/resume`, `/rewind`, `/model`, `/help`,
//! then the lessons, the map, the quiz and the progress pane (their own
//! modules). The macOS twin is `TutorSheets.swift`.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use romlens_ffi::tutor::session::{RewindWhat, TurnBlockInfo, TurnInfo};

use crate::gfxdraw::caption;
use crate::messageview::OnLink;
use crate::model::Document;
use crate::model::tutor::{COMMANDS, Sheet};

/// Puts text in the composer.
pub type Composer = Rc<dyn Fn(&str)>;

pub fn build(doc: &Rc<Document>, sheet: Sheet, composer: Composer, on_link: OnLink) -> adw::Dialog {
    match sheet {
        Sheet::Resume => resume(doc),
        Sheet::Rewind => rewind(doc, composer),
        Sheet::Model => model(doc),
        Sheet::Help => help(),
        Sheet::Lessons | Sheet::Map | Sheet::Progress => {
            crate::lessonsheets::library(doc, sheet, composer, on_link)
        }
        Sheet::Quiz => crate::quizview::quiz(doc, on_link),
    }
}

/// A dialog with a title, a body and a row of buttons at the bottom.
pub fn dialog(
    title: &str,
    width: i32,
    body: &impl IsA<gtk::Widget>,
    buttons: &[&gtk::Button],
) -> adw::Dialog {
    let d = adw::Dialog::builder()
        .title(title)
        .content_width(width)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin_start(16);
    root.set_margin_end(16);
    root.set_margin_top(10);
    root.set_margin_bottom(16);
    root.append(body);
    if !buttons.is_empty() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_halign(gtk::Align::End);
        for b in buttons {
            row.append(*b);
        }
        root.append(&row);
    }
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&root));
    d.set_child(Some(&view));
    d
}

pub fn date(seconds: u64) -> String {
    glib::DateTime::from_unix_local(seconds as i64)
        .ok()
        .and_then(|d| d.format("%b %-d, %Y, %H:%M").ok())
        .map_or_else(String::new, |s| s.to_string())
}

fn list_frame(list: &gtk::ListBox, height: i32) -> gtk::ScrolledWindow {
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    gtk::ScrolledWindow::builder()
        .min_content_height(height)
        .max_content_height(height + 120)
        .propagate_natural_height(true)
        .child(list)
        .build()
}

fn wrap_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build()
}

// MARK: /resume

/// The project's conversations, latest first, each by the name the model gave
/// it after its first answer.
fn resume(doc: &Rc<Document>) -> adw::Dialog {
    let open = doc.tutor().session().and_then(|s| s.conversation_id());
    let list = gtk::ListBox::new();
    let ids: Rc<RefCell<Vec<String>>> = Rc::default();
    let scroll = list_frame(&list, 240);
    let empty = gtk::Label::new(Some("No conversations about this ROM yet."));
    empty.add_css_class("dim-label");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.append(&scroll);
    body.append(&empty);

    let delete = gtk::Button::with_label("Delete");
    delete.add_css_class("destructive-action");
    let new = gtk::Button::with_label("New Conversation");
    let cancel = gtk::Button::with_label("Cancel");
    let go = gtk::Button::with_label("Resume");
    go.add_css_class("suggested-action");
    let d = dialog("Conversations", 480, &body, &[&delete, &new, &cancel, &go]);

    let fill = {
        let (doc, list, ids, open, empty) = (
            Rc::clone(doc),
            list.clone(),
            Rc::clone(&ids),
            open.clone(),
            empty.clone(),
        );
        Rc::new(move || {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let all = doc.tutor().conversations();
            empty.set_visible(all.is_empty());
            *ids.borrow_mut() = all.iter().map(|c| c.id.clone()).collect();
            for c in &all {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&c.title))
                    .subtitle(glib::markup_escape_text(&format!(
                        "{} · {} · ${:.3}{}",
                        date(c.updated),
                        c.model,
                        c.cost,
                        if open.as_deref() == Some(&c.id) {
                            " · open"
                        } else {
                            ""
                        }
                    )))
                    .build();
                list.append(&row);
            }
            if let Some(i) = open
                .as_deref()
                .and_then(|o| all.iter().position(|c| c.id == o))
                .or((!all.is_empty()).then_some(0))
                && let Some(r) = list.row_at_index(i as i32)
            {
                list.select_row(Some(&r));
            }
        })
    };
    fill();
    let selected = {
        let (list, ids) = (list.clone(), Rc::clone(&ids));
        move || {
            list.selected_row()
                .and_then(|r| ids.borrow().get(r.index() as usize).cloned())
        }
    };
    let resume = {
        let (doc, d, open) = (Rc::clone(doc), d.clone(), open.clone());
        Rc::new(move |id: String| {
            if doc.tutor().busy {
                return;
            }
            if open.as_deref() != Some(&id) {
                doc.edit_tutor(|t| t.resume(&id));
            }
            d.close();
        })
    };
    {
        let (selected, resume) = (selected.clone(), Rc::clone(&resume));
        go.connect_clicked(move |_| {
            if let Some(id) = selected() {
                resume(id);
            }
        });
    }
    {
        let (ids, resume) = (Rc::clone(&ids), Rc::clone(&resume));
        list.connect_row_activated(move |_, r| {
            if let Some(id) = ids.borrow().get(r.index() as usize).cloned() {
                resume(id);
            }
        });
    }
    {
        let (doc, selected, fill, open) = (
            Rc::clone(doc),
            selected.clone(),
            Rc::clone(&fill),
            open.clone(),
        );
        delete.connect_clicked(move |_| {
            if let Some(id) = selected()
                && open.as_deref() != Some(&id)
            {
                doc.edit_tutor(|t| t.delete_conversation(&id));
                fill();
            }
        });
    }
    let update = {
        let (list, delete, go, doc, open, ids) = (
            list.clone(),
            delete.clone(),
            go.clone(),
            Rc::clone(doc),
            open,
            ids,
        );
        move || {
            let id = list
                .selected_row()
                .and_then(|r| ids.borrow().get(r.index() as usize).cloned());
            delete.set_sensitive(id.is_some() && id != open);
            go.set_sensitive(id.is_some() && !doc.tutor().busy);
        }
    };
    update();
    list.connect_selected_rows_changed(move |_| update());
    new.set_sensitive(!doc.tutor().busy);
    {
        let (doc, d) = (Rc::clone(doc), d.clone());
        new.connect_clicked(move |_| {
            doc.edit_tutor(|t| t.new_conversation());
            d.close();
        });
    }
    {
        let d = d.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
    }
    d.set_default_widget(Some(&go));
    d
}

// MARK: /rewind

/// Back to an earlier question, with the tutor's edits since, or either alone.
fn rewind(doc: &Rc<Document>, composer: Composer) -> adw::Dialog {
    let points: Vec<TurnInfo> = doc
        .tutor()
        .session()
        .map_or_else(Vec::new, |s| s.rewind_points())
        .into_iter()
        .rev()
        .collect();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.append(&caption(
        "Go back to before a question. Its words come back into the composer.",
    ));
    let list = gtk::ListBox::new();
    for t in &points {
        let l = wrap_label(&words(t));
        l.set_lines(2);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_margin_top(6);
        l.set_margin_bottom(6);
        l.set_margin_start(8);
        l.set_margin_end(8);
        list.append(&l);
    }
    if let Some(r) = list.row_at_index(0) {
        list.select_row(Some(&r));
    }
    body.append(&list_frame(&list, 200));
    let both = gtk::CheckButton::with_label("The conversation and the tutor's edits");
    let conversation = gtk::CheckButton::with_label("The conversation only");
    let edits = gtk::CheckButton::with_label("The tutor's edits only");
    conversation.set_group(Some(&both));
    edits.set_group(Some(&both));
    both.set_active(true);
    for b in [&both, &conversation, &edits] {
        body.append(b);
    }
    let report = caption("");
    report.set_wrap(true);
    body.append(&report);

    let cancel = gtk::Button::with_label("Cancel");
    let go = gtk::Button::with_label("Rewind");
    go.add_css_class("suggested-action");
    go.set_sensitive(!points.is_empty() && !doc.tutor().busy);
    let d = dialog("Rewind", 480, &body, &[&cancel, &go]);
    {
        let d = d.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
    }
    let (doc, d2) = (Rc::clone(doc), d.clone());
    go.connect_clicked(move |_| {
        let Some(t) = list
            .selected_row()
            .and_then(|r| points.get(r.index() as usize))
        else {
            return;
        };
        let what = if conversation.is_active() {
            RewindWhat::Conversation
        } else if edits.is_active() {
            RewindWhat::Edits
        } else {
            RewindWhat::Both
        };
        match doc.edit_tutor(|m| m.rewind(t.index, what)) {
            Ok(r) => {
                if let Some(p) = &r.prompt {
                    composer(p);
                }
                doc.tutor_edited();
                if let Some(e) = r.edits.as_ref().filter(|e| !e.kept.is_empty()) {
                    report.set_text(&format!(
                        "Kept {} edit{} you changed afterwards: {}.",
                        e.kept.len(),
                        if e.kept.len() == 1 { "" } else { "s" },
                        e.kept.join(", ")
                    ));
                    return;
                }
                d2.close();
            }
            Err(e) => report.set_text(&e),
        }
    });
    d.set_default_widget(Some(&go));
    d
}

/// The words of a question, for the list.
fn words(t: &TurnInfo) -> String {
    t.blocks
        .iter()
        .rev()
        .find_map(|b| match b {
            TurnBlockInfo::Text { text } if !text.starts_with('[') => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "A picture".into())
}

// MARK: /model

/// The provider, the model and its effort, from the next question on.
fn model(doc: &Rc<Document>) -> adw::Dialog {
    let settings = crate::settings::tutor();
    let (endpoints, current) = {
        let st = settings.borrow();
        let t = doc.tutor();
        let s = t.session();
        let default = st.default_endpoint();
        let id = s
            .and_then(|s| s.endpoint())
            .map_or(default.id.clone(), |e| e.id);
        let e = st.endpoint(&id).unwrap_or(default);
        let model = s
            .and_then(|s| s.model())
            .or_else(|| st.model(&e))
            .unwrap_or_default();
        let effort = s.and_then(|s| s.effort()).unwrap_or_default();
        (st.endpoints(), (e.id, model, effort))
    };
    let names: Vec<String> = {
        let st = settings.borrow();
        endpoints
            .iter()
            .map(|e| format!("{}{}", e.name, if st.ready(e) { "" } else { " (no key)" }))
            .collect()
    };
    let group = adw::PreferencesGroup::new();
    let provider = adw::ComboRow::builder()
        .title("Provider")
        .model(&gtk::StringList::new(
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .build();
    provider.set_selected(
        endpoints
            .iter()
            .position(|e| e.id == current.0)
            .unwrap_or(0) as u32,
    );
    // The model is a list for a provider with a table, else a field (a local
    // server's models come from asking it).
    let model_combo = adw::ComboRow::builder().title("Model").build();
    let model_entry = adw::EntryRow::builder().title("Model").build();
    let effort = adw::ComboRow::builder().title("Effort").build();
    for r in [
        provider.upcast_ref::<gtk::Widget>(),
        model_combo.upcast_ref(),
        model_entry.upcast_ref(),
        effort.upcast_ref(),
    ] {
        group.add(r);
    }
    group.set_margin_bottom(4);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.append(&group);
    body.append(&caption(
        "The conversation goes on with the new model; its cache starts cold.",
    ));

    let cancel = gtk::Button::with_label("Cancel");
    let go = gtk::Button::with_label("Use");
    go.add_css_class("suggested-action");
    let d = dialog("Model", 420, &body, &[&cancel, &go]);
    {
        let d = d.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
    }

    // The ids behind the model list, and what the efforts are called.
    let ids: Rc<RefCell<Vec<String>>> = Rc::default();
    let efforts: Rc<RefCell<Vec<String>>> = Rc::default();
    let endpoints = Rc::new(endpoints);
    let current = Rc::new(current);
    let refill_efforts = {
        let (effort, efforts, ids, model_combo, endpoints, provider) = (
            effort.clone(),
            Rc::clone(&efforts),
            Rc::clone(&ids),
            model_combo.clone(),
            Rc::clone(&endpoints),
            provider.clone(),
        );
        let settings = Rc::clone(&settings);
        Rc::new(move |wanted: &str| {
            let e = &endpoints[provider.selected() as usize];
            let table = settings.borrow().table_models(e);
            let id = ids
                .borrow()
                .get(model_combo.selected() as usize)
                .cloned()
                .unwrap_or_default();
            let list = table
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.efforts.clone())
                .unwrap_or_default();
            effort.set_visible(!list.is_empty());
            let mut labels = vec!["The model's default".to_owned()];
            labels.extend(list.iter().cloned());
            effort.set_model(Some(&gtk::StringList::new(
                &labels.iter().map(String::as_str).collect::<Vec<_>>(),
            )));
            effort.set_selected(
                list.iter()
                    .position(|f| f == wanted)
                    .map_or(0, |i| i as u32 + 1),
            );
            *efforts.borrow_mut() = list;
        })
    };
    let refill_models = {
        let (ids, model_combo, model_entry, endpoints, provider, refill_efforts) = (
            Rc::clone(&ids),
            model_combo.clone(),
            model_entry.clone(),
            Rc::clone(&endpoints),
            provider.clone(),
            Rc::clone(&refill_efforts),
        );
        let settings = Rc::clone(&settings);
        let current = Rc::clone(&current);
        Rc::new(move |first: bool| {
            let e = &endpoints[provider.selected() as usize];
            let table = settings.borrow().table_models(e);
            let wanted = if first && current.0 == e.id {
                current.1.clone()
            } else {
                settings.borrow().model(e).unwrap_or_default()
            };
            model_combo.set_visible(!table.is_empty());
            model_entry.set_visible(table.is_empty());
            *ids.borrow_mut() = table.iter().map(|m| m.id.clone()).collect();
            if table.is_empty() {
                model_entry.set_text(&wanted);
            } else {
                model_combo.set_model(Some(&gtk::StringList::new(
                    &table.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
                )));
                model_combo
                    .set_selected(table.iter().position(|m| m.id == wanted).unwrap_or(0) as u32);
            }
            refill_efforts(if first && current.0 == e.id {
                &current.2
            } else {
                ""
            });
        })
    };
    refill_models(true);
    {
        let refill = Rc::clone(&refill_models);
        provider.connect_selected_notify(move |_| refill(false));
    }
    {
        let refill = Rc::clone(&refill_efforts);
        model_combo.connect_selected_notify(move |_| refill(""));
    }
    {
        let (doc, d, endpoints, provider, ids, efforts, model_combo, model_entry, effort) = (
            Rc::clone(doc),
            d.clone(),
            Rc::clone(&endpoints),
            provider.clone(),
            ids,
            efforts,
            model_combo,
            model_entry,
            effort,
        );
        go.connect_clicked(move |_| {
            let e = &endpoints[provider.selected() as usize];
            let model = if model_combo.is_visible() {
                ids.borrow()
                    .get(model_combo.selected() as usize)
                    .cloned()
                    .unwrap_or_default()
            } else {
                model_entry.text().trim().to_owned()
            };
            if model.is_empty() || doc.tutor().busy {
                return;
            }
            let chosen = (effort.is_visible() && effort.selected() > 0)
                .then(|| {
                    efforts
                        .borrow()
                        .get(effort.selected() as usize - 1)
                        .cloned()
                })
                .flatten();
            doc.edit_tutor(|t| t.use_model(e, &model, chosen));
            d.close();
        });
    }
    d.set_default_widget(Some(&go));
    d
}

// MARK: /help

fn help() -> adw::Dialog {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.append(&wrap_label(
        "It reads Romlens's analysis with its tools (the listing, the C, the graphs, registers, tiles, \
         frames, sound) and cites what it finds; click a citation to go there in the main window.",
    ));
    let grid = gtk::Grid::builder()
        .column_spacing(14)
        .row_spacing(4)
        .build();
    let keys = [
        ("Return", "Send"),
        ("Shift+Return", "A new line"),
        ("Up, Down", "Earlier questions, at the first or last line"),
        ("Shift+Tab", "Read-only, Ask before edits, Accept edits"),
        ("Esc", "Stop the answer; twice to rewind"),
        ("Paste, drop", "Attach a screenshot or photo"),
    ];
    for (i, (k, a)) in keys.iter().enumerate() {
        let key = gtk::Label::builder().label(*k).xalign(0.0).build();
        key.add_css_class("heading");
        grid.attach(&key, 0, i as i32, 1, 1);
        grid.attach(&wrap_label(a), 1, i as i32, 1, 1);
    }
    body.append(&grid);
    body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let cmds = gtk::Grid::builder()
        .column_spacing(14)
        .row_spacing(4)
        .build();
    for (i, c) in COMMANDS.iter().filter(|c| c.name != "/clear").enumerate() {
        let n = gtk::Label::builder()
            .label(c.name)
            .xalign(0.0)
            .width_request(90)
            .build();
        n.add_css_class("monospace");
        cmds.attach(&n, 0, i as i32, 1, 1);
        let a = wrap_label(c.about);
        a.add_css_class("dim-label");
        a.set_hexpand(true);
        cmds.attach(&a, 1, i as i32, 1, 1);
    }
    body.append(&cmds);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(420)
        .propagate_natural_height(true)
        .child(&body)
        .build();
    dialog("The tutor", 520, &scroll, &[])
}
