//! The window's named actions, their shortcuts and the editor context menu.
//! Every command is a `gio` action, so a menu item, a shortcut and a button
//! all reach it the same way. The macOS twin is the menu bar plus
//! `RomWindowController`'s responder-chain actions and `validateMenuItem`.
//!
//! Actions whose sheet or tab has not been built yet are registered disabled
//! (see `PENDING`), so the menu already has its final shape and each one
//! switches on as its feature lands.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::hex::AddressStyle;
use crate::model::{Change, Document, Sheet, Tab};

type Enabled = fn(&Document) -> bool;
type Getter = fn(&Document) -> bool;
type Run = fn(&Rc<Document>, &adw::ApplicationWindow);

struct Entry {
    name: &'static str,
    enabled: Enabled,
    run: Run,
}

/// A two-state action (a check item in a menu, a toggle button in the header).
struct Toggle {
    name: &'static str,
    get: fn(&Document) -> bool,
    set: fn(&Document, bool),
}

fn has_selection(d: &Document) -> bool {
    d.selected().is_some()
}

fn always(_: &Document) -> bool {
    true
}

/// Zoom acts on the graph and on the atlas.
fn zooms(d: &Document) -> bool {
    matches!(d.tab(), Tab::Graph | Tab::Atlas)
}

fn never(_: &Document) -> bool {
    false
}

fn nothing(_: &Rc<Document>, _: &adw::ApplicationWindow) {}

/// Registered but not wired to a feature yet; the menu already has its final
/// shape, and each one switches on as its feature lands.
macro_rules! pending {
    ($name:literal) => {
        Entry {
            name: $name,
            enabled: never,
            run: nothing,
        }
    };
}

/// File › Import: a trace, symbols or ca65 debug information.
macro_rules! import {
    ($name:literal, $kind:ident) => {
        Entry {
            name: $name,
            enabled: always,
            run: |d, w| {
                crate::transferview::import(w, d, crate::model::transfer::ImportKind::$kind)
            },
        }
    };
}

/// File › Export, once there is an analysis to export.
macro_rules! export {
    ($name:literal, $kind:ident) => {
        Entry {
            name: $name,
            enabled: |d| d.session.has_snapshot(),
            run: |d, w| {
                crate::transferview::export(w, d, crate::model::transfer::ExportKind::$kind)
            },
        }
    };
}

/// Mark the selection as one kind of data, with that kind's defaults.
macro_rules! mark_as {
    ($name:literal, $kind:ident) => {
        Entry {
            name: $name,
            enabled: has_selection,
            run: |d, _| {
                let _ = d.mark(
                    romlens_ffi::OverrideKind::Data,
                    romlens_ffi::DataKind::$kind,
                );
            },
        }
    };
}

