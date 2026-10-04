//! Set Flags…: pin M, X, E, the data bank and the direct page at the
//! selected instruction. The analyzer re-runs from here with these values.

use std::rc::Rc;

use adw::prelude::*;
use romlens_ffi::FlagOverride;

use super::{Frame, caption, destructive, frame, set_status, status};
use crate::model::Document;

/// The text of a hex field: `Some(None)` is empty, `Some(Some(v))` a value,
/// and `None` is not hexadecimal or too long.
pub fn parse_hex(text: &str, digits: usize) -> Option<Option<u32>> {
    let t = text.trim().replace('$', "");
    if t.is_empty() {
        return Some(None);
    }
    if t.len() > digits {
        return None;
    }
    u32::from_str_radix(&t, 16).ok().map(Some)
}

fn tri(value: Option<bool>) -> u32 {
    match value {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    }
}

fn untri(index: u32) -> Option<bool> {
    match index {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

fn tri_row(title: &str, set: &str, clear: &str, value: Option<bool>) -> adw::ComboRow {
    adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(&["Unchanged", set, clear]))
        .selected(tri(value))
        .build()
}

pub fn build(doc: &Rc<Document>) -> Frame {
    let f = frame("Set Flags", 420, "Apply");
    if let Some(a) = doc.selected_address() {
        f.body.append(&caption(&format!(
            "at {}",
            romlens_ffi::format_snes_address(a)
        )));
    }
    let current = doc.details().flag_override;
    let cur = current.unwrap_or_default();

    let group = adw::PreferencesGroup::new();
    let m = tri_row("M (accumulator)", "8-bit", "16-bit", cur.m);
    let x = tri_row("X (index)", "8-bit", "16-bit", cur.x);
    let e = tri_row("E (emulation)", "emulation", "native", cur.e);
    let dbr = adw::EntryRow::builder().title("Data bank").build();
    dbr.set_text(&cur.dbr.map_or(String::new(), |v| format!("${v:02X}")));
    let dp = adw::EntryRow::builder().title("Direct page").build();
    dp.set_text(&cur.dp.map_or(String::new(), |v| format!("${v:04X}")));
    for row in [
        m.upcast_ref::<gtk::Widget>(),
        x.upcast_ref(),
        e.upcast_ref(),
        dbr.upcast_ref(),
        dp.upcast_ref(),
    ] {
        group.add(row);
    }
    f.body.append(&group);
    let message = status();
    f.body.append(&message);

    let valid = {
        let (dbr, dp, message, primary) =
            (dbr.clone(), dp.clone(), message.clone(), f.primary.clone());
        move || {
            let ok = parse_hex(&dbr.text(), 2).is_some() && parse_hex(&dp.text(), 4).is_some();
            primary.set_sensitive(ok);
            if ok {
                set_status(
                    &message,
                    "Leave a field unchanged to keep the analyzer's value.",
                    false,
                );
            } else {
                set_status(&message, "Bank and direct page are hexadecimal.", true);
            }
        }
    };
    valid();
    for row in [&dbr, &dp] {
        let valid = valid.clone();
        row.connect_changed(move |_| valid());
    }

    if current.is_some() {
        let clear = destructive("Clear");
        clear.set_halign(gtk::Align::Start);
        clear.connect_clicked({
            let (doc, dialog) = (Rc::clone(doc), f.dialog.clone());
            move |_| {
                let _ = doc.set_flag_override(None);
                dialog.close();
            }
        });
        f.body.append(&clear);
    }
    f.primary.connect_clicked({
        let (doc, dialog) = (Rc::clone(doc), f.dialog.clone());
        move |_| {
            let (Some(dbr), Some(dp)) = (parse_hex(&dbr.text(), 2), parse_hex(&dp.text(), 4))
            else {
                return;
            };
            let flags = FlagOverride {
                m: untri(m.selected()),
                x: untri(x.selected()),
                e: untri(e.selected()),
                dbr: dbr.map(|v| v as u8),
                dp: dp.map(|v| v as u16),
            };
            let empty = flags == FlagOverride::default();
            let _ = doc.set_flag_override((!empty).then_some(flags));
            dialog.close();
        }
    });
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_fields_distinguish_empty_from_invalid() {
        assert_eq!(parse_hex("", 2), Some(None));
        assert_eq!(parse_hex("  ", 2), Some(None));
        assert_eq!(parse_hex("$7E", 2), Some(Some(0x7E)));
        assert_eq!(parse_hex("7e", 2), Some(Some(0x7E)));
        assert_eq!(parse_hex("$0300", 4), Some(Some(0x300)));
        assert_eq!(parse_hex("$123", 2), None);
        assert_eq!(parse_hex("zz", 2), None);
    }

    #[test]
    fn the_three_state_rows_round_trip() {
        for v in [None, Some(true), Some(false)] {
            assert_eq!(untri(tri(v)), v);
        }
    }
}
