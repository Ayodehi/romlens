//! A quiz (docs/28): one question at a time, each answered and then shown
//! right or not with what it teaches, and at the end what it proved. The
//! macOS twin is `QuizSheet`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use romlens_ffi::tutor::quiz::{
    AnswerResultInfo, GivenInfo, QuestionInfo, QuestionKindInfo, QuizInfo, QuizPurposeInfo,
};

use crate::gfxdraw::caption;
use crate::messageview::{self, OnLink};
use crate::model::tutor::address;
use crate::model::{Change, Document};
use crate::tutorsheets::dialog;

struct Quiz {
    doc: Rc<Document>,
    on_link: OnLink,
    body: gtk::Box,
    dialog: RefCell<Option<adw::Dialog>>,
    /// The question shown.
    at: Cell<usize>,
    /// What the body was built from, so a half-typed answer survives events
    /// that change nothing it shows.
    shown: RefCell<String>,
}

pub fn quiz(doc: &Rc<Document>, on_link: OnLink) -> adw::Dialog {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(480)
        .child(&body)
        .build();
    let close = gtk::Button::with_label("Close");
    let d = dialog("Quiz", 580, &scroll, &[&close]);
    d.set_content_height(620);
    let q = Rc::new(Quiz {
        doc: Rc::clone(doc),
        on_link,
        body,
        dialog: RefCell::new(Some(d.clone())),
        at: Cell::new(0),
        shown: RefCell::default(),
    });
    // Pick up where the student left off.
    if let Some(quiz) = doc.tutor().quiz.clone() {
        q.at.set(
            quiz.questions
                .iter()
                .position(|x| !quiz.results.iter().any(|r| r.question == x.id))
                .unwrap_or(quiz.questions.len()),
        );
    }
    q.render();
    let weak = Rc::downgrade(&q);
    doc.subscribe(move |c| {
        if c == Change::Tutor
            && let Some(q) = weak.upgrade()
            && q.dialog.borrow().is_some()
        {
            q.render();
        }
    });
    {
        let d = d.clone();
        close.connect_clicked(move |_| {
            d.close();
        });
    }
    let (doc, cell) = (Rc::clone(doc), Rc::clone(&q));
    d.connect_closed(move |_| {
        doc.edit_tutor(|t| t.finish_quiz());
        cell.dialog.borrow_mut().take();
    });
    d
}

fn purpose(p: QuizPurposeInfo) -> &'static str {
    match p {
        QuizPurposeInfo::Prove => "Proving",
        QuizPurposeInfo::Review => "Reviewing",
        QuizPurposeInfo::Practice => "Practising",
    }
}

impl Quiz {
    fn render(self: &Rc<Self>) {
        let (quiz, error) = {
            let t = self.doc.tutor();
            (t.quiz.clone(), t.quiz_error.clone())
        };
        let Some(quiz) = quiz else { return };
        let key = format!(
            "{}|{}|{}|{}|{:?}|{}|{error:?}",
            quiz.id,
            self.at.get(),
            quiz.questions.len(),
            quiz.writing,
            quiz.results
                .iter()
                .map(|r| (
                    r.question.as_str(),
                    r.credit.map(f32::to_bits),
                    r.feedback.is_some()
                ))
                .collect::<Vec<_>>(),
            quiz.finished,
        );
        if *self.shown.borrow() == key {
            return;
        }
        *self.shown.borrow_mut() = key;
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
        let head = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let title = gtk::Label::builder()
            .label(format!(
                "{} {}, level {}",
                purpose(quiz.purpose),
                quiz.concept_name,
                quiz.level
            ))
            .xalign(0.0)
            .wrap(true)
            .build();
        title.add_css_class("title-3");
        head.append(&title);
        let level = gtk::Label::builder()
            .label(&quiz.level_name)
            .xalign(0.0)
            .build();
        level.add_css_class("dim-label");
        head.append(&level);
        self.body.append(&head);
        self.body.append(&self.dots(&quiz));
        self.body
            .append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        if let Some(question) = quiz.questions.get(self.at.get()) {
            self.body.append(&self.card(
                question,
                quiz.results.iter().find(|r| r.question == question.id),
            ));
        } else {
            self.body.append(&self.outcome(&quiz));
        }
        if let Some(e) = error {
            let l = gtk::Label::builder()
                .label(e)
                .xalign(0.0)
                .wrap(true)
                .build();
            l.add_css_class("error");
            self.body.append(&l);
        }
        if quiz.writing {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.append(&adw::Spinner::new());
            row.append(&caption("The tutor is writing questions about the game…"));
            self.body.append(&row);
        }
    }

