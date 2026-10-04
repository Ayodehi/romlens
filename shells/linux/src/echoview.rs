//! Echo, noise and pitch modulation (docs/23, A13): the echo's buffer in audio
//! RAM, its delay and feedback, the FIR filter's eight taps and what they do
//! to each frequency. The macOS twin is `EchoView`, `TapBars` and
//! `FirResponse`.

use std::rc::Rc;

use adw::prelude::*;
use gtk::cairo;
use romlens_ffi::DspRegisterInfo;

use crate::audioview::{dollar, heading_label};
use crate::canvas::with_alpha;
use crate::gfxdraw::*;
use crate::inspector::explain::register_part;
use crate::model::audio::{Tab, fir_response};
use crate::model::{Change, Document};

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&body)
        .build();
    let refresh = {
        let (doc, body) = (Rc::clone(doc), body);
        move || {
            if doc.audio_tab() != Some(Tab::Echo) {
                return;
            }
            let Some(state) = doc.audio().state() else {
                return;
            };
            let regs = state.registers;
            while let Some(c) = body.first_child() {
                body.remove(&c);
            }
            fill(&body, &regs);
        }
    };
    refresh();
    doc.subscribe(move |c| {
        if matches!(c, Change::Audio | Change::Graphics | Change::Layout) {
            refresh();
        }
    });
    scroll.upcast()
}

fn card(title: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.add_css_class("voice-strip");
    b.append(&heading_label(title));
    b
}

fn wrapped(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build()
}

/// A register with its value in words and, opened, field by field.
fn register_row(r: &DspRegisterInfo) -> gtk::Widget {
    let head = gtk::Label::builder()
        .label(format!("{}  {}", dollar(r.register, 2), r.short))
        .xalign(0.0)
        .wrap(true)
        .build();
    head.add_css_class("monospace");
    head.add_css_class("caption");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    for p in &r.parts {
        body.append(&register_part(p));
    }
    gtk::Expander::builder()
        .label_widget(&head)
        .child(&body)
        .build()
        .upcast()
}

fn fill(body: &gtk::Box, regs: &[DspRegisterInfo]) {
    let r = |i: usize| regs.get(i);
    let v = |i: usize| r(i).map_or(0u8, |r| r.value);
    let mut taps = [0i8; 8];
    for (k, t) in taps.iter_mut().enumerate() {
        *t = v(k << 4 | 0xF) as i8;
    }
    let edl = usize::from(v(0x7D) & 0xF);
    let esa = usize::from(v(0x6D)) << 8;

    let echo = card("The echo");
    echo.append(&wrapped(&format!(
        "A ring buffer in audio RAM at {}, {} bytes: {} ms of delay (EDL × 16 ms). Each sample, \
         the voices EON names are added in, and what comes back out is fed through the FIR \
         filter, mixed in at EVOL and fed back at EFB.",
        dollar(esa as u64, 4),
        if edl == 0 { 4 } else { edl * 2048 },
        edl * 16
    )));
    if v(0x6C) & 0x20 != 0 {
        let w = wrapped(
            "FLG bit 5 is set: echo writes are off, so the buffer is not written (the driver has \
             its RAM for other things, or the echo is being set up).",
        );
        w.add_css_class("warning");
        w.add_css_class("caption");
        echo.append(&w);
    }
    for i in [0x4D, 0x0D, 0x2C, 0x3C, 0x6D, 0x7D] {
        if let Some(reg) = r(i) {
            echo.append(&register_row(reg));
        }
    }
    body.append(&echo);

    let fir = card("The FIR filter");
    fir.append(&wrapped(
        "Eight taps, each a signed fraction of 128, weight the last eight echo samples: FIR0 the \
         oldest, FIR7 the newest. Their sum shapes the echo's sound: taps that add up smoothly \
         keep the low frequencies and dull the high, as a room does.",
    ));
    let charts = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    let bars = gtk::DrawingArea::builder()
        .content_width(220)
        .content_height(120)
        .build();
    bars.set_tooltip_text(Some("Each tap, FIR0 (oldest) to FIR7 (newest)"));
    bars.set_draw_func(move |a, cr, w, h| draw_taps(a, cr, f64::from(w), f64::from(h), &taps));
    let response = gtk::DrawingArea::builder()
        .content_height(120)
        .hexpand(true)
        .build();
    response.set_tooltip_text(Some(
        "How loud the echo comes back at each frequency, from 0 to 16 kHz: the line at 0 dB is unchanged",
    ));
    let points = fir_response(&taps, 128);
    response.set_draw_func(move |a, cr, w, h| {
        draw_response(a, cr, f64::from(w), f64::from(h), &points)
    });
    charts.append(&bars);
    charts.append(&response);
    fir.append(&charts);
    let list: Vec<String> = taps
        .iter()
        .enumerate()
        .map(|(i, t)| format!("FIR{i} {t}"))
        .collect();
    let tap_line = caption(&format!("Taps: {}", list.join(" · ")));
    tap_line.add_css_class("monospace");
    fir.append(&tap_line);
    body.append(&fir);

    let noise = card("Noise and pitch modulation");
    noise.append(&wrapped(
        "A voice NON names plays the noise generator in place of its sample, at FLG's noise \
         clock. A voice PMON names bends its pitch by the wave of the voice before it.",
    ));
    for i in [0x3D, 0x2D, 0x6C] {
        if let Some(reg) = r(i) {
            noise.append(&register_row(reg));
        }
    }
    body.append(&noise);
}

