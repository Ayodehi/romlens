//! `/progress` (docs/28): what the student has proven across the map, the
//! reviews due, and, unless hidden in Settings, points, the rank, the streak
//! and achievements, with this game's milestones. The macOS twin is
//! `ProgressPane`.

use std::rc::Rc;

use adw::prelude::*;
use romlens_ffi::tutor::quiz::{AchievementInfo, ProgressInfo};

use crate::gfxdraw::caption;
use crate::model::Document;
use crate::tutorsheets::date;

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).build();
    l.add_css_class("heading");
    l
}

fn column() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_end(8)
        .margin_top(4)
        .margin_bottom(4)
        .build()
}

/// The pane, for a tab of the lessons dialog. `start_review` begins a quiz on
/// what is due.
pub fn pane(doc: &Rc<Document>, start_review: Rc<dyn Fn()>) -> gtk::Widget {
    // Milestones may have been reached since the last look.
    let session = doc.tutor().session().cloned();
    if let Some(s) = session {
        let _ = s.check_milestones();
    }
    doc.edit_tutor_quiet(|t| t.refresh_progress());
    let Some(p) = doc.tutor().progress.clone() else {
        let l = gtk::Label::new(Some("No record of your learning here yet."));
        l.add_css_class("dim-label");
        return l.upcast();
    };
    let show = crate::settings::tutor().borrow().stored().show_progress;
    let col = column();
    if show {
        col.append(&score(&p));
    }
    col.append(&groups(&p));
    col.append(&due(&p, start_review));
    if show {
        col.append(&achievements(
            &p.achievements
                .iter()
                .filter(|a| !a.game)
                .cloned()
                .collect::<Vec<_>>(),
            "Achievements".into(),
        ));
        let game = p
            .achievements
            .iter()
            .find(|a| a.game)
            .and_then(|a| a.rom_title.clone())
            .unwrap_or_else(|| "this game".into());
        let mine: Vec<AchievementInfo> =
            p.achievements.iter().filter(|a| a.game).cloned().collect();
        if !mine.is_empty() {
            col.append(&achievements(&mine, format!("Found in {game}")));
        }
        col.append(&recent(&p));
    } else {
        col.append(&caption(
            "Points and achievements are hidden (Settings, Tutor).",
        ));
    }
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&col)
        .build()
        .upcast()
}

fn score(p: &ProgressInfo) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let xp = gtk::Label::builder().label(p.xp.to_string()).build();
    xp.add_css_class("title-1");
    xp.add_css_class("numeric");
    row.append(&xp);
    let pts = gtk::Label::new(Some("points"));
    pts.add_css_class("dim-label");
    pts.set_valign(gtk::Align::End);
    row.append(&pts);
    row.append(&gtk::Box::builder().hexpand(true).build());
    let right = gtk::Box::new(gtk::Orientation::Vertical, 1);
    let rank = gtk::Label::builder().label(&p.rank).xalign(1.0).build();
    rank.add_css_class("title-3");
    right.append(&rank);
    if p.streak > 0 {
        let best = if p.best_streak > p.streak {
            format!(" · best {}", p.best_streak)
        } else {
            String::new()
        };
        let s = caption(&format!("{}-day streak{best}", p.streak));
        s.set_xalign(1.0);
        right.append(&s);
    }
    row.append(&right);
    b.append(&row);
    let bar = gtk::ProgressBar::new();
    bar.set_fraction(f64::from(p.corpus) / f64::from(p.corpus_max.max(1)));
    b.append(&bar);
    b.append(&caption(&format!(
        "{} of {} levels proven across the map",
        p.corpus, p.corpus_max
    )));
    if let (Some(next), Some(needs)) = (&p.next_rank, &p.next_needs) {
        let mut chars = needs.chars();
        let lower = chars
            .next()
            .map_or_else(String::new, |c| c.to_lowercase().chain(chars).collect());
        let l = caption(&format!("Next rank, {next}: {lower}."));
        l.set_wrap(true);
        b.append(&l);
    }
    b.upcast()
}

fn groups(p: &ProgressInfo) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.append(&heading("What you have proven"));
    let grid = gtk::Grid::builder()
        .column_spacing(14)
        .row_spacing(4)
        .build();
    for (i, g) in p.groups.iter().enumerate() {
        let i = i as i32;
        grid.attach(
            &gtk::Label::builder().label(&g.name).xalign(0.0).build(),
            0,
            i,
            1,
            1,
        );
        for (c, text) in [
            (1, format!("{} of {} proven", g.proven, g.concepts)),
            (2, format!("{} learned", g.learned)),
        ] {
            let l = gtk::Label::builder().label(text).xalign(0.0).build();
            l.add_css_class("dim-label");
            grid.attach(&l, c, i, 1, 1);
        }
        if g.level > 0 {
            let l = gtk::Label::builder()
                .label(format!("all to level {}", g.level))
                .xalign(0.0)
                .build();
            l.add_css_class("accent");
            grid.attach(&l, 3, i, 1, 1);
        }
    }
    b.append(&grid);
    b.upcast()
}

fn due(p: &ProgressInfo, start_review: Rc<dyn Fn()>) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let h = heading("Ready to review");
    h.set_hexpand(true);
    head.append(&h);
    if !p.due.is_empty() {
        let go = gtk::Button::with_label("Review now");
        go.connect_clicked(move |_| start_review());
        head.append(&go);
    }
    b.append(&head);
    if p.due.is_empty() {
        let l = caption("Nothing yet: what you prove comes back after a day, then less often.");
        l.set_wrap(true);
        b.append(&l);
    } else {
        for d in &p.due {
            b.append(
                &gtk::Label::builder()
                    .label(format!("{}, level {}", d.concept_name, d.level))
                    .xalign(0.0)
                    .build(),
            );
        }
    }
    b.upcast()
}

fn achievements(list: &[AchievementInfo], title: String) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let earned = list.iter().filter(|a| a.unlocked.is_some()).count();
    b.append(&heading(&format!("{title}  {earned} of {}", list.len())));
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(1)
        .max_children_per_line(3)
        .column_spacing(8)
        .row_spacing(8)
        .homogeneous(true)
        .build();
    for a in list {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let icon = gtk::Image::from_icon_name(if a.unlocked.is_some() {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        });
        icon.set_valign(gtk::Align::Start);
        if a.unlocked.is_some() {
            icon.add_css_class("warning");
        }
        row.append(&icon);
        let col = gtk::Box::new(gtk::Orientation::Vertical, 1);
        let t = gtk::Label::builder()
            .label(&a.title)
            .xalign(0.0)
            .wrap(true)
            .build();
        t.add_css_class("heading");
        col.append(&t);
        let d = caption(
            &a.unlocked
                .map_or_else(|| a.detail.clone(), |u| format!("Earned {}", date(u))),
        );
        d.set_wrap(true);
        col.append(&d);
        row.append(&col);
        row.set_opacity(if a.unlocked.is_some() { 1.0 } else { 0.6 });
        flow.append(&row);
    }
    b.append(&flow);
    b.upcast()
}

fn recent(p: &ProgressInfo) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    if p.recent.is_empty() {
        return b.upcast();
    }
    b.append(&heading("Latest points"));
    for l in p.recent.iter().take(8) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let n = caption(&format!("+{}", l.points));
        n.set_width_chars(5);
        n.set_xalign(1.0);
        n.add_css_class("numeric");
        row.append(&n);
        row.append(&caption(&l.why));
        b.append(&row);
    }
    b.upcast()
}
