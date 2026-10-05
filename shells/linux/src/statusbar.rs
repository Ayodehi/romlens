//! The status bar under the tab groups (docs/29): the whole ROM as a slim
//! strip to click, and what the analyzer made of it. The swatches double as
//! the strip's legend, so the numbers and the picture are one thing rather
//! than two. It was a band over the editor, which put it above every view,
//! the sound views included. Focus on Code hides it. The macOS twin is
//! `WorkspaceStatusBar`.

use std::rc::Rc;

use gtk::prelude::*;

use crate::model::session::AnalysisState;
use crate::model::{Change, Document};
use crate::{palette, stripview};

pub fn build(doc: &Rc<Document>) -> gtk::Box {
    let band = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_start(12)
        .margin_end(12)
        .margin_top(4)
        .margin_bottom(4)
        .build();

    let strip = stripview::build(doc);
    strip.set_height_request(10);
    strip.set_tooltip_text(Some("The whole ROM, one column per pixel. Click to jump."));
    // Rounded like the macOS strip; clipped by the frame.
    let frame = gtk::Frame::new(None);
    frame.set_child(Some(&strip));
    frame.add_css_class("romlens-strip-frame");
    band.append(&frame);

    let status = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(14)
        .height_request(16)
        .build();
    status.add_css_class("caption");
    status.add_css_class("numeric");
    band.append(&status);

    let bar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bar.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    bar.append(&band);

    let refresh = {
        let (doc, status, frame, bar) =
            (Rc::clone(doc), status.clone(), frame.clone(), bar.clone());
        move || {
            frame.set_visible(doc.panes().strip);
            bar.set_visible(!doc.is_focused());
            rebuild_status(&status, &doc);
        }
    };
    refresh();
    doc.subscribe(move |c| {
        if matches!(c, Change::Status | Change::Layout) {
            refresh();
        }
    });
    bar
}

fn rebuild_status(status: &gtk::Box, doc: &Rc<Document>) {
    while let Some(child) = status.first_child() {
        status.remove(&child);
    }
    let session = &doc.session;
    let spacer = || gtk::Box::builder().hexpand(true).build();
    let link = |label: &str, action: &str| {
        let b = gtk::Button::builder()
            .label(label)
            .action_name(action)
            .build();
        b.add_css_class("flat");
        b.add_css_class("link");
        b
    };
    match (session.analysis(), session.stats()) {
        (AnalysisState::Running { .. }, Some(stats)) => {
            // A rerun (every second in a live session) keeps showing the last
            // numbers rather than swapping them for a bar and back.
            append_stats(status, &stats);
            let spinner = gtk::Spinner::builder().spinning(true).build();
            spinner.set_tooltip_text(Some("Analyzing again"));
            status.append(&spinner);
        }
        (AnalysisState::Running { fraction, phase }, None) => {
            let bar = gtk::ProgressBar::builder()
                .fraction(fraction)
                .valign(gtk::Align::Center)
                .width_request(120)
                .build();
            status.append(&bar);
            status.append(&gtk::Label::new(Some(phase)));
            status.append(&spacer());
            status.append(&link("Cancel", "win.cancel-analysis"));
        }
        (AnalysisState::Failed(message), _) => {
            let label = gtk::Label::new(Some("Analysis failed"));
            label.add_css_class("warning");
            label.set_tooltip_text(Some(&message));
            status.append(&label);
            status.append(&spacer());
            status.append(&link("Retry", "win.analyze"));
        }
        (AnalysisState::Idle, Some(stats)) => {
            append_stats(status, &stats);
            let again = gtk::Button::builder()
                .icon_name("view-refresh-symbolic")
                .action_name("win.analyze")
                .tooltip_text("Analyze again")
                .build();
            again.add_css_class("flat");
            again.add_css_class("circular");
            status.append(&again);
        }
        (AnalysisState::Idle, None) => {
            let label = gtk::Label::new(Some("Not analyzed"));
            label.add_css_class("dim-label");
            status.append(&label);
            status.append(&spacer());
            status.append(&link("Analyze", "win.analyze"));
        }
    }
}

fn append_stats(status: &gtk::Box, stats: &romlens_ffi::AnalysisStats) {
    let total = (stats.code_bytes + stats.data_bytes + stats.unknown_bytes).max(1) as f64;
    for (code, name, bytes) in [
        (1u8, "code", stats.code_bytes),
        (2, "data", stats.data_bytes),
        (0, "unknown", stats.unknown_bytes),
    ] {
        status.append(&swatch(code, name, bytes as f64 / total));
    }
    status.append(&gtk::Box::builder().hexpand(true).build());
    let regions = gtk::Label::new(Some(&format!("{} regions", stats.regions)));
    regions.add_css_class("dim-label");
    status.append(&regions);
}

/// A swatch in the strip's own colour, with the kind's name and share.
fn swatch(code: u8, name: &str, fraction: f64) -> gtk::Box {
    let row = gtk::Box::builder().spacing(5).build();
    let dot = gtk::DrawingArea::builder()
        .content_width(9)
        .content_height(9)
        .valign(gtk::Align::Center)
        .build();
    dot.set_draw_func(move |a, cr, w, h| {
        let accent = adw::StyleManager::default().accent_color_rgba();
        let fg = a.color();
        let c = palette::strip_color(code, &accent, &fg);
        crate::canvas::set_source(cr, &c);
        cr.rectangle(0.0, 0.0, f64::from(w), f64::from(h));
        let _ = cr.fill();
        if code == 0 {
            crate::canvas::set_source(cr, &crate::canvas::with_alpha(&fg, 0.3));
            cr.rectangle(0.5, 0.5, f64::from(w) - 1.0, f64::from(h) - 1.0);
            let _ = cr.stroke();
        }
    });
    row.append(&dot);
    row.append(&gtk::Label::new(Some(name)));
    row.append(&gtk::Label::new(Some(&format!("{:.1}%", fraction * 100.0))));
    row.set_tooltip_text(Some(&format!(
        "{name}: {:.1}% of the image",
        fraction * 100.0
    )));
    row
}
