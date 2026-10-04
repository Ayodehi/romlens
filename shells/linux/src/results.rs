//! The results pane under the editor: the last Find or the last Find
//! References, each keeping its own list so switching loses neither. The
//! macOS twin is `SearchResultsView` and `ReferencesView`.

use std::rc::Rc;

use gtk::prelude::*;
use romlens_ffi::SearchHit;

use crate::lists::{ValueList, child_at, row_box, spacer};
use crate::model::references::Row as RefRow;
use crate::model::{Change, Document, ResultsKind};

pub fn build(doc: &Rc<Document>) -> gtk::Box {
    let pane = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .height_request(200)
        .build();
    pane.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let title = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .margin_start(12)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    title.add_css_class("heading");
    let summary = gtk::Label::new(None);
    summary.add_css_class("caption");
    summary.add_css_class("dim-label");
    let close = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .action_name("win.toggle-results")
        .tooltip_text("Hide results (Ctrl+F reopens)")
        .build();
    close.add_css_class("flat");
    let head = gtk::Box::builder()
        .spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    head.append(&title);
    head.append(&summary);
    head.append(&close);
    pane.append(&head);
    pane.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let find = find_list(doc);
    let refs = reference_list(doc);
    let empty = gtk::Label::builder()
        .vexpand(true)
        .valign(gtk::Align::Center)
        .justify(gtk::Justification::Center)
        .wrap(true)
        .build();
    empty.add_css_class("dim-label");
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.add_named(&find.widget, Some("find"));
    stack.add_named(&refs.widget, Some("refs"));
    stack.add_named(&empty, Some("empty"));
    pane.append(&stack);

    let refresh = {
        let (doc, pane) = (Rc::clone(doc), pane.clone());
        move || {
            pane.set_visible(doc.panes().results);
            match doc.results_kind() {
                ResultsKind::Find => {
                    let s = doc.search();
                    title.set_text(&if s.searched.is_empty() {
                        "Find".to_owned()
                    } else {
                        format!("“{}”", s.searched)
                    });
                    summary.set_text(&s.summary());
                    if s.hits.is_empty() {
                        empty.set_text(&s.error.clone().unwrap_or_else(|| {
                            if s.searched.is_empty() {
                                "Search with Ctrl+F.".to_owned()
                            } else {
                                "No matches".to_owned()
                            }
                        }));
                        stack.set_visible_child_name("empty");
                    } else {
                        find.set(s.hits.clone());
                        find.set_current(s.current.map(|c| c as u32));
                        stack.set_visible_child_name("find");
                    }
                }
                ResultsKind::References => {
                    let r = doc.references();
                    title.set_text(&format!("References to {}", r.target_name));
                    summary.set_text(&r.summary());
                    if r.rows.is_empty() {
                        empty.set_text(&format!(
                            "Nothing the analyzer found refers to {}.",
                            r.target_name
                        ));
                        stack.set_visible_child_name("empty");
                    } else {
                        refs.set(r.rows.clone());
                        refs.set_current(r.current.map(|c| c as u32));
                        stack.set_visible_child_name("refs");
                    }
                }
            }
        }
    };
    refresh();
    doc.subscribe(move |c| {
        if matches!(c, Change::Layout | Change::Results | Change::AddressStyle) {
            refresh();
        }
    });
    pane
}

fn mono(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("monospace");
    l.set_xalign(0.0);
    l
}

fn dim(text: &str) -> gtk::Label {
    let l = mono(text);
    l.add_css_class("dim-label");
    l
}

fn set_text(w: &gtk::Widget, text: &str) {
    if let Some(l) = w.downcast_ref::<gtk::Label>() {
        l.set_text(text);
    }
}

