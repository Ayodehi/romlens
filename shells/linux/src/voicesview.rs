//! The eight voices as channel strips (docs/23): each one's sample, note,
//! volume, envelope and flags, and beside them the DSP registers of the voice
//! selected, then the global ones, each explained. The macOS twin is
//! `VoicesView`, `VoiceStrip`, `EnvelopeView` and `DspRegistersList`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::cairo;
use romlens_ffi::{DspRegisterInfo, VoiceInfo};

use crate::audioview::dollar;
use crate::canvas::with_alpha;
use crate::gfxdraw::*;
use crate::inspector::explain::register_part;
use crate::model::audio::Tab;
use crate::model::{Change, Document};

const HOLD_MS: u32 = 1500;
const RELEASE_MS: u32 = 250;

struct View {
    doc: Rc<Document>,
    strips: Vec<Rc<Strip>>,
    registers: gtk::Box,
    rows: RefCell<Vec<Rc<RegisterRow>>>,
    rows_for: Cell<Option<Option<usize>>>,
}

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .column_spacing(12)
        .row_spacing(12)
        .min_children_per_line(1)
        .max_children_per_line(4)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .valign(gtk::Align::Start)
        .build();
    let strips: Vec<Rc<Strip>> = (0..8).map(|i| Strip::build(doc, i)).collect();
    for s in &strips {
        flow.insert(&s.root, -1);
    }
    let left = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .child(&flow)
        .build();
    let registers = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(8)
        .margin_end(8)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    let right = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .width_request(300)
        .child(&registers)
        .build();
    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&left)
        .end_child(&right)
        .resize_start_child(true)
        .resize_end_child(false)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .build();
    let view = Rc::new(View {
        doc: Rc::clone(doc),
        strips,
        registers,
        rows: RefCell::new(Vec::new()),
        rows_for: Cell::new(None),
    });
    view.refresh();
    let weak = Rc::downgrade(&view);
    doc.subscribe(move |c| {
        if matches!(c, Change::Audio | Change::Graphics | Change::Layout)
            && let Some(v) = weak.upgrade()
        {
            v.refresh();
        }
    });
    let weak = Rc::downgrade(&view);
    adw::StyleManager::default().connect_accent_color_notify(move |_| {
        if let Some(v) = weak.upgrade() {
            for s in &v.strips {
                s.redraw();
            }
        }
    });
    // The view lives as long as its widget.
    paned.connect_destroy(move |_| {
        let _ = &view;
    });
    paned.upcast()
}

impl View {
    fn refresh(&self) {
        if self.doc.audio_tab() != Some(Tab::Voices) {
            return;
        }
        let a = self.doc.audio();
        let Some(state) = a.state() else { return };
        for (strip, voice) in self.strips.iter().zip(&state.voices) {
            let i = usize::from(voice.index);
            strip.update(
                voice,
                a.muted() & (1 << i) != 0,
                a.is_solo(i),
                a.selected_voice == Some(i),
            );
        }
        let selected = a.selected_voice;
        drop(a);
        self.show_registers(&state.registers, selected);
    }

    /// The selected voice's registers, then the globals. The rows are kept
    /// while the voice stays the same, and only their words change.
    fn show_registers(&self, regs: &[DspRegisterInfo], voice: Option<usize>) {
        if self.rows_for.get() != Some(voice) {
            self.rows_for.set(Some(voice));
            while let Some(c) = self.registers.first_child() {
                self.registers.remove(&c);
            }
            let mut rows = Vec::new();
            let mut section = |title: &str, keep: &dyn Fn(&DspRegisterInfo) -> bool| {
                let h = gtk::Label::builder()
                    .label(title)
                    .xalign(0.0)
                    .margin_top(6)
                    .build();
                h.add_css_class("heading");
                self.registers.append(&h);
                for r in regs.iter().filter(|r| !r.unused && keep(r)) {
                    let row = RegisterRow::new(r);
                    self.registers.append(&row.expander);
                    rows.push(row);
                }
            };
            if let Some(v) = voice {
                section(&format!("Voice {v}"), &|r| r.voice == Some(v as u8));
            }
            section("Global", &|r| r.voice.is_none());
            *self.rows.borrow_mut() = rows;
        }
        let by: HashMap<u8, &DspRegisterInfo> = regs.iter().map(|r| (r.register, r)).collect();
        for row in self.rows.borrow().iter() {
            if let Some(r) = by.get(&row.register) {
                row.update(r);
            }
        }
    }
}

