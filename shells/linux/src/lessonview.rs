//! A lesson (docs/25), one step at a time: its title and level, the step
//! ("3 of 8"), a predict question before the step's answer, Back and Next,
//! and at the end the lessons that could come next. Each step moves the main
//! window to what it is about. The macOS twin is `LessonCard`.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use romlens_ffi::tutor::session::{LessonCheckedInfo, LessonInfo, LessonStepInfo};

use crate::gfxdraw::caption;
use crate::messageview::{self, OnLink};
use crate::model::{Change, Document};

/// The card for lesson `id`, which keeps itself current as the lesson is
/// written, checked and stepped through.
pub fn card(doc: &Rc<Document>, id: &str, on_link: &OnLink) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("card");
    outer.add_css_class("lesson-card");
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    outer.append(&body);
    fill(doc, id, &body, on_link);
    let (weak, id, on_link) = (body.downgrade(), id.to_owned(), Rc::clone(on_link));
    let weak_doc = Rc::downgrade(doc);
    doc.subscribe(move |c| {
        if c != Change::Tutor {
            return;
        }
        if let (Some(body), Some(doc)) = (weak.upgrade(), weak_doc.upgrade()) {
            // Not while a guess is being typed: the field keeps its text.
            if body.root().is_some() {
                fill(&doc, &id, &body, &on_link);
            }
        }
    });
    outer.upcast()
}

fn fill(doc: &Rc<Document>, id: &str, body: &gtk::Box, on_link: &OnLink) {
    let lesson = doc.edit_tutor_quiet(|t| t.lesson(id));
    let key = lesson.as_ref().map(|l| signature(doc, l));
    // The same card as before is left alone, so a half-typed guess stays.
    if let Some(k) = &key
        && unsafe {
            body.data::<String>("signature")
                .is_some_and(|s| s.as_ref() == k)
        }
    {
        return;
    }
    if let Some(k) = key {
        unsafe { body.set_data("signature", k) };
    }
    while let Some(c) = body.first_child() {
        body.remove(&c);
    }
    let Some(lesson) = lesson else {
        let l = gtk::Label::builder()
            .label("A lesson that is no longer kept")
            .xalign(0.0)
            .build();
        l.add_css_class("dim-label");
        body.append(&l);
        return;
    };
    let count = lesson.steps.len();
    let at = doc.tutor().step_of(&lesson.id).min(count.saturating_sub(1));

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = gtk::Image::from_icon_name("accessories-dictionary-symbolic");
    icon.add_css_class("accent");
    head.append(&icon);
    let title = gtk::Label::builder()
        .label(&lesson.title)
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .build();
    title.add_css_class("heading");
    head.append(&title);
    let level = gtk::Label::new(Some(&lesson.level_name));
    level.add_css_class("caption");
    level.add_css_class("pill");
    level.set_tooltip_text(Some(&format!(
        "How deep the lesson goes: {}",
        lesson
            .concepts
            .iter()
            .map(|c| format!("{} (level {})", c.name, c.level))
            .collect::<Vec<_>>()
            .join(", ")
    )));
    head.append(&level);
    body.append(&head);

    if count == 0 {
        let l = gtk::Label::builder()
            .label(if lesson.finished {
                "This lesson has no steps."
            } else {
                "The tutor is writing the first step…"
            })
            .xalign(0.0)
            .build();
        l.add_css_class("dim-label");
        body.append(&l);
    } else {
        body.append(&step(doc, &lesson, at, on_link));
        body.append(&controls(doc, &lesson, at));
    }
    if lesson.finished && at + 1 == count && !lesson.next.is_empty() {
        let next = gtk::Box::new(gtk::Orientation::Vertical, 4);
        next.append(&caption("Where next"));
        for offer in &lesson.next {
            let b = gtk::Button::with_label(&offer.title);
            b.add_css_class("flat");
            b.set_halign(gtk::Align::Start);
            b.set_sensitive(!doc.tutor().busy);
            b.set_tooltip_text(Some(&format!("A lesson at level {}", offer.level)));
            let (doc, offer) = (Rc::clone(doc), offer.clone());
            b.connect_clicked(move |_| {
                if let Some(text) = doc.edit_tutor(|t| t.take_offer(&offer)) {
                    doc.tutor_send(&text);
                }
            });
            next.append(&b);
        }
        body.append(&next);
    }
    if lesson.checking {
        let l = caption("Checking…");
        l.set_tooltip_text(Some(
            "The tutor is checking this lesson against the ROM, and will correct any step that is wrong.",
        ));
        body.append(&l);
    } else if let Some(c) = &lesson.checked {
        let l = caption(&checked(c));
        l.set_tooltip_text(Some("Checked against the ROM after it was written."));
        body.append(&l);
    }
    if !lesson.this_rom {
        body.append(&caption(
            "Made with another ROM: its addresses are that game's.",
        ));
    }
}