fn draw_taps(a: &gtk::DrawingArea, cr: &cairo::Context, w: f64, h: f64, taps: &[i8; 8]) {
    let fg = a.color();
    fill_rect(cr, 0.0, 0.0, w, h, &with_alpha(&fg, 0.05));
    let mid = h / 2.0;
    let bar = w / 8.0;
    for (i, t) in taps.iter().enumerate() {
        let tall = f64::from(*t) / 128.0 * mid;
        let colour = if *t >= 0 {
            accent()
        } else {
            gtk::gdk::RGBA::new(1.0, 0.58, 0.0, 1.0)
        };
        let y = if tall >= 0.0 { mid - tall } else { mid };
        fill_rect(
            cr,
            i as f64 * bar + 3.0,
            y,
            bar - 6.0,
            tall.abs().max(1.0),
            &colour,
        );
    }
    crate::canvas::set_source(cr, &with_alpha(&fg, 0.4));
    cr.set_line_width(0.5);
    cr.move_to(0.0, mid);
    cr.line_to(w, mid);
    let _ = cr.stroke();
}

fn draw_response(a: &gtk::DrawingArea, cr: &cairo::Context, w: f64, h: f64, points: &[f64]) {
    let fg = a.color();
    fill_rect(cr, 0.0, 0.0, w, h, &with_alpha(&fg, 0.05));
    let y = |db: f64| h * (12.0 - db.clamp(-36.0, 12.0)) / 48.0;
    cr.set_line_width(0.5);
    for db in [12.0, 0.0, -12.0, -24.0, -36.0] {
        crate::canvas::set_source(cr, &with_alpha(&fg, if db == 0.0 { 0.5 } else { 0.15 }));
        cr.move_to(0.0, y(db));
        cr.line_to(w, y(db));
        let _ = cr.stroke();
    }
    crate::canvas::set_source(cr, &accent());
    cr.set_line_width(1.5);
    for (i, db) in points.iter().enumerate() {
        let x = i as f64 / (points.len() - 1) as f64 * w;
        if i == 0 {
            cr.move_to(x, y(*db));
        } else {
            cr.line_to(x, y(*db));
        }
    }
    let _ = cr.stroke();
    for (t, x, yy) in [("0 Hz", 3.0, h - 14.0), ("+12 dB", 3.0, 2.0)] {
        let l = mono_layout(a, 8.0, t);
        text(cr, &l, x, yy, &with_alpha(&fg, 0.6));
    }
    let l = mono_layout(a, 8.0, "16 kHz");
    let (lw, _) = l.pixel_size();
    text(
        cr,
        &l,
        w - f64::from(lw) - 3.0,
        h - 14.0,
        &with_alpha(&fg, 0.6),
    );
}
