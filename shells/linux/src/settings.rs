//! Settings (Ctrl+,): the tutor's providers and their keys, its defaults, the
//! pictures it may draw, and what it sends (docs/24, "Settings"), beside what
//! the app remembers about the listing. The macOS twin is `SettingsView`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::gio;

use crate::config::Settings;
use crate::model::tutor_settings::{Endpoint, Kind, ModePreference, TutorSettings};
#[cfg(not(test))]
use crate::secrets::SecretService;

type Shared = Rc<RefCell<TutorSettings>>;

thread_local! {
    #[cfg(not(test))]
    static TUTOR: Shared = Rc::new(RefCell::new(TutorSettings::load(Arc::new(SecretService))));
    // Tests never touch the person's settings or their keyring.
    #[cfg(test)]
    static TUTOR: Shared = {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir()
            .join(format!("romlens-tutor-settings-{}-{n}", std::process::id()))
            .join("tutor.json");
        Rc::new(RefCell::new(TutorSettings::load_from(
            &path,
            Arc::new(crate::secrets::MemoryKeyStore::default()),
        )))
    };
}

/// The tutor's settings, one for the whole app.
pub fn tutor() -> Shared {
    TUTOR.with(Rc::clone)
}

/// A function the Providers page calls to rebuild itself, filled in once it exists.
type Rebuild = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

pub fn show(app: &adw::Application) {
    let dialog = adw::PreferencesDialog::builder().title("Settings").build();
    dialog.add(&general_page(app));
    let settings = tutor();
    dialog.add(&providers_page(&settings));
    dialog.add(&tutor_page(&settings));
    dialog.add(&images_page(&settings));
    dialog.add(&privacy_page());
    // A development aid: open on a page, to look at it without a click.
    if let Ok(page) = std::env::var("ROMLENS_SETTINGS_PAGE") {
        dialog.set_visible_page_name(&page);
    }
    dialog.present(
        app.active_window()
            .or_else(|| app.windows().into_iter().next())
            .as_ref(),
    );
}

// MARK: General

fn general_page(app: &adw::Application) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("General")
        .name("general")
        .icon_name("preferences-system-symbolic")
        .build();
    let group = adw::PreferencesGroup::builder().title("Listing").build();
    let explanations = adw::SwitchRow::builder()
        .title("Show explanations")
        .subtitle("Explained comments on hardware writes, and a note above each idiom, in the listing and the C")
        .active(!Settings::load().hide_explanations)
        .build();
    explanations.connect_active_notify({
        let app = app.clone();
        move |row| {
            let show = row.is_active();
            // The open windows follow, each through its own action, which also
            // remembers the choice.
            let mut any = false;
            for w in app.windows() {
                if let Some(a) = w
                    .downcast_ref::<adw::ApplicationWindow>()
                    .and_then(|w| w.lookup_action("toggle-explanations"))
                    && a.state().and_then(|s| s.get::<bool>()) != Some(show)
                {
                    a.activate(None);
                    any = true;
                }
            }
            if !any {
                let mut saved = Settings::load();
                saved.hide_explanations = !show;
                saved.save();
            }
        }
    });
    group.add(&explanations);
    page.add(&group);
    page
}

// MARK: Providers

fn providers_page(settings: &Shared) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Providers")
        .name("providers")
        .icon_name("network-server-symbolic")
        .build();
    let group = adw::PreferencesGroup::builder()
        .title("Providers")
        .description("Where the tutor's questions go, and the keys that let it. Keys are kept in the desktop's secret store, never in a file.")
        .build();
    page.add(&group);
    let add_group = adw::PreferencesGroup::new();
    let add = adw::ButtonRow::builder()
        .title("Add an Endpoint…")
        .start_icon_name("list-add-symbolic")
        .build();
    add_group.add(&add);
    page.add(&add_group);

    let rows: Rc<RefCell<Vec<adw::ExpanderRow>>> = Rc::new(RefCell::new(Vec::new()));
    let rebuild: Rebuild = Rc::new(RefCell::new(None));
    let build: Rc<dyn Fn()> = {
        let (settings, group, rows, rebuild) = (
            Rc::clone(settings),
            group.clone(),
            Rc::clone(&rows),
            Rc::clone(&rebuild),
        );
        Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                group.remove(&r);
            }
            let endpoints = settings.borrow().endpoints();
            for e in endpoints {
                let row = endpoint_row(&settings, &e, &rebuild);
                group.add(&row);
                rows.borrow_mut().push(row);
            }
        })
    };
    *rebuild.borrow_mut() = Some(Rc::clone(&build));
    build();
    add.connect_activated({
        let (settings, build) = (Rc::clone(settings), Rc::clone(&build));
        move |row| add_endpoint(row, &settings, Rc::clone(&build))
    });
    page
}

