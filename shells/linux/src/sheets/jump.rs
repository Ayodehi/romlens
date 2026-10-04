//! Ctrl+L. Resolves on every keystroke so you see `0x00041C = $80:841C` or
//! the core's message before pressing Enter.

use std::rc::Rc;

use adw::prelude::*;

use super::{Frame, frame, mono_entry, set_status, status};
use crate::model::Document;

pub fn build(doc: &Rc<Document>) -> Frame {
    let f = frame("Jump to Address", 420, "Jump");
    let entry = mono_entry("$80:841C, 80841C or 0x41C");
    let message = status();
    f.body.append(&entry);
    f.body.append(&message);
    f.primary.set_sensitive(false);
    set_status(
        &message,
        "File offsets start with 0x; SNES addresses with $ or bank:offset.",
        false,
    );

    let update = {
        let (doc, message, primary) = (Rc::clone(doc), message.clone(), f.primary.clone());
        move |e: &gtk::Entry| {
            let text = e.text();
            if text.trim().is_empty() {
                primary.set_sensitive(false);
                set_status(
                    &message,
                    "File offsets start with 0x; SNES addresses with $ or bank:offset.",
                    false,
                );
                return;
            }
            match doc.preview_address(&text) {
                Ok(r) => {
                    primary.set_sensitive(true);
                    set_status(&message, &describe(r), false);
                }
                Err(e) => {
                    primary.set_sensitive(false);
                    set_status(&message, &e.to_string(), true);
                }
            }
        }
    };
    entry.connect_changed(update);
    f.primary.connect_clicked({
        let (doc, entry, dialog) = (Rc::clone(doc), entry.clone(), f.dialog.clone());
        move |_| {
            if doc.jump_text(&entry.text()).is_ok() {
                dialog.close();
            }
        }
    });
    f.dialog.set_focus(Some(&entry));
    f
}

/// What the expression resolved to, in the units the editor shows.
pub fn describe(r: romlens_ffi::ResolvedAddress) -> String {
    format!(
        "{} = {}  ·  row {}",
        romlens_ffi::format_file_offset(r.file_offset),
        r.snes_address
            .map_or("unreachable".to_owned(), romlens_ffi::format_snes_address),
        r.row
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_reads_like_the_macos_sheet() {
        let r = romlens_ffi::ResolvedAddress {
            file_offset: 0x41C,
            snes_address: Some(0x80_841C),
            row: 65,
        };
        assert_eq!(describe(r), "0x00041C = $80:841C  ·  row 65");
        let r = romlens_ffi::ResolvedAddress {
            file_offset: 0,
            snes_address: None,
            row: 0,
        };
        assert!(describe(r).contains("unreachable"));
    }
}