// MARK: Registers

struct RegisterRow {
    register: u8,
    expander: gtk::Expander,
    label: gtk::Label,
    body: gtk::Box,
    value: Cell<u8>,
}

impl RegisterRow {
    fn new(r: &DspRegisterInfo) -> Rc<Self> {
        let addr = gtk::Label::new(Some(&dollar(r.register, 2)));
        addr.add_css_class("monospace");
        addr.add_css_class("caption");
        addr.add_css_class("dim-label");
        let label = gtk::Label::builder()
            .label(&r.short)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .build();
        label.add_css_class("monospace");
        label.add_css_class("caption");
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        head.append(&addr);
        head.append(&label);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
        body.set_margin_top(4);
        body.set_margin_bottom(4);
        let expander = gtk::Expander::builder()
            .label_widget(&head)
            .child(&body)
            .build();
        let row = Rc::new(Self {
            register: r.register,
            expander,
            label,
            body,
            value: Cell::new(r.value),
        });
        row.fill(r);
        row
    }

    fn fill(&self, r: &DspRegisterInfo) {
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
        for p in &r.parts {
            self.body.append(&register_part(p));
        }
    }

    fn update(&self, r: &DspRegisterInfo) {
        self.label.set_text(&r.short);
        if self.value.replace(r.value) != r.value {
            self.fill(r);
        }
    }
}

// MARK: Strips

/// A flag chip and how a voice says it is on.
type Flag = (gtk::Label, fn(&VoiceInfo) -> bool);

struct Strip {
    root: gtk::Box,
    doc: Rc<Document>,
    voice: Rc<RefCell<Option<VoiceInfo>>>,
    dot: gtk::DrawingArea,
    title: gtk::Label,
    note: gtk::Label,
    mute: gtk::ToggleButton,
    solo: gtk::ToggleButton,
    sample: gtk::Label,
    pitch: gtk::Label,
    left: gtk::DrawingArea,
    right: gtk::DrawingArea,
    env: gtk::DrawingArea,
    words: gtk::Label,
    flags: Vec<Flag>,
    curves: RefCell<HashMap<u32, Vec<u16>>>,
    syncing: Cell<bool>,
}

fn volume_bar(voice: Rc<RefCell<Option<VoiceInfo>>>, left: bool) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::builder()
        .content_height(8)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    a.set_draw_func(move |a, cr, w, h| {
        let value = voice
            .borrow()
            .as_ref()
            .map_or(0, |v| if left { v.volume_left } else { v.volume_right });
        let (w, h) = (f64::from(w), f64::from(h));
        let mid = w / 2.0;
        let len = f64::from(value.unsigned_abs()) / 128.0 * mid;
        let fg = a.color();
        fill_rect(cr, 0.0, 0.0, w, h, &with_alpha(&fg, 0.15));
        let colour = if value < 0 {
            gtk::gdk::RGBA::new(1.0, 0.58, 0.0, 1.0)
        } else {
            accent()
        };
        let x = if value < 0 { mid - len } else { mid };
        fill_rect(cr, x, 0.0, len, h, &colour);
        fill_rect(cr, mid, 0.0, 1.0, h, &with_alpha(&fg, 0.5));
    });
    a
}

