//! The sample directory, and one sample opened (docs/23): its waveform with
//! the loop point, its BRR blocks, a block's sixteen values decoded step by
//! step with the filter's formula in numbers, and a keyboard to play it. The
//! macOS twin is `SamplesView`, `SampleDetail`, `WaveformView`, `BlockStrip`,
//! `BlockSteps` and `SampleKeyboard`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::cairo;
use romlens_ffi::{BrrBlockInfo, BrrSampleInfo, SampleInfo};

use crate::audioview::dollar;
use crate::canvas::with_alpha;
use crate::gfxdraw::*;
use crate::model::audio::Tab;
use crate::model::{Change, Document, Tab as EditorTab};

/// What the detail was built for: the sample, the block and the source.
type DetailKey = (Option<u8>, Option<usize>, String);

struct View {
    doc: Rc<Document>,
    list: gtk::ListBox,
    detail: gtk::Box,
    rows: RefCell<Vec<(u8, gtk::ListBoxRow)>>,
    key: RefCell<String>,
    detail_key: RefCell<Option<DetailKey>>,
    syncing: Cell<bool>,
}

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .build();
    list.add_css_class("navigation-sidebar");
    let head = gtk::Box::builder()
        .spacing(10)
        .margin_start(18)
        .margin_end(12)
        .margin_top(6)
        .build();
    for (t, w) in [
        ("#", 3),
        ("Start", 6),
        ("Loop", 6),
        ("Bytes", 6),
        ("At $1000", 9),
        ("Played", 6),
    ] {
        let l = gtk::Label::builder()
            .label(t)
            .width_chars(w)
            .xalign(1.0)
            .build();
        l.add_css_class("caption");
        l.add_css_class("dim-label");
        head.append(&l);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&list)
        .build();
    let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
    left.append(&head);
    left.append(&scroll);
    left.set_width_request(380);
    let detail = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    let right = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&detail)
        .build();
    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&left)
        .end_child(&right)
        .resize_start_child(false)
        .resize_end_child(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .build();
    let view = Rc::new(View {
        doc: Rc::clone(doc),
        list,
        detail,
        rows: RefCell::new(Vec::new()),
        key: RefCell::new(String::new()),
        detail_key: RefCell::new(None),
        syncing: Cell::new(false),
    });
    let weak = Rc::downgrade(&view);
    view.list.connect_row_selected(move |_, row| {
        let Some(v) = weak.upgrade() else { return };
        if v.syncing.get() {
            return;
        }
        let index = row.and_then(|r| {
            v.rows
                .borrow()
                .iter()
                .find(|(_, x)| x == r)
                .map(|(i, _)| *i)
        });
        v.doc.edit_audio(|a| {
            a.selected_sample = index;
            a.selected_block = None;
        });
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
    paned.connect_destroy(move |_| {
        let _ = &view;
    });
    paned.upcast()
}

fn mono_label(text: &str, chars: i32) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .width_chars(chars)
        .xalign(1.0)
        .build();
    l.add_css_class("monospace");
    l
}

impl View {
    fn refresh(self: &Rc<Self>) {
        if self.doc.audio_tab() != Some(Tab::Samples) {
            return;
        }
        let a = self.doc.audio();
        let samples = a.state().map(|s| s.samples).unwrap_or_default();
        let key = samples
            .iter()
            .map(|s| format!("{}:{}:{}:{:?}", s.index, s.start, s.loop_at, s.played))
            .collect::<Vec<_>>()
            .join(",");
        let (selected, block) = (a.selected_sample, a.selected_block);
        let playing = a.is_playing();
        drop(a);
        if *self.key.borrow() != key {
            *self.key.borrow_mut() = key;
            self.fill_list(&samples);
        }
        self.syncing.set(true);
        match selected.and_then(|i| {
            self.rows
                .borrow()
                .iter()
                .find(|(x, _)| *x == i)
                .map(|(_, r)| r.clone())
        }) {
            Some(r) => self.list.select_row(Some(&r)),
            None => self.list.unselect_all(),
        }
        self.syncing.set(false);
        // While playing the sample does not change: leave the detail alone.
        let detail_key = (selected, block, self.doc.audio().source_description());
        let same = self.detail_key.borrow().as_ref() == Some(&detail_key);
        let same_selection = self
            .detail_key
            .borrow()
            .as_ref()
            .is_some_and(|k| k.0 == selected && k.1 == block);
        if !same && !(playing && same_selection) {
            *self.detail_key.borrow_mut() = Some(detail_key);
            self.show_detail(&samples);
        }
    }

    fn fill_list(&self, samples: &[SampleInfo]) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let mut rows = self.rows.borrow_mut();
        rows.clear();
        for s in samples {
            let line = gtk::Box::builder()
                .spacing(10)
                .margin_start(6)
                .margin_end(6)
                .margin_top(3)
                .margin_bottom(3)
                .build();
            line.append(&mono_label(&s.index.to_string(), 3));
            line.append(&mono_label(&dollar(s.start, 4), 6));
            line.append(&mono_label(
                &if s.loops {
                    dollar(s.loop_at, 4)
                } else {
                    "-".to_owned()
                },
                6,
            ));
            line.append(&mono_label(&(s.blocks * 9).to_string(), 6));
            let note = mono_label(
                &s.tuning_note
                    .as_ref()
                    .map_or_else(|| "-".to_owned(), |n| format!("≈ {n}")),
                9,
            );
            note.set_tooltip_text(Some(&s.tuning_hz.map_or_else(
                || "No loop to estimate a note from".to_owned(),
                |h| format!("{h:.0} Hz at pitch $1000, estimated from the loop"),
            )));
            line.append(&note);
            let played = mono_label(s.played.map_or("", |p| if p { "yes" } else { "no" }), 6);
            played.add_css_class("dim-label");
            played.set_tooltip_text(Some(
                "Whether the DSP read it while the SPC700's execution log was kept",
            ));
            line.append(&played);
            let row = gtk::ListBoxRow::builder().child(&line).build();
            self.list.append(&row);
            rows.push((s.index, row));
        }
    }

    fn show_detail(self: &Rc<Self>, samples: &[SampleInfo]) {
        while let Some(c) = self.detail.first_child() {
            self.detail.remove(&c);
        }
        let (selected, block) = {
            let a = self.doc.audio();
            (a.selected_sample, a.selected_block)
        };
        let found = selected.and_then(|i| {
            let entry = samples.iter().find(|s| s.index == i)?.clone();
            let sample = self.doc.audio().sample(i, 4096)?;
            Some((entry, sample))
        });
        let Some((entry, sample)) = found else {
            let page = adw::StatusPage::builder()
                .icon_name("audio-x-generic-symbolic")
                .title("Choose a Sample")
                .description(
                    "The directory at DIR × $100 lists each sample's start and loop point: four \
                     bytes an entry, the entry a voice's SRCN names.",
                )
                .vexpand(true)
                .build();
            self.detail.append(&page);
            return;
        };

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = gtk::Label::new(Some(&format!("Sample {}", entry.index)));
        title.add_css_class("heading");
        let summary = caption(&summary_text(&entry, &sample));
        summary.set_hexpand(true);
        summary.set_wrap(true);
        head.append(&title);
        head.append(&summary);
        if let Some(rom) = self.doc.audio().rom_origin(entry.start) {
            let b = gtk::Button::with_label("Show in ROM");
            b.set_tooltip_text(Some(&format!(
                "The ROM bytes the upload sent here, file offset {}",
                romlens_ffi::format_file_offset(rom)
            )));
            let doc = Rc::clone(&self.doc);
            b.connect_clicked(move |_| {
                doc.set_tab(if doc.has_disassembly() {
                    EditorTab::Disassembly
                } else {
                    EditorTab::Hex
                });
                doc.jump_to(rom);
            });
            head.append(&b);
        }
        self.detail.append(&head);
        self.detail.append(&waveform(&self.doc, &sample, block));
        self.detail.append(&keyboard(&self.doc, entry.index));
        self.detail.append(&block_strip(&self.doc, &sample, block));
        match block.and_then(|b| sample.blocks.get(b).map(|blk| (b, blk))) {
            Some((i, b)) => self.detail.append(&block_steps(b, i)),
            None => self.detail.append(&caption(
                "Click a block, in the strip or the waveform, to see its sixteen values decoded.",
            )),
        }
    }
}

