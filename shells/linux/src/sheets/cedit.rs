//! Names a local, notes a routine, comments a statement, or writes a C
//! version (docs/24, U10): the same annotations the tutor makes. The macOS
//! twin is `CEditSheet`.

use std::rc::Rc;

use adw::prelude::*;
use romlens_ffi::cnotes::{CAuthor, CVersionInfo};
use romlens_ffi::{Command, format_snes_address};

use super::{Frame, destructive, frame, mono_entry, set_status, status};
use crate::model::{CEdit, Document};

fn title(e: &CEdit) -> String {
    match e {
        CEdit::Local { local, .. } => format!("Name the local {local}"),
        CEdit::Note { routine } => format!("Note on {}", format_snes_address(*routine)),
        CEdit::Comment { address } => format!("C comment at {}", format_snes_address(*address)),
        CEdit::Version { name, .. } => if name.is_none() {
            "New C version"
        } else {
            "Edit C version"
        }
        .into(),
    }
}

fn hint(e: &CEdit) -> &'static str {
    match e {
        CEdit::Local { .. } => "The C calls it this in this routine only.",
        CEdit::Note { .. } => {
            "Printed above the routine in the C: what it does, takes and returns."
        }
        CEdit::Comment { .. } => "Printed in the C before the statement this instruction makes.",
        CEdit::Version { .. } => {
            "Your own C for this routine, kept beside the generated C and never compiled. Lines keep the anchors they had."
        }
    }
}

/// What is there to remove.
fn can_remove(doc: &Document, e: &CEdit) -> bool {
    let wb = doc.workbench();
    match e {
        CEdit::Local { routine, local } => {
            wb.local_names(*routine).iter().any(|n| &n.local == local)
        }
        CEdit::Note { routine } => wb.routine_note(*routine).is_some(),
        CEdit::Comment { address } => wb.c_comments().iter().any(|c| c.address == *address),
        CEdit::Version { name, .. } => name.is_some(),
    }
}

/// The text the sheet starts with.
fn initial(doc: &Document, e: &CEdit) -> String {
    let wb = doc.workbench();
    match e {
        CEdit::Local { routine, local } => wb
            .local_names(*routine)
            .into_iter()
            .find(|n| &n.local == local)
            .map_or_else(|| local.clone(), |n| n.name),
        CEdit::Note { routine } => wb.routine_note(*routine).unwrap_or_default(),
        CEdit::Comment { address } => wb
            .c_comments()
            .into_iter()
            .find(|c| c.address == *address)
            .map(|c| c.text)
            .unwrap_or_default(),
        CEdit::Version { routine, name } => name
            .as_ref()
            .and_then(|n| doc.c_versions(*routine).into_iter().find(|v| &v.name == n))
            .map(|v| v.version.text)
            .or_else(|| doc.decompile().result.as_ref().map(|r| r.text.clone()))
            .unwrap_or_default(),
    }
}

/// The command a save makes. `value` is the trimmed text, `None` to remove;
/// `name` is the version's name as typed.
pub fn command(doc: &Document, e: &CEdit, text: &str, name: &str, remove: bool) -> Vec<Command> {
    let trimmed = text.trim();
    let value = (!remove && !trimmed.is_empty()).then(|| trimmed.to_owned());
    match e {
        CEdit::Local { routine, local } => vec![Command::SetLocalName {
            routine: *routine,
            local: local.clone(),
            name: value,
        }],
        CEdit::Note { routine } => vec![Command::SetRoutineNote {
            routine: *routine,
            text: value,
        }],
        CEdit::Comment { address } => vec![Command::SetCComment {
            address: *address,
            text: value,
        }],
        CEdit::Version { routine, name: old } => {
            let kept = old
                .as_ref()
                .and_then(|o| doc.c_versions(*routine).into_iter().find(|v| &v.name == o))
                .map(|v| v.version);
            let lines = text.split('\n').count() as u32;
            let anchors = kept
                .map(|k| k.anchors)
                .unwrap_or_default()
                .into_iter()
                .filter(|a| a.last <= lines)
                .collect();
            let name = name.trim().to_owned();
            let mut out = Vec::new();
            // A rename removes the old name first.
            if let Some(old) = old
                && *old != name
                && !remove
            {
                out.push(Command::SetCVersion {
                    routine: *routine,
                    name: old.clone(),
                    version: None,
                });
            }
            out.push(Command::SetCVersion {
                routine: *routine,
                name: if remove {
                    old.clone().unwrap_or(name)
                } else {
                    name
                },
                version: value.map(|_| CVersionInfo {
                    text: text.to_owned(),
                    author: CAuthor::User,
                    anchors,
                }),
            });
            out
        }
    }
}