    /// One dot a question: right, half, wrong, or not yet; the one shown ringed.
    fn dots(self: &Rc<Self>, quiz: &QuizInfo) -> gtk::Widget {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for (i, q) in quiz.questions.iter().enumerate() {
            let r = quiz.results.iter().find(|r| r.question == q.id);
            let area = gtk::DrawingArea::builder()
                .content_width(18)
                .content_height(18)
                .build();
            let (shown, colour) = (i == self.at.get(), dot_colour(r));
            area.set_draw_func(move |_, cr, w, h| {
                let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
                cr.set_source_rgba(colour.0, colour.1, colour.2, colour.3);
                cr.arc(cx, cy, 6.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
                if shown {
                    cr.set_source_rgba(0.5, 0.5, 0.5, 0.9);
                    cr.set_line_width(1.5);
                    cr.arc(cx, cy, 8.5, 0.0, std::f64::consts::TAU);
                    let _ = cr.stroke();
                }
            });
            area.set_tooltip_text(Some(&q.prompt));
            if r.is_some() {
                let click = gtk::GestureClick::new();
                let weak = Rc::downgrade(self);
                click.connect_pressed(move |_, _, _, _| {
                    if let Some(me) = weak.upgrade() {
                        me.at.set(i);
                        me.render();
                    }
                });
                area.add_controller(click);
            }
            row.append(&area);
        }
        row.upcast()
    }

    /// A question: its answer control, the hint, and once answered, the result.
    fn card(
        self: &Rc<Self>,
        question: &QuestionInfo,
        result: Option<&AnswerResultInfo>,
    ) -> gtk::Widget {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let prompt = messageview::message(&self.doc, &question.prompt, &self.on_link);
        prompt.add_css_class("title-4");
        b.append(&prompt);
        let state = Rc::new(Answer::default());
        let control = answer_control(question, &state);
        control.set_sensitive(result.is_none());
        b.append(&control);
        let hint = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .visible(false)
            .build();
        hint.add_css_class("dim-label");
        b.append(&hint);

        let go: Rc<dyn Fn()> = {
            let (me, id, state, q) = (
                Rc::clone(self),
                question.id.clone(),
                Rc::clone(&state),
                question.clone(),
            );
            Rc::new(move || {
                if state.ready(&q.kind) {
                    me.doc
                        .edit_tutor(|t| t.answer_question(&id, state.given(&q.kind)));
                    me.render();
                }
            })
        };
        if let Some(r) = result {
            b.append(&self.result(r));
            let next = gtk::Button::with_label("Next");
            next.add_css_class("suggested-action");
            next.set_halign(gtk::Align::Start);
            let me = Rc::clone(self);
            next.connect_clicked(move |_| {
                me.at.set(me.at.get() + 1);
                if me.at.get()
                    >= me
                        .doc
                        .tutor()
                        .quiz
                        .as_ref()
                        .map_or(0, |q| q.questions.len())
                {
                    me.doc.edit_tutor(|t| t.finish_quiz());
                }
                me.render();
            });
            b.append(&next);
        } else {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let check = gtk::Button::with_label("Check");
            check.add_css_class("suggested-action");
            check.set_sensitive(false);
            *state.on_change.borrow_mut() = Some(Box::new({
                let (check, state, kind) =
                    (check.clone(), Rc::clone(&state), question.kind.clone());
                move || check.set_sensitive(state.ready(&kind))
            }));
            {
                let go = Rc::clone(&go);
                check.connect_clicked(move |_| go());
            }
            *state.on_enter.borrow_mut() = Some(go);
            let skip = gtk::Button::with_label("Skip");
            {
                let (me, id) = (Rc::clone(self), question.id.clone());
                skip.connect_clicked(move |_| {
                    me.doc
                        .edit_tutor(|t| t.answer_question(&id, GivenInfo::Skipped));
                    me.render();
                });
            }
            row.append(&check);
            row.append(&skip);
            if question.has_hint {
                let hb = gtk::Button::with_label("Hint");
                hb.set_tooltip_text(Some("A hint halves what the answer earns"));
                let (me, id, hint) = (Rc::clone(self), question.id.clone(), hint.clone());
                hb.connect_clicked(move |b| {
                    if let Some(h) = me.doc.edit_tutor_quiet(|t| t.question_hint(&id)) {
                        hint.set_text(&h);
                        hint.set_visible(true);
                        b.set_visible(false);
                    }
                });
                row.append(&hb);
            }
            b.append(&row);
        }
        b.append(&caption(&question.source));
        b.upcast()
    }

    /// Right or not, the right answer, and what it teaches.
    fn result(&self, r: &AnswerResultInfo) -> gtk::Widget {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
        b.add_css_class("quiz-result");
        let (icon, text, class) = match r.credit {
            None => (
                "content-loading-symbolic",
                "The tutor is marking your answer…".to_owned(),
                None,
            ),
            Some(c) if c >= 1.0 => ("emblem-ok-symbolic", "Right".to_owned(), Some("success")),
            Some(c) if c > 0.0 => (
                "emblem-ok-symbolic",
                if r.hinted {
                    "Right, with the hint"
                } else {
                    "Partly right"
                }
                .to_owned(),
                Some("warning"),
            ),
            Some(_) => (
                "window-close-symbolic",
                format!("Not quite: {}", r.right_answer),
                Some("error"),
            ),
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&gtk::Image::from_icon_name(icon));
        let l = gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .wrap(true)
            .build();
        if let Some(c) = class {
            l.add_css_class(c);
        }
        row.append(&l);
        b.append(&row);
        if let Some(f) = &r.feedback {
            let l = gtk::Label::builder()
                .label(format!("<i>{}</i>", glib::markup_escape_text(f)))
                .use_markup(true)
                .xalign(0.0)
                .wrap(true)
                .build();
            b.append(&l);
        }
        b.append(&messageview::message(
            &self.doc,
            &r.explanation,
            &self.on_link,
        ));
        for link in r.cite.iter().filter(|c| c.starts_with("romlens://")) {
            let show = gtk::Button::with_label("Show it in Romlens");
            show.add_css_class("flat");
            show.set_halign(gtk::Align::Start);
            let (doc, link) = (Rc::clone(&self.doc), link.clone());
            show.connect_clicked(move |_| {
                messageview::follow(&doc, &link);
            });
            b.append(&show);
        }
        b.upcast()
    }

    /// What the quiz came to.
    fn outcome(self: &Rc<Self>, quiz: &QuizInfo) -> gtk::Widget {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let o = &quiz.outcome;
        let score = gtk::Label::builder()
            .label(format!("{} of {}", o.score, o.questions))
            .xalign(0.0)
            .build();
        score.add_css_class("title-1");
        b.append(&score);
        let line = |text: &str, class: Option<&str>| {
            let l = gtk::Label::builder()
                .label(text)
                .xalign(0.0)
                .wrap(true)
                .build();
            if let Some(c) = class {
                l.add_css_class(c);
            }
            l
        };
        if !o.done {
            b.append(&line(
                "Some answers are still being marked.",
                Some("dim-label"),
            ));
        } else if o.passed {
            b.append(&line(
                &format!("Proven: {}, level {}", quiz.concept_name, quiz.level),
                Some("success"),
            ));
            b.append(&line(
                "It comes back for a short review tomorrow, then less and less often.",
                Some("dim-label"),
            ));
        } else if quiz.purpose == QuizPurposeInfo::Prove {
            b.append(&line(
                "Not proven yet: a proof takes 4 of 5, with 3 of Romlens's questions right without a hint.",
                None,
            ));
            let again = gtk::Button::with_label("Try again with new questions");
            again.set_halign(gtk::Align::Start);
            let (me, concept, level) = (Rc::clone(self), quiz.concept.clone(), quiz.level);
            again.connect_clicked(move |_| {
                me.at.set(0);
                me.doc.edit_tutor(|t| {
                    t.start_quiz(Some(concept.clone()), Some(level), QuizPurposeInfo::Prove)
                });
                me.render();
            });
            b.append(&again);
        } else if quiz.purpose == QuizPurposeInfo::Review {
            b.append(&line(
                "Reviewed. What you missed comes back sooner.",
                Some("dim-label"),
            ));
        } else {
            b.append(&line(
                "Practice: it proves nothing, but every right answer counts.",
                Some("dim-label"),
            ));
        }
        b.upcast()
    }
}

fn dot_colour(r: Option<&AnswerResultInfo>) -> (f64, f64, f64, f64) {
    match r.map(|r| r.credit) {
        None => (0.5, 0.5, 0.5, 0.25),
        Some(None) => (0.5, 0.5, 0.5, 0.6),
        Some(Some(c)) if c >= 1.0 => (0.2, 0.7, 0.35, 1.0),
        Some(Some(c)) if c > 0.0 => (0.95, 0.6, 0.1, 1.0),
        Some(Some(_)) => (0.85, 0.2, 0.2, 0.85),
    }
}

/// What is chosen or typed for the question shown.
#[derive(Default)]
struct Answer {
    choice: Cell<Option<u32>>,
    text: RefCell<String>,
    bits: RefCell<std::collections::BTreeSet<u8>>,
    on_change: RefCell<Option<Box<dyn Fn()>>>,
    on_enter: RefCell<Option<Rc<dyn Fn()>>>,
}

impl Answer {
    fn changed(&self) {
        if let Some(f) = self.on_change.borrow().as_ref() {
            f();
        }
    }

