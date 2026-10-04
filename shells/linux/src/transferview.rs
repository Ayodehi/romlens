//! File › Import and File › Export: the dialogs and the report. The macOS
//! twins are `ImportController` and `ExportController`.

use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use crate::files::alert;
use crate::model::Document;
use crate::model::transfer::{self, ExportKind, ImportKind};
use crate::sheets;

fn filters(kind: ImportKind) -> gio::ListStore {
    let own = gtk::FileFilter::new();
    own.set_name(Some(kind.filter_name()));
    for ext in kind.extensions() {
        own.add_suffix(ext);
    }
    let all = gtk::FileFilter::new();
    all.set_name(Some("All files"));
    all.add_pattern("*");
    let store = gio::ListStore::new::<gtk::FileFilter>();
    store.append(&own);
    // A trace has no reserved extension, so the chooser must not refuse one
    // that a person exported under some other name.
    store.append(&all);
    store
}

pub fn import(window: &adw::ApplicationWindow, doc: &Rc<Document>, kind: ImportKind) {
    let dialog = gtk::FileDialog::builder()
        .title(kind.title())
        .filters(&filters(kind))
        .build();
    let (w, d) = (window.clone(), Rc::clone(doc));
    dialog.open(Some(window), gio::Cancellable::NONE, move |picked| {
        let Some(path) = picked.ok().and_then(|f| f.path()) else {
            return;
        };
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        d.import(kind, path, move |result| match result {
            Ok(r) => alert(
                Some(w.upcast_ref()),
                &format!("Imported {}", r.source),
                &transfer::summary(kind, &r),
            ),
            Err(e) => alert(
                Some(w.upcast_ref()),
                &format!("Could not import {name}"),
                &e.to_string(),
            ),
        });
    });
}

pub fn export(window: &adw::ApplicationWindow, doc: &Rc<Document>, kind: ExportKind) {
    if kind == ExportKind::Assembly {
        assembly_options(window, doc);
    } else {
        choose_file(window, doc, kind, false);
    }
}

/// The listing holds ROM bytes, so the sharing notice comes first (docs/12),
/// and byte columns, which add more of them, are off unless asked for.
fn assembly_options(window: &adw::ApplicationWindow, doc: &Rc<Document>) {
    let f = sheets::frame(ExportKind::Assembly.title(), 420, "Choose File…");
    let notice = gtk::Label::builder()
        .label(transfer::ASSEMBLY_MESSAGE)
        .xalign(0.0)
        .build();
    notice.add_css_class("dim-label");
    f.body.append(&notice);
    let group = adw::PreferencesGroup::new();
    let bytes = adw::SwitchRow::builder()
        .title("Include byte columns")
        .build();
    group.add(&bytes);
    f.body.append(&group);
    f.primary.connect_clicked({
        let (w, d, dialog) = (window.clone(), Rc::clone(doc), f.dialog.clone());
        move |_| {
            let include = bytes.is_active();
            dialog.close();
            choose_file(&w, &d, ExportKind::Assembly, include);
        }
    });
    f.dialog.present(Some(window));
}

fn choose_file(
    window: &adw::ApplicationWindow,
    doc: &Rc<Document>,
    kind: ExportKind,
    include_bytes: bool,
) {
    let folder = doc
        .project_path()
        .or_else(|| doc.rom_path())
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf));
    let dialog = gtk::FileDialog::builder()
        .title(kind.title())
        .initial_name(format!("{}.{}", doc.display_name(), kind.extension()))
        .accept_label("Export")
        .build();
    if let Some(f) = folder {
        dialog.set_initial_folder(Some(&gio::File::for_path(f)));
    }
    let (w, d) = (window.clone(), Rc::clone(doc));
    dialog.save(Some(window), gio::Cancellable::NONE, move |picked| {
        let Some(path) = picked.ok().and_then(|f| f.path()) else {
            return;
        };
        let shown = path.display().to_string();
        d.export(kind, include_bytes, path, move |result| {
            if let Err(e) = result {
                alert(
                    Some(w.upcast_ref()),
                    "Could not export",
                    &format!("{shown}\n\n{e}"),
                );
            }
        });
    });
}
