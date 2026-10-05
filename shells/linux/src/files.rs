//! Opening, saving and closing documents: the part of the macOS shell that
//! `NSDocument`, `NSDocumentController` and the open and save panels do. A
//! window per project; a ROM opens as an untitled project named after it; a
//! `.romlens` package opens through the ROM it belongs to, which is found by
//! hash or asked for.
//!
//! Closing follows `autosavesInPlace`: a project that has a file is saved
//! when its window closes, with no question; an untitled one asks.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use romlens_ffi::{Rom, RomIdentityInfo, RomlensError};

use crate::glib_runtime::GlibRuntime;
use crate::model::Document;
use crate::{locator, package, snapshot, window};

const APP_NAME: &str = "Romlens";
const AUTOSAVE: Duration = Duration::from_secs(30);

type WeakWindow = glib::WeakRef<adw::ApplicationWindow>;

thread_local! {
    /// The window each open ROM is in, by payload hash, so opening a ROM that
    /// is already open shows its window rather than a second one.
    static OPEN: RefCell<Vec<(String, WeakWindow)>> = const { RefCell::new(Vec::new()) };
}

fn existing_window(sha256: &str) -> Option<adw::ApplicationWindow> {
    OPEN.with(|o| {
        let mut open = o.borrow_mut();
        open.retain(|(_, w)| w.upgrade().is_some());
        open.iter()
            .find(|(s, _)| s == sha256)
            .and_then(|(_, w)| w.upgrade())
    })
}

fn register(sha256: &str, window: &adw::ApplicationWindow) {
    OPEN.with(|o| o.borrow_mut().push((sha256.to_owned(), window.downgrade())));
}

// MARK: Messages

pub fn alert(parent: Option<&gtk::Window>, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("ok", "OK");
    dialog.present(parent);
}

fn describe(path: &Path, e: &RomlensError) -> String {
    format!("{}\n\n{e}", path.display())
}

// MARK: Opening

/// Open whatever `path` is: a project package, or a ROM.
pub fn open_path(app: &adw::Application, path: &Path) {
    if package::is_package(path) {
        open_project(app, path);
    } else {
        open_rom(app, path);
    }
}

/// A ROM already open (by hash) is shown; otherwise a new untitled project
/// adopts it.
pub fn open_rom(app: &adw::Application, path: &Path) {
    match Document::open(path, Rc::new(GlibRuntime)) {
        Ok(doc) => {
            if let Some(w) = existing_window(&doc.info.sha256) {
                w.present();
                return;
            }
            locator::remember(&doc.info.sha256, path);
            add_recent(path, false);
            show(app, doc);
        }
        Err(e) => alert(
            app.active_window().as_ref(),
            "Can't Open ROM",
            &describe(path, &e),
        ),
    }
}

/// A project: its files, then the ROM they describe.
pub fn open_project(app: &adw::Application, path: &Path) {
    let parent = app.active_window();
    let files =
        match romlens_ffi::workbench::read_project_package(path.to_string_lossy().into_owned()) {
            Ok(f) => f,
            Err(e) => return alert(parent.as_ref(), "Can't Open Project", &describe(path, &e)),
        };
    let (files, local) = package::split_local(files);
    let identity = match romlens_ffi::workbench::project_identity(files.clone()) {
        Ok(i) => i,
        Err(e) => return alert(parent.as_ref(), "Can't Open Project", &describe(path, &e)),
    };
    if let Some(w) = existing_window(&identity.sha256) {
        w.present();
        return;
    }

    let (app, package_path, sha) = (app.clone(), path.to_path_buf(), identity.sha256.clone());
    let kept = local.clone();
    let finish = move |rom_path: PathBuf| {
        let parent = app.active_window();
        let rom = match Rom::open(rom_path.to_string_lossy().into_owned()) {
            Ok(r) => r,
            Err(e) => return alert(parent.as_ref(), "Can't Open ROM", &describe(&rom_path, &e)),
        };
        match Document::from_project(rom, files, &package_path, &rom_path, Rc::new(GlibRuntime)) {
            Ok(doc) => {
                // The window as it was left (docs/29), before it is built.
                doc.restore_local(&kept);
                locator::remember(&sha, &rom_path);
                add_recent(&package_path, true);
                show(&app, doc);
            }
            Err(e) => alert(
                parent.as_ref(),
                "Can't Open Project",
                &describe(&package_path, &e),
            ),
        }
    };
    match locator::locate(&identity, path, &local) {
        Some(rom) => finish(rom),
        None => ask_for_rom(
            identity,
            None,
            Box::new(move |p| {
                if let Some(rom) = p {
                    finish(rom);
                }
            }),
        ),
    }
}

