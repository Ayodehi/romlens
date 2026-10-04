//! `/lessons`, `/map` and `/progress` (docs/25, docs/28): the student's
//! lessons, to read again, the concept map shaded by how far they have got,
//! and the progress pane, as three tabs of one dialog. The macOS twin is
//! `LessonsSheet`, `LessonLibrary` and `ConceptMap`.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use romlens_ffi::tutor::quiz::QuizPurposeInfo;
use romlens_ffi::tutor::session::{ConceptInfo, LearnerInfo, LessonInfo};

use crate::gfxdraw::caption;
use crate::messageview::OnLink;
use crate::model::Document;
use crate::model::tutor::Sheet;
use crate::tutorsheets::{date, dialog};

/// A function a view calls to rebuild itself, filled in once it exists.
type Refill = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

pub fn library(
    doc: &Rc<Document>,
    sheet: Sheet,
    composer: crate::tutorsheets::Composer,
    on_link: OnLink,
) -> adw::Dialog {
    let _ = composer;
    let stack = gtk::Stack::builder()
        .hhomogeneous(true)
        .vexpand(true)
        .build();
    let d_cell: Rc<RefCell<Option<adw::Dialog>>> = Rc::default();
    stack.add_titled(&lessons(doc, &on_link), Some("lessons"), "Lessons");
    stack.add_titled(&map(doc, &d_cell), Some("map"), "What you know");
    let review: Rc<dyn Fn()> = {
        let (doc, d) = (Rc::clone(doc), Rc::clone(&d_cell));
        Rc::new(move || {
            doc.edit_tutor(|t| t.start_quiz(None, None, QuizPurposeInfo::Review));
            if let Some(d) = d.borrow().as_ref() {
                d.close();
            }
        })
    };
    stack.add_titled(
        &crate::progressview::pane(doc, review),
        Some("progress"),
        "Progress",
    );
    stack.set_visible_child_name(match sheet {
        Sheet::Map => "map",
        Sheet::Progress => "progress",
        _ => "lessons",
    });
    let switcher = gtk::StackSwitcher::builder()
        .stack(&stack)
        .halign(gtk::Align::Center)
        .build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.append(&switcher);
    body.append(&stack);
    body.set_size_request(-1, 460);
    let done = gtk::Button::with_label("Done");
    done.add_css_class("suggested-action");
    let d = dialog("Lessons", 720, &body, &[&done]);
    d.set_content_height(600);
    *d_cell.borrow_mut() = Some(d.clone());
    let dd = d.clone();
    done.connect_clicked(move |_| {
        dd.close();
    });
    d.set_default_widget(Some(&done));
    d
}

/// The lessons, newest first, and the one picked as its card.
fn lessons(doc: &Rc<Document>, on_link: &OnLink) -> gtk::Widget {
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    let all: Rc<RefCell<Vec<LessonInfo>>> = Rc::new(RefCell::new(doc.tutor().lessons()));
    let right = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_start(4)
        .margin_end(4)
        .build();
    let hint = gtk::Label::new(Some("Pick a lesson to read it again."));
    hint.add_css_class("dim-label");
    hint.set_vexpand(true);
    right.append(&hint);
    let right_scroll = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .min_content_width(320)
        .child(&right)
        .build();

    let fill = {
        let (list, all, doc) = (list.clone(), Rc::clone(&all), Rc::clone(doc));
        Rc::new(move || {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            *all.borrow_mut() = doc.tutor().lessons();
            for l in all.borrow().iter() {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&l.title))
                    .subtitle(glib::markup_escape_text(&format!(
                        "{} · {} steps · {}{}",
                        l.level_name,
                        l.steps.len(),
                        date(l.created),
                        if l.finished { "" } else { " · not ended" }
                    )))
                    .build();
                list.append(&row);
            }
        })
    };
    fill();
    {
        let (all, right, doc, on_link) = (
            Rc::clone(&all),
            right.clone(),
            Rc::clone(doc),
            Rc::clone(on_link),
        );
        list.connect_row_selected(move |_, row| {
            while let Some(c) = right.first_child() {
                right.remove(&c);
            }
            if let Some(l) = row.and_then(|r| all.borrow().get(r.index() as usize).cloned()) {
                right.append(&crate::lessonview::card(&doc, &l.id, &on_link));
            }
        });
    }
    if let Some(r) = list.row_at_index(0) {
        list.select_row(Some(&r));
    }
    // Delete: a right-click on a row.
    let menu = gio::Menu::new();
    menu.append(Some("Delete Lesson"), Some("lesson.delete"));
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(&list);
    popover.set_has_arrow(false);
    let target: Rc<RefCell<Option<String>>> = Rc::default();
    let actions = gio::SimpleActionGroup::new();
    let delete = gio::SimpleAction::new("delete", None);
    {
        let (doc, target, fill) = (Rc::clone(doc), Rc::clone(&target), Rc::clone(&fill));
        delete.connect_activate(move |_, _| {
            if let Some(id) = target.borrow_mut().take() {
                doc.edit_tutor(|t| t.delete_lesson(&id));
                fill();
            }
        });
    }
    actions.add_action(&delete);
    list.insert_action_group("lesson", Some(&actions));
    let click = gtk::GestureClick::builder().button(3).build();
    {
        let (list2, all, target) = (list.clone(), Rc::clone(&all), target);
        click.connect_pressed(move |_, _, x, y| {
            let Some(row) = list2.row_at_y(y as i32) else {
                return;
            };
            *target.borrow_mut() = all.borrow().get(row.index() as usize).map(|l| l.id.clone());
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        });
    }
    list.add_controller(click);

    let left: gtk::Widget = if all.borrow().is_empty() {
        let l = gtk::Label::builder()
            .label("No lessons yet. Turn on Explain, or ask with /learn.")
            .wrap(true)
            .justify(gtk::Justification::Center)
            .margin_start(12)
            .margin_end(12)
            .build();
        l.add_css_class("dim-label");
        l.upcast()
    } else {
        gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .build()
            .upcast()
    };
    left.set_width_request(240);
    let paned = gtk::Paned::builder()
        .start_child(&left)
        .end_child(&right_scroll)
        .position(250)
        .vexpand(true)
        .build();
    paned.upcast()
}