fn summary_text(entry: &SampleInfo, sample: &BrrSampleInfo) -> String {
    let mut s = format!(
        "{}, {} blocks, {} samples",
        dollar(entry.start, 4),
        sample.blocks.len(),
        sample.samples.len()
    );
    match (sample.loops, sample.loop_block) {
        (true, Some(lb)) => s += &format!(", loops from block {lb}"),
        (false, _) => s += ", plays once",
        _ => {}
    }
    if sample.unterminated {
        s += ", no end block found";
    }
    s
}

// MARK: Waveform

/// The decoded values, the loop point, and the block boundaries.
fn waveform(doc: &Rc<Document>, sample: &BrrSampleInfo, selected: Option<usize>) -> gtk::Widget {
    let area = gtk::DrawingArea::builder()
        .content_height(140)
        .hexpand(true)
        .build();
    let values = Rc::new(sample.samples.clone());
    let loop_block = sample.loop_block;
    let blocks = sample.blocks.len();
    {
        let values = Rc::clone(&values);
        area.set_draw_func(move |a, cr, w, h| {
            let (w, h) = (f64::from(w), f64::from(h));
            let fg = a.color();
            fill_rect(cr, 0.0, 0.0, w, h, &with_alpha(&fg, 0.05));
            let n = values.len().max(1);
            let x = |i: usize| i as f64 / n as f64 * w;
            if let Some(b) = selected {
                fill_rect(
                    cr,
                    x(b * 16),
                    0.0,
                    (x(16) - x(0)).max(1.0),
                    h,
                    &with_alpha(&accent(), 0.15),
                );
            }
            if let Some(lb) = loop_block {
                let lx = x(lb as usize * 16);
                cr.set_source_rgba(0.2, 0.8, 0.3, 0.06);
                cr.rectangle(lx, 0.0, w - lx, h);
                let _ = cr.fill();
                cr.set_source_rgb(0.2, 0.8, 0.3);
                cr.set_line_width(1.0);
                cr.move_to(lx, 0.0);
                cr.line_to(lx, h);
                let _ = cr.stroke();
                let l = mono_layout(a, 8.5, "loop");
                text(cr, &l, 3.0, 3.0, &gtk::gdk::RGBA::new(0.2, 0.8, 0.3, 1.0));
            }
            let mid = h / 2.0;
            crate::canvas::set_source(cr, &with_alpha(&fg, 0.3));
            cr.set_line_width(0.5);
            cr.move_to(0.0, mid);
            cr.line_to(w, mid);
            let _ = cr.stroke();
            // One column of pixels at a time: the lowest and highest value
            // in it, so a long sample stays a drawing and not a blur.
            crate::canvas::set_source(cr, &fg);
            cr.set_line_width(1.0);
            let columns = w.max(1.0) as usize;
            for c in 0..columns {
                let (from, to) = (
                    c * values.len() / columns,
                    ((c + 1) * values.len() / columns)
                        .max(c * values.len() / columns + 1)
                        .min(values.len()),
                );
                if from >= to {
                    break;
                }
                let slice = &values[from..to];
                let (lo, hi) = (
                    slice.iter().copied().min().unwrap_or(0),
                    slice.iter().copied().max().unwrap_or(0),
                );
                cr.move_to(c as f64 + 0.5, mid - f64::from(hi) / 32768.0 * mid);
                cr.line_to(c as f64 + 0.5, mid - f64::from(lo) / 32768.0 * mid - 0.5);
            }
            let _ = cr.stroke();
        });
    }
    let click = gtk::GestureClick::new();
    let (doc, area_for) = (Rc::clone(doc), area.clone());
    click.connect_pressed(move |_, _, x, _| {
        let w = f64::from(area_for.width()).max(1.0);
        let n = values.len().max(1);
        let i = (x / w * n as f64) as usize;
        let b = (i / 16).min(blocks.saturating_sub(1));
        doc.edit_audio(|a| a.selected_block = Some(b));
    });
    area.add_controller(click);
    area.upcast()
}