impl Strip {
    fn build(doc: &Rc<Document>, index: usize) -> Rc<Self> {
        let caption_label = |t: &str| {
            let l = caption(t);
            l.set_wrap(true);
            l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            l.set_max_width_chars(34);
            l
        };
        let dot = gtk::DrawingArea::builder()
            .content_width(8)
            .content_height(8)
            .valign(gtk::Align::Center)
            .build();
        let title = gtk::Label::new(Some(&format!("Voice {index}")));
        title.add_css_class("heading");
        let note = gtk::Label::builder().xalign(1.0).hexpand(true).build();
        note.add_css_class("monospace");
        note.set_tooltip_text(Some(
            "The note, estimated from the loop of its sample: a pitch is a rate, not a note",
        ));
        let mute = gtk::ToggleButton::with_label("M");
        mute.set_tooltip_text(Some(
            "Mute: leave this voice out of what you hear; it still runs",
        ));
        let solo = gtk::ToggleButton::with_label("S");
        solo.set_tooltip_text(Some("Solo: hear only this voice"));
        let sample = caption_label("");
        sample.add_css_class("monospace");
        let pitch = caption_label("");
        pitch.set_lines(2);
        pitch.set_ellipsize(gtk::pango::EllipsizeMode::End);
        pitch.set_height_request(34);
        let env = gtk::DrawingArea::builder()
            .content_height(64)
            .hexpand(true)
            .build();
        env.set_tooltip_text(Some(
            "The envelope these settings make, keyed on then off at 1.5 s, run through Romlens's DSP; the green line is ENVX now",
        ));
        let words = caption_label("");
        words.set_height_request(48);
        words.set_valign(gtk::Align::Start);
        let flag = |text: &str, tip: &str| {
            let l = gtk::Label::new(Some(text));
            l.add_css_class("caption");
            l.add_css_class("chip");
            l.set_tooltip_text(Some(tip));
            l
        };
        let flags: Vec<Flag> = vec![
            (flag("echo", "EON: it feeds the echo"), |v| v.echo),
            (
                flag("noise", "NON: it plays the noise generator, not its sample"),
                |v| v.noise,
            ),
            (
                flag(
                    "pitch mod",
                    "PMON: its pitch follows the wave of the voice before",
                ),
                |v| v.modulated,
            ),
            (
                flag("keyed off", "KOFF holds it off: its envelope releases"),
                |v| v.keyed_off,
            ),
            (
                flag(
                    "ended",
                    "ENDX: its sample reached a block with the end flag",
                ),
                |v| v.ended,
            ),
        ];

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&dot);
        head.append(&title);
        head.append(&note);
        head.append(&mute);
        head.append(&solo);
        let voice: Rc<RefCell<Option<VoiceInfo>>> = Rc::new(RefCell::new(None));
        let (left, right) = (
            volume_bar(Rc::clone(&voice), true),
            volume_bar(Rc::clone(&voice), false),
        );
        let vols = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        for (label, bar) in [("L", &left), ("R", &right)] {
            let pair = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            pair.set_hexpand(true);
            pair.append(&caption(label));
            pair.append(bar);
            vols.append(&pair);
        }
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.add_css_class("voice-strip");
        root.set_size_request(300, -1);
        root.append(&head);
        root.append(&sample);
        root.append(&pitch);
        root.append(&vols);
        root.append(&env);
        root.append(&words);
        let flag_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        for (l, _) in &flags {
            flag_row.append(l);
        }
        root.append(&flag_row);