/// The context bytes as Pango markup, with the matched run bold in the accent
/// colour: a hex pattern's hits are indistinguishable without their context.
pub fn context_markup(hit: &SearchHit, accent: &str) -> String {
    let match_end = hit.match_start + hit.len;
    let mut out = String::new();
    for (i, byte) in hit.context.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let in_match = (hit.match_start..match_end).contains(&(i as u32));
        if in_match {
            out.push_str(&format!(
                "<span weight=\"bold\" foreground=\"{accent}\">{byte:02X}</span>"
            ));
        } else {
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

fn accent_hex() -> String {
    let c = adw::StyleManager::default().accent_color_rgba();
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c.red()), b(c.green()), b(c.blue()))
}

fn find_list(doc: &Rc<Document>) -> ValueList<SearchHit> {
    let d = Rc::clone(doc);
    ValueList::new(
        true,
        || {
            let r = row_box();
            r.append(&dim(""));
            r.append(&dim(""));
            let context = mono("");
            context.set_use_markup(true);
            r.append(&context);
            r.append(&spacer());
            let kind = gtk::Label::new(None);
            kind.add_css_class("caption");
            kind.add_css_class("dim-label");
            r.append(&kind);
            r.upcast()
        },
        |row: &gtk::Widget, hit: &SearchHit| {
            set_text(
                &child_at(row, 0),
                &romlens_ffi::format_file_offset(hit.file_offset),
            );
            set_text(
                &child_at(row, 1),
                hit.snes_address.as_deref().unwrap_or("--:----"),
            );
            if let Some(l) = child_at(row, 2).downcast_ref::<gtk::Label>() {
                l.set_markup(&context_markup(hit, &accent_hex()));
            }
            set_text(&child_at(row, 4), &hit.region_kind);
        },
        move |i, _| d.go_to_hit(i as usize),
    )
}

fn reference_list(doc: &Rc<Document>) -> ValueList<RefRow> {
    let (d, style) = (Rc::clone(doc), Rc::clone(doc));
    ValueList::new(
        true,
        || {
            let r = row_box();
            r.append(&dim(""));
            r.append(&dim(""));
            let routine = mono("");
            routine.set_width_chars(18);
            routine.set_ellipsize(gtk::pango::EllipsizeMode::End);
            r.append(&routine);
            let text = mono("");
            text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            r.append(&text);
            r.append(&spacer());
            let kind = gtk::Label::new(None);
            kind.add_css_class("caption");
            kind.add_css_class("dim-label");
            r.append(&kind);
            r.upcast()
        },
        move |row: &gtk::Widget, x: &RefRow| {
            use crate::hex::AddressStyle;
            let style = style.address_style();
            let (offset, snes) = (child_at(row, 0), child_at(row, 1));
            set_text(&offset, &romlens_ffi::format_file_offset(x.file_offset));
            offset.set_visible(style != AddressStyle::Snes);
            set_text(
                &snes,
                &x.snes_address
                    .map_or("--:----".into(), romlens_ffi::format_snes_address),
            );
            snes.set_visible(style != AddressStyle::File);
            set_text(&child_at(row, 2), x.routine.as_deref().unwrap_or(""));
            set_text(&child_at(row, 3), &x.text);
            let kind = if x.observed {
                format!("{}, seen", x.kind_name)
            } else if x.certain {
                x.kind_name.clone()
            } else {
                format!("{}?", x.kind_name)
            };
            let tail = child_at(row, 5);
            set_text(&tail, &kind);
            tail.set_tooltip_text(if x.observed {
                Some("An execution log saw the game do this")
            } else if x.certain {
                None
            } else {
                Some("The analyzer is not sure of this reference")
            });
        },
        move |i, _| d.go_to_reference(i as usize),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_matched_bytes_are_marked_in_the_context() {
        let hit = SearchHit {
            file_offset: 0,
            snes_address: None,
            len: 2,
            context: vec![0x00, 0x78, 0x18, 0xFB],
            match_start: 1,
            region_kind: String::new(),
        };
        assert_eq!(
            context_markup(&hit, "#3584e4"),
            "00 <span weight=\"bold\" foreground=\"#3584e4\">78</span> \
             <span weight=\"bold\" foreground=\"#3584e4\">18</span> FB"
        );
    }
}