fn ready_icon(settings: &Shared, e: &Endpoint) -> gtk::Image {
    let ready = settings.borrow().ready(e);
    let icon = gtk::Image::from_icon_name(if ready {
        "object-select-symbolic"
    } else {
        "dialog-password-symbolic"
    });
    icon.set_tooltip_text(Some(if ready {
        if e.needs_key {
            "A key is saved"
        } else {
            "Needs no key"
        }
    } else {
        "No key yet"
    }));
    icon
}

fn endpoint_row(settings: &Shared, e: &Endpoint, rebuild: &Rebuild) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::builder()
        .title(&e.name)
        .subtitle(&e.base_url)
        .build();
    row.add_suffix(&ready_icon(settings, e));
    let current = Rc::new(RefCell::new(e.clone()));

    if !e.built_in {
        let name = adw::EntryRow::builder().title("Name").text(&e.name).build();
        let url = adw::EntryRow::builder()
            .title("Base URL")
            .text(&e.base_url)
            .build();
        let kind = adw::ComboRow::builder()
            .title("Protocol")
            .model(&gtk::StringList::new(&[
                Kind::Chat.title(),
                Kind::Responses.title(),
            ]))
            .selected(u32::from(e.kind == Kind::Responses))
            .build();
        let vision = adw::SwitchRow::builder()
            .title("Its models see pictures")
            .active(e.vision)
            .build();
        let choice = adw::SwitchRow::builder()
            .title("It takes tool_choice")
            .active(e.toolchoice())
            .build();
        let needs = adw::SwitchRow::builder()
            .title("It needs a key")
            .active(e.needs_key)
            .build();
        let save = {
            let (settings, current) = (Rc::clone(settings), Rc::clone(&current));
            let (name, url, kind, vision, choice, needs) = (
                name.clone(),
                url.clone(),
                kind.clone(),
                vision.clone(),
                choice.clone(),
                needs.clone(),
            );
            move || {
                let mut c = current.borrow_mut();
                c.name = name.text().to_string();
                c.base_url = url.text().to_string();
                c.kind = if kind.selected() == 1 {
                    Kind::Responses
                } else {
                    Kind::Chat
                };
                c.vision = vision.is_active();
                c.tool_choice = choice.is_active();
                c.needs_key = needs.is_active();
                settings.borrow_mut().update(c.clone());
            }
        };
        for w in [&name, &url] {
            let s = save.clone();
            w.connect_changed(move |_| s());
        }
        let s = save.clone();
        kind.connect_selected_notify(move |_| s());
        for w in [&vision, &choice, &needs] {
            let s = save.clone();
            w.connect_active_notify(move |_| s());
        }
        for w in [
            name.upcast_ref::<gtk::Widget>(),
            url.upcast_ref(),
            kind.upcast_ref(),
            vision.upcast_ref(),
            choice.upcast_ref(),
            needs.upcast_ref(),
        ] {
            row.add_row(w);
        }
    } else {
        let at = adw::ActionRow::builder()
            .title("Endpoint")
            .subtitle(&e.base_url)
            .build();
        row.add_row(&at);
    }

    // The key.
    let key = adw::PasswordEntryRow::builder()
        .title(if settings.borrow().has_key(e) {
            "API key (one is saved)"
        } else {
            "API key"
        })
        .build();
    let status = adw::ActionRow::builder().title("").build();
    status.add_css_class("dim-label");
    status.set_visible(false);
    let buttons = adw::ActionRow::builder().title("").build();
    let save_key = gtk::Button::with_label("Save Key");
    let remove_key = gtk::Button::with_label("Remove Key");
    let test = gtk::Button::with_label("Test");
    for b in [&save_key, &remove_key, &test] {
        b.set_valign(gtk::Align::Center);
        buttons.add_suffix(b);
    }
    remove_key.set_sensitive(settings.borrow().has_key(e));
    if e.needs_key || e.built_in {
        row.add_row(&key);
    }
    row.add_row(&buttons);
    row.add_row(&status);
    let say = {
        let status = status.clone();
        move |t: &str| {
            status.set_title(t);
            status.set_visible(!t.is_empty());
        }
    };
    save_key.connect_clicked({
        let (settings, current, key, say, remove_key, rebuild) = (
            Rc::clone(settings),
            Rc::clone(&current),
            key.clone(),
            say.clone(),
            remove_key.clone(),
            Rc::clone(rebuild),
        );
        move |_| {
            let id = current.borrow().id.clone();
            match settings.borrow().keys.set_key(&id, Some(&key.text())) {
                Ok(()) => {
                    key.set_text("");
                    remove_key.set_sensitive(true);
                    say("Saved.");
                    if let Some(f) = rebuild.borrow().clone() {
                        f();
                    }
                }
                Err(e) => say(&e),
            }
        }
    });
    remove_key.connect_clicked({
        let (settings, current, say, rebuild) = (
            Rc::clone(settings),
            Rc::clone(&current),
            say.clone(),
            Rc::clone(rebuild),
        );
        move |b| {
            let id = current.borrow().id.clone();
            match settings.borrow().keys.set_key(&id, None) {
                Ok(()) => {
                    b.set_sensitive(false);
                    say("Removed.");
                    if let Some(f) = rebuild.borrow().clone() {
                        f();
                    }
                }
                Err(e) => say(&e),
            }
        }
    });
    test.connect_clicked({
        let (settings, current, say) = (Rc::clone(settings), Rc::clone(&current), say);
        move |b| {
            let (info, id) = {
                let c = current.borrow();
                (c.info(), c.id.clone())
            };
            let key = settings.borrow().keys.key(&id);
            b.set_sensitive(false);
            say("Testing…");
            let (b, say) = (b.clone(), say.clone());
            gtk::glib::spawn_future_local(async move {
                let result = gio::spawn_blocking(move || {
                    romlens_ffi::tutor::session::tutor_list_models(info, key)
                        .map_err(|e| e.to_string())
                })
                .await;
                b.set_sensitive(true);
                match result {
                    Ok(Ok(ids)) if ids.is_empty() => say("Reached it, but it lists no models."),
                    Ok(Ok(ids)) => say(&format!(
                        "It answers: {} models, {}{}",
                        ids.len(),
                        ids.iter().take(6).cloned().collect::<Vec<_>>().join(", "),
                        if ids.len() > 6 { "…" } else { "" }
                    )),
                    Ok(Err(e)) => say(&e),
                    Err(_) => say("The test did not finish."),
                }
            });
        }
    });

    if !e.built_in {
        let remove = adw::ButtonRow::builder()
            .title("Remove This Endpoint")
            .build();
        remove.add_css_class("destructive-action");
        let (settings, current, rebuild) =
            (Rc::clone(settings), Rc::clone(&current), Rc::clone(rebuild));
        remove.connect_activated(move |_| {
            settings.borrow_mut().remove(&current.borrow().id);
            if let Some(f) = rebuild.borrow().clone() {
                f();
            }
        });
        row.add_row(&remove);
    }
    row
}