const ENTRIES: &[Entry] = &[
    // Go
    Entry {
        name: "follow-reference",
        enabled: has_selection,
        run: |d, _| d.follow_reference(),
    },
    Entry {
        name: "go-back",
        enabled: |d| d.can_go_back(),
        run: |d, _| d.go_back(),
    },
    Entry {
        name: "go-forward",
        enabled: |d| d.can_go_forward(),
        run: |d, _| d.go_forward(),
    },
    Entry {
        name: "go-header",
        enabled: always,
        run: |d, _| d.go_to_header(),
    },
    Entry {
        name: "go-reset",
        enabled: always,
        run: |d, _| d.go_to_reset(),
    },
    Entry {
        name: "find",
        enabled: always,
        run: |d, _| d.show_sheet(Some(Sheet::Find)),
    },
    Entry {
        name: "find-next",
        enabled: |d| d.search().has_results(),
        run: |d, _| d.step_search(1),
    },
    Entry {
        name: "find-previous",
        enabled: |d| d.search().has_results(),
        run: |d, _| d.step_search(-1),
    },
    Entry {
        name: "jump-to-address",
        enabled: always,
        run: |d, _| d.show_sheet(Some(Sheet::Jump)),
    },
    Entry {
        name: "find-references",
        enabled: |d| d.selected_address().is_some(),
        run: |d, _| d.find_references(),
    },
    // Edit
    Entry {
        name: "undo",
        enabled: |d| d.session.can_undo(),
        run: |d, _| _ = d.undo(),
    },
    Entry {
        name: "redo",
        enabled: |d| d.session.can_redo(),
        run: |d, _| _ = d.redo(),
    },
    Entry {
        name: "rename-label",
        enabled: has_selection,
        run: |d, _| d.show_sheet(Some(Sheet::RenameLabel)),
    },
    Entry {
        name: "remove-label",
        enabled: |d| d.can_remove_label(),
        run: |d, _| {
            let _ = d.remove_label_or_variable();
        },
    },
    Entry {
        name: "define-variable",
        enabled: always,
        run: |d, _| d.begin_define_variable(None),
    },
    Entry {
        name: "edit-comment",
        enabled: has_selection,
        run: |d, _| d.show_sheet(Some(Sheet::Comment)),
    },
    Entry {
        name: "mark-code",
        enabled: has_selection,
        run: |d, _| {
            let _ = d.mark(romlens_ffi::OverrideKind::Code, romlens_ffi::DataKind::Byte);
        },
    },
    Entry {
        name: "mark-data",
        enabled: has_selection,
        run: |d, _| {
            let _ = d.mark(romlens_ffi::OverrideKind::Data, romlens_ffi::DataKind::Byte);
        },
    },
    Entry {
        name: "mark-unknown",
        enabled: has_selection,
        run: |d, _| {
            let _ = d.mark(
                romlens_ffi::OverrideKind::Unknown,
                romlens_ffi::DataKind::Byte,
            );
        },
    },
    Entry {
        name: "mark-data-options",
        enabled: has_selection,
        run: |d, _| d.show_sheet(Some(Sheet::DataType)),
    },
    mark_as!("mark-string", String),
    mark_as!("mark-word", Word),
    mark_as!("mark-pointer", Pointer),
    mark_as!("mark-graphics", Graphics),
    mark_as!("mark-palette", Palette),
    mark_as!("mark-tilemap", Tilemap),
    mark_as!("mark-compressed", Compressed),
    Entry {
        name: "clear-mark",
        enabled: |d| d.marked_range().is_some(),
        run: |d, _| {
            let _ = d.clear_mark();
        },
    },
    Entry {
        name: "set-flags",
        enabled: has_selection,
        run: |d, _| d.show_sheet(Some(Sheet::Flags)),
    },
    Entry {
        name: "copy-address",
        enabled: |d| d.selected_address().is_some(),
        run: |d, w| {
            if let Some(text) = d.selected_address_text() {
                w.clipboard().set_text(&text);
            }
        },
    },
    Entry {
        name: "copy-line",
        enabled: |d| d.selected_line_text().is_some(),
        run: |d, w| {
            if let Some(text) = d.selected_line_text() {
                w.clipboard().set_text(&text);
            }
        },
    },
    // Analysis
    Entry {
        name: "analyze",
        enabled: |d| !d.session.analysis().is_running(),
        run: |d, _| d.start_analysis(),
    },
    Entry {
        name: "cancel-analysis",
        enabled: |d| d.session.analysis().is_running(),
        run: |d, _| d.session.cancel_analysis(),
    },
    Entry {
        name: "export-c",
        enabled: |d| d.decompile().result.is_some(),
        run: |d, w| crate::cview::export(w, d),
    },
    // Analysis views and tools that arrive with their tabs.
    Entry {
        name: "decompile-routine",
        enabled: has_selection,
        run: |d, _| d.set_tab(Tab::C),
    },
    Entry {
        name: "show-graph",
        enabled: has_selection,
        run: |d, _| d.set_tab(Tab::Graph),
    },
    Entry {
        name: "zoom-in",
        enabled: zooms,
        run: |d, _| d.request_zoom(crate::model::Zoom::In),
    },
    Entry {
        name: "zoom-out",
        enabled: zooms,
        run: |d, _| d.request_zoom(crate::model::Zoom::Out),
    },
    Entry {
        name: "zoom-fit",
        enabled: zooms,
        run: |d, _| d.request_zoom(crate::model::Zoom::Fit),
    },
    // Graphics, audio and the tutor
    pending!("show-frame"),
    pending!("show-layers"),
    pending!("show-tiles"),
    pending!("show-palette"),
    pending!("show-oam"),
    pending!("show-tilemap"),
    pending!("show-voices"),
    pending!("show-timeline"),
    pending!("show-samples"),
    pending!("show-audio-ram"),
    pending!("show-ports"),
    pending!("show-echo"),
    pending!("show-scope"),
    pending!("show-tutor"),
    // File
    Entry {
        name: "save",
        enabled: |d| d.session.is_dirty() || d.project_path().is_none(),
        run: |d, w| crate::files::save(w, d),
    },
    Entry {
        name: "save-as",
        enabled: always,
        run: |d, w| crate::files::save_as(w, d, None),
    },
    Entry {
        name: "duplicate",
        enabled: always,
        run: |d, w| crate::files::duplicate(w, d),
    },
    Entry {
        name: "revert",
        enabled: |d| d.project_path().is_some() && d.session.is_dirty(),
        run: |d, w| crate::files::revert(w, d),
    },
    pending!("open-recording"),
    pending!("import-snapshot"),
    pending!("export-frame-region"),
    pending!("close-recording"),
    pending!("live-session"),
    Entry {
        name: "compare-with",
        enabled: always,
        run: |d, w| crate::files::choose_compare(w, d, false),
    },
    Entry {
        name: "compare-with-project",
        enabled: always,
        run: |d, w| crate::files::choose_compare(w, d, true),
    },
    Entry {
        name: "close-compare",
        enabled: |d| d.compare().is_active(),
        run: |d, _| d.close_compare(),
    },
    Entry {
        name: "show-source",
        enabled: |d| d.source().has_files(),
        run: |d, _| d.set_tab(Tab::Source),
    },
    Entry {
        name: "show-compare",
        enabled: |d| d.compare().is_active(),
        run: |d, _| d.set_tab(Tab::Compare),
    },
    import!("import-trace", Trace),
    import!("import-symbols", Symbols),
    import!("import-dbg", Dbg),
    export!("export-assembly", Assembly),
    export!("export-annotations", Annotations),
    export!("export-symbols", Symbols),
    pending!("save-recorder-script"),
];