pub fn build(doc: &Rc<Document>) -> Frame {
    let Some(edit) = doc.c_edit() else {
        return frame("C", 400, "Save");
    };
    let is_version = matches!(edit, CEdit::Version { .. });
    let f = frame(&title(&edit), if is_version { 560 } else { 400 }, "Save");
    let hint_label = status();
    set_status(&hint_label, hint(&edit), false);
    f.body.append(&hint_label);

    let start = initial(doc, &edit);
    let name_entry = gtk::Entry::builder()
        .placeholder_text("Plain words")
        .activates_default(true)
        .build();
    // One field for a local's name, a text view for the rest.
    let (get, focus): (Rc<dyn Fn() -> String>, gtk::Widget) = match &edit {
        CEdit::Local { .. } => {
            let e = mono_entry("a C name, e.g. slot");
            e.set_text(&start);
            f.body.append(&e);
            let e2 = e.clone();
            (Rc::new(move || e2.text().to_string()), e.upcast())
        }
        _ => {
            if let CEdit::Version { name, .. } = &edit {
                name_entry.set_text(name.as_deref().unwrap_or("My version"));
                f.body.append(&name_entry);
            }
            let tv = gtk::TextView::builder()
                .wrap_mode(if is_version {
                    gtk::WrapMode::None
                } else {
                    gtk::WrapMode::WordChar
                })
                .monospace(is_version)
                .top_margin(6)
                .bottom_margin(6)
                .left_margin(8)
                .right_margin(8)
                .build();
            tv.buffer().set_text(&start);
            let scroll = gtk::ScrolledWindow::builder()
                .min_content_height(if is_version { 260 } else { 90 })
                .child(&tv)
                .build();
            scroll.add_css_class("card");
            f.body.append(&scroll);
            let tv2 = tv.clone();
            (
                Rc::new(move || {
                    let b = tv2.buffer();
                    b.text(&b.start_iter(), &b.end_iter(), false).to_string()
                }),
                tv.upcast(),
            )
        }
    };
    let message = status();
    f.body.append(&message);

    let run = {
        let (doc, edit, get, name_entry, dialog, message) = (
            Rc::clone(doc),
            edit.clone(),
            Rc::clone(&get),
            name_entry.clone(),
            f.dialog.clone(),
            message,
        );
        Rc::new(move |remove: bool| {
            let name = name_entry.text().to_string();
            for c in command(&doc, &edit, &get(), &name, remove) {
                if let Err(e) = doc.run_c_command(c) {
                    set_status(&message, &e.to_string(), true);
                    return;
                }
            }
            // A saved version is the one shown.
            if let CEdit::Version { .. } = &edit
                && !remove
                && !get().trim().is_empty()
            {
                doc.show_c_version(Some(name.trim().to_owned()));
            }
            dialog.close();
        })
    };
    {
        let run = Rc::clone(&run);
        f.primary.connect_clicked(move |_| run(false));
    }
    if can_remove(doc, &edit) {
        let remove = destructive("Remove");
        remove.set_halign(gtk::Align::Start);
        let run = Rc::clone(&run);
        remove.connect_clicked(move |_| run(true));
        f.body.append(&remove);
    }
    f.dialog.set_focus(Some(&focus));
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing::TestRuntime;

    fn doc() -> Rc<Document> {
        let rt = TestRuntime::new();
        let rom =
            romlens_ffi::Rom::from_bytes(romlens_ffi::make_explain_test_rom(), "e.sfc".into())
                .unwrap();
        let d = Document::new(rom, rt.clone());
        d.start_analysis();
        rt.pump();
        d
    }

    fn run(d: &Rc<Document>, e: &CEdit, text: &str, name: &str, remove: bool) {
        for c in command(d, e, text, name, remove) {
            d.run_c_command(c).unwrap();
        }
    }

    #[test]
    fn a_version_is_written_renamed_shown_and_removed() {
        let d = doc();
        let new = CEdit::Version {
            routine: 0x8000,
            name: None,
        };
        run(&d, &new, "void f(void) {\n}\n", "  Mine ", false);
        let names = |d: &Rc<Document>| {
            d.c_versions(0x8000)
                .into_iter()
                .map(|v| v.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&d), ["Mine"], "the name is trimmed");
        assert_eq!(d.c_versions(0x8000)[0].version.author, CAuthor::User);
        d.show_c_version(Some("Mine".into()));
        assert_eq!(d.shown_version().as_deref(), Some("Mine"));

        // Renaming removes the old name, and the one shown is no longer.
        let edit = CEdit::Version {
            routine: 0x8000,
            name: Some("Mine".into()),
        };
        assert_eq!(
            command(&d, &edit, "void f(void) {}\n", "Better", false).len(),
            2
        );
        run(&d, &edit, "void f(void) {}\n", "Better", false);
        assert_eq!(names(&d), ["Better"]);
        assert_eq!(
            d.shown_version(),
            None,
            "a version that is gone is not shown"
        );

        // Empty text removes, as does the Remove button.
        let edit = CEdit::Version {
            routine: 0x8000,
            name: Some("Better".into()),
        };
        run(&d, &edit, "  \n", "Better", false);
        assert!(names(&d).is_empty());
        run(&d, &new, "x;", "Again", false);
        let edit = CEdit::Version {
            routine: 0x8000,
            name: Some("Again".into()),
        };
        run(&d, &edit, "x;", "Again", true);
        assert!(names(&d).is_empty());
    }

    #[test]
    fn a_local_a_note_and_a_comment_are_kept_and_taken_back() {
        let d = doc();
        let wb = d.workbench();
        let local = CEdit::Local {
            routine: 0x8000,
            local: "v1".into(),
        };
        run(&d, &local, " slot ", "", false);
        assert_eq!(wb.local_names(0x8000)[0].name, "slot");
        assert!(can_remove(&d, &local));
        assert_eq!(initial(&d, &local), "slot");
        run(&d, &local, "", "", true);
        assert!(wb.local_names(0x8000).is_empty());
        assert_eq!(
            initial(&d, &local),
            "v1",
            "an unnamed local starts as itself"
        );

        let note = CEdit::Note { routine: 0x8000 };
        run(&d, &note, "Starts the game.\n", "", false);
        assert_eq!(wb.routine_note(0x8000).as_deref(), Some("Starts the game."));
        run(&d, &note, "", "", false);
        assert_eq!(wb.routine_note(0x8000), None, "blank text removes the note");

        let comment = CEdit::Comment { address: 0x8000 };
        run(&d, &comment, "Reset", "", false);
        assert_eq!(wb.c_comments()[0].text, "Reset");
        assert!(
            d.session.is_dirty(),
            "the annotations are part of the project"
        );
    }

    #[test]
    fn asking_for_the_sheet_remembers_what_it_is_for() {
        let d = doc();
        assert_eq!(d.active_sheet(), None);
        d.begin_c_edit(CEdit::Note { routine: 0x8000 });
        assert_eq!(d.active_sheet(), Some(crate::model::Sheet::CEdit));
        assert_eq!(d.c_edit(), Some(CEdit::Note { routine: 0x8000 }));
        assert_eq!(title(&d.c_edit().unwrap()), "Note on $00:8000");
    }
}