impl Endpoint {
    fn toolchoice(&self) -> bool {
        self.tool_choice
    }
}

fn add_endpoint(from: &impl IsA<gtk::Widget>, settings: &Shared, rebuilt: Rc<dyn Fn()>) {
    let dialog = adw::AlertDialog::new(
        Some("Add an Endpoint"),
        Some(
            "Ollama, LM Studio, vLLM and LiteLLM all speak Chat Completions; Ollama from \
             0.13.3, LM Studio and LiteLLM speak Responses too.",
        ),
    );
    let name = adw::EntryRow::builder()
        .title("Name")
        .text("Ollama")
        .build();
    let url = adw::EntryRow::builder()
        .title("Base URL")
        .text("http://localhost:11434/v1")
        .build();
    let kind = adw::ComboRow::builder()
        .title("Protocol")
        .model(&gtk::StringList::new(&[
            Kind::Chat.title(),
            Kind::Responses.title(),
        ]))
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");
    for w in [
        name.upcast_ref::<gtk::Widget>(),
        url.upcast_ref(),
        kind.upcast_ref(),
    ] {
        list.append(w);
    }
    dialog.set_extra_child(Some(&list));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("add", "Add");
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));
    dialog.set_close_response("cancel");
    let settings = Rc::clone(settings);
    dialog.choose(Some(from), gio::Cancellable::NONE, move |response| {
        if response != "add" {
            return;
        }
        let (n, u) = (name.text().to_string(), url.text().to_string());
        if n.trim().is_empty() || !u.contains("://") {
            return;
        }
        let taken: Vec<String> = settings
            .borrow()
            .endpoints()
            .into_iter()
            .map(|e| e.id)
            .collect();
        let k = if kind.selected() == 1 {
            Kind::Responses
        } else {
            Kind::Chat
        };
        settings
            .borrow_mut()
            .add(Endpoint::local(&n, &u, k, &taken));
        rebuilt();
    });
}

