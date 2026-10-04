//! Define Variable: name an address and give it a type, so instructions that
//! use it read `STA PlayerX` (and `STA PlayerX+1` inside it) rather than an
//! address. Opened from an instruction, it starts at the operand's address.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use romlens_ffi::VarWidth;

use super::label::validation;
use super::{Frame, destructive, frame, set_status, status};
use crate::model::{Document, VariableDraft};

const WIDTHS: [(VarWidth, &str); 3] = [
    (VarWidth::Byte, "byte"),
    (VarWidth::Word, "word"),
    (VarWidth::Long, "long"),
];

/// Why the draft cannot be saved, if it cannot.
pub fn problem(d: &VariableDraft) -> Option<String> {
    let Some(address) = d.resolved_address() else {
        return Some(if d.address.trim().is_empty() {
            "An address: $7E:0094, 7F8000, or $0094 for low RAM.".to_owned()
        } else {
            "Not an address; use a form like $7E:0094.".to_owned()
        });
    };
    let name = d.name.trim();
    if name.is_empty() {
        return Some("A name: letters, digits and underscores.".to_owned());
    }
    validation(name, Some(address))
}

pub fn build(doc: &Rc<Document>) -> Frame {
    let initial = doc.variable_draft();
    let editing = initial.existing.is_some();
    let f = frame(
        if editing {
            "Edit Variable"
        } else {
            "Define Variable"
        },
        440,
        if editing { "Save" } else { "Define" },
    );
    let draft = Rc::new(RefCell::new(initial.clone()));

    let group = adw::PreferencesGroup::new();
    let address = adw::EntryRow::builder().title("Address").build();
    address.set_text(&initial.address);
    if let Some(existing) = initial.existing {
        // The address of an existing variable cannot change.
        address.set_text(&romlens_ffi::format_snes_address(existing));
        address.set_editable(false);
    }
    let name = adw::EntryRow::builder().title("Name").build();
    name.set_text(&initial.name);
    let width = adw::ComboRow::builder()
        .title("Type")
        .model(&gtk::StringList::new(&WIDTHS.map(|(_, n)| n)))
        .selected(
            WIDTHS
                .iter()
                .position(|(w, _)| *w == initial.width)
                .unwrap_or(0) as u32,
        )
        .build();
    let count = adw::SpinRow::with_range(1.0, 4096.0, 1.0);
    count.set_title("Elements");
    count.set_subtitle("More than one makes an array");
    count.set_value(f64::from(initial.count));
    for row in [
        address.upcast_ref::<gtk::Widget>(),
        name.upcast_ref(),
        width.upcast_ref(),
        count.upcast_ref(),
    ] {
        group.add(row);
    }
    f.body.append(&group);
    let message = status();
    f.body.append(&message);

    let update = {
        let (draft, message, primary) = (Rc::clone(&draft), message.clone(), f.primary.clone());
        move || {
            let d = draft.borrow();
            match problem(&d) {
                Some(p) => {
                    primary.set_sensitive(false);
                    set_status(&message, &p, true);
                }
                None => {
                    primary.set_sensitive(true);
                    let n = d.size();
                    set_status(
                        &message,
                        &format!(
                            "{n} byte{}; an access inside reads as name+offset.",
                            if n == 1 { "" } else { "s" }
                        ),
                        false,
                    );
                }
            }
        }
    };
    update();
    address.connect_changed({
        let (draft, update) = (Rc::clone(&draft), update.clone());
        move |e| {
            draft.borrow_mut().address = e.text().to_string();
            update();
        }
    });
    name.connect_changed({
        let (draft, update) = (Rc::clone(&draft), update.clone());
        move |e| {
            draft.borrow_mut().name = e.text().to_string();
            update();
        }
    });
    width.connect_selected_notify({
        let (draft, update) = (Rc::clone(&draft), update.clone());
        move |r| {
            if let Some((w, _)) = WIDTHS.get(r.selected() as usize) {
                draft.borrow_mut().width = *w;
                update();
            }
        }
    });
    count.connect_value_notify({
        let (draft, update) = (Rc::clone(&draft), update.clone());
        move |r| {
            draft.borrow_mut().count = r.value() as u16;
            update();
        }
    });

    if let Some(existing) = initial.existing {
        let remove = destructive("Remove");
        remove.set_halign(gtk::Align::Start);
        remove.connect_clicked({
            let (doc, dialog) = (Rc::clone(doc), f.dialog.clone());
            move |_| {
                let _ = doc.remove_variable(existing);
                dialog.close();
            }
        });
        f.body.append(&remove);
    }
    f.primary.connect_clicked({
        let (doc, draft, dialog) = (Rc::clone(doc), draft, f.dialog.clone());
        move |_| match doc.define_variable(&draft.borrow()) {
            Ok(()) => {
                dialog.close();
            }
            Err(e) => set_status(&message, &e.to_string(), true),
        }
    });
    f.dialog.set_focus(Some(&name));
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(address: &str, name: &str) -> VariableDraft {
        VariableDraft {
            address: address.into(),
            name: name.into(),
            ..VariableDraft::default()
        }
    }

    #[test]
    fn each_missing_piece_is_asked_for_in_turn() {
        assert!(problem(&draft("", "")).unwrap().starts_with("An address"));
        assert!(
            problem(&draft("zz", "X"))
                .unwrap()
                .starts_with("Not an address")
        );
        assert!(problem(&draft("$0094", "")).unwrap().starts_with("A name"));
        assert!(problem(&draft("$0094", "1bad")).is_some());
        assert_eq!(problem(&draft("$0094", "PlayerX")), None);
    }

    #[test]
    fn an_existing_variable_needs_no_typed_address() {
        let d = VariableDraft {
            existing: Some(0x7E_0094),
            name: "PlayerX".into(),
            ..VariableDraft::default()
        };
        assert_eq!(problem(&d), None);
    }
}
