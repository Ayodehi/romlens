//! What an instruction does to the hardware, field by field, and the idioms
//! it is part of (docs/20). The macOS twin is `ExplanationSection`,
//! `RegisterAccessView` and `IdiomView`.

use std::rc::Rc;

use gtk::prelude::*;
use romlens_ffi::{
    ExplanationInfo, IdiomInfo, IdiomTableInfo, RegisterAccessInfo, RegisterPartInfo,
};

use super::ui::*;
use crate::model::Document;

pub fn section(doc: &Rc<Document>, x: &ExplanationInfo, selected_insn: Option<u32>) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
    b.append(&heading("Explanation"));
    if let Some(r) = &x.register {
        b.append(&register_access(r));
        if let Some((port, value)) = sound_command(r) {
            let play = gtk::Button::with_label("Play This Command");
            play.set_halign(gtk::Align::Start);
            play.set_tooltip_text(Some(&format!(
                "Boot the ROM's sound driver in Romlens and send it ${value:02X} on port {port}, as this store does"
            )));
            let doc = Rc::clone(doc);
            play.connect_clicked(move |_| {
                doc.open_audio(crate::model::audio::Tab::Voices);
                doc.play_command(port, value);
            });
            b.append(&play);
        }
    }
    for idiom in &x.idioms {
        b.append(&idiom_view(doc, idiom, selected_insn));
    }
    b.upcast()
}

/// A store to a sound port ($2140 to $2143) of a known value: the port and the
/// byte, to play as the game would send it.
pub fn sound_command(r: &RegisterAccessInfo) -> Option<(u8, u8)> {
    let address = r.parts.first()?.address;
    if !r.store || !(0x2140..=0x2143).contains(&address) {
        return None;
    }
    Some(((address - 0x2140) as u8, (r.value? & 0xFF) as u8))
}

/// Whether an explanation has anything to show.
pub fn has_content(x: &ExplanationInfo) -> bool {
    x.register.is_some() || !x.idioms.is_empty()
}

fn register_access(a: &RegisterAccessInfo) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    for part in &a.parts {
        b.append(&register_part(part));
    }
    let footer = if let Some(source) = a.source {
        Some(format!(
            "The value is loaded from {}.",
            a.source_name
                .clone()
                .unwrap_or_else(|| romlens_ffi::format_snes_address(source))
        ))
    } else if a.indexed {
        Some("Indexed: which register depends on X or Y.".to_owned())
    } else if !a.store {
        Some("A read: the fields show what it reports.".to_owned())
    } else if a.value.is_none() {
        Some("The value is worked out at run time.".to_owned())
    } else {
        None
    };
    if let Some(f) = footer {
        b.append(&note(&f));
    }
    b.upcast()
}

pub fn register_part(part: &RegisterPartInfo) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let short = mono(&part.short);
    short.add_css_class("heading");
    b.append(&short);
    let about = gtk::Label::new(Some(&part.about));
    about.add_css_class("dim-label");
    about.set_xalign(0.0);
    about.set_wrap(true);
    b.append(&about);
    if part.twice {
        b.append(&note("Written twice in a row: low byte, then high."));
    }
    if !part.fields.is_empty() {
        let known = part.value.is_some();
        let g = gtk::Grid::builder()
            .column_spacing(10)
            .row_spacing(3)
            .build();
        let mut head = vec!["Bits", "Field"];
        if known {
            head.extend(["Value", "Meaning"]);
        }
        for (c, h) in head.iter().enumerate() {
            let l = note(h);
            g.attach(&l, c as i32, 0, 1, 1);
        }
        for (r, f) in part.fields.iter().enumerate() {
            let row = r as i32 + 1;
            let bits = mono(&f.bits);
            bits.add_css_class("dim-label");
            g.attach(&bits, 0, row, 1, 1);
            let name = gtk::Label::new(Some(&f.name));
            name.set_xalign(0.0);
            g.attach(&name, 1, row, 1, 1);
            if known {
                g.attach(
                    &mono(&f.raw.map_or(String::new(), |v| v.to_string())),
                    2,
                    row,
                    1,
                    1,
                );
                let m = gtk::Label::new(f.meaning.as_deref());
                m.set_xalign(0.0);
                m.set_wrap(true);
                g.attach(&m, 3, row, 1, 1);
            }
        }
        b.append(&g);
    }
    b.upcast()
}

