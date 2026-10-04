//! The marked range's preview options (checklist 2.25): where the palette is,
//! how many tiles across, and for a tilemap its size and where its tiles are.
//! Addresses are typed the way the jump sheet takes them and resolved by the
//! core; an empty field is the default. The macOS twin is
//! `PreviewOptionsForm`.

use std::rc::Rc;

use adw::prelude::*;
use romlens_ffi::RegionParamsInfo;

use super::{Frame, caption, frame, mono_entry, set_status, status};
use crate::model::Document;
use crate::model::graphics::{SCREEN_SIZES, screen_title};

pub fn build(doc: &Rc<Document>) -> Frame {
    let f = frame("Preview Options", 380, "Set");
    let tilemap = doc
        .details()
        .preview
        .as_ref()
        .is_some_and(|p| p.kind == "tilemap");
    let current = doc.preview_options().unwrap_or_default();

    let palette = mono_entry("grayscale");
    if let Some(p) = current.palette {
        palette.set_text(&romlens_ffi::format_snes_address(p));
    }
    f.body.append(&caption("Palette at"));
    f.body.append(&palette);

    let tiles = mono_entry("none");
    let size = gtk::DropDown::from_strings(&SCREEN_SIZES.map(screen_title));
    let columns = gtk::SpinButton::with_range(1.0, 64.0, 1.0);
    columns.set_value(f64::from(current.columns.unwrap_or(16)));
    if tilemap {
        if let Some(t) = current.tiles {
            tiles.set_text(&romlens_ffi::format_snes_address(t));
        }
        f.body.append(&caption("Tiles at"));
        f.body.append(&tiles);
        f.body.append(&caption("Size"));
        size.set_selected(
            SCREEN_SIZES
                .iter()
                .position(|s| Some(*s) == current.screen_size)
                .unwrap_or(0) as u32,
        );
        f.body.append(&size);
    } else {
        f.body.append(&caption("Tiles across"));
        f.body.append(&columns);
    }
    let message = status();
    f.body.append(&message);

    f.primary.connect_clicked({
        let (doc, dialog) = (Rc::clone(doc), f.dialog.clone());
        move |_| {
            let result = (|| {
                let palette = doc.snes_address_of(&palette.text())?;
                let params = if tilemap {
                    RegionParamsInfo {
                        palette,
                        columns: None,
                        screen_size: Some(SCREEN_SIZES[size.selected() as usize]),
                        tiles: doc.snes_address_of(&tiles.text())?,
                    }
                } else {
                    RegionParamsInfo {
                        palette,
                        columns: Some(columns.value() as u16),
                        screen_size: None,
                        tiles: None,
                    }
                };
                doc.set_preview_options(params)
            })();
            match result {
                Ok(()) => {
                    dialog.close();
                }
                Err(e) => set_status(&message, &e.to_string(), true),
            }
        }
    });
    f
}