const TOGGLES: &[Toggle] = &[
    Toggle {
        name: "toggle-navigator",
        get: |d| d.panes().navigator,
        set: |d, v| d.set_pane(|p| &mut p.navigator, v),
    },
    Toggle {
        name: "toggle-inspector",
        get: |d| d.panes().inspector,
        set: |d, v| d.set_pane(|p| &mut p.inspector, v),
    },
    Toggle {
        name: "toggle-strip",
        get: |d| d.panes().strip,
        set: |d, v| d.set_pane(|p| &mut p.strip, v),
    },
    Toggle {
        name: "toggle-results",
        get: |d| d.panes().results,
        set: |d, v| d.set_pane(|p| &mut p.results, v),
    },
    Toggle {
        name: "focus-on-code",
        get: |d| d.is_focused(),
        set: |d, v| {
            if v != d.is_focused() {
                d.toggle_focus();
            }
        },
    },
    Toggle {
        name: "toggle-explanations",
        get: |d| d.explanations(),
        set: |d, v| {
            d.set_explanations(v);
            // Remembered for the next window, as the macOS default is.
            let mut saved = crate::config::Settings::load();
            saved.hide_explanations = !v;
            saved.save();
        },
    },
];

/// Shortcuts: Ctrl for the macOS Command key, Alt for Option. Where the macOS
/// Option+Command combination collides with a desktop binding another
/// modifier is used (docs/08): the view tabs take Alt+digit, as GNOME
/// terminals and browsers switch tabs, and the tutor, live session and
/// compare take Alt+Shift. Every shortcut is here and only here.
pub const ACCELS: &[(&str, &[&str])] = &[
    // File
    ("app.open", &["<Control>o"]),
    ("app.open-project", &["<Control><Shift>o"]),
    ("win.open-recording", &["<Control><Alt>o"]),
    ("win.save", &["<Control>s"]),
    ("win.save-as", &["<Control><Shift>s"]),
    ("win.duplicate", &["<Control><Alt><Shift>s"]),
    ("win.live-session", &["<Alt><Shift>l"]),
    ("win.compare-with", &["<Alt><Shift>d"]),
    ("window.close", &["<Control>w"]),
    ("app.quit", &["<Control>q"]),
    ("app.settings", &["<Control>comma"]),
    ("app.shortcuts", &["<Control>question"]),
    // Edit
    ("win.undo", &["<Control>z"]),
    ("win.redo", &["<Control><Shift>z"]),
    ("win.define-variable", &["<Control><Alt>v"]),
    ("win.copy-address", &["<Control><Alt>c"]),
    ("win.copy-line", &["<Control><Shift>c"]),
    // Go
    ("win.find", &["<Control>f"]),
    ("win.find-next", &["<Control>g"]),
    ("win.find-previous", &["<Control><Shift>g"]),
    ("win.jump-to-address", &["<Control>l"]),
    ("win.follow-reference", &["<Control>Return"]),
    ("win.find-references", &["<Control><Shift>f"]),
    ("win.go-back", &["<Control>bracketleft", "<Alt>Left"]),
    ("win.go-forward", &["<Control>bracketright", "<Alt>Right"]),
    ("win.go-header", &["<Control><Shift>h"]),
    ("win.go-reset", &["<Control><Shift>r"]),
    // View
    ("win.show-tab::hex", &["<Alt>1"]),
    ("win.show-tab::disassembly", &["<Alt>2"]),
    ("win.show-tab::both", &["<Alt>3"]),
    ("win.show-tab::c", &["<Alt>8"]),
    ("win.show-tab::graph", &["<Alt>9"]),
    ("win.show-tab::atlas", &["<Alt><Shift>a"]),
    ("win.show-tiles", &["<Alt>4"]),
    ("win.show-palette", &["<Alt>5"]),
    ("win.show-oam", &["<Alt>6"]),
    ("win.show-tilemap", &["<Alt>7"]),
    ("win.show-tutor", &["<Alt><Shift>t"]),
    ("win.toggle-navigator", &["<Control>0", "F9"]),
    ("win.toggle-inspector", &["<Alt>0"]),
    ("win.toggle-strip", &["<Control><Shift>0"]),
    ("win.toggle-results", &["<Alt><Shift>0"]),
    ("win.focus-on-code", &["<Control><Alt>f"]),
    ("win.toggle-explanations", &["<Control><Alt>e"]),
    ("win.zoom-in", &["<Control>equal", "<Control>plus"]),
    ("win.zoom-out", &["<Control>minus"]),
    ("win.zoom-fit", &["<Control><Alt>0"]),
    ("win.address-style::both", &["<Control>1"]),
    ("win.address-style::snes", &["<Control>2"]),
    ("win.address-style::file", &["<Control>3"]),
    // Analysis
    ("win.cancel-analysis", &["<Control>period"]),
    // In the C pane (handled by the pane, not a window action)
    ("local.c-fold", &["<Alt><Shift>Left"]),
    ("local.c-unfold", &["<Alt><Shift>Right"]),
    ("local.c-fold-all", &["<Control><Alt><Shift>Left"]),
    ("local.c-unfold-all", &["<Control><Alt><Shift>Right"]),
];