// MARK: Blocks

/// Each block's header byte: shift, filter and the loop and end flags.
fn block_strip(doc: &Rc<Document>, sample: &BrrSampleInfo, selected: Option<usize>) -> gtk::Widget {
    let row = gtk::Box::builder()
        .spacing(2)
        .margin_top(2)
        .margin_bottom(8)
        .build();
    for (i, b) in sample.blocks.iter().enumerate() {
        let cell = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(1)
            .build();
        let lines = [
            (dollar(b.header, 2), false),
            (format!("s{} f{}", b.shift, b.filter), true),
            (
                format!(
                    "{}{}",
                    if b.loops { "L" } else { " " },
                    if b.end { "E" } else { " " }
                ),
                false,
            ),
        ];
        for (t, dim) in lines {
            let l = gtk::Label::new(Some(&t));
            l.add_css_class("monospace");
            l.add_css_class("caption");
            if dim {
                l.add_css_class("dim-label");
            }
            cell.append(&l);
        }
        let button = gtk::Button::builder().child(&cell).build();
        button.add_css_class("flat");
        if selected == Some(i) {
            button.add_css_class("block-selected");
        } else if sample.loop_block.is_some_and(|l| l as usize == i) {
            button.add_css_class("block-loop");
        }
        button.set_tooltip_text(Some(&format!(
            "Block {i} at {}: shift {}, filter {}{}{}",
            dollar(b.offset, 4),
            b.shift,
            b.filter,
            if b.loops { ", loop flag" } else { "" },
            if b.end { ", end flag" } else { "" }
        )));
        let doc = Rc::clone(doc);
        button.connect_clicked(move |_| doc.edit_audio(|a| a.selected_block = Some(i)));
        row.append(&button);
    }
    gtk::ScrolledWindow::builder()
        .vscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(72)
        .child(&row)
        .build()
        .upcast()
}

