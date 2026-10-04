//! Development aid: with `ROMLENS_SNAPSHOT=out.png` set, save the window as a
//! PNG a couple of seconds after it opens, then quit. Lets the UI be checked
//! without a screenshot tool, and is the hook for visual regression tests.
//!
//! Set up the state to capture with:
//! - `ROMLENS_TAB`: the editor tab to show first (`hex`, `disassembly`)
//! - `ROMLENS_SELECT`: a file offset (decimal or 0x hex) to jump to
//! - `ROMLENS_SIZE`: `WIDTHxHEIGHT` for the window
//! - `ROMLENS_ACTIONS`: comma-separated window actions to run, each
//!   `name` or `name::string-parameter` (`focus-on-code`, `address-style::snes`)
//! - `ROMLENS_FIND`: run Find with this query (hex bytes) and show the results
//! - `ROMLENS_REFS`: Find References to the selected item
//! - `ROMLENS_MENU`: capture a menu instead of the window: `primary`, or
//!   the label of the menu button (`File + SNES`)

use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{glib, graphene};

use crate::model::Document;

pub fn maybe_capture(window: &adw::ApplicationWindow, doc: &Rc<Document>) {
    let Ok(path) = std::env::var("ROMLENS_SNAPSHOT") else {
        return;
    };
    if let Some((w, h)) = std::env::var("ROMLENS_SIZE").ok().and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    }) {
        window.set_default_size(w, h);
    }

    // Once the first analysis has had time to land.
    glib::timeout_add_local_once(Duration::from_millis(1200), {
        let (window, doc) = (window.clone(), Rc::clone(doc));
        move || {
            if let Ok(tab) = std::env::var("ROMLENS_TAB") {
                run_action(&window, &format!("show-tab::{tab}"));
            }
            if let Ok(text) = std::env::var("ROMLENS_SELECT") {
                let parsed = match text.strip_prefix("0x") {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => text.parse().ok(),
                };
                if let Some(offset) = parsed {
                    doc.jump_to(offset);
                }
            }
            for spec in std::env::var("ROMLENS_ACTIONS")
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
            {
                run_action(&window, spec);
            }
            if let Ok(q) = std::env::var("ROMLENS_FIND") {
                doc.edit_search(|s| s.query = q);
                doc.run_search();
                doc.show_results(crate::model::ResultsKind::Find);
            }
            if std::env::var("ROMLENS_REFS").is_ok() {
                doc.find_references();
            }
            if let Ok(which) = std::env::var("ROMLENS_MENU")
                && let Some(button) = find_menu_button(window.upcast_ref(), &which)
            {
                button.popup();
            }
        }
    });

    let window = window.clone();
    glib::timeout_add_local_once(Duration::from_millis(2600), move || {
        let target: gtk::Widget = std::env::var("ROMLENS_MENU")
            .ok()
            .and_then(|which| find_menu_button(window.upcast_ref(), &which))
            .and_then(|b| b.popover())
            .map_or_else(|| window.clone().upcast(), |p| p.upcast());
        if !save(&target, &path) {
            eprintln!("snapshot failed");
        }
        if let Some(app) = window.application() {
            app.quit();
        }
    });
}

/// `name` or `name::parameter`, run as a window action.
fn run_action(window: &adw::ApplicationWindow, spec: &str) {
    let (name, param) = match spec.split_once("::") {
        Some((n, p)) => (n, Some(p.to_variant())),
        None => (spec, None),
    };
    // `app.name` reaches the application; anything else is a window action.
    match name.strip_prefix("app.") {
        Some(app_action) => {
            if let Some(app) = window.application() {
                gtk::gio::prelude::ActionGroupExt::activate_action(
                    &app,
                    app_action,
                    param.as_ref(),
                );
            }
        }
        None => {
            let _ = WidgetExt::activate_action(window, &format!("win.{name}"), param.as_ref());
        }
    }
}

fn find_menu_button(root: &gtk::Widget, which: &str) -> Option<gtk::MenuButton> {
    let mut stack = vec![root.clone()];
    while let Some(w) = stack.pop() {
        if let Ok(b) = w.clone().downcast::<gtk::MenuButton>() {
            let matches = if which == "primary" {
                b.icon_name().as_deref() == Some("open-menu-symbolic")
            } else {
                b.label().as_deref() == Some(which)
            };
            if matches {
                return Some(b);
            }
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            stack.push(c);
        }
    }
    None
}

fn save(target: &gtk::Widget, path: &str) -> bool {
    let paintable = gtk::WidgetPaintable::new(Some(target));
    let (w, h) = (target.width(), target.height());
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(w), f64::from(h));
    snapshot
        .to_node()
        .zip(target.native())
        .and_then(|(node, native)| {
            let renderer = native.renderer()?;
            let texture = renderer.render_texture(
                &node,
                Some(&graphene::Rect::new(0.0, 0.0, w as f32, h as f32)),
            );
            texture.save_to_png(path).ok()
        })
        .is_some()
}