#[cfg(test)]
/// Whether `action` (as a menu item names it) is registered somewhere: on the
/// window by `install`, on the application, or built into GTK.
pub fn knows(action: &str) -> bool {
    let Some((scope, rest)) = action.split_once('.') else {
        return false;
    };
    let bare = rest.split("::").next().unwrap_or(rest);
    match scope {
        "win" => {
            ENTRIES.iter().any(|e| e.name == bare)
                || TOGGLES.iter().any(|t| t.name == bare)
                || ["show-tab", "address-style"].contains(&bare)
        }
        "app" => APP_ACTIONS.contains(&bare),
        "window" => bare == "close",
        "local" => bare.starts_with("c-"),
        _ => false,
    }
}

/// Actions on the application (see `main.rs`).
#[cfg(test)]
pub const APP_ACTIONS: &[&str] = &[
    "open",
    "open-project",
    "quit",
    "settings",
    "shortcuts",
    "about",
];

/// The Keyboard Shortcuts window: each command's name beside its shortcut,
/// grouped as the menu is. The shortcut itself comes from `ACCELS`, so the
/// window cannot say something the app does not do.
pub const SHORTCUT_GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "File",
        &[
            ("Open ROM", "app.open"),
            ("Open Project", "app.open-project"),
            ("Save", "win.save"),
            ("Save As", "win.save-as"),
            ("Duplicate", "win.duplicate"),
            ("Open Recording", "win.open-recording"),
            ("Start Live Session", "win.live-session"),
            ("Compare With", "win.compare-with"),
            ("Close Window", "window.close"),
            ("Quit", "app.quit"),
        ],
    ),
    (
        "Edit",
        &[
            ("Undo", "win.undo"),
            ("Redo", "win.redo"),
            ("Define Variable", "win.define-variable"),
            ("Copy Address", "win.copy-address"),
            ("Copy Line", "win.copy-line"),
        ],
    ),
    (
        "Go",
        &[
            ("Find", "win.find"),
            ("Find Next", "win.find-next"),
            ("Find Previous", "win.find-previous"),
            ("Jump to Address", "win.jump-to-address"),
            ("Follow Reference", "win.follow-reference"),
            ("Find References", "win.find-references"),
            ("Back", "win.go-back"),
            ("Forward", "win.go-forward"),
            ("Header", "win.go-header"),
            ("Reset Vector", "win.go-reset"),
        ],
    ),
    (
        "View",
        &[
            ("Hex", "win.show-tab::hex"),
            ("Disassembly", "win.show-tab::disassembly"),
            ("Both", "win.show-tab::both"),
            ("C", "win.show-tab::c"),
            ("Graph", "win.show-tab::graph"),
            ("Atlas", "win.show-tab::atlas"),
            ("Tile Decoder", "win.show-tiles"),
            ("Palette", "win.show-palette"),
            ("OAM", "win.show-oam"),
            ("Tilemap", "win.show-tilemap"),
            ("Tutor", "win.show-tutor"),
            ("Navigator", "win.toggle-navigator"),
            ("Inspector", "win.toggle-inspector"),
            ("Overview Strip", "win.toggle-strip"),
            ("Results", "win.toggle-results"),
            ("Focus on Code", "win.focus-on-code"),
            ("Explanations", "win.toggle-explanations"),
            ("Zoom In", "win.zoom-in"),
            ("Zoom Out", "win.zoom-out"),
            ("Zoom to Fit", "win.zoom-fit"),
            ("File and SNES Addresses", "win.address-style::both"),
            ("SNES Addresses Only", "win.address-style::snes"),
            ("File Offsets Only", "win.address-style::file"),
        ],
    ),
    (
        "C pane",
        &[
            ("Fold", "local.c-fold"),
            ("Unfold", "local.c-unfold"),
            ("Fold All", "local.c-fold-all"),
            ("Unfold All", "local.c-unfold-all"),
        ],
    ),
    (
        "General",
        &[
            ("Cancel Analysis", "win.cancel-analysis"),
            ("Settings", "app.settings"),
            ("Keyboard Shortcuts", "app.shortcuts"),
        ],
    ),
];

