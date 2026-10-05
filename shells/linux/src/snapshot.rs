//! Development aid: with `ROMLENS_SNAPSHOT=out.png` set, save the window as a
//! PNG a couple of seconds after it opens, then quit. Lets the UI be checked
//! without a screenshot tool, and is the hook for visual regression tests.
//!
//! Set up the state to capture with:
//! - `ROMLENS_TAB`: the editor tab to show first (`hex`, `disassembly`)
//! - `ROMLENS_OPEN`: comma-separated views to open in turn, by key
//!   (`code.assembly`, `code.c`, `graphics.tiles`, `audio.voices`, `atlas`)
//! - `ROMLENS_SPLIT`: comma-separated splits of the focused tab, `right` or
//!   `down`, made after the views open
//! - `ROMLENS_LAYOUT`: an Editor Layout preset (`two-columns`, `three`)
//! - `ROMLENS_CITE`: comma-separated SNES addresses in hex an answer's
//!   paragraph is pointing at, outlined in the Assembly and Hex tabs
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
//! - `ROMLENS_TUTOR`: ask the tutor this, answered by a scripted local server, and
//!   capture the Tutor window (run with `XDG_CONFIG_HOME` and `XDG_DATA_HOME` set
//!   to scratch folders: it adds an endpoint). `ROMLENS_TUTOR_LINES` has more
//!   composer lines to submit after it, such as `/help` or `/lessons`
//! - `ROMLENS_TUTOR_SCRIPT=edit`: the scripted model reads, then proposes a label
//! - `ROMLENS_CVERSION`: write a C version of the selected routine and show it
//!   (`tutor` for the tutor's, else yours); `ROMLENS_CEDIT` opens the sheet for
//!   one: `local`, `note`, `comment` or `version`
//! - `ROMLENS_WAIT`: milliseconds before the capture (default 3400)
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
            for key in std::env::var("ROMLENS_OPEN").unwrap_or_default().split(',') {
                if let Some(content) = crate::model::workspace::EditorContent::from_key(key.trim())
                {
                    doc.show(content);
                }
            }
            for edge in std::env::var("ROMLENS_SPLIT")
                .unwrap_or_default()
                .split(',')
            {
                match edge.trim() {
                    "right" => doc.split_focused(crate::model::workspace::DropEdge::Right),
                    "down" => doc.split_focused(crate::model::workspace::DropEdge::Bottom),
                    _ => {}
                }
            }
            if let Ok(preset) = std::env::var("ROMLENS_LAYOUT") {
                run_action(&window, &format!("editor-layout::{preset}"));
            }
            if let Ok(cited) = std::env::var("ROMLENS_CITE") {
                let addresses: Vec<u32> = cited
                    .split(',')
                    .filter_map(|a| u32::from_str_radix(a.trim().trim_start_matches('$'), 16).ok())
                    .collect();
                doc.point_at_citations(&addresses);
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
            if std::env::var("ROMLENS_CVERSION").is_ok() || std::env::var("ROMLENS_CEDIT").is_ok() {
                // The C takes a moment to come.
                let doc = Rc::clone(&doc);
                glib::timeout_add_local_once(Duration::from_millis(1200), move || {
                    scripted_cedit(&doc);
                });
            }
            if let Ok(question) = std::env::var("ROMLENS_TUTOR") {
                scripted_tutor(&doc, &question);
            }
            if let Ok(which) = std::env::var("ROMLENS_MENU")
                && let Some(button) = find_menu_button(window.upcast_ref(), &which)
            {
                button.popup();
            }
        }
    });

    let window = window.clone();
    let wait = std::env::var("ROMLENS_WAIT")
        .ok()
        .and_then(|w| w.parse().ok())
        .unwrap_or(3400);
    glib::timeout_add_local_once(Duration::from_millis(wait), move || {
        let target: gtk::Widget = std::env::var("ROMLENS_MENU")
            .ok()
            .and_then(|which| find_menu_button(window.upcast_ref(), &which))
            .and_then(|b| b.popover())
            .map_or_else(|| window.clone().upcast(), |p| p.upcast());
        if std::env::var("ROMLENS_DUMP").is_ok() {
            dump(&target, 0);
        }
        if !save(&target, &path) {
            eprintln!("snapshot failed");
        }
        if let Some(app) = window.application() {
            app.quit();
        }
    });
}

