//! A piano roll per voice (docs/23, A13): each note from its key-on to its
//! key-off, placed up and down its lane by pitch and coloured by sample,
//! around the frame. A note opens where it came from: the SPC700 instruction
//! that wrote it, and the S-CPU's requests before it with the 65816 code that
//! sends them. Beside it, Nintendo's N-SPC driver's song read (A14). The macOS
//! twin is `NoteTimelineView`, `PianoRoll`, `NoteDetail` and `SongPane`.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::cairo;
use romlens_ffi::{NspcDriverInfo, NspcEntryInfo, NspcEntryKind};

use crate::audioview::{dollar, heading_label, show_in_rom};
use crate::canvas::with_alpha;
use crate::gfxdraw::*;
use crate::model::audio::{NoteSpan, Source, Tab, note_spans, pitch_y};
use crate::model::{Change, Document};

const SPANS: [u64; 5] = [120, 300, 600, 1800, 3600];

struct View {
    doc: Rc<Document>,
    summary: gtk::Label,
    roll: gtk::DrawingArea,
    detail: gtk::Box,
    song: gtk::Box,
    span: gtk::DropDown,
    /// The song pane is rebuilt only when what it shows changes.
    song_key: RefCell<String>,
    detail_key: RefCell<String>,
    syncing: std::cell::Cell<bool>,
}

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let summary = gtk::Label::builder().xalign(0.0).hexpand(true).build();
    summary.add_css_class("caption");
    summary.add_css_class("dim-label");
    let labels: Vec<String> = SPANS.iter().map(u64::to_string).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let span = gtk::DropDown::from_strings(&refs);
    span.set_tooltip_text(Some("Frames across"));
    let controls = gtk::Box::builder()
        .spacing(8)
        .margin_start(12)
        .margin_end(12)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    controls.append(&summary);
    controls.append(&caption("Frames across"));
    controls.append(&span);

    let roll = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .content_height(8 * 36)
        .build();
    roll.set_tooltip_text(Some(
        "Each voice's notes: up and down by pitch, coloured by sample; click one to see where it came from",
    ));
    let detail = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_start(12)
        .margin_end(12)
        .margin_top(10)
        .margin_bottom(10)
        .build();
    let detail_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(120)
        .child(&detail)
        .build();
    let vertical = gtk::Paned::builder()
        .orientation(gtk::Orientation::Vertical)
        .start_child(&{
            let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
            b.append(&controls);
            b.append(&roll);
            b
        })
        .end_child(&detail_scroll)
        .resize_start_child(true)
        .resize_end_child(false)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .build();
    let song = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    let song_scroll = gtk::ScrolledWindow::builder()
        .width_request(360)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&song)
        .build();
    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&vertical)
        .end_child(&song_scroll)
        .resize_start_child(true)
        .resize_end_child(false)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .build();

    let view = Rc::new(View {
        doc: Rc::clone(doc),
        summary,
        roll,
        detail,
        song,
        span,
        song_key: RefCell::new(String::new()),
        detail_key: RefCell::new(String::new()),
        syncing: std::cell::Cell::new(false),
    });
    view.wire();
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

/// A colour per sample.
fn sample_colour(source: u8) -> (f64, f64, f64) {
    let hue = f64::from((u32::from(source) * 37) % 360) / 360.0;
    hsv(hue, 0.6, 0.9)
}