/// The concepts grouped, each shaded by the level reached. A concept can be
/// marked known, so lessons skip what the student already has.
fn map(doc: &Rc<Document>, dialog: &Rc<RefCell<Option<adw::Dialog>>>) -> gtk::Widget {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .margin_end(8)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&body)
        .build();
    let fill: Refill = Rc::default();
    let f: Rc<dyn Fn()> = {
        let (body, doc, dialog, fill) = (
            body.clone(),
            Rc::clone(doc),
            Rc::clone(dialog),
            Rc::clone(&fill),
        );
        Rc::new(move || {
            while let Some(c) = body.first_child() {
                body.remove(&c);
            }
            let Some(l) = doc.tutor().learner() else {
                return;
            };
            let again = fill.borrow().clone().unwrap_or_else(|| Rc::new(|| {}));
            body.append(&legend(&l));
            for group in &l.groups {
                let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
                let h = gtk::Label::builder().label(group).xalign(0.0).build();
                h.add_css_class("heading");
                b.append(&h);
                let flow = gtk::FlowBox::builder()
                    .selection_mode(gtk::SelectionMode::None)
                    .min_children_per_line(1)
                    .max_children_per_line(5)
                    .column_spacing(6)
                    .row_spacing(6)
                    .homogeneous(true)
                    .build();
                for c in l.concepts.iter().filter(|c| &c.group == group) {
                    flow.append(&chip(&doc, c, &l, &dialog, &again));
                }
                b.append(&flow);
                body.append(&b);
            }
        })
    };
    *fill.borrow_mut() = Some(Rc::clone(&f));
    f();
    scroll.upcast()
}

fn legend(l: &LearnerInfo) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    for (i, name) in l.levels.iter().enumerate() {
        let one = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let swatch = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        swatch.add_css_class("concept");
        swatch.add_css_class(&format!("concept-{}", i + 1));
        swatch.set_size_request(14, 10);
        swatch.set_valign(gtk::Align::Center);
        one.append(&swatch);
        one.append(&caption(&format!("{} {name}", i + 1)));
        row.append(&one);
    }
    row.upcast()
}