// MARK: Tutor

fn tutor_page(settings: &Shared) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Tutor")
        .name("tutor")
        .icon_name("applications-science-symbolic")
        .build();
    let defaults = adw::PreferencesGroup::builder()
        .title("A New Conversation")
        .build();
    let endpoints = settings.borrow().endpoints();
    let names: Vec<&str> = endpoints.iter().map(|e| e.name.as_str()).collect();
    let provider = adw::ComboRow::builder()
        .title("Provider")
        .model(&gtk::StringList::new(&names))
        .build();
    let current = settings.borrow().default_endpoint();
    provider.set_selected(
        endpoints
            .iter()
            .position(|e| e.id == current.id)
            .unwrap_or(0) as u32,
    );
    let model_combo = adw::ComboRow::builder().title("Model").build();
    let model_entry = adw::EntryRow::builder()
        .title("Model, as the server names it (for example qwen3:32b)")
        .build();
    let effort = adw::ComboRow::builder().title("Effort").build();
    defaults.add(&provider);
    defaults.add(&model_combo);
    defaults.add(&model_entry);
    defaults.add(&effort);

    // What the choice of provider changes: the table of models or a typed
    // name, and the efforts the model takes.
    let syncing = Rc::new(std::cell::Cell::new(false));
    let refill = {
        let (settings, model_combo, model_entry, effort, syncing) = (
            Rc::clone(settings),
            model_combo.clone(),
            model_entry.clone(),
            effort.clone(),
            Rc::clone(&syncing),
        );
        Rc::new(move || {
            syncing.set(true);
            let e = settings.borrow().default_endpoint();
            let table = settings.borrow().table_models(&e);
            let chosen = settings.borrow().model(&e);
            model_combo.set_visible(!table.is_empty());
            model_entry.set_visible(table.is_empty());
            if table.is_empty() {
                model_entry.set_text(chosen.as_deref().unwrap_or(""));
            } else {
                let labels: Vec<String> = table
                    .iter()
                    .map(|m| {
                        format!(
                            "{}  ${}/${}",
                            m.name,
                            price(m.price_input),
                            price(m.price_output)
                        )
                    })
                    .collect();
                let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                model_combo.set_model(Some(&gtk::StringList::new(&refs)));
                model_combo.set_selected(
                    table
                        .iter()
                        .position(|m| Some(&m.id) == chosen.as_ref())
                        .unwrap_or(0) as u32,
                );
            }
            let efforts = table
                .iter()
                .find(|m| Some(&m.id) == chosen.as_ref())
                .map(|m| m.efforts.clone())
                .unwrap_or_default();
            effort.set_visible(!efforts.is_empty());
            if !efforts.is_empty() {
                let mut items = vec!["The model's default".to_owned()];
                items.extend(efforts.iter().cloned());
                let refs: Vec<&str> = items.iter().map(String::as_str).collect();
                effort.set_model(Some(&gtk::StringList::new(&refs)));
                let now = settings.borrow().effort(&e);
                effort.set_selected(
                    now.and_then(|n| efforts.iter().position(|x| *x == n))
                        .map_or(0, |i| i as u32 + 1),
                );
            }
            syncing.set(false);
        })
    };
    refill();
    provider.connect_selected_notify({
        let (settings, refill, syncing) =
            (Rc::clone(settings), Rc::clone(&refill), Rc::clone(&syncing));
        move |p| {
            if syncing.get() {
                return;
            }
            let all = settings.borrow().endpoints();
            if let Some(e) = all.get(p.selected() as usize) {
                let id = e.id.clone();
                settings.borrow_mut().edit(|s| s.endpoint = id);
                refill();
            }
        }
    });
    model_combo.connect_selected_notify({
        let (settings, refill, syncing) =
            (Rc::clone(settings), Rc::clone(&refill), Rc::clone(&syncing));
        move |c| {
            if syncing.get() {
                return;
            }
            let e = settings.borrow().default_endpoint();
            if let Some(m) = settings
                .borrow()
                .table_models(&e)
                .get(c.selected() as usize)
            {
                let (id, model) = (e.id.clone(), m.id.clone());
                settings.borrow_mut().edit(|s| {
                    s.models.insert(id, model);
                });
            }
            refill();
        }
    });
    model_entry.connect_changed({
        let (settings, syncing) = (Rc::clone(settings), Rc::clone(&syncing));
        move |row| {
            if syncing.get() {
                return;
            }
            let id = settings.borrow().default_endpoint().id;
            let text = row.text().to_string();
            settings.borrow_mut().edit(|s| {
                if text.trim().is_empty() {
                    s.models.remove(&id);
                } else {
                    s.models.insert(id, text.trim().to_owned());
                }
            });
        }
    });
    effort.connect_selected_notify({
        let (settings, syncing) = (Rc::clone(settings), Rc::clone(&syncing));
        move |c| {
            if syncing.get() {
                return;
            }
            let e = settings.borrow().default_endpoint();
            let table = settings.borrow().table_models(&e);
            let chosen = settings.borrow().model(&e);
            let efforts = table
                .iter()
                .find(|m| Some(&m.id) == chosen.as_ref())
                .map(|m| m.efforts.clone())
                .unwrap_or_default();
            let sel = c.selected() as usize;
            settings
                .borrow_mut()
                .edit(|s| match sel.checked_sub(1).and_then(|i| efforts.get(i)) {
                    Some(x) => {
                        s.efforts.insert(e.id.clone(), x.clone());
                    }
                    None => {
                        s.efforts.remove(&e.id);
                    }
                });
        }
    });
    page.add(&defaults);

    let behaviour = adw::PreferencesGroup::builder().title("Behaviour").build();
    let modes: Vec<&str> = ModePreference::ALL.iter().map(|m| m.title()).collect();
    let edits = adw::ComboRow::builder()
        .title("Edits")
        .subtitle("Shift+Tab in the Tutor window changes it for a conversation")
        .model(&gtk::StringList::new(&modes))
        .selected(
            ModePreference::ALL
                .iter()
                .position(|m| *m == settings.borrow().stored().mode)
                .unwrap_or(1) as u32,
        )
        .build();
    edits.connect_selected_notify({
        let settings = Rc::clone(settings);
        move |c| {
            let m = ModePreference::ALL[c.selected() as usize];
            settings.borrow_mut().edit(|s| s.mode = m);
        }
    });
    let cap = adw::SpinRow::builder()
        .title("Stop a conversation at (dollars)")
        .subtitle("Zero for no limit")
        .adjustment(&gtk::Adjustment::new(
            settings.borrow().stored().cost_cap.unwrap_or(0.0),
            0.0,
            10_000.0,
            0.5,
            5.0,
            0.0,
        ))
        .digits(2)
        .build();
    cap.connect_value_notify({
        let settings = Rc::clone(settings);
        move |r| {
            let v = r.value();
            settings
                .borrow_mut()
                .edit(|s| s.cost_cap = (v > 0.0).then_some(v));
        }
    });
    behaviour.add(&edits);
    behaviour.add(&cap);
    for (title, subtitle, get, set) in toggles() {
        let row = adw::SwitchRow::builder()
            .title(title)
            .subtitle(subtitle)
            .active(get(settings.borrow().stored()))
            .build();
        let settings = Rc::clone(settings);
        row.connect_active_notify(move |r| {
            let on = r.is_active();
            settings.borrow_mut().edit(|s| set(s, on));
            // Open Tutors follow: their sessions, and what they show.
            let check = settings.borrow().stored().check_lessons;
            crate::model::tutor::check_lessons_changed(check);
            crate::tutorview::settings_changed();
        });
        behaviour.add(&row);
    }
    page.add(&behaviour);
    page
}