/// The last resort: ask, verifying the hash, until the person gives up.
fn ask_for_rom(
    identity: RomIdentityInfo,
    mismatch: Option<String>,
    done: Box<dyn FnOnce(Option<PathBuf>)>,
) {
    let name = if identity.title.trim().is_empty() {
        "this project".to_owned()
    } else {
        format!("“{}”", identity.title.trim())
    };
    let body = match &mismatch {
        Some(m) => format!("{m}. Choose the ROM {name} was made from."),
        None => format!("Romlens can't find the ROM {name} was made from. Choose it."),
    };
    let dialog = adw::AlertDialog::new(Some("Locate the ROM"), Some(&body));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("locate", "Locate…");
    dialog.set_response_appearance("locate", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("locate"));
    dialog.set_close_response("cancel");
    let parent = gio::Application::default()
        .and_downcast::<gtk::Application>()
        .and_then(|a| a.active_window());
    dialog.choose(parent.as_ref(), gio::Cancellable::NONE, move |response| {
        if response != "locate" {
            return done(None);
        }
        let chooser = gtk::FileDialog::builder()
            .title("Locate ROM")
            .filters(&rom_filters())
            .build();
        let parent = gio::Application::default()
            .and_downcast::<gtk::Application>()
            .and_then(|a| a.active_window());
        chooser.open(parent.as_ref(), gio::Cancellable::NONE, move |picked| {
            let Some(path) = picked.ok().and_then(|f| f.path()) else {
                return done(None);
            };
            if locator::matches(&identity, &path) {
                done(Some(path));
            } else {
                let file = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                ask_for_rom(identity, Some(format!("{file} is a different ROM")), done);
            }
        });
    });
}

fn rom_filters() -> gio::ListStore {
    let roms = gtk::FileFilter::new();
    roms.set_name(Some("SNES ROM images"));
    for pattern in ["*.sfc", "*.smc", "*.swc", "*.fig"] {
        roms.add_suffix(&pattern[2..]);
    }
    let all = gtk::FileFilter::new();
    all.set_name(Some("All files"));
    all.add_pattern("*");
    let store = gio::ListStore::new::<gtk::FileFilter>();
    store.append(&roms);
    store.append(&all);
    store
}

/// Open… : a ROM image.
pub fn choose_rom(app: &adw::Application) {
    let dialog = gtk::FileDialog::builder()
        .title("Open ROM")
        .filters(&rom_filters())
        .build();
    let app = app.clone();
    dialog.open(
        app.active_window().as_ref(),
        gio::Cancellable::NONE,
        move |picked| {
            if let Some(path) = picked.ok().and_then(|f| f.path()) {
                open_path(&app, &path);
            }
        },
    );
}

/// Open Project… : a `.romlens` package, which is a folder.
pub fn choose_project(app: &adw::Application) {
    let dialog = gtk::FileDialog::builder().title("Open Project").build();
    let app = app.clone();
    dialog.select_folder(
        app.active_window().as_ref(),
        gio::Cancellable::NONE,
        move |picked| {
            if let Some(path) = picked.ok().and_then(|f| f.path()) {
                open_project(&app, &path);
            }
        },
    );
}

/// Compare With… : another version of this ROM, a ROM file, or (`project`)
/// a saved project, which is a folder so a file chooser cannot offer both.
pub fn choose_compare(window: &adw::ApplicationWindow, doc: &Rc<Document>, project: bool) {
    let dialog = gtk::FileDialog::builder()
        .title(if project {
            "Compare With Project"
        } else {
            "Compare With"
        })
        .build();
    let start = doc
        .project_path()
        .or_else(|| doc.rom_path())
        .and_then(|p| p.parent().map(Path::to_path_buf));
    if let Some(dir) = start {
        dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
    }
    let doc = Rc::clone(doc);
    let done = move |picked: Result<gio::File, glib::Error>| {
        if let Some(path) = picked.ok().and_then(|f| f.path()) {
            doc.compare_with(path);
        }
    };
    if project {
        dialog.select_folder(Some(window), gio::Cancellable::NONE, done);
    } else {
        dialog.set_filters(Some(&rom_filters()));
        dialog.open(Some(window), gio::Cancellable::NONE, done);
    }
}

