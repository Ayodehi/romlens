//! The primary (hamburger) menu: the macOS menu bar's tree, flattened into
//! sections and submenus. Every item names an action from `actions.rs`, so a
//! command not built yet shows insensitive in its final place. The macOS twin
//! is `MainMenu`.

use gtk::gio;

use crate::model::Tab;

fn section(items: &[(&str, &str)]) -> gio::Menu {
    let m = gio::Menu::new();
    for (label, action) in items {
        m.append(Some(label), Some(action));
    }
    m
}

fn submenu(parent: &gio::Menu, label: &str, children: &[&gio::Menu]) {
    let m = gio::Menu::new();
    for c in children {
        m.append_section(None, *c);
    }
    parent.append_submenu(Some(label), &m);
}

pub fn primary_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    menu.append_section(
        None,
        &section(&[
            ("Open…", "app.open"),
            ("Open Project…", "app.open-project"),
            ("Save", "win.save"),
            ("Save As…", "win.save-as"),
            ("Duplicate", "win.duplicate"),
            ("Revert to Saved", "win.revert"),
        ]),
    );
    menu.append_section(
        None,
        &section(&[
            ("Close Tab", "win.close-tab"),
            ("Close Window", "window.close"),
        ]),
    );
    menu.append_section(
        None,
        &section(&[
            ("Open Recording…", "win.open-recording"),
            ("Import Snapshot…", "win.import-snapshot"),
            ("Export Frame Region…", "win.export-frame-region"),
            ("Close Recording", "win.close-recording"),
            ("Live Session", "win.live-session"),
            ("Compare With…", "win.compare-with"),
            ("Compare With Project…", "win.compare-with-project"),
            ("Close Comparison", "win.close-compare"),
        ]),
    );
    let sections = gio::Menu::new();
    submenu(
        &sections,
        "Import",
        &[&section(&[
            ("Execution Trace…", "win.import-trace"),
            ("Symbols…", "win.import-symbols"),
            ("ca65 Debug Information…", "win.import-dbg"),
        ])],
    );
    submenu(
        &sections,
        "Export",
        &[&section(&[
            ("Assembly Listing…", "win.export-assembly"),
            ("Labels and Comments…", "win.export-annotations"),
            ("Symbol File…", "win.export-symbols"),
        ])],
    );
    submenu(&sections, "Edit", &[&edit_items()]);
    let view = view_items();
    submenu(&sections, "View", &view.iter().collect::<Vec<_>>());
    submenu(&sections, "Go", &[&go_items()]);
    menu.append_section(None, &sections);
    menu.append_section(
        None,
        &section(&[
            ("Save Mesen Recorder Script…", "win.save-recorder-script"),
            ("Settings", "app.settings"),
            ("Keyboard Shortcuts", "app.shortcuts"),
            ("About Romlens", "app.about"),
        ]),
    );
    menu
}

fn edit_items() -> gio::Menu {
    let m = gio::Menu::new();
    m.append_section(
        None,
        &section(&[("Undo", "win.undo"), ("Redo", "win.redo")]),
    );
    m.append_section(
        None,
        &section(&[
            ("Rename Label…", "win.rename-label"),
            ("Remove Label", "win.remove-label"),
            ("Define Variable…", "win.define-variable"),
            ("Comment…", "win.edit-comment"),
        ]),
    );
    let marks = gio::Menu::new();
    marks.append_section(
        None,
        &section(&[
            ("Code", "win.mark-code"),
            ("Data", "win.mark-data"),
            ("Unknown", "win.mark-unknown"),
        ]),
    );
    marks.append_section(None, &section(&[("Data…", "win.mark-data-options")]));
    marks.append_section(
        None,
        &section(&[
            ("String", "win.mark-string"),
            ("Word", "win.mark-word"),
            ("Pointer", "win.mark-pointer"),
            ("Graphics", "win.mark-graphics"),
            ("Palette", "win.mark-palette"),
            ("Tilemap", "win.mark-tilemap"),
            ("Compressed", "win.mark-compressed"),
        ]),
    );
    let marking = gio::Menu::new();
    marking.append_submenu(Some("Mark as"), &marks);
    marking.append(Some("Clear Mark"), Some("win.clear-mark"));
    marking.append(Some("Set Flags…"), Some("win.set-flags"));
    m.append_section(None, &marking);
    m.append_section(
        None,
        &section(&[
            ("Copy Address", "win.copy-address"),
            ("Copy Line", "win.copy-line"),
        ]),
    );
    m
}

/// The action that shows `tab`. Source and Compare have their own, so the
/// menu can grey them out while there is nothing to show.
pub fn tab_action(tab: Tab) -> String {
    match tab {
        Tab::Source => "win.show-source".to_owned(),
        Tab::Compare => "win.show-compare".to_owned(),
        _ => format!("win.show-tab::{}", tab.id()),
    }
}