type Toggle = (
    &'static str,
    &'static str,
    fn(&crate::model::tutor_settings::Stored) -> bool,
    fn(&mut crate::model::tutor_settings::Stored, bool),
);

fn toggles() -> Vec<Toggle> {
    vec![
        (
            "Show thinking and tool calls",
            "Details in the Tutor window's status line, or /details, switches it there too",
            |s| s.show_work,
            |s, v| s.show_work = v,
        ),
        (
            "Check each lesson in the background",
            "Once a lesson ends, the same model checks it against the ROM and corrects its steps. What it costs is shown on the lesson",
            |s| s.check_lessons,
            |s, v| s.check_lessons = v,
        ),
        (
            "Add the tutor's questions about the game to quizzes",
            "Up to two a quiz, each checked by Romlens before it is asked; a written answer is marked by the model. Their cost is added to the conversation's",
            |s| s.tutor_quiz_questions,
            |s, v| s.tutor_quiz_questions = v,
        ),
        (
            "Show points and achievements",
            "Points, your rank, achievements and the banner when you earn them. Off, the map still shows what you have proven and what is ready to review",
            |s| s.show_progress,
            |s, v| s.show_progress = v,
        ),
    ]
}

fn price(p: f64) -> String {
    if p < 1.0 {
        format!("{p:.2}")
    } else {
        format!("{p}")
    }
}