/// Put a document in a window, and make the window the one for its ROM.
fn show(app: &adw::Application, doc: Rc<Document>) {
    // The welcome window has done its job.
    for w in app.windows() {
        if w.widget_name() == window::WELCOME {
            w.close();
        }
    }
    let window = window::open_document(app, Rc::clone(&doc));
    register(&doc.info.sha256, &window);
    install(app, &window, &doc);
    snapshot::maybe_capture(&window, &doc);
}

// MARK: Recent files

pub fn add_recent(path: &Path, project: bool) {
    let uri = gio::File::for_path(path).uri();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let data = gtk::RecentData::new(
        name.as_deref(),
        None,
        if project {
            "application/x-romlens-project"
        } else {
            "application/x-snes-rom"
        },
        APP_NAME,
        &format!(
            "{} %u",
            std::env::current_exe()
                .map_or_else(|_| "romlens".into(), |e| e.to_string_lossy().into_owned())
        ),
        &[APP_NAME],
        false,
    );
    gtk::RecentManager::default().add_full(&uri, &data);
}

/// What Romlens opened recently, newest first, that still exists.
pub fn recent(limit: usize) -> Vec<(String, PathBuf)> {
    let mut items: Vec<_> = gtk::RecentManager::default()
        .items()
        .into_iter()
        .filter(|i| i.has_application(APP_NAME))
        .filter_map(|i| {
            let path = gio::File::for_uri(&i.uri()).path()?;
            path.exists()
                .then(|| (i.modified().to_unix(), i.display_name().to_string(), path))
        })
        .collect();
    items.sort_by_key(|(t, _, _)| std::cmp::Reverse(*t));
    items
        .into_iter()
        .take(limit)
        .map(|(_, n, p)| (n, p))
        .collect()
}

// MARK: Saving

fn parent_of(window: &adw::ApplicationWindow) -> gtk::Window {
    window.clone().upcast()
}

fn write_project(
    window: &adw::ApplicationWindow,
    doc: &Rc<Document>,
    path: &Path,
    then: Option<Box<dyn FnOnce()>>,
) {
    match doc.save_to(path) {
        Ok(()) => {
            if let Some(rom) = doc.rom_path() {
                locator::remember(&doc.info.sha256, &rom);
            }
            add_recent(path, true);
            if let Some(then) = then {
                then();
            }
        }
        Err(e) => alert(
            Some(&parent_of(window)),
            "Can't Save Project",
            &describe(path, &e),
        ),
    }
}

/// Save: to where the project lives, or ask for a place the first time.
pub fn save(window: &adw::ApplicationWindow, doc: &Rc<Document>) {
    match doc.project_path() {
        Some(path) => write_project(window, doc, &path, None),
        None => save_as(window, doc, None),
    }
}

fn save_dialog(window: &adw::ApplicationWindow, doc: &Document, suffix: &str) -> gtk::FileDialog {
    let folder = doc
        .project_path()
        .or_else(|| doc.rom_path())
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let dialog = gtk::FileDialog::builder()
        .title("Save Project")
        .initial_name(format!("{}{suffix}.romlens", doc.display_name()))
        .accept_label("Save")
        .build();
    if let Some(f) = folder {
        dialog.set_initial_folder(Some(&gio::File::for_path(f)));
    }
    let _ = window;
    dialog
}

/// Save As…: choose where the project lives from now on.
pub fn save_as(
    window: &adw::ApplicationWindow,
    doc: &Rc<Document>,
    then: Option<Box<dyn FnOnce()>>,
) {
    let (w, d) = (window.clone(), Rc::clone(doc));
    save_dialog(window, doc, "").save(Some(window), gio::Cancellable::NONE, move |picked| {
        if let Some(path) = picked.ok().and_then(|f| f.path()) {
            write_project(&w, &d, &package::with_extension(&path), then);
        }
    });
}

/// Duplicate: a copy somewhere else; this window stays with the original.
pub fn duplicate(window: &adw::ApplicationWindow, doc: &Rc<Document>) {
    let (w, d) = (window.clone(), Rc::clone(doc));
    save_dialog(window, doc, " copy").save(Some(window), gio::Cancellable::NONE, move |picked| {
        let Some(path) = picked.ok().and_then(|f| f.path()) else {
            return;
        };
        let path = package::with_extension(&path);
        match d.duplicate_to(&path) {
            Ok(()) => add_recent(&path, true),
            Err(e) => alert(
                Some(&parent_of(&w)),
                "Can't Duplicate Project",
                &describe(&path, &e),
            ),
        }
    });
}