fn chip(
    doc: &Rc<Document>,
    c: &ConceptInfo,
    l: &LearnerInfo,
    dialog: &Rc<RefCell<Option<adw::Dialog>>>,
    again: &Rc<dyn Fn()>,
) -> gtk::Widget {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let due = c.due.is_some_and(|d| d <= now);
    let b = gtk::Box::new(gtk::Orientation::Vertical, 1);
    b.add_css_class("concept");
    b.add_css_class(&format!("concept-{}", c.level));
    if c.proven > 0 {
        b.add_css_class(&format!("proven-{}", c.proven));
    }
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let name = gtk::Label::builder()
        .label(&c.name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    head.append(&name);
    if due {
        let clock = gtk::Image::from_icon_name("alarm-symbolic");
        clock.set_tooltip_text(Some("Ready to review"));
        head.append(&clock);
    }
    b.append(&head);
    let level = if c.level == 0 {
        "not yet".to_owned()
    } else {
        format!(
            "{} · {}{}{}",
            c.level,
            l.levels
                .get(c.level as usize - 1)
                .map_or("", String::as_str),
            if c.marked { " (marked)" } else { "" },
            if c.proven > 0 {
                format!(" · proven {}", c.proven)
            } else {
                String::new()
            }
        )
    };
    let sub = caption(&level);
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    b.append(&sub);
    let mut tip = c.line.clone();
    if !c.needs.is_empty() {
        tip += &format!("\nRests on: {}", c.needs.join(", "));
    }
    if c.proven > 0 {
        tip += &format!("\nProven to level {} in a quiz.", c.proven);
    }
    b.set_tooltip_text(Some(&tip));

    // The context menu: quizzes, and marking what is already known.
    let menu = gio::Menu::new();
    let first = if c.proven >= 5 {
        "Practise Level 5…"
    } else {
        "Prove the Next Level…"
    };
    menu.append(Some(first), Some("concept.prove"));
    let practise = gio::Menu::new();
    let known = gio::Menu::new();
    for (n, name) in l.levels.iter().enumerate() {
        practise.append(
            Some(&format!("{} · {name}", n + 1)),
            Some(&format!("concept.practise({})", n + 1)),
        );
        known.append(
            Some(&format!("{} · {name}", n + 1)),
            Some(&format!("concept.known({})", n + 1)),
        );
    }
    menu.append_submenu(Some("Practise"), &practise);
    let marks = gio::Menu::new();
    marks.append_submenu(Some("Mark as Known"), &known);
    if c.marked {
        marks.append(Some("Clear the Mark"), Some("concept.clear"));
    }
    menu.append_section(None, &marks);
    let actions = gio::SimpleActionGroup::new();
    let start = {
        let (doc, dialog, id) = (Rc::clone(doc), Rc::clone(dialog), c.id.clone());
        Rc::new(move |level: Option<u8>, purpose: QuizPurposeInfo| {
            doc.edit_tutor(|t| t.start_quiz(Some(id.clone()), level, purpose));
            if let Some(d) = dialog.borrow().as_ref() {
                d.close();
            }
        })
    };
    let prove = gio::SimpleAction::new("prove", None);
    {
        let (start, proven) = (Rc::clone(&start), c.proven);
        prove.connect_activate(move |_, _| {
            if proven >= 5 {
                start(Some(5), QuizPurposeInfo::Practice);
            } else {
                start(None, QuizPurposeInfo::Prove);
            }
        });
    }
    actions.add_action(&prove);
    let practise_action = gio::SimpleAction::new("practise", Some(glib::VariantTy::INT32));
    practise_action.connect_activate(move |_, v| {
        if let Some(n) = v.and_then(|v| v.get::<i32>()) {
            start(Some(n as u8), QuizPurposeInfo::Practice);
        }
    });
    actions.add_action(&practise_action);
    let mark = |level: Option<u8>| {
        let (doc, id, again) = (Rc::clone(doc), c.id.clone(), Rc::clone(again));
        move || {
            doc.edit_tutor(|t| t.mark_known(&id, level));
            again();
        }
    };
    let known_action = gio::SimpleAction::new("known", Some(glib::VariantTy::INT32));
    {
        let (doc, id, again) = (Rc::clone(doc), c.id.clone(), Rc::clone(again));
        known_action.connect_activate(move |_, v| {
            if let Some(n) = v.and_then(|v| v.get::<i32>()) {
                doc.edit_tutor(|t| t.mark_known(&id, Some(n as u8)));
                again();
            }
        });
    }
    actions.add_action(&known_action);
    let clear = gio::SimpleAction::new("clear", None);
    {
        let f = mark(None);
        clear.connect_activate(move |_, _| f());
    }
    actions.add_action(&clear);
    b.insert_action_group("concept", Some(&actions));
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(&b);
    popover.set_has_arrow(false);
    let click = gtk::GestureClick::builder().button(3).build();
    click.connect_pressed(move |_, _, x, y| {
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    b.add_controller(click);
    b.upcast()
}