/// A block's sixteen values: each nibble shifted, the filter's prediction from
/// the two before, clamped, wrapped, doubled.
fn block_steps(block: &BrrBlockInfo, index: usize) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let title = gtk::Label::builder()
        .label(format!(
            "Block {index} at {}: header {}",
            dollar(block.offset, 4),
            dollar(block.header, 2)
        ))
        .xalign(0.0)
        .build();
    title.add_css_class("heading");
    b.append(&title);
    let formula = caption(&format!(
        "shift {} · filter {}: {} · {}{}{}",
        block.shift,
        block.filter,
        block.filter_formula,
        block.filter_meaning,
        if block.loops { " · loop flag" } else { "" },
        if block.end { " · end flag" } else { "" }
    ));
    formula.set_wrap(true);
    b.append(&formula);
    let grid = gtk::Grid::builder()
        .column_spacing(14)
        .row_spacing(2)
        .build();
    let heads = [
        "#",
        "nibble",
        "shifted",
        "p1",
        "p2",
        "prediction",
        "sum, clamped",
        "kept (15 bits)",
        "played",
    ];
    for (c, h) in heads.iter().enumerate() {
        let l = caption(h);
        l.set_xalign(1.0);
        grid.attach(&l, c as i32, 0, 1, 1);
    }
    for (i, s) in block.steps.iter().enumerate() {
        let r = i as i32 + 1;
        let cells: [(String, bool, bool, String); 9] = [
            (i.to_string(), true, false, String::new()),
            (s.nibble.to_string(), false, false, String::new()),
            (s.shifted.to_string(), false, false, String::new()),
            (s.p1.to_string(), true, false, String::new()),
            (s.p2.to_string(), true, false, String::new()),
            (s.prediction.to_string(), false, false, String::new()),
            (
                s.clamped.to_string(),
                false,
                s.clipped,
                if s.clipped {
                    format!("{} + {} is past 16 bits: clamped", s.shifted, s.prediction)
                } else {
                    format!("{} + {}", s.shifted, s.prediction)
                },
            ),
            (
                s.result.to_string(),
                false,
                s.wrapped,
                if s.wrapped {
                    "Wrapped to 15 bits: the DSP keeps one bit less than it clamps to".to_owned()
                } else {
                    String::new()
                },
            ),
            (s.output.to_string(), false, false, String::new()),
        ];
        for (c, (t, dim, warn, tip)) in cells.iter().enumerate() {
            let l = gtk::Label::builder().label(t).xalign(1.0).build();
            l.add_css_class("monospace");
            l.add_css_class("caption");
            if *dim {
                l.add_css_class("dim-label");
            }
            if *warn {
                l.add_css_class("warning");
            }
            if !tip.is_empty() {
                l.set_tooltip_text(Some(tip));
            }
            grid.attach(&l, c as i32, r, 1, 1);
        }
    }
    b.append(&grid);
    if let Some(first) = block.steps.first() {
        let l = caption(&format!(
            "Value 0: {} shifted by {} is {}; the filter predicts {} from {} and {}; {} + {} = {}, kept as {} and played doubled as {}.",
            first.nibble,
            block.shift,
            first.shifted,
            first.prediction,
            first.p1,
            first.p2,
            first.shifted,
            first.prediction,
            first.clamped,
            first.result,
            first.output
        ));
        l.set_wrap(true);
        b.append(&l);
    }
    b.upcast()
}