/// The tabs, then the graphics and sound views, then panes and zoom.
fn view_items() -> Vec<gio::Menu> {
    let tabs = gio::Menu::new();
    for tab in Tab::BUILT {
        tabs.append(Some(tab.title()), Some(&tab_action(tab)));
    }
    let graphics = section(&[
        ("Frame", "win.show-frame"),
        ("Layers", "win.show-layers"),
        ("Tile Decoder", "win.show-tiles"),
        ("Palette", "win.show-palette"),
        ("OAM", "win.show-oam"),
        ("Tilemap", "win.show-tilemap"),
    ]);
    let audio = section(&[
        ("Voices", "win.show-voices"),
        ("Timeline", "win.show-timeline"),
        ("Samples", "win.show-samples"),
        ("Audio RAM", "win.show-audio-ram"),
        ("Ports", "win.show-ports"),
        ("Echo & Effects", "win.show-echo"),
        ("Scope", "win.show-scope"),
    ]);
    let kinds = gio::Menu::new();
    kinds.append_submenu(Some("Graphics"), &graphics);
    kinds.append_submenu(Some("Audio"), &audio);
    kinds.append(Some("Tutor"), Some("win.show-tutor"));
    // Tabs and groups (docs/29).
    let groups = section(&[
        ("Split Right", "win.split-right"),
        ("Split Down", "win.split-down"),
    ]);
    let layouts = gio::Menu::new();
    for p in crate::model::workspace::LayoutPreset::ALL {
        layouts.append(
            Some(p.title()),
            Some(&format!("win.editor-layout::{}", p.id())),
        );
    }
    groups.append_submenu(Some("Editor Layout"), &layouts);
    groups.append(Some("Next Tab"), Some("win.next-tab"));
    groups.append(Some("Previous Tab"), Some("win.previous-tab"));
    let focus = gio::Menu::new();
    for n in 1..=4 {
        focus.append(
            Some(&format!("Group {n}")),
            Some(&format!("win.focus-group::{n}")),
        );
    }
    groups.append_submenu(Some("Focus Group"), &focus);
    let addresses = section(&[
        ("File Offset and SNES Address", "win.address-style::both"),
        ("SNES Address Only", "win.address-style::snes"),
        ("File Offset Only", "win.address-style::file"),
    ]);
    let panes = section(&[
        ("Show Sidebar", "win.toggle-navigator"),
        ("Show Inspector", "win.toggle-inspector"),
        ("Show Overview Strip", "win.toggle-strip"),
        ("Show Results", "win.toggle-results"),
        ("Focus on Code", "win.focus-on-code"),
        ("Show Explanations", "win.toggle-explanations"),
    ]);
    let zoom = section(&[
        ("Zoom In", "win.zoom-in"),
        ("Zoom Out", "win.zoom-out"),
        ("Zoom to Fit", "win.zoom-fit"),
    ]);
    vec![tabs, kinds, groups, addresses, panes, zoom]
}

fn go_items() -> gio::Menu {
    let m = gio::Menu::new();
    m.append_section(
        None,
        &section(&[
            ("Find…", "win.find"),
            ("Find Next", "win.find-next"),
            ("Find Previous", "win.find-previous"),
        ]),
    );
    m.append_section(
        None,
        &section(&[
            ("Open Quickly…", "win.open-quickly"),
            ("Jump to Address…", "win.jump-to-address"),
            ("Follow Reference", "win.follow-reference"),
            ("Find References", "win.find-references"),
            ("Back", "win.go-back"),
            ("Forward", "win.go-forward"),
        ]),
    );
    m.append_section(
        None,
        &section(&[
            ("Header", "win.go-header"),
            ("Reset Vector", "win.go-reset"),
        ]),
    );
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::glib;
    use gtk::prelude::*;

    fn collect(model: &gio::MenuModel, out: &mut Vec<String>) {
        for i in 0..model.n_items() {
            if let Some(a) = model
                .item_attribute_value(i, "action", Some(glib::VariantTy::STRING))
                .and_then(|v| v.get::<String>())
            {
                out.push(a);
            }
            for link in ["section", "submenu"] {
                if let Some(child) = model.item_link(i, link) {
                    collect(&child, out);
                }
            }
        }
    }

    #[test]
    fn every_menu_item_names_a_known_action() {
        let mut names = Vec::new();
        collect(primary_menu().upcast_ref(), &mut names);
        assert!(names.len() > 70, "{} items", names.len());
        for name in &names {
            assert!(
                crate::actions::knows(name),
                "menu names unknown action {name}"
            );
        }
    }

    #[test]
    fn the_menu_has_no_duplicate_items() {
        // An action with a parameter (a tab, an address style) appears once
        // per value, so the target is part of the identity.
        fn walk(model: &gio::MenuModel, out: &mut Vec<String>) {
            for i in 0..model.n_items() {
                if let Some(a) = model
                    .item_attribute_value(i, "action", Some(glib::VariantTy::STRING))
                    .and_then(|v| v.get::<String>())
                {
                    let target = model
                        .item_attribute_value(i, "target", None)
                        .map(|t| t.print(false).to_string())
                        .unwrap_or_default();
                    out.push(format!("{a}::{target}"));
                }
                for link in ["section", "submenu"] {
                    if let Some(child) = model.item_link(i, link) {
                        walk(&child, out);
                    }
                }
            }
        }
        let mut names = Vec::new();
        walk(primary_menu().upcast_ref(), &mut names);
        let mut seen = std::collections::HashSet::new();
        for n in names {
            assert!(seen.insert(n.clone()), "{n} appears twice");
        }
    }
}