fn hsv(h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    match (i as i32).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

impl View {
    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.roll.set_draw_func(move |a, cr, w, h| {
            if let Some(v) = weak.upgrade() {
                v.draw(a, cr, f64::from(w), f64::from(h));
            }
        });
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |_, _, x, y| {
            if let Some(v) = weak.upgrade() {
                v.pick(x, y);
            }
        });
        self.roll.add_controller(click);
        let weak = Rc::downgrade(self);
        self.span.connect_selected_notify(move |d| {
            if let Some(v) = weak.upgrade()
                && !v.syncing.get()
            {
                let s = SPANS[d.selected() as usize];
                v.doc.edit_audio(|a| a.timeline_span = s);
            }
        });
    }

    /// The first frame shown, the frames across, and the frame centred.
    fn window(&self) -> (u64, u64, u64) {
        let a = self.doc.audio();
        let (now, span) = (a.timeline_now(), a.timeline_span);
        (now.saturating_sub(span / 2), span, now)
    }

    fn refresh(self: &Rc<Self>) {
        if self.doc.audio_tab() != Some(Tab::Timeline) {
            return;
        }
        self.doc.load_notes();
        let a = self.doc.audio();
        let ons = a
            .notes()
            .iter()
            .filter(|n| n.kind == romlens_ffi::NoteKindInfo::On)
            .count();
        self.summary.set_text(&if a.notes_loading() {
            "Reading the notes…".to_owned()
        } else if a.source == Source::Recording {
            format!("{ons} notes keyed on in the recording, from its DSP writes")
        } else {
            format!(
                "{ons} notes the ROM's driver has played in Romlens since it started{}",
                if ons == 0 {
                    ": send it a command on the ports"
                } else {
                    ""
                }
            )
        });
        self.syncing.set(true);
        self.span.set_selected(
            SPANS
                .iter()
                .position(|s| *s == a.timeline_span)
                .unwrap_or(2) as u32,
        );
        self.syncing.set(false);
        let detail_key = format!(
            "{:?}|{:?}",
            a.selected_note.as_ref().map(|n| (n.voice, n.spc_cycle)),
            a.note_source().map(|s| (s.spc_pc, s.commands.len()))
        );
        drop(a);
        self.roll.queue_draw();
        if *self.detail_key.borrow() != detail_key {
            *self.detail_key.borrow_mut() = detail_key;
            self.show_detail();
        }
        self.show_song();
    }

    fn spans_in_view(&self) -> Vec<NoteSpan> {
        let (from, span, now) = self.window();
        let a = self.doc.audio();
        note_spans(a.notes(), now.max(from + span))
            .into_iter()
            .filter(|s| s.end >= from && s.on.frame <= from + span)
            .collect()
    }

    fn draw(&self, a: &gtk::DrawingArea, cr: &cairo::Context, w: f64, h: f64) {
        let (from, span, now) = self.window();
        let lane = h / 8.0;
        let x = |f: u64| (f as f64 - from as f64) / span as f64 * w;
        let fg = a.color();
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.25);
        cr.rectangle(0.0, 0.0, w, h);
        let _ = cr.fill();
        for v in (1..8).step_by(2) {
            fill_rect(cr, 0.0, v as f64 * lane, w, lane, &with_alpha(&fg, 0.05));
        }
        let selected = self.doc.audio().selected_note.clone();
        for s in self.spans_in_view() {
            let y = f64::from(s.on.voice) * lane + pitch_y(s.on.pitch) * (lane - 6.0);
            let is_selected = selected
                .as_ref()
                .is_some_and(|n| n.spc_cycle == s.on.spc_cycle && n.voice == s.on.voice);
            if is_selected {
                cr.set_source_rgb(1.0, 1.0, 1.0);
            } else {
                let (r, g, b) = sample_colour(s.on.source);
                cr.set_source_rgb(r, g, b);
            }
            cr.rectangle(x(s.on.frame), y, (x(s.end) - x(s.on.frame)).max(2.0), 5.0);
            let _ = cr.fill();
        }
        let nx = x(now);
        crate::canvas::set_source(cr, &accent());
        cr.set_line_width(1.0);
        cr.move_to(nx, 0.0);
        cr.line_to(nx, h);
        let _ = cr.stroke();
        for v in 0..8 {
            let l = mono_layout(a, 8.0, &v.to_string());
            text(cr, &l, 3.0, v as f64 * lane + 2.0, &with_alpha(&fg, 0.6));
        }
    }

    fn pick(&self, x: f64, y: f64) {
        let (from, span, _) = self.window();
        let (w, h) = (
            f64::from(self.roll.width()).max(1.0),
            f64::from(self.roll.height()).max(1.0),
        );
        let voice = (y / (h / 8.0)) as u8;
        let f = from as f64 + x / w * span as f64;
        let hit = self
            .spans_in_view()
            .into_iter()
            .filter(|s| {
                s.on.voice == voice && s.on.frame as f64 <= f + 2.0 && s.end as f64 >= f - 2.0
            })
            .min_by(|a, b| {
                (a.on.frame as f64 - f)
                    .abs()
                    .partial_cmp(&(b.on.frame as f64 - f).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|s| s.on);
        self.doc.edit_audio(|a| a.select_note(hit));
    }

    // MARK: The note selected

    fn show_detail(&self) {
        while let Some(c) = self.detail.first_child() {
            self.detail.remove(&c);
        }
        let a = self.doc.audio();
        let Some(n) = a.selected_note.clone() else {
            let l = caption(
                "Click a note to see the DSP write that started it, the SPC700 instruction that \
                 made the write, and the S-CPU's request before it.",
            );
            l.set_wrap(true);
            self.detail.append(&l);
            return;
        };
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let t = heading_label(&format!("Voice {}: keyed on at frame {}", n.voice, n.frame));
        t.set_hexpand(true);
        head.append(&t);
        if a.source == Source::Recording {
            let b = gtk::Button::with_label("Go to Frame");
            let (doc, frame) = (Rc::clone(&self.doc), n.frame);
            b.connect_clicked(move |_| doc.set_frame(frame));
            head.append(&b);
        }
        self.detail.append(&head);
        let name = a
            .state()
            .and_then(|s| {
                s.samples
                    .iter()
                    .find(|s| s.index == n.source)
                    .and_then(|s| s.tuning_hz)
            })
            .map(|hz| romlens_ffi::note_for_frequency(hz * f64::from(n.pitch) / 4096.0));
        self.detail.append(
            &gtk::Label::builder()
                .label(format!(
                    "pitch {}, sample {}{}",
                    dollar(n.pitch, 4),
                    n.source,
                    name.map_or_else(String::new, |n| format!(", ≈ {n}"))
                ))
                .xalign(0.0)
                .build(),
        );
        if let Some(pc) = a.note_source().and_then(|s| s.spc_pc) {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&gtk::Label::new(Some(&format!(
                "Written by the SPC700 at {}: its KON write to the DSP.",
                dollar(pc, 4)
            ))));
            let b = gtk::Button::with_label("Show in Audio RAM");
            b.add_css_class("flat");
            let doc = Rc::clone(&self.doc);
            b.connect_clicked(move |_| {
                doc.edit_audio(|a| a.show_spc(pc));
                doc.open_audio(Tab::Aram);
            });
            row.append(&b);
            self.detail.append(&row);
        } else if a.source == Source::Recording && a.note_source().is_some() {
            self.detail.append(&caption(
                "The replay could not place the instruction that wrote it.",
            ));
        }
        if let Some(src) = a.note_source()
            && !src.commands.is_empty()
        {
            self.detail.append(
                &gtk::Label::builder()
                    .label("What the S-CPU last asked on each port before it:")
                    .xalign(0.0)
                    .build(),
            );
            for c in &src.commands {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let l = gtk::Label::new(Some(&format!(
                    "port {} = {} at frame {}",
                    c.port,
                    dollar(c.value, 2),
                    c.frame
                )));
                l.add_css_class("monospace");
                row.append(&l);
                for site in a.command_sites(c.port, c.value).into_iter().take(3) {
                    let b = gtk::Button::with_label(&format!(
                        "sent at {}{}",
                        romlens_ffi::format_snes_address(site.routine),
                        site.via.map_or_else(String::new, |v| format!(
                            " via {}",
                            romlens_ffi::format_snes_address(v)
                        ))
                    ));
                    b.add_css_class("flat");
                    b.add_css_class("caption");
                    b.set_tooltip_text(Some(
                        "The 65816 code that stores this value, found by tracing it",
                    ));
                    let (doc, at) = (Rc::clone(&self.doc), site.at);
                    b.connect_clicked(move |_| show_in_rom(&doc, at));
                    row.append(&b);
                }
                self.detail.append(&row);
            }
        }
    }

    // MARK: The song

    fn show_song(&self) {
        let a = self.doc.audio();
        let state = a.state();
        let nspc = state.as_ref().and_then(|s| s.nspc.clone());
        let key = format!(
            "{:?}|{:?}|{}|{:?}|{}",
            nspc.as_ref().map(|d| (
                &d.dialect,
                d.songs.len(),
                d.playing.as_ref().map(|p| (p.song, p.block))
            )),
            a.chosen_song,
            a.song_voice,
            nspc.as_ref()
                .and_then(|d| d.playing.as_ref())
                .map(|p| p.positions.clone()),
            a.source_description()
        );
        if *self.song_key.borrow() == key {
            return;
        }
        *self.song_key.borrow_mut() = key;
        while let Some(c) = self.song.first_child() {
            self.song.remove(&c);
        }
        let Some(d) = nspc else {
            self.song.append(&heading_label("Song"));
            let l = caption(
                "The driver is not Nintendo's N-SPC, so its songs are its own data. The piano roll \
                 reads the DSP, so it works for any driver.",
            );
            l.set_wrap(true);
            self.song.append(&l);
            return;
        };
        drop(a);
        self.fill_song(&d);
    }

    fn fill_song(&self, d: &NspcDriverInfo) {
        let (chosen, voice) = {
            let a = self.doc.audio();
            (a.chosen_song, a.song_voice)
        };
        let playing = d.playing.as_ref();
        let number = chosen.or(playing.and_then(|p| p.song)).unwrap_or(1);
        let list = self.doc.audio().nspc_song(number);
        let is_playing = playing.is_some_and(|p| p.song == Some(number));
        let block = if is_playing {
            playing.map(|p| p.block)
        } else {
            list.iter()
                .find(|e| e.kind == NspcEntryKind::Block)
                .map(|e| e.block)
        };
        let tracks: Vec<u16> = list
            .iter()
            .find(|e| Some(e.block) == block)
            .map(|e| e.tracks.clone())
            .unwrap_or_default();
        let track = tracks.get(voice).copied().unwrap_or(0);
        let now = if is_playing {
            playing.and_then(|p| p.positions.get(voice).copied().flatten())
        } else {
            None
        };
        let events = if track == 0 {
            Vec::new()
        } else {
            self.doc.audio().nspc_track(track)
        };

        self.song.append(&heading_label("Song"));
        let about = caption(&format!(
            "{}: its table of command lengths at {}{}",
            d.dialect,
            dollar(d.lengths_at, 4),
            d.song_table.map_or_else(String::new, |t| format!(
                ", {} songs at {}",
                d.songs.len(),
                dollar(t, 4)
            ))
        ));
        about.set_wrap(true);
        self.song.append(&about);

        let pick = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let names: Vec<String> = d
            .songs
            .iter()
            .enumerate()
            .map(|(i, s)| {
                format!(
                    "{} at {}{}",
                    i + 1,
                    dollar(*s, 4),
                    if playing.is_some_and(|p| p.song == Some(i as u8 + 1)) {
                        " (playing)"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let songs = gtk::DropDown::from_strings(&refs);
        songs.set_selected(u32::from(number.saturating_sub(1)));
        songs.set_tooltip_text(Some(
            "Song n is what a game sends the driver to play it: entry n - 1 of the table",
        ));
        let doc = Rc::clone(&self.doc);
        songs.connect_selected_notify(move |s| {
            let n = s.selected() as u8 + 1;
            if doc.audio().chosen_song != Some(n) {
                doc.edit_audio(|a| a.chosen_song = Some(n));
            }
        });
        let voice_names: Vec<String> = (0..8)
            .map(|v| {
                format!(
                    "Voice {v}{}",
                    if tracks.get(v).is_some_and(|t| *t != 0) {
                        ""
                    } else {
                        " (silent)"
                    }
                )
            })
            .collect();
        let vrefs: Vec<&str> = voice_names.iter().map(String::as_str).collect();
        let voices = gtk::DropDown::from_strings(&vrefs);
        voices.set_selected(voice as u32);
        let doc = Rc::clone(&self.doc);
        voices.connect_selected_notify(move |s| {
            let v = s.selected() as usize;
            if doc.audio().song_voice != v {
                doc.edit_audio(|a| a.song_voice = v);
            }
        });
        pick.append(&songs);
        pick.append(&voices);
        self.song.append(&pick);

        let entries = gtk::Box::new(gtk::Orientation::Vertical, 1);
        for e in &list {
            let l = gtk::Label::builder()
                .label(entry_text(e))
                .xalign(0.0)
                .build();
            l.add_css_class("monospace");
            l.add_css_class("caption");
            if Some(e.block) == block && e.kind == NspcEntryKind::Block {
                l.add_css_class("accent");
            }
            entries.append(&l);
        }
        self.song.append(
            &gtk::ScrolledWindow::builder()
                .max_content_height(120)
                .propagate_natural_height(true)
                .child(&entries)
                .build(),
        );
        self.song
            .append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        self.song.append(&caption(&if track == 0 {
            format!("Voice {voice} is silent in this block.")
        } else {
            format!(
                "Voice {voice}'s track at {}{}",
                dollar(track, 4),
                now.map_or_else(String::new, |n| format!(", reading {} now", dollar(n, 4)))
            )
        }));
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for e in &events {
            let here = now
                .is_some_and(|n| n >= e.at && usize::from(n) < usize::from(e.at) + e.bytes.len());
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let mark = gtk::Label::new(Some(if here { "▶" } else { " " }));
            mark.add_css_class("success");
            let bytes = e
                .bytes
                .iter()
                .map(|b| hex(*b, 2))
                .collect::<Vec<_>>()
                .join(" ");
            for (t, dim, w) in [
                (dollar(e.at, 4), true, 0),
                (bytes, true, 10),
                (e.text.clone(), e.kind != "note", 0),
            ] {
                let l = gtk::Label::builder()
                    .label(&t)
                    .xalign(0.0)
                    .width_chars(w)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build();
                l.add_css_class("monospace");
                l.add_css_class("caption");
                if dim {
                    l.add_css_class("dim-label");
                }
                if t.is_empty() {
                    continue;
                }
                if row.first_child().is_none() {
                    row.append(&mark);
                }
                row.append(&l);
            }
            rows.append(&row);
        }
        let scroll = gtk::ScrolledWindow::builder()
            .min_content_height(200)
            .vexpand(true)
            .child(&rows)
            .build();
        self.song.append(&scroll);
    }
}

fn entry_text(e: &NspcEntryInfo) -> String {
    let at = dollar(e.at, 4);
    match e.kind {
        NspcEntryKind::Block => {
            let voices: Vec<String> = e
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| **t != 0)
                .map(|(i, _)| i.to_string())
                .collect();
            format!(
                "{at}  block {}  voices {}",
                dollar(e.block, 4),
                voices.join(",")
            )
        }
        NspcEntryKind::Repeat => format!("{at}  back to {}, {} more", dollar(e.to, 4), e.count),
        NspcEntryKind::Jump => format!("{at}  back to {} for ever", dollar(e.to, 4)),
        NspcEntryKind::End => format!("{at}  end"),
    }
}