// MARK: Keyboard

const WHITE_W: f64 = 24.0;
const WHITE_H: f64 = 76.0;
const BLACK_W: f64 = 15.0;
const BLACK_H: f64 = 46.0;
const GAP: f64 = 1.0;

/// Semitones from A4 of each white key: C4 to B5.
fn whites() -> Vec<i32> {
    (0..2)
        .flat_map(|o| [0, 2, 4, 5, 7, 9, 11].map(move |s| o * 12 + s - 9))
        .collect()
}

/// Each black key: the white key it follows, and its semitone from A4.
fn blacks() -> Vec<(usize, i32)> {
    (0..2usize)
        .flat_map(|o| {
            [(0, 1), (1, 3), (3, 6), (4, 8), (5, 10)]
                .map(move |(a, s)| (o * 7 + a, o as i32 * 12 + s - 9))
        })
        .collect()
}

/// The key under a point: black keys sit on top.
fn key_at(x: f64, y: f64) -> Option<i32> {
    if y < 0.0 || y > WHITE_H {
        return None;
    }
    if y <= BLACK_H {
        for (after, semitone) in blacks() {
            let left = (after + 1) as f64 * (WHITE_W + GAP) - GAP / 2.0 - BLACK_W / 2.0;
            if (left..left + BLACK_W).contains(&x) {
                return Some(semitone);
            }
        }
    }
    let i = (x / (WHITE_W + GAP)).floor();
    whites().get(i as usize).copied().filter(|_| i >= 0.0)
}

