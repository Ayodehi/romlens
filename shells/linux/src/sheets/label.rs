//! `n` names the selected address; `;` comments it.

use std::rc::Rc;

use adw::prelude::*;
use romlens_ffi::{CommentKind, LabelSource};

use super::{Frame, caption, destructive, frame, mono_entry, set_status, status};
use crate::model::Document;

const NAME_HINT: &str = "Letters, digits and underscores; up to 64 characters.";

/// The core's reason a name is not acceptable, or `None` when it is (or empty).
pub fn validation(name: &str, address: Option<u32>) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    romlens_ffi::workbench::validate_label_name(name.to_owned(), address)
        .err()
        .map(|e| e.to_string())
}

pub fn rename(doc: &Rc<Document>) -> Frame {
    let f = frame("Rename Label", 420, "Rename");
    let address = doc.selected_address();
    if let Some(a) = address {
        f.body
            .append(&caption(&romlens_ffi::format_snes_address(a)));
    }
    let details = doc.details();
    let auto = details
        .label
        .as_ref()
        .filter(|l| l.source == LabelSource::Auto);
    let entry = mono_entry(auto.map_or("Label", |l| l.name.as_str()));
    if let Some(l) = details
        .label
        .as_ref()
        .filter(|l| l.source == LabelSource::User)
    {
        entry.set_text(&l.name);
    }
    let message = status();
    set_status(&message, NAME_HINT, false);
    f.body.append(&entry);
    f.body.append(&message);

    let update = {
        let (message, primary) = (message.clone(), f.primary.clone());
        move |e: &gtk::Entry| {
            let text = e.text();
            let problem = validation(&text, address);
            primary.set_sensitive(problem.is_none() && !text.trim().is_empty());
            match problem {
                Some(p) => set_status(&message, &p, true),
                None => set_status(&message, NAME_HINT, false),
            }
        }
    };
    update(&entry);
    entry.connect_changed(update);

    if doc.can_remove_label() {
        let remove = destructive("Remove");
        remove.set_halign(gtk::Align::Start);
        remove.connect_clicked({
            let (doc, dialog) = (Rc::clone(doc), f.dialog.clone());
            move |_| {
                let _ = doc.remove_label_or_variable();
                dialog.close();
            }
        });
        f.body.append(&remove);
    }
    f.primary.connect_clicked({
        let (doc, entry, dialog, message) =
            (Rc::clone(doc), entry.clone(), f.dialog.clone(), message);
        move |_| match doc.set_label(Some(entry.text().trim().to_owned())) {
            Ok(()) => {
                dialog.close();
            }
            Err(e) => set_status(&message, &e.to_string(), true),
        }
    });
    f.dialog.set_focus(Some(&entry));
    f
}

pub fn comment(doc: &Rc<Document>) -> Frame {
    let f = frame("Comment", 460, "Save");
    if let Some(a) = doc.selected_address() {
        f.body
            .append(&caption(&romlens_ffi::format_snes_address(a)));
    }
    let details = doc.details();
    let existing = {
        let (line, block) = (details.line_comment.clone(), details.block_comment.clone());
        move |k: CommentKind| {
            match k {
                CommentKind::Line => &line,
                CommentKind::Block => &block,
            }
            .as_ref()
            .map(|c| c.text.clone())
            .unwrap_or_default()
        }
    };

    let line_button = gtk::ToggleButton::with_label("Line");
    let block_button = gtk::ToggleButton::with_label("Block");
    block_button.set_group(Some(&line_button));
    line_button.set_active(true);
    let kinds = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    kinds.add_css_class("linked");
    kinds.set_halign(gtk::Align::Start);
    kinds.append(&line_button);
    kinds.append(&block_button);
    f.body.append(&kinds);

    let line = mono_entry("Comment");
    line.remove_css_class("monospace");
    let block = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(6)
        .bottom_margin(6)
        .left_margin(8)
        .right_margin(8)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(100)
        .child(&block)
        .build();
    scroll.add_css_class("card");
    let stack = gtk::Stack::new();
    stack.add_named(&line, Some("line"));
    stack.add_named(&scroll, Some("block"));
    f.body.append(&stack);

    let remove = destructive("Remove");
    remove.set_halign(gtk::Align::Start);
    f.body.append(&remove);

    let load = {
        let (line, block, stack, remove, existing) = (
            line.clone(),
            block.clone(),
            stack.clone(),
            remove.clone(),
            existing.clone(),
        );
        move |kind: CommentKind| {
            let text = existing(kind);
            match kind {
                CommentKind::Line => {
                    line.set_text(&text);
                    stack.set_visible_child_name("line");
                    line.grab_focus();
                }
                CommentKind::Block => {
                    block.buffer().set_text(&text);
                    stack.set_visible_child_name("block");
                    block.grab_focus();
                }
            }
            remove.set_visible(!text.is_empty());
        }
    };
    load(CommentKind::Line);
    line_button.connect_toggled({
        let load = load.clone();
        move |b| {
            if b.is_active() {
                load(CommentKind::Line);
            }
        }
    });
    block_button.connect_toggled(move |b| {
        if b.is_active() {
            load(CommentKind::Block);
        }
    });

    let current = {
        let (line_button, line, block) = (line_button.clone(), line.clone(), block.clone());
        move || {
            if line_button.is_active() {
                (CommentKind::Line, line.text().to_string())
            } else {
                let buf = block.buffer();
                (
                    CommentKind::Block,
                    buf.text(&buf.start_iter(), &buf.end_iter(), false)
                        .to_string(),
                )
            }
        }
    };
    f.primary.connect_clicked({
        let (doc, dialog, current) = (Rc::clone(doc), f.dialog.clone(), current.clone());
        move |_| {
            let (kind, text) = current();
            let text = text.trim().to_owned();
            let _ = doc.set_comment(kind, (!text.is_empty()).then_some(text));
            dialog.close();
        }
    });
    remove.connect_clicked({
        let (doc, dialog) = (Rc::clone(doc), f.dialog.clone());
        move |_| {
            let (kind, _) = current();
            let _ = doc.set_comment(kind, None);
            dialog.close();
        }
    });
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_core_decides_what_a_label_may_be() {
        assert_eq!(validation("", None), None);
        assert_eq!(validation("   ", None), None);
        assert_eq!(validation("Boot", Some(0x80_8000)), None);
        assert!(validation("1bad", None).is_some());
        assert!(validation("has space", None).is_some());
        assert!(validation(&"x".repeat(65), None).is_some());
    }
}