/// A C version of the routine shown, and the sheets that edit the C's notes.
fn scripted_cedit(doc: &Rc<Document>) {
    use crate::model::CEdit;
    use romlens_ffi::cnotes::{CAnchorInfo, CAuthor, CVersionInfo};
    let Some((routine, text)) = doc
        .decompile()
        .result
        .as_ref()
        .map(|r| (r.entry, r.text.clone()))
    else {
        eprintln!("no routine is shown: select an instruction with ROMLENS_SELECT");
        return;
    };
    if let Ok(who) = std::env::var("ROMLENS_CVERSION") {
        let mine = format!("// My own reading of this routine.\n{text}");
        let version = CVersionInfo {
            text: mine,
            author: if who == "tutor" {
                CAuthor::Tutor
            } else {
                CAuthor::User
            },
            anchors: vec![CAnchorInfo {
                first: 2,
                last: 3,
                start: routine,
                end: routine + 3,
            }],
        };
        let _ = doc.run_c_command(romlens_ffi::Command::SetCVersion {
            routine,
            name: "My version".into(),
            version: Some(version),
        });
        doc.show_c_version(Some("My version".into()));
    }
    if let Ok(which) = std::env::var("ROMLENS_CEDIT") {
        doc.begin_c_edit(match which.as_str() {
            "note" => CEdit::Note { routine },
            "comment" => CEdit::Comment { address: routine },
            "local" => CEdit::Local {
                routine,
                local: "v1".into(),
            },
            _ => CEdit::Version {
                routine,
                name: doc.shown_version(),
            },
        });
    }
}

/// Opens the Tutor window with a scripted model behind it: the question is
/// answered with a fixed reply that shows what the transcript can draw.
fn scripted_tutor(doc: &Rc<Document>, question: &str) {
    use crate::model::tutor_settings::{Endpoint, Kind};
    use romlens_ffi::tutor::session::{tutor_test_server, tutor_test_text_reply};
    let reply = "The **reset vector** at [$00:FFFC](romlens://a/FFFC) points at \
        [$00:8000](romlens://a/8000), where the game starts: it turns the screen off \
        with `INIDISP` and sets up the PPU before anything else.\n\n\
        ```asm\n$00:8000  SEI\n$00:8001  LDA #$8F\n$00:8003  STA $2100 ; force blank\n```\n\n\
        | Register | Meaning |\n|---|---|\n| `$2100` | Screen on or off, and brightness |\n\
        | `$4200` | NMI and joypad enables |\n\nTry `/learn DMA` next.";
    let replies = if std::env::var("ROMLENS_TUTOR_SCRIPT").is_ok_and(|s| s == "edit") {
        // A read, then an edit waiting for the answer.
        use romlens_ffi::tutor::session::tutor_test_call_reply;
        vec![
            tutor_test_call_reply("disassemble".into(), r#"{"address":"$00:8000"}"#.into()),
            tutor_test_call_reply(
                "set_label".into(),
                r#"{"address":"$00:8000","name":"Reset","reason":"The reset vector points here"}"#
                    .into(),
            ),
            tutor_test_text_reply(reply.into()),
        ]
    } else {
        vec![tutor_test_text_reply(reply.into())]
    };
    let url = tutor_test_server(replies);
    let settings = crate::settings::tutor();
    let taken: Vec<String> = settings
        .borrow()
        .endpoints()
        .into_iter()
        .map(|e| e.id)
        .collect();
    let e = Endpoint::local("Scripted", &url, Kind::Chat, &taken);
    let id = e.id.clone();
    settings.borrow_mut().add(e);
    settings.borrow_mut().edit(|s| {
        s.endpoint = id.clone();
        s.models.insert(id, "qwen3".into());
    });
    // The drawer's tutor, unless the script opened it as a tab.
    if doc.focused_content() != Some(crate::model::workspace::EditorContent::Tutor) {
        doc.show_tutor_in_drawer();
    }
    doc.tutor_submit(question);
    // More lines after the answer is in.
    if let Ok(lines) = std::env::var("ROMLENS_TUTOR_LINES") {
        let doc = Rc::clone(doc);
        glib::timeout_add_local_once(Duration::from_millis(1200), move || {
            for l in lines.split('|') {
                doc.tutor_submit(l);
            }
        });
    }
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

/// Development aid: the widget tree with each widget's size.
fn dump(w: &gtk::Widget, depth: usize) {
    eprintln!(
        "{}{} {}x{} {}",
        "  ".repeat(depth),
        w.type_().name(),
        w.width(),
        w.height(),
        w.css_classes().join(".")
    );
    let mut c = w.first_child();
    while let Some(child) = c {
        dump(&child, depth + 1);
        c = child.next_sibling();
    }
}