/// Two octaves of keys, C4 to B5, for playing one sample by hand, laid out as
/// a piano's.
fn keyboard(doc: &Rc<Document>, sample: u8) -> gtk::Widget {
    let width = whites().len() as f64 * (WHITE_W + GAP) - GAP;
    let area = gtk::DrawingArea::builder()
        .content_width(width as i32)
        .content_height(WHITE_H as i32)
        .halign(gtk::Align::Start)
        .build();
    let down: Rc<Cell<Option<i32>>> = Rc::new(Cell::new(None));
    {
        let down = Rc::clone(&down);
        area.set_draw_func(move |a, cr, _, _| {
            let fg = a.color();
            let key = |cr: &cairo::Context, x: f64, w: f64, h: f64, k: i32, black: bool| {
                if down.get() == Some(k) {
                    crate::canvas::set_source(cr, &accent());
                } else if black {
                    cr.set_source_rgb(0.05, 0.05, 0.05);
                } else {
                    cr.set_source_rgb(0.98, 0.98, 0.98);
                }
                cr.rectangle(x, 0.0, w, h);
                let _ = cr.fill();
                outline(
                    cr,
                    x,
                    0.0,
                    w,
                    h,
                    &with_alpha(&fg, if black { 0.8 } else { 0.5 }),
                    0.5,
                );
            };
            for (i, k) in whites().into_iter().enumerate() {
                let x = i as f64 * (WHITE_W + GAP);
                key(cr, x, WHITE_W, WHITE_H, k, false);
                if (k + 9) % 12 == 0 {
                    let l = mono_layout(a, 7.0, &format!("C{}", 4 + (k + 9) / 12));
                    text(
                        cr,
                        &l,
                        x + 6.0,
                        WHITE_H - 14.0,
                        &gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.5),
                    );
                }
            }
            for (after, k) in blacks() {
                let x = (after + 1) as f64 * (WHITE_W + GAP) - GAP / 2.0 - BLACK_W / 2.0;
                key(cr, x, BLACK_W, BLACK_H, k, true);
            }
        });
    }
    let press = {
        let (doc, down, area) = (Rc::clone(doc), Rc::clone(&down), area.clone());
        move |k: Option<i32>| {
            if down.get() == k {
                return;
            }
            down.set(k);
            area.queue_draw();
            match k {
                Some(k) => {
                    let pitch = doc.audio().pitch(sample, k);
                    doc.edit_audio(|a| a.press(sample, pitch));
                }
                None => doc.edit_audio(|a| a.release()),
            }
        }
    };
    // Held with the pointer, and slid across the keys.
    let drag = gtk::GestureDrag::new();
    let p = press.clone();
    drag.connect_drag_begin(move |_, x, y| p(key_at(x, y)));
    let p = press.clone();
    drag.connect_drag_update(move |g, dx, dy| {
        if let Some((sx, sy)) = g.start_point() {
            p(key_at(sx + dx, sy + dy));
        }
    });
    drag.connect_drag_end(move |_, _, _| press(None));
    area.add_controller(drag);

    let note = {
        let audio = doc.audio();
        let tuned = audio
            .state()
            .and_then(|s| {
                s.samples
                    .iter()
                    .find(|s| s.index == sample)
                    .and_then(|s| s.tuning_hz)
            })
            .is_some();
        if tuned {
            "C4 to B5, from the sample's tuning estimated from its loop. Hold a key: KON on press, KOFF on release, the envelope at full by direct gain."
        } else {
            "The sample has no loop to tune it from, so A4 is its own rate (pitch $1000)."
        }
    };
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    b.append(&area);
    let c = caption(note);
    c.set_wrap(true);
    b.append(&c);
    b.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_octaves_of_white_keys_run_from_c4_to_b5() {
        let w = whites();
        assert_eq!(w.len(), 14);
        assert_eq!(w[0], -9, "C4 is nine semitones below A4");
        assert_eq!(w[5], 0, "A4");
        assert_eq!(*w.last().unwrap(), 14, "B5");
        assert_eq!(blacks().len(), 10);
    }

    #[test]
    fn a_black_key_wins_over_the_white_one_beneath_it_and_only_in_its_height() {
        // C#4 sits between the first two white keys.
        let boundary = WHITE_W + GAP / 2.0;
        assert_eq!(key_at(boundary, 10.0), Some(-8), "C#4");
        assert_eq!(
            key_at(boundary, BLACK_H + 5.0),
            Some(-9),
            "below the black key it is C4"
        );
        assert_eq!(key_at(WHITE_W + GAP + 5.0, BLACK_H + 5.0), Some(-7), "D4");
        assert_eq!(key_at(-1.0, 10.0), None);
        assert_eq!(key_at(10.0, WHITE_H + 1.0), None);
        // There is no black key between E and F.
        let e_f = 3.0 * (WHITE_W + GAP) - GAP / 2.0;
        assert_eq!(key_at(e_f - 1.0, 10.0), Some(-5), "E4");
        assert_eq!(key_at(e_f + 1.0, 10.0), Some(-4), "F4");
        assert!(!blacks().iter().any(|(after, _)| *after == 2));
    }
}
