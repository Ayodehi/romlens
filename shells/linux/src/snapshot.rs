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
//! - `ROMLENS_IMPORT_DBG`: import this ca65 `.dbg` first, which brings the Source tab
//! - `ROMLENS_COMPARE`: compare with this ROM file or project folder first
//! - `ROMLENS_COMPARE_ITEM`: choose a change in the Compare list: `routine:0`, `data:1`...
//! - `ROMLENS_SCREEN`: open the inspector's Screen section
//! - `ROMLENS_RECORDING`: attach this `.romrec` first, then `ROMLENS_FRAME` (a frame
//!   number) and `ROMLENS_PIXEL` (`x,y`, kept in the Frame view)
//! - `ROMLENS_PLAY`: start playing the sound view's source after it opens
//! - `ROMLENS_SAMPLE`: select a sample (and a block) in the Samples view: `0` or `0:1`
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

    // Imports and comparisons take a moment, so they start first.
    glib::timeout_add_local_once(Duration::from_millis(1000), {
        let doc = Rc::clone(doc);
        move || {
            if let Ok(dbg) = std::env::var("ROMLENS_IMPORT_DBG") {
                doc.import(crate::model::transfer::ImportKind::Dbg, dbg.into(), |_| {});
            }
            if let Ok(path) = std::env::var("ROMLENS_RECORDING")
                && let Ok(session) = romlens_ffi::RecordingSession::open(path, false)
            {
                let _ = doc.attach_recording(session, "recording.romrec");
                if let Some(n) = std::env::var("ROMLENS_FRAME")
                    .ok()
                    .and_then(|f| f.parse().ok())
                {
                    doc.set_frame(n);
                }
                if let Some((x, y)) = std::env::var("ROMLENS_PIXEL")
                    .ok()
                    .and_then(|p| {
                        p.split_once(',')
                            .map(|(x, y)| (x.parse().ok(), y.parse().ok()))
                    })
                    .and_then(|(x, y)| Some((x?, y?)))
                {
                    doc.edit_graphics(|g| {
                        g.selected_pixel = Some((x, y));
                        None
                    });
                }
            }
            if let Ok(other) = std::env::var("ROMLENS_COMPARE") {
                doc.compare_with(other.into());
            }
        }
    });

    // Once the first analysis has had time to land.
    glib::timeout_add_local_once(Duration::from_millis(2000), {
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
            if let Ok(spec) = std::env::var("ROMLENS_COMPARE_ITEM")
                && let Some((kind, n)) = spec.split_once(':')
                && let Ok(n) = n.parse()
            {
                use crate::model::compare::Item;
                doc.select_compare_item(match kind {
                    "routine" => Some(Item::Routine(n)),
                    "data" => Some(Item::Data(n)),
                    "run" => Some(Item::Run(n)),
                    "move" => Some(Item::Move(n)),
                    _ => None,
                });
            }
            if std::env::var("ROMLENS_SCREEN").is_ok() {
                doc.set_show_screen(true);
            }
            if let Ok(spec) = std::env::var("ROMLENS_SAMPLE") {
                let mut parts = spec.split(':');
                let sample = parts.next().and_then(|p| p.parse().ok());
                let block = parts.next().and_then(|p| p.parse().ok());
                doc.edit_audio(|a| {
                    a.selected_sample = sample;
                    a.selected_block = block;
                });
            }
            if std::env::var("ROMLENS_PLAY").is_ok() {
                doc.edit_audio(|a| a.play());
                // Say whether the sound output could start: a headless
                // machine has none.
                eprintln!("audio output: {:?}", doc.audio().output.problem);
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
    glib::timeout_add_local_once(Duration::from_millis(3400), move || {
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