    fn ready(&self, kind: &QuestionKindInfo) -> bool {
        match kind {
            QuestionKindInfo::Choice { .. } | QuestionKindInfo::Line { .. } => {
                self.choice.get().is_some()
            }
            QuestionKindInfo::Number { .. } | QuestionKindInfo::Text => {
                !self.text.borrow().trim().is_empty()
            }
            QuestionKindInfo::Bits { .. } => !self.bits.borrow().is_empty(),
        }
    }

    fn given(&self, kind: &QuestionKindInfo) -> GivenInfo {
        match kind {
            QuestionKindInfo::Choice { .. } => GivenInfo::Choice {
                index: self.choice.get().unwrap_or(0),
            },
            QuestionKindInfo::Line { .. } => GivenInfo::Line {
                index: self.choice.get().unwrap_or(0),
            },
            QuestionKindInfo::Number { .. } => GivenInfo::Number {
                text: self.text.borrow().clone(),
            },
            QuestionKindInfo::Bits { .. } => GivenInfo::Bits {
                bits: self.bits.borrow().iter().copied().collect(),
            },
            QuestionKindInfo::Text => GivenInfo::Text {
                text: self.text.borrow().clone(),
            },
        }
    }
}

fn answer_control(q: &QuestionInfo, a: &Rc<Answer>) -> gtk::Widget {
    match &q.kind {
        QuestionKindInfo::Choice { choices } => {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let first = gtk::CheckButton::new();
            for (i, c) in choices.iter().enumerate() {
                let r = gtk::CheckButton::new();
                r.set_child(Some(
                    &gtk::Label::builder()
                        .label(c)
                        .xalign(0.0)
                        .wrap(true)
                        .build(),
                ));
                if i > 0 {
                    r.set_group(Some(&first));
                } else {
                    // The first button is the group's anchor.
                    first.set_child(r.child().as_ref());
                }
                let a = Rc::clone(a);
                let btn = if i == 0 { first.clone() } else { r.clone() };
                btn.connect_toggled(move |b| {
                    if b.is_active() {
                        a.choice.set(Some(i as u32));
                        a.changed();
                    }
                });
                b.append(&btn);
            }
            b.upcast()
        }
        QuestionKindInfo::Number { hex } => {
            let e = gtk::Entry::builder()
                .placeholder_text(if *hex {
                    "A hex number, such as $2100"
                } else {
                    "A number"
                })
                .width_chars(24)
                .halign(gtk::Align::Start)
                .build();
            e.add_css_class("monospace");
            let a2 = Rc::clone(a);
            e.connect_changed(move |e| {
                *a2.text.borrow_mut() = e.text().to_string();
                a2.changed();
            });
            let a2 = Rc::clone(a);
            e.connect_activate(move |_| {
                let f = a2.on_enter.borrow().clone();
                if let Some(f) = f {
                    f();
                }
            });
            e.upcast()
        }
        QuestionKindInfo::Bits {
            width,
            fields,
            register,
        } => {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 3);
            for bit in (0..*width).rev() {
                let t = gtk::ToggleButton::with_label(&bit.to_string());
                t.add_css_class("quiz-bit");
                t.add_css_class("monospace");
                let a = Rc::clone(a);
                t.connect_toggled(move |t| {
                    if t.is_active() {
                        a.bits.borrow_mut().insert(bit);
                    } else {
                        a.bits.borrow_mut().remove(&bit);
                    }
                    a.changed();
                });
                row.append(&t);
            }
            b.append(&row);
            if !fields.is_empty() {
                let f: Vec<String> = fields
                    .iter()
                    .map(|f| {
                        if f.lo == f.hi {
                            format!("{}: bit {}", f.name, f.lo)
                        } else {
                            format!("{}: bits {}-{}", f.name, f.lo, f.hi)
                        }
                    })
                    .collect();
                b.append(&caption(&format!("{register}: {}", f.join(", "))));
            }
            b.upcast()
        }
        QuestionKindInfo::Line { lines } => {
            let list = gtk::ListBox::new();
            list.add_css_class("boxed-list");
            for l in lines {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                row.set_margin_start(6);
                row.set_margin_top(2);
                row.set_margin_bottom(2);
                let ad = gtk::Label::new(Some(&address(l.address)));
                ad.add_css_class("dim-label");
                row.append(&ad);
                row.append(&gtk::Label::new(Some(&l.text)));
                row.add_css_class("monospace");
                list.append(&row);
            }
            let a = Rc::clone(a);
            list.connect_row_selected(move |_, r| {
                a.choice.set(r.map(|r| r.index() as u32));
                a.changed();
            });
            list.upcast()
        }
        QuestionKindInfo::Text => {
            let tv = gtk::TextView::builder()
                .wrap_mode(gtk::WrapMode::WordChar)
                .left_margin(6)
                .right_margin(6)
                .top_margin(6)
                .bottom_margin(6)
                .build();
            let a = Rc::clone(a);
            tv.buffer().connect_changed(move |b| {
                *a.text.borrow_mut() = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
                a.changed();
            });
            gtk::ScrolledWindow::builder()
                .min_content_height(70)
                .max_content_height(110)
                .child(&tv)
                .build()
                .upcast()
        }
    }
}
