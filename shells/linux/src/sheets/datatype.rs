//! Mark as ▸ Data…, for the kinds that take a parameter.
//!
//! The parameters are what turn a row of numbers into `dw SUB_808423`, so the
//! sheet explains the bank rule rather than just offering it: a sixteen-bit
//! entry names an offset and something has to supply the bank, and choosing
//! wrongly is the difference between a label and a number.

use std::rc::Rc;

use adw::prelude::*;
use romlens_ffi::{BankRule, DataKind, TableElem};

use super::{Frame, caption, frame, set_status, status};
use crate::model::{Document, MarkOptions};

/// The kinds a person marks by hand, in the order they are reached for.
const KINDS: [(DataKind, &str); 12] = [
    (DataKind::Table, "Table"),
    (DataKind::Pointer, "Pointer"),
    (DataKind::Byte, "Byte"),
    (DataKind::Word, "Word"),
    (DataKind::Long, "Long"),
    (DataKind::String, "String"),
    (DataKind::Graphics, "Graphics"),
    (DataKind::Tilemap, "Tilemap"),
    (DataKind::Palette, "Palette"),
    (DataKind::Compressed, "Compressed"),
    (DataKind::Struct, "Struct"),
    (DataKind::Sample, "Sound Sample (BRR)"),
];
const STRIDES: [u8; 5] = [1, 2, 3, 4, 8];
const BPPS: [u8; 3] = [2, 4, 8];
const ELEMS: [(TableElem, &str); 3] = [
    (TableElem::Raw, "Raw bytes"),
    (TableElem::Pointer, "A pointer to data"),
    (TableElem::Code, "A pointer to code"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankChoice {
    Same,
    Fixed,
    Entry,
}

const BANKS: [(BankChoice, &str); 3] = [
    (BankChoice::Same, "Same bank as the table"),
    (BankChoice::Fixed, "A fixed bank"),
    (BankChoice::Entry, "Each entry carries its own"),
];

/// Which parameters a choice of kind takes.
#[derive(Debug, PartialEq, Eq)]
pub struct Takes {
    pub stride: bool,
    pub bpp: bool,
    pub elem: bool,
    pub bank: bool,
}

pub fn takes(kind: DataKind, elem: TableElem) -> Takes {
    Takes {
        stride: kind == DataKind::Table,
        bpp: kind == DataKind::Graphics,
        elem: kind == DataKind::Table,
        bank: kind == DataKind::Pointer || (kind == DataKind::Table && elem != TableElem::Raw),
    }
}

/// The bank rule a choice and a typed bank give, or `None` for an invalid
/// fixed bank.
pub fn bank_rule(choice: BankChoice, fixed: &str) -> Option<BankRule> {
    match choice {
        BankChoice::Same => Some(BankRule::SameBank),
        BankChoice::Entry => Some(BankRule::FromEntry),
        BankChoice::Fixed => u8::from_str_radix(fixed.trim().trim_start_matches('$'), 16)
            .ok()
            .map(|bank| BankRule::Fixed { bank }),
    }
}

pub fn explanation(takes_bank: bool, choice: BankChoice, fixed: &str) -> String {
    if !takes_bank {
        return "The analyzer keeps its own guess for everything you do not mark.".into();
    }
    match choice {
        BankChoice::Same => "Entries resolve in the bank the table itself is in, which is what a \
                             `JMP (abs,X)` dispatch does."
            .into(),
        BankChoice::Fixed => format!(
            "Every entry resolves in bank ${}, whatever bank the table is in.",
            fixed.trim().trim_start_matches('$').to_uppercase()
        ),
        BankChoice::Entry => {
            "Each entry carries its own bank, so entries are at least three bytes.".into()
        }
    }
}

fn combo(title: &str, items: &[&str]) -> adw::ComboRow {
    adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(items))
        .build()
}

pub fn build(doc: &Rc<Document>) -> Frame {
    let f = frame("Mark as Data", 460, "Mark");
    f.body.append(&caption(&range_text(doc)));

    let group = adw::PreferencesGroup::new();
    let kind = combo("Kind", &KINDS.map(|(_, n)| n));
    let stride = combo(
        "Bytes per entry",
        &STRIDES
            .map(|s| s.to_string())
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    stride.set_selected(1);
    let bpp = combo(
        "Bitplanes",
        &BPPS
            .map(|b| format!("{b} bpp"))
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    bpp.set_selected(1);
    let elem = combo("Each entry is", &ELEMS.map(|(_, n)| n));
    elem.set_selected(2);
    let bank = combo("Target bank", &BANKS.map(|(_, n)| n));
    let fixed = adw::EntryRow::builder().title("Bank").build();
    fixed.set_text("C0");
    for row in [
        kind.upcast_ref::<gtk::Widget>(),
        stride.upcast_ref(),
        bpp.upcast_ref(),
        elem.upcast_ref(),
        bank.upcast_ref(),
        fixed.upcast_ref(),
    ] {
        group.add(row);
    }
    f.body.append(&group);
    let message = status();
    f.body.append(&message);

    let selected = {
        let (kind, elem, bank) = (kind.clone(), elem.clone(), bank.clone());
        move || {
            (
                KINDS[kind.selected() as usize].0,
                ELEMS[elem.selected() as usize].0,
                BANKS[bank.selected() as usize].0,
            )
        }
    };
    let refresh = {
        let (stride, bpp, elem, bank, fixed, message, primary, selected) = (
            stride.clone(),
            bpp.clone(),
            elem.clone(),
            bank.clone(),
            fixed.clone(),
            message.clone(),
            f.primary.clone(),
            selected.clone(),
        );
        move || {
            let (k, e, b) = selected();
            let t = takes(k, e);
            stride.set_visible(t.stride);
            bpp.set_visible(t.bpp);
            elem.set_visible(t.elem);
            bank.set_visible(t.bank);
            fixed.set_visible(t.bank && b == BankChoice::Fixed);
            let valid = !t.bank || bank_rule(b, &fixed.text()).is_some();
            primary.set_sensitive(valid);
            set_status(&message, &explanation(t.bank, b, &fixed.text()), false);
        }
    };
    refresh();
    for row in [&kind, &elem, &bank] {
        let refresh = refresh.clone();
        row.connect_selected_notify(move |_| refresh());
    }
    {
        let refresh = refresh.clone();
        fixed.connect_changed(move |_| refresh());
    }

    f.primary.connect_clicked({
        let (doc, dialog) = (Rc::clone(doc), f.dialog.clone());
        move |_| {
            let (k, e, b) = selected();
            let t = takes(k, e);
            let options = MarkOptions {
                stride: t.stride.then(|| STRIDES[stride.selected() as usize]),
                bpp: t.bpp.then(|| BPPS[bpp.selected() as usize]),
                elem: t.elem.then_some(e),
                bank: if t.bank {
                    bank_rule(b, &fixed.text())
                } else {
                    None
                },
            };
            let _ = doc.mark_with(k, options);
            dialog.close();
        }
    });
    f
}

fn range_text(doc: &Document) -> String {
    match doc.highlighted_range() {
        None => "No selection".into(),
        Some(r) => format!(
            "{}  {} byte{}",
            romlens_ffi::format_file_offset(r.start),
            r.len(),
            if r.len() == 1 { "" } else { "s" }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_takes_its_own_parameters() {
        let t = takes(DataKind::Table, TableElem::Code);
        assert_eq!(
            t,
            Takes {
                stride: true,
                bpp: false,
                elem: true,
                bank: true
            }
        );
        // A raw table has no targets, so no bank to choose.
        assert!(!takes(DataKind::Table, TableElem::Raw).bank);
        assert!(takes(DataKind::Pointer, TableElem::Raw).bank);
        assert!(takes(DataKind::Graphics, TableElem::Raw).bpp);
        assert_eq!(
            takes(DataKind::String, TableElem::Code),
            Takes {
                stride: false,
                bpp: false,
                elem: false,
                bank: false
            }
        );
    }

    #[test]
    fn the_bank_rule_follows_the_choice() {
        assert_eq!(bank_rule(BankChoice::Same, ""), Some(BankRule::SameBank));
        assert_eq!(
            bank_rule(BankChoice::Entry, "zz"),
            Some(BankRule::FromEntry)
        );
        assert_eq!(
            bank_rule(BankChoice::Fixed, "$C0"),
            Some(BankRule::Fixed { bank: 0xC0 })
        );
        assert_eq!(
            bank_rule(BankChoice::Fixed, " 7e "),
            Some(BankRule::Fixed { bank: 0x7E })
        );
        assert_eq!(bank_rule(BankChoice::Fixed, "1FF"), None);
        assert_eq!(bank_rule(BankChoice::Fixed, ""), None);
    }

    #[test]
    fn the_explanation_names_the_rule() {
        assert!(explanation(false, BankChoice::Same, "").contains("keeps its own guess"));
        assert!(explanation(true, BankChoice::Same, "").contains("bank the table itself is in"));
        assert!(explanation(true, BankChoice::Fixed, "$c0").contains("bank $C0"));
        assert!(explanation(true, BankChoice::Entry, "").contains("three bytes"));
    }
}