/// The shortcuts for an action, as GTK accelerator strings.
pub fn accels_for(action: &str) -> &'static [&'static str] {
    ACCELS
        .iter()
        .find(|(a, _)| *a == action)
        .map_or(&[], |(_, keys)| keys)
}

pub fn set_accels(app: &impl IsA<gtk::Application>) {
    for (action, keys) in ACCELS {
        app.set_accels_for_action(action, keys);
    }
}

pub fn style_id(style: AddressStyle) -> &'static str {
    match style {
        AddressStyle::Both => "both",
        AddressStyle::Snes => "snes",
        AddressStyle::File => "file",
    }
}

/// Register every action on the window and keep their state in step with the
/// document.
pub fn install(window: &adw::ApplicationWindow, doc: &Rc<Document>) {
    let mut installed: Vec<(gio::SimpleAction, Enabled)> = Vec::new();
    for entry in ENTRIES {
        let action = gio::SimpleAction::new(entry.name, None);
        let (d, w, run) = (Rc::clone(doc), window.clone(), entry.run);
        action.connect_activate(move |_, _| run(&d, &w));
        window.add_action(&action);
        installed.push((action, entry.enabled));
    }

    let mut toggles: Vec<(gio::SimpleAction, Getter)> = Vec::new();
    for t in TOGGLES {
        let action = gio::SimpleAction::new_stateful(t.name, None, &(t.get)(doc).to_variant());
        let (d, set, get) = (Rc::clone(doc), t.set, t.get);
        // Activating a stateful action with no parameter flips it; the
        // document is the truth, and its change event sets the state.
        action.connect_activate(move |_, _| set(&d, !get(&d)));
        window.add_action(&action);
        toggles.push((action, t.get));
    }

    // Tabs and address columns: a string parameter, one state.
    let tab = gio::SimpleAction::new_stateful(
        "show-tab",
        Some(glib::VariantTy::STRING),
        &doc.tab().id().to_variant(),
    );
    tab.connect_activate({
        let doc = Rc::clone(doc);
        move |_, param| {
            if let Some(t) = param
                .and_then(|p| p.get::<String>())
                .and_then(|id| Tab::from_id(&id))
                && Tab::BUILT.contains(&t)
                && doc.tab_available(t)
            {
                doc.set_tab(t);
            }
        }
    });
    window.add_action(&tab);

    let style = gio::SimpleAction::new_stateful(
        "address-style",
        Some(glib::VariantTy::STRING),
        &style_id(doc.address_style()).to_variant(),
    );
    style.connect_activate({
        let doc = Rc::clone(doc);
        move |_, param| {
            let Some(id) = param.and_then(|p| p.get::<String>()) else {
                return;
            };
            if let Some(s) = AddressStyle::ALL.into_iter().find(|s| style_id(*s) == id) {
                doc.set_address_style(s);
            }
        }
    });
    window.add_action(&style);

    let refresh = {
        let doc = Rc::clone(doc);
        move || {
            for (action, enabled) in &installed {
                action.set_enabled(enabled(&doc));
            }
            for (action, get) in &toggles {
                action.set_state(&get(&doc).to_variant());
            }
            tab.set_state(&doc.tab().id().to_variant());
            style.set_state(&style_id(doc.address_style()).to_variant());
        }
    };
    refresh();
    doc.subscribe(move |change| {
        if !matches!(change, Change::Scroll | Change::Sheet | Change::Navigator) {
            refresh();
        }
    });
}