// MARK: Images

fn images_page(settings: &Shared) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Images")
        .name("images")
        .icon_name("image-x-generic-symbolic")
        .build();
    let group = adw::PreferencesGroup::builder()
        .title("Pictures")
        .description(
            "The tutor sends the image model a description in words only, never a picture from \
             the game, and says its pictures are generated. OpenAI's Images API, or an endpoint \
             that speaks it (LiteLLM, a local server).",
        )
        .build();
    let endpoints = settings.borrow().image_endpoints();
    let mut names = vec!["No pictures"];
    names.extend(endpoints.iter().map(|e| e.name.as_str()));
    let combo = adw::ComboRow::builder()
        .title("Draw with")
        .model(&gtk::StringList::new(&names))
        .build();
    let current = settings.borrow().image_endpoint().map(|e| e.id);
    combo.set_selected(
        current
            .and_then(|c| endpoints.iter().position(|e| e.id == c))
            .map_or(0, |i| i as u32 + 1),
    );
    let model = adw::EntryRow::builder()
        .title("Model")
        .text(&settings.borrow().stored().image_model)
        .build();
    model.set_sensitive(combo.selected() > 0);
    combo.connect_selected_notify({
        let (settings, model) = (Rc::clone(settings), model.clone());
        move |c| {
            let all = settings.borrow().image_endpoints();
            let sel = c.selected() as usize;
            let id = sel
                .checked_sub(1)
                .and_then(|i| all.get(i))
                .map(|e| e.id.clone());
            model.set_sensitive(id.is_some());
            settings.borrow_mut().edit(|s| s.image_endpoint = id);
        }
    });
    model.connect_changed({
        let settings = Rc::clone(settings);
        move |r| {
            let t = r.text().to_string();
            settings.borrow_mut().edit(|s| s.image_model = t);
        }
    });
    group.add(&combo);
    group.add(&model);
    page.add(&group);
    page
}

// MARK: Privacy

/// What leaves the machine (`12-content-policy.md` rule 8).
fn privacy_page() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Privacy")
        .name("privacy")
        .icon_name("security-high-symbolic")
        .build();
    let note = |title: &str, lines: &[&str]| {
        let g = adw::PreferencesGroup::builder().title(title).build();
        for l in lines {
            let row = adw::ActionRow::builder().title(*l).title_lines(0).build();
            g.add(&row);
        }
        g
    };
    page.add(&note(
        "What the tutor sends",
        &[
            "Only when you ask a question, and only to the provider you chose: your question, what you have selected in Romlens, the pictures you attach, and what the tutor's tools read for it (listings, bytes, C, pictures of tiles and frames). Never the whole ROM or a recording.",
            "An endpoint on your own machine sends nothing off it.",
            "Image generation is sent only a description in words, never a picture from the game.",
        ],
    ));
    page.add(&note(
        "What stays here",
        &["Your keys are in the desktop's secret store. Conversations, with the pictures in them, are in Romlens's own folder, never in a project you share, and nothing offers them for export."],
    ));
    page
}