/// Revert to Saved: discard what has changed since the last save.
pub fn revert(window: &adw::ApplicationWindow, doc: &Rc<Document>) {
    let dialog = adw::AlertDialog::new(
        Some("Revert to Saved?"),
        Some("Everything changed since the last save will be lost."),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("revert", "Revert");
    dialog.set_response_appearance("revert", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    let (w, d) = (window.clone(), Rc::clone(doc));
    dialog.choose(Some(window), gio::Cancellable::NONE, move |response| {
        if response == "revert"
            && let Err(e) = d.revert()
        {
            alert(Some(&parent_of(&w)), "Can't Revert", &e.to_string());
        }
    });
}

// MARK: Window behaviour

/// Close, autosave and drag-and-drop for a document window.
fn install(app: &adw::Application, window: &adw::ApplicationWindow, doc: &Rc<Document>) {
    // A project that has a file saves itself every so often, as an
    // `autosavesInPlace` document does. The timer stops with the window.
    let (weak_window, weak_doc) = (window.downgrade(), Rc::downgrade(doc));
    glib::timeout_add_local(AUTOSAVE, move || {
        let (Some(w), Some(d)) = (weak_window.upgrade(), weak_doc.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        if d.session.is_dirty()
            && let Some(path) = d.project_path()
        {
            write_project(&w, &d, &path, None);
        }
        glib::ControlFlow::Continue
    });

    let closing = Rc::new(Cell::new(false));
    window.connect_close_request({
        let (doc, closing) = (Rc::clone(doc), Rc::clone(&closing));
        move |w| {
            if closing.get() || !doc.session.is_dirty() {
                return glib::Propagation::Proceed;
            }
            // A project with a file is saved, not asked about.
            if let Some(path) = doc.project_path() {
                return match doc.save_to(&path) {
                    Ok(()) => glib::Propagation::Proceed,
                    Err(e) => {
                        alert(
                            Some(w.upcast_ref()),
                            "Can't Save Project",
                            &describe(&path, &e),
                        );
                        glib::Propagation::Stop
                    }
                };
            }
            ask_to_save(w, &doc, &closing);
            glib::Propagation::Stop
        }
    });

    window.connect_destroy({
        let doc = Rc::clone(doc);
        move |_| {
            doc.keep_workspace();
            doc.close();
        }
    });

    accept_drops(app, window.upcast_ref());
}

/// Dropping a ROM or a project on a window opens it.
pub fn accept_drops(app: &adw::Application, window: &gtk::Window) {
    let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    let app = app.clone();
    target.connect_drop(move |_, value, _, _| {
        let Ok(list) = value.get::<gdk::FileList>() else {
            return false;
        };
        for path in list.files().iter().filter_map(|f| f.path()) {
            open_path(&app, &path);
        }
        true
    });
    window.add_controller(target);
}

/// An untitled project with changes: save, discard or stay.
fn ask_to_save(window: &adw::ApplicationWindow, doc: &Rc<Document>, closing: &Rc<Cell<bool>>) {
    let dialog = adw::AlertDialog::new(
        Some(&format!("Save changes to “{}”?", doc.display_name())),
        Some("Your changes will be lost if you don't save them."),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("discard", "Discard");
    dialog.add_response("save", "Save…");
    dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");
    let (w, d, closing) = (window.clone(), Rc::clone(doc), Rc::clone(closing));
    dialog.choose(
        Some(window),
        gio::Cancellable::NONE,
        move |response| match response.as_str() {
            "discard" => {
                closing.set(true);
                w.close();
            }
            "save" => {
                let (w2, closing) = (w.clone(), Rc::clone(&closing));
                save_as(
                    &w,
                    &d,
                    Some(Box::new(move || {
                        closing.set(true);
                        w2.close();
                    })),
                );
            }
            _ => {}
        },
    );
}

/// Quit: close every window, so each gets its chance to save or ask. The
/// application exits when the last one is gone.
pub fn quit(app: &adw::Application) {
    for w in app.windows() {
        w.close();
    }
}