/// The right-click menu both canvases show.
pub fn context_menu(doc: &Document) -> gio::Menu {
    let menu = gio::Menu::new();
    let section = |items: &[(&str, &str)]| {
        let s = gio::Menu::new();
        for (label, action) in items {
            s.append(Some(label), Some(&format!("win.{action}")));
        }
        s
    };
    let find_refs = match doc.reference_target() {
        Some((name, n)) => format!("Find References to {name} ({n})"),
        None => "Find References".to_owned(),
    };
    menu.append_section(
        None,
        &section(&[
            ("Follow Reference", "follow-reference"),
            (&find_refs, "find-references"),
            ("Decompile Routine", "decompile-routine"),
            ("Show Graph", "show-graph"),
        ]),
    );
    let remove = match doc.details().label {
        Some(l) if doc.can_remove_label() => format!("Remove Label “{}”", l.name),
        _ => "Remove Label".to_owned(),
    };
    menu.append_section(
        None,
        &section(&[
            ("Rename Label…", "rename-label"),
            (&remove, "remove-label"),
            ("Define Variable…", "define-variable"),
            ("Comment…", "edit-comment"),
        ]),
    );
    menu.append_section(
        None,
        &section(&[
            ("Mark as Code", "mark-code"),
            ("Mark as Data", "mark-data"),
            ("Mark as Unknown", "mark-unknown"),
            ("Clear Mark", "clear-mark"),
            ("Set Flags…", "set-flags"),
        ]),
    );
    menu.append_section(
        None,
        &section(&[("Copy Address", "copy-address"), ("Copy Line", "copy-line")]),
    );
    menu
}