/// What the card shows, so an unchanged card is not rebuilt.
fn signature(doc: &Rc<Document>, l: &LessonInfo) -> String {
    let t = doc.tutor();
    let at = t.step_of(&l.id);
    let step = l.steps.get(at.min(l.steps.len().saturating_sub(1)));
    format!(
        "{}|{}|{}|{at}|{}|{}|{}|{:?}|{:?}|{}|{}",
        l.title,
        l.steps.len(),
        l.finished,
        l.checking,
        l.checked.as_ref().map_or(0, |c| c.changed),
        step.is_some_and(|s| t.is_revealed(&l.id, at) || s.guessed.is_some()),
        step.map(|s| (&s.guessed, s.guess_right, s.guess_credit)),
        step.map(|s| (&s.guess_note, s.guess_marking)),
        step.map_or(0, |s| s.body.len() + s.title.len()),
        t.busy,
    )
}

/// "Checked · $0.03", or with the steps it corrected.
pub fn checked(c: &LessonCheckedInfo) -> String {
    let cost = format!("${:.2}", c.cost);
    match c.changed {
        0 => format!("Checked · {cost}"),
        1 => format!("Checked · 1 step corrected · {cost}"),
        n => format!("Checked · {n} steps corrected · {cost}"),
    }
}

fn step(doc: &Rc<Document>, lesson: &LessonInfo, i: usize, on_link: &OnLink) -> gtk::Widget {
    let s = &lesson.steps[i];
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let title = gtk::Label::builder()
        .label(&s.title)
        .xalign(0.0)
        .wrap(true)
        .build();
    title.add_css_class("heading");
    b.append(&title);
    let revealed = doc.tutor().is_revealed(&lesson.id, i);
    match &s.predict {
        Some(q) if !revealed && s.guessed.is_none() => {
            b.append(&guess(doc, &lesson.id, i, q, s.checks_guess))
        }
        predict => {
            if let Some(q) = predict {
                let l = gtk::Label::builder()
                    .label(format!("<i>{}</i>", glib::markup_escape_text(q)))
                    .use_markup(true)
                    .xalign(0.0)
                    .wrap(true)
                    .build();
                l.add_css_class("dim-label");
                b.append(&l);
                if let Some(g) = &s.guessed {
                    b.append(&verdict(s, g));
                }
            }
            b.append(&messageview::message(doc, &s.body, on_link));
            if let Some(p) = &s.picture {
                b.append(&picture(doc, &lesson.id, p));
            }
        }
    }
    if let (Some(f), Some(words)) = (&s.focus, &s.focus_text) {
        let button = gtk::Button::with_label(&format!("Show {words} in Romlens"));
        button.add_css_class("flat");
        button.set_halign(gtk::Align::Start);
        let (doc, f) = (Rc::clone(doc), f.clone());
        button.connect_clicked(move |_| {
            messageview::follow(&doc, &f);
        });
        b.append(&button);
    }
    b.upcast()
}

/// A predict question, and a guess before Show (docs/28): the first guess at a
/// step earns points, and where the step says its answer, Romlens checks it.
fn guess(
    doc: &Rc<Document>,
    lesson: &str,
    step: usize,
    question: &str,
    checks: bool,
) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let q = gtk::Label::builder()
        .label(format!("<i>{}</i>", glib::markup_escape_text(question)))
        .use_markup(true)
        .xalign(0.0)
        .wrap(true)
        .build();
    b.append(&q);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let entry = gtk::Entry::builder()
        .placeholder_text(if checks {
            "Your guess (Romlens checks it)"
        } else {
            "Your guess"
        })
        .hexpand(true)
        .build();
    let check = gtk::Button::with_label("Check");
    check.set_sensitive(false);
    let show = gtk::Button::with_label("Show");
    row.append(&entry);
    row.append(&check);
    row.append(&show);
    b.append(&row);
    b.append(&caption("Guess first, or just show it."));
    let go = {
        let (doc, lesson, entry) = (Rc::clone(doc), lesson.to_owned(), entry.clone());
        move || {
            let g = entry.text().trim().to_owned();
            if !g.is_empty() {
                doc.edit_tutor(|t| t.answer_predict(&lesson, step, &g));
            }
        }
    };
    entry.connect_changed({
        let check = check.clone();
        move |e| check.set_sensitive(!e.text().trim().is_empty())
    });
    entry.connect_activate({
        let go = go.clone();
        move |_| go()
    });
    check.connect_clicked(move |_| go());
    let (doc, lesson) = (Rc::clone(doc), lesson.to_owned());
    show.connect_clicked(move |_| doc.edit_tutor(|t| t.reveal(&lesson, step)));
    b.upcast()
}