        let strip = Rc::new(Self {
            root,
            doc: Rc::clone(doc),
            voice,
            dot,
            title,
            note,
            mute,
            solo,
            sample,
            pitch,
            left,
            right,
            env,
            words,
            flags,
            curves: RefCell::new(HashMap::new()),
            syncing: Cell::new(false),
        });
        strip.wire(index);
        strip
    }

    fn wire(self: &Rc<Self>, index: usize) {
        let weak = Rc::downgrade(self);
        self.dot.set_draw_func(move |_, cr, w, h| {
            let Some(s) = weak.upgrade() else { return };
            let sounding = s.voice.borrow().as_ref().is_some_and(|v| v.sounding);
            if sounding {
                cr.set_source_rgb(0.2, 0.8, 0.3);
            } else {
                cr.set_source_rgba(0.5, 0.5, 0.5, 0.35);
            }
            cr.arc(
                f64::from(w) / 2.0,
                f64::from(h) / 2.0,
                4.0,
                0.0,
                std::f64::consts::TAU,
            );
            let _ = cr.fill();
        });
        let weak = Rc::downgrade(self);
        self.env.set_draw_func(move |a, cr, w, h| {
            if let Some(s) = weak.upgrade() {
                s.draw_envelope(a, cr, f64::from(w), f64::from(h));
            }
        });
        let (doc, weak) = (Rc::clone(&self.doc), Rc::downgrade(self));
        self.mute.connect_toggled(move |_| {
            if let Some(s) = weak.upgrade()
                && !s.syncing.get()
            {
                doc.edit_audio(|a| a.toggle_mute(index));
            }
        });
        let (doc, weak) = (Rc::clone(&self.doc), Rc::downgrade(self));
        self.solo.connect_toggled(move |_| {
            if let Some(s) = weak.upgrade()
                && !s.syncing.get()
            {
                doc.edit_audio(|a| a.toggle_solo(index));
            }
        });
        let click = gtk::GestureClick::new();
        let doc = Rc::clone(&self.doc);
        click.connect_pressed(move |_, _, _, _| {
            doc.edit_audio(|a| a.selected_voice = Some(index));
        });
        self.root.add_controller(click);
    }

    fn redraw(&self) {
        for a in [&self.dot, &self.left, &self.right, &self.env] {
            a.queue_draw();
        }
    }

    fn update(&self, v: &VoiceInfo, muted: bool, solo: bool, selected: bool) {
        self.syncing.set(true);
        self.mute.set_active(muted);
        self.solo.set_active(solo);
        self.syncing.set(false);
        self.title.set_text(&format!("Voice {}", v.index));
        self.note.set_text(
            &v.note
                .as_ref()
                .map_or_else(String::new, |n| format!("≈ {n}")),
        );
        self.sample
            .set_text(&match (v.sample_start, v.sample_loop) {
                (Some(s), Some(l)) => format!(
                    "sample {} at {}, loop {}",
                    v.source,
                    dollar(s, 4),
                    dollar(l, 4)
                ),
                _ => format!("sample {}", v.source),
            });
        self.pitch
            .set_text(&format!("pitch {}: {}", dollar(v.pitch, 4), v.pitch_words));
        self.words.set_text(&v.envelope_words);
        for (label, on) in &self.flags {
            let on = on(v);
            label.remove_css_class(if on { "flag-off" } else { "flag-on" });
            label.add_css_class(if on { "flag-on" } else { "flag-off" });
        }
        if selected {
            self.root.add_css_class("voice-selected");
        } else {
            self.root.remove_css_class("voice-selected");
        }
        *self.voice.borrow_mut() = Some(v.clone());
        self.redraw();
    }

    /// The envelope a voice's ADSR or GAIN makes, run through the DSP itself:
    /// held for 1.5 s, then released, with ENVX now marked as a line.
    fn draw_envelope(&self, a: &gtk::DrawingArea, cr: &cairo::Context, w: f64, h: f64) {
        let Some(v) = self.voice.borrow().clone() else {
            return;
        };
        let key = u32::from(v.adsr1) << 16 | u32::from(v.adsr2) << 8 | u32::from(v.gain);
        let curve = self
            .curves
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                romlens_ffi::envelope_curve(v.adsr1, v.adsr2, v.gain, HOLD_MS, RELEASE_MS, 64)
            })
            .clone();
        let fg = a.color();
        fill_rect(cr, 0.0, 0.0, w, h, &with_alpha(&fg, 0.06));
        let n = curve.len();
        if n > 1 {
            let (x, y) = (
                |i: usize| i as f64 / (n - 1) as f64 * w,
                |v: u16| h - 2.0 - f64::from(v) / 2047.0 * (h - 4.0),
            );
            let c = accent();
            cr.set_source_rgba(
                f64::from(c.red()),
                f64::from(c.green()),
                f64::from(c.blue()),
                1.0,
            );
            cr.set_line_width(1.5);
            cr.move_to(0.0, y(curve[0]));
            for (i, p) in curve.iter().enumerate().skip(1) {
                cr.line_to(x(i), y(*p));
            }
            let _ = cr.stroke();
            // Key off.
            let off = f64::from(HOLD_MS) / f64::from(HOLD_MS + RELEASE_MS) * w;
            set_dash(cr, &fg, off, h);
        }
        // ENVX now, 0 to 127 of the 11-bit level.
        let now = h - f64::from(v.envx) / 127.0 * h;
        cr.set_source_rgba(0.2, 0.8, 0.3, 0.8);
        cr.set_line_width(1.0);
        cr.move_to(0.0, now);
        cr.line_to(w, now);
        let _ = cr.stroke();
        let l = mono_layout(a, 8.0, &format!("ENVX {}", v.envx));
        let (lw, _) = l.pixel_size();
        text(
            cr,
            &l,
            w - f64::from(lw) - 3.0,
            2.0,
            &gtk::gdk::RGBA::new(0.2, 0.8, 0.3, 1.0),
        );
        let l = mono_layout(a, 8.0, "key off at 1.5 s");
        let (lw, lh) = l.pixel_size();
        text(
            cr,
            &l,
            w - f64::from(lw) - 3.0,
            h - f64::from(lh) - 2.0,
            &with_alpha(&fg, 0.45),
        );
    }
}

fn set_dash(cr: &cairo::Context, fg: &gtk::gdk::RGBA, x: f64, h: f64) {
    crate::canvas::set_source(cr, &with_alpha(fg, 0.5));
    cr.set_line_width(1.0);
    cr.set_dash(&[3.0, 3.0], 0.0);
    cr.move_to(x, 0.0);
    cr.line_to(x, h);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
}