fn idiom_view(doc: &Rc<Document>, idiom: &IdiomInfo, selected_insn: Option<u32>) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.add_css_class("romlens-idiom");
    let title = gtk::Label::new(Some(&format!("💡 {}", idiom.title)));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    b.append(&title);
    b.append(&body(&idiom.summary));
    if let Some(t) = &idiom.table {
        b.append(&idiom_table(doc, t, selected_insn));
    }
    let why = gtk::Expander::new(Some("Why games do this"));
    why.set_expanded(true);
    let text = gtk::Label::new(Some(&idiom.why));
    text.add_css_class("dim-label");
    text.set_xalign(0.0);
    text.set_wrap(true);
    why.set_child(Some(&text));
    b.append(&why);
    let select = small_button("Select Its Instructions");
    select.set_halign(gtk::Align::Start);
    let (d, note_at) = (Rc::clone(doc), idiom.note_at);
    select.connect_clicked(move |_| d.select_idiom(note_at));
    b.append(&select);
    b.upcast()
}

/// An idiom's details as rows; the row with the selected instruction is
/// marked, and clicking a row selects its instructions.
fn idiom_table(doc: &Rc<Document>, t: &IdiomTableInfo, selected: Option<u32>) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let g = gtk::Grid::builder()
        .column_spacing(14)
        .row_spacing(2)
        .build();
    for (c, h) in t.columns.iter().enumerate() {
        g.attach(&note(h), c as i32, 0, 1, 1);
    }
    for (r, row) in t.rows.iter().enumerate() {
        let here = selected.is_some_and(|s| row.offsets.contains(&s));
        for (c, cell) in row.cells.iter().enumerate() {
            let l = gtk::Label::new(Some(cell));
            l.set_xalign(0.0);
            if c > 0 {
                l.add_css_class("monospace");
            }
            if here {
                l.add_css_class("heading");
            }
            g.attach(&l, c as i32, r as i32 + 1, 1, 1);
        }
        // A click on any cell of the row selects its instructions.
        for c in 0..row.cells.len() {
            if let Some(w) = g.child_at(c as i32, r as i32 + 1) {
                let click = gtk::GestureClick::new();
                let (d, offsets) = (Rc::clone(doc), row.offsets.clone());
                click.connect_pressed(move |_, _, _, _| select_row(&d, &offsets));
                w.add_controller(click);
            }
        }
    }
    b.append(&g);
    if let Some(n) = &t.note {
        b.append(&note(n));
    }
    b.upcast()
}

fn select_row(doc: &Document, offsets: &[u32]) {
    let (Some(first), Some(last)) = (offsets.iter().min(), offsets.iter().max()) else {
        return;
    };
    let len = doc
        .workbench()
        .instruction_at(*last)
        .map_or(1, |i| u32::from(i.len));
    doc.select_range(*first..*last + len);
    doc.request_scroll(*first);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access(address: u32, store: bool, value: Option<u32>) -> RegisterAccessInfo {
        let rom = romlens_ffi::Rom::from_bytes(romlens_ffi::make_sound_test_rom(), "s.sfc".into())
            .unwrap();
        let wb = romlens_ffi::Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        // Any real register access, with the fields we care about replaced.
        let mut found = (0..0x8000u32)
            .filter_map(|o| wb.explain_at(o).register)
            .next()
            .expect("the fixture writes a register");
        found.store = store;
        found.value = value;
        found.parts[0].address = address;
        found
    }

    #[test]
    fn only_a_store_of_a_known_value_to_a_sound_port_can_be_played() {
        assert_eq!(
            sound_command(&access(0x2140, true, Some(0x1234))),
            Some((0, 0x34))
        );
        assert_eq!(
            sound_command(&access(0x2143, true, Some(0x05))),
            Some((3, 0x05))
        );
        assert_eq!(
            sound_command(&access(0x2144, true, Some(1))),
            None,
            "past the ports"
        );
        assert_eq!(sound_command(&access(0x213F, true, Some(1))), None);
        assert_eq!(
            sound_command(&access(0x2141, false, Some(1))),
            None,
            "a read"
        );
        assert_eq!(
            sound_command(&access(0x2141, true, None)),
            None,
            "value unknown until run time"
        );
    }
}