/// What became of a guess: Romlens's check, the tutor's mark and its line, the
/// mark on its way, or, with neither, the answer to compare it with.
fn verdict(s: &LessonStepInfo, guess: &str) -> gtk::Widget {
    let (word, icon, class) = match (s.guess_right, s.guess_credit) {
        (Some(true), _) => (": right", "emblem-ok-symbolic", Some("success")),
        (Some(false), _) => (": not quite", "dialog-warning-symbolic", Some("warning")),
        (None, Some(c)) if (c - 0.5).abs() < f32::EPSILON => (
            ": partly right",
            "dialog-question-symbolic",
            Some("warning"),
        ),
        _ => ("", "user-available-symbolic", None),
    };
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&gtk::Image::from_icon_name(icon));
    let l = gtk::Label::builder()
        .label(format!("You guessed “{guess}”{word}"))
        .xalign(0.0)
        .wrap(true)
        .build();
    l.add_css_class("caption");
    if let Some(c) = class {
        l.add_css_class(c);
    }
    row.append(&l);
    b.append(&row);
    let note = if s.guess_marking {
        Some("The tutor is marking your guess…".to_owned())
    } else if let Some(n) = &s.guess_note {
        Some(n.clone())
    } else if s.guess_right.is_none() {
        Some("Compare it with the answer below.".to_owned())
    } else {
        None
    };
    if let Some(n) = note {
        b.append(&caption(&n));
    }
    b.upcast()
}

fn picture(doc: &Rc<Document>, lesson: &str, id: &str) -> gtk::Widget {
    let data = doc
        .tutor()
        .session()
        .and_then(|s| s.lesson_picture(lesson.to_owned(), id.to_owned()));
    let Some(tex) = data.and_then(|d| gdk::Texture::from_bytes(&glib::Bytes::from_owned(d)).ok())
    else {
        return gtk::Box::new(gtk::Orientation::Vertical, 0).upcast();
    };
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let pic = gtk::Picture::for_paintable(&tex);
    pic.set_content_fit(gtk::ContentFit::ScaleDown);
    pic.set_halign(gtk::Align::Start);
    pic.set_height_request(280.min(tex.height()));
    b.append(&pic);
    b.upcast()
}

fn controls(doc: &Rc<Document>, lesson: &LessonInfo, at: usize) -> gtk::Widget {
    let count = lesson.steps.len();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let back = gtk::Button::with_label("Back");
    back.set_icon_name("go-previous-symbolic");
    back.set_sensitive(at > 0);
    let next = gtk::Button::with_label("Next");
    next.set_sensitive(at + 1 < count);
    let count_label = caption(&format!(
        "{} of {count}{}",
        at + 1,
        if lesson.finished { "" } else { "…" }
    ));
    count_label.add_css_class("numeric");
    for (b, to) in [(&back, at.wrapping_sub(1)), (&next, at + 1)] {
        let (doc, lesson) = (Rc::clone(doc), lesson.clone());
        b.connect_clicked(move |_| go(&doc, &lesson, to));
    }
    row.append(&back);
    row.append(&count_label);
    row.append(&next);
    let spacer = gtk::Box::builder().hexpand(true).build();
    row.append(&spacer);
    if at + 1 < count {
        let skip = gtk::Button::with_label("Skip to the end");
        skip.add_css_class("flat");
        skip.add_css_class("caption");
        let (doc, lesson) = (Rc::clone(doc), lesson.clone());
        skip.connect_clicked(move |_| go(&doc, &lesson, count - 1));
        row.append(&skip);
    }
    row.upcast()
}

/// Moves the card to step `to`, and the window's tabs to what it is about.
fn go(doc: &Rc<Document>, lesson: &LessonInfo, to: usize) {
    if to >= lesson.steps.len() {
        return;
    }
    if let Some(focus) = doc.edit_tutor(|t| t.show_step(lesson, to)) {
        messageview::follow(doc, &focus);
    }
}