/// Show the context menu on right-click. `select_at` is told where the click
/// landed so the canvas can select what is under the pointer first.
pub fn attach_context_menu(
    area: &gtk::DrawingArea,
    doc: &Rc<Document>,
    select_at: impl Fn(f64, f64) + 'static,
) {
    let click = gtk::GestureClick::builder()
        .button(gdk::BUTTON_SECONDARY)
        .build();
    let (area_weak, doc) = (area.downgrade(), Rc::clone(doc));
    click.connect_pressed(move |_, _, x, y| {
        let Some(area) = area_weak.upgrade() else {
            return;
        };
        area.grab_focus();
        select_at(x, y);
        let popover = gtk::PopoverMenu::from_model(Some(&context_menu(&doc)));
        popover.set_parent(&area);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|p| {
            let p = p.clone();
            // Unparenting inside the closed signal confuses the popover.
            glib::idle_add_local_once(move || p.unparent());
        });
        popover.popup();
    });
    area.add_controller(click);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing::{TestRuntime, test_rom};

    /// Every action name a model refers to, walking sections.
    fn action_names(model: &gio::MenuModel, out: &mut Vec<String>) {
        for i in 0..model.n_items() {
            if let Some(name) = model
                .item_attribute_value(i, "action", Some(glib::VariantTy::STRING))
                .and_then(|v| v.get::<String>())
            {
                out.push(name);
            }
            if let Some(section) = model.item_link(i, "section") {
                action_names(&section, out);
            }
        }
    }

    fn action_exists(bare: &str) -> bool {
        ENTRIES.iter().any(|e| e.name == bare)
            || TOGGLES.iter().any(|t| t.name == bare)
            || ["show-tab", "address-style"].contains(&bare)
    }

    #[test]
    fn every_context_menu_item_has_an_action() {
        let doc = Document::new(test_rom(), TestRuntime::new());
        let menu = context_menu(&doc);
        let mut names = Vec::new();
        action_names(menu.upcast_ref(), &mut names);
        assert!(names.len() >= 15, "{names:?}");
        for name in names {
            let bare = name.strip_prefix("win.").expect("window action");
            assert!(action_exists(bare), "no action {bare}");
        }
    }

    #[test]
    fn entry_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for name in ENTRIES
            .iter()
            .map(|e| e.name)
            .chain(TOGGLES.iter().map(|t| t.name))
        {
            assert!(seen.insert(name), "duplicate {name}");
        }
    }

    #[test]
    fn every_accelerator_targets_an_action() {
        for (action, keys) in ACCELS {
            if let Some(bare) = action.strip_prefix("win.") {
                let bare = bare.split("::").next().unwrap();
                assert!(action_exists(bare), "no action {bare}");
            } else {
                assert!(
                    action.starts_with("app.")
                        || action.starts_with("window.")
                        || action.starts_with("local."),
                    "{action}"
                );
            }
            for key in *keys {
                // Parsing needs a display, so check the shape instead: known
                // modifiers, then a key name.
                let mut rest = *key;
                while let Some(tail) = rest.strip_prefix('<') {
                    let (modifier, after) = tail.split_once('>').expect("unclosed modifier");
                    assert!(
                        ["Control", "Shift", "Alt", "Super"].contains(&modifier),
                        "unknown modifier {modifier} in {key}"
                    );
                    rest = after;
                }
                assert!(
                    !rest.is_empty() && !rest.contains(['<', '>', ' ']),
                    "bad key in {key}"
                );
            }
        }
    }

    #[test]
    fn enabled_rules_follow_the_selection() {
        let doc = Document::new(test_rom(), TestRuntime::new());
        let follow = ENTRIES
            .iter()
            .find(|e| e.name == "follow-reference")
            .unwrap();
        assert!(!(follow.enabled)(&doc));
        doc.select(Some(0));
        assert!((follow.enabled)(&doc));
    }

    #[test]
    fn no_two_commands_share_a_shortcut() {
        let mut seen = std::collections::HashMap::new();
        for (action, keys) in ACCELS {
            for key in *keys {
                if let Some(other) = seen.insert(*key, *action) {
                    panic!("{key} is bound to both {other} and {action}");
                }
            }
        }
    }

    #[test]
    fn the_shortcuts_window_lists_every_shortcut_exactly_once() {
        let mut listed = std::collections::HashSet::new();
        for (_, rows) in SHORTCUT_GROUPS {
            for (title, action) in *rows {
                assert!(!title.is_empty());
                assert!(
                    !accels_for(action).is_empty(),
                    "{action} has no shortcut to show"
                );
                assert!(listed.insert(*action), "{action} is listed twice");
            }
        }
        for (action, _) in ACCELS {
            assert!(
                listed.contains(action),
                "{action} has a shortcut but is not in the window"
            );
        }
    }
}
