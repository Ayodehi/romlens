//! The Tutor (docs/24, docs/29): the conversation, then the composer, then a
//! status line. It lives in the inspector's drawer, or in a tab of the
//! window; there is no Tutor window any more. The macOS twin is `TutorView`, `TranscriptView`, `ComposerArea`, `EditCard`
//! and `StatusLine`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use romlens_ffi::tutor::session::ProposalInfo;

use crate::gfxdraw::caption;
use crate::messageview::{self, OnLink};
use crate::model::tutor::{self, CardState, Live, Row, Sheet, ToolRow, TutorModel};
use crate::model::tutor_settings::ModePreference;
use crate::model::{Change, Document};
use crate::tutorsheets;

thread_local! {
    /// Every tutor view open, for a setting changed in Settings.
    static VIEWS: RefCell<Vec<std::rc::Weak<View>>> = const { RefCell::new(Vec::new()) };
}

/// The tutor for a project's window: in the inspector's drawer, or as a tab
/// (docs/29). Several views can show one conversation, which is the
/// document's; each is made when it first shows.
pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let view = View::build(doc);
    // The widget keeps its view, and the view the document.
    view.root.connect_destroy({
        let view = Rc::clone(&view);
        move |_| {
            let _ = &view;
        }
    });
    VIEWS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|x| x.strong_count() > 0);
        v.push(Rc::downgrade(&view));
    });
    view.root.clone().upcast()
}

/// A Tutor setting changed: every open Tutor shows it.
pub fn settings_changed() {
    let views: Vec<Rc<View>> = VIEWS.with(|v| {
        v.borrow()
            .iter()
            .filter_map(std::rc::Weak::upgrade)
            .collect()
    });
    for v in views {
        // Forces the transcript to be built again, and the status line.
        v.history_key.borrow_mut().clear();
        v.doc.edit_tutor(|_| {});
    }
}

struct View {
    doc: Rc<Document>,
    root: adw::ToolbarView,
    scroll: gtk::ScrolledWindow,
    history_box: gtk::Box,
    live_box: gtk::Box,
    notes: gtk::Box,
    composer: gtk::TextView,
    commands: gtk::Box,
    chip: gtk::ToggleButton,
    attachments: gtk::Box,
    frame_button: gtk::Button,
    placeholder: gtk::Label,
    send: gtk::Button,
    status: Status,
    banner: gtk::Label,
    title: adw::WindowTitle,
    /// What the history was built from.
    history_key: RefCell<String>,
    live_scheduled: Cell<bool>,
    following: Cell<bool>,
    programmatic: Cell<bool>,
    last_escape: Cell<Option<std::time::Instant>>,
    on_link: RefCell<Option<OnLink>>,
    sheet_open: Cell<bool>,
}

struct Status {
    model: gtk::Button,
    mode: gtk::Button,
    context: gtk::Label,
    cost: gtk::Label,
    due: gtk::Button,
    xp: gtk::Button,
    conversations: gtk::Button,
    lessons: gtk::Button,
    explain: gtk::ToggleButton,
    details: gtk::ToggleButton,
    stop: gtk::Button,
}

fn flat(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("flat");
    b.add_css_class("caption");
    b
}

fn icon_toggle(icon: &str, tip: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.set_icon_name(icon);
    b.add_css_class("flat");
    b.set_tooltip_text(Some(tip));
    b
}

impl View {
    fn build(doc: &Rc<Document>) -> Rc<Self> {
        // The transcript.
        let history_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(14)
            .build();
        let live_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .build();
        let notes = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(14)
            .margin_start(14)
            .margin_end(14)
            .margin_top(14)
            .margin_bottom(14)
            .build();
        content.set_valign(gtk::Align::Start);
        content.append(&history_box);
        content.append(&live_box);
        content.append(&notes);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&content)
            .build();

        // The composer.
        let composer = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(10)
            .right_margin(10)
            .build();
        composer.set_tooltip_text(Some("Ask the tutor"));
        let composer_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(40)
            .max_content_height(130)
            .propagate_natural_height(true)
            .hexpand(true)
            .child(&composer)
            .build();
        composer_scroll.add_css_class("card");
        let placeholder = gtk::Label::builder()
            .label("Ask about the ROM, or / for commands")
            .xalign(0.0)
            .can_target(false)
            .margin_start(11)
            .margin_top(9)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .build();
        placeholder.add_css_class("dim-label");
        let field = gtk::Overlay::new();
        field.set_child(Some(&composer_scroll));
        field.add_overlay(&placeholder);
        field.set_hexpand(true);
        let send = gtk::Button::from_icon_name("go-up-symbolic");
        send.add_css_class("suggested-action");
        send.add_css_class("circular");
        send.set_valign(gtk::Align::End);
        send.set_tooltip_text(Some("Send (Return)"));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&field);
        row.append(&send);

        let commands = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .build();
        commands.add_css_class("card");
        commands.set_visible(false);
        let chip = gtk::ToggleButton::new();
        chip.add_css_class("pill");
        chip.add_css_class("caption");
        chip.add_css_class("monospace");
        let frame_button = flat("Frame");
        frame_button.set_tooltip_text(Some("Attach the recording's frame as a picture"));
        let attachments = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chips.append(&chip);
        chips.append(&frame_button);
        chips.append(&attachments);
        let area = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_start(10)
            .margin_end(10)
            .margin_top(8)
            .margin_bottom(4)
            .build();
        area.append(&commands);
        area.append(&chips);
        area.append(&row);

        // The status line.
        let status = Status {
            model: flat(""),
            mode: flat(""),
            context: caption(""),
            cost: caption(""),
            due: flat(""),
            xp: flat(""),
            conversations: {
                let b = flat("");
                b.set_icon_name("document-open-recent-symbolic");
                b.set_tooltip_text(Some("Go back to an earlier conversation (/resume)"));
                b
            },
            lessons: {
                let b = flat("");
                b.set_icon_name("accessories-dictionary-symbolic");
                b.set_tooltip_text(Some(
                    "Your lessons, and what you have learned (/lessons, /map)",
                ));
                b
            },
            explain: icon_toggle(
                "view-reveal-symbolic",
                "Answer with lessons, as deep as you have got (/explain, /learn <topic>)",
            ),
            details: icon_toggle(
                "view-list-symbolic",
                "Show the tutor's thinking and tool calls (/details)",
            ),
            stop: {
                let b = gtk::Button::with_label("Stop");
                b.add_css_class("destructive-action");
                b
            },
        };
        let bar = gtk::Box::builder()
            .spacing(6)
            .margin_start(10)
            .margin_end(10)
            .margin_bottom(6)
            .build();
        let spacer = gtk::Box::builder().hexpand(true).build();
        for w in [
            status.model.upcast_ref::<gtk::Widget>(),
            status.mode.upcast_ref(),
            status.context.upcast_ref(),
            status.cost.upcast_ref(),
            spacer.upcast_ref(),
            status.due.upcast_ref(),
            status.xp.upcast_ref(),
            status.conversations.upcast_ref(),
            status.lessons.upcast_ref(),
            status.explain.upcast_ref(),
            status.details.upcast_ref(),
            status.stop.upcast_ref(),
        ] {
            bar.append(w);
        }
        // The banner for what was just earned.
        let banner = gtk::Label::builder()
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Start)
            .margin_top(10)
            .visible(false)
            .build();
        banner.add_css_class("osd");
        banner.add_css_class("heading");
        banner.add_css_class("pill");

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&scroll);
        body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        body.append(&area);
        body.append(&bar);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&body));
        overlay.add_overlay(&banner);

        let title = adw::WindowTitle::new("Tutor", "");
        // Inside the window, not a window: no window buttons.
        let header = adw::HeaderBar::builder()
            .show_start_title_buttons(false)
            .show_end_title_buttons(false)
            .build();
        header.set_title_widget(Some(&title));
        let new = gtk::Button::from_icon_name("list-add-symbolic");
        new.set_tooltip_text(Some("New conversation (/new)"));
        header.pack_start(&new);
        let help = gtk::Button::from_icon_name("help-about-symbolic");
        help.set_tooltip_text(Some("What the tutor can do and the keys it takes (/help)"));
        header.pack_end(&help);
        let root = adw::ToolbarView::new();
        root.add_top_bar(&header);
        root.set_content(Some(&overlay));

        let view = Rc::new(Self {
            doc: Rc::clone(doc),
            root,
            scroll,
            history_box,
            live_box,
            notes,
            composer,
            commands,
            chip,
            attachments,
            frame_button,
            placeholder,
            send,
            status,
            banner,
            title,
            history_key: RefCell::new(String::new()),
            live_scheduled: Cell::new(false),
            following: Cell::new(true),
            programmatic: Cell::new(false),
            last_escape: Cell::new(None),
            on_link: RefCell::new(None),
            sheet_open: Cell::new(false),
        });
        view.wire(&new, &help);
        // Made on first use, so a project that never opens the tutor costs
        // nothing; then the transcript.
        doc.tutor_session();
        doc.edit_tutor(|t| {
            t.refresh();
            t.refresh_progress();
        });
        view.refresh(true);
        view
    }

    fn link_handler(self: &Rc<Self>) -> OnLink {
        if let Some(h) = self.on_link.borrow().as_ref() {
            return Rc::clone(h);
        }
        let weak = Rc::downgrade(self);
        let h: OnLink = Rc::new(move |label, uri| {
            let Some(v) = weak.upgrade() else {
                return false;
            };
            if let Some(entry) = crate::model::markdown::entry_for(uri) {
                crate::glossary::show(label, entry, {
                    let weak = Rc::downgrade(&v);
                    Box::new(move |term| {
                        if let Some(v) = weak.upgrade() {
                            v.set_composer(&format!("Tell me more about {term}."));
                        }
                    })
                });
                return true;
            }
            messageview::follow(&v.doc, uri)
        });
        *self.on_link.borrow_mut() = Some(Rc::clone(&h));
        h
    }

    fn set_composer(&self, text: &str) {
        self.composer.buffer().set_text(text);
        self.composer.grab_focus();
    }

    fn composer_text(&self) -> String {
        let b = self.composer.buffer();
        b.text(&b.start_iter(), &b.end_iter(), false).to_string()
    }

    // MARK: Wiring

    fn wire(self: &Rc<Self>, new: &gtk::Button, help: &gtk::Button) {
        let doc = Rc::clone(&self.doc);
        new.connect_clicked({
            let doc = Rc::clone(&doc);
            move |_| doc.tutor_submit("/new")
        });
        help.connect_clicked({
            let doc = Rc::clone(&doc);
            move |_| doc.tutor_submit("/help")
        });
        let weak = Rc::downgrade(self);
        self.send.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.submit();
            }
        });

        // The composer's keys.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(v) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            v.key(key, state)
        });
        self.composer.add_controller(keys);
        let weak = Rc::downgrade(self);
        self.composer.buffer().connect_changed(move |_| {
            if let Some(v) = weak.upgrade() {
                v.composer_changed();
            }
        });
        // A picture on the clipboard is attached; text pastes as text.
        let paste = gtk::ShortcutController::new();
        let weak = Rc::downgrade(self);
        paste.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("<Control>v"),
            Some(gtk::CallbackAction::new(move |_, _| {
                weak.upgrade().is_some_and(|v| v.paste_picture()).into()
            })),
        ));
        paste.set_propagation_phase(gtk::PropagationPhase::Capture);
        self.composer.add_controller(paste);

        // Drop a picture or a file onto the window.
        let drop = gtk::DropTarget::new(gio::File::static_type(), gdk::DragAction::COPY);
        let weak = Rc::downgrade(self);
        drop.connect_drop(move |_, value, _, _| {
            let (Some(v), Ok(file)) = (weak.upgrade(), value.get::<gio::File>()) else {
                return false;
            };
            v.attach_file(&file)
        });
        self.root.add_controller(drop);

        // The selection chip, the frame, and what is attached.
        let doc = Rc::clone(&self.doc);
        self.chip.connect_toggled(move |b| {
            let on = b.is_active();
            if doc.tutor().include_selection != on {
                doc.edit_tutor(|t| t.include_selection = on);
            }
        });
        let doc = Rc::clone(&self.doc);
        self.frame_button
            .connect_clicked(move |_| doc.tutor_attach_frame());

        // The status line.
        let weak = Rc::downgrade(self);
        self.status.model.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.doc.tutor_submit("/model");
            }
        });
        let doc = Rc::clone(&self.doc);
        self.status.mode.connect_clicked(move |_| {
            doc.edit_tutor(|t| t.cycle_mode());
        });
        let doc = Rc::clone(&self.doc);
        self.status
            .due
            .connect_clicked(move |_| doc.tutor_submit("/review"));
        let doc = Rc::clone(&self.doc);
        self.status
            .xp
            .connect_clicked(move |_| doc.tutor_submit("/progress"));
        let doc = Rc::clone(&self.doc);
        self.status
            .conversations
            .connect_clicked(move |_| doc.tutor_submit("/resume"));
        let doc = Rc::clone(&self.doc);
        self.status
            .lessons
            .connect_clicked(move |_| doc.tutor_submit("/lessons"));
        let doc = Rc::clone(&self.doc);
        self.status.explain.connect_clicked(move |b| {
            let on = b.is_active();
            doc.edit_tutor(|t| {
                if t.start_if_needed() {
                    t.set_explain(on);
                }
            });
        });
        let doc = Rc::clone(&self.doc);
        self.status.details.connect_clicked(move |b| {
            let on = b.is_active();
            crate::settings::tutor()
                .borrow_mut()
                .edit(|s| s.show_work = on);
            doc.edit_tutor(|_| {});
        });
        let doc = Rc::clone(&self.doc);
        self.status
            .stop
            .connect_clicked(move |_| doc.tutor().stop());

        // Follow the stream until the person scrolls up, and again once they
        // scroll back to the end.
        let adj = self.scroll.vadjustment();
        let weak = Rc::downgrade(self);
        let last = Cell::new(0.0f64);
        adj.connect_value_changed(move |a| {
            let Some(v) = weak.upgrade() else { return };
            if v.programmatic.get() {
                last.set(a.value());
                return;
            }
            let at_end = a.value() + a.page_size() >= a.upper() - 40.0;
            if a.value() < last.get() - 1.0 {
                v.following.set(false);
            } else if a.value() > last.get() + 1.0 && at_end {
                v.following.set(true);
            }
            last.set(a.value());
        });
        let weak = Rc::downgrade(self);
        adj.connect_upper_notify(move |_| {
            if let Some(v) = weak.upgrade()
                && v.following.get()
            {
                v.to_end();
            }
        });

        let weak = Rc::downgrade(self);
        self.doc.subscribe(move |c| {
            if matches!(c, Change::Tutor | Change::Graphics | Change::Selection)
                && let Some(v) = weak.upgrade()
            {
                v.refresh(c == Change::Tutor);
            }
        });
    }

    fn to_end(&self) {
        let a = self.scroll.vadjustment();
        self.programmatic.set(true);
        a.set_value(a.upper() - a.page_size());
        self.programmatic.set(false);
    }

    // MARK: The composer

    fn submit(self: &Rc<Self>) {
        let text = self.composer_text();
        let t = text.trim();
        if t.is_empty() && self.doc.tutor().attachments.is_empty() {
            return;
        }
        if !t.starts_with('/') && !self.doc.tutor().can_send(t) {
            return;
        }
        self.composer.buffer().set_text("");
        self.following.set(true);
        self.doc.tutor_submit(t);
        // /rewind's words and /learn's topic come back through the model.
    }

    fn key(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let text = self.composer_text();
        let buffer = self.composer.buffer();
        let at = buffer.iter_at_mark(&buffer.get_insert());
        match key {
            gdk::Key::Return | gdk::Key::KP_Enter
                if !shift && !state.contains(gdk::ModifierType::ALT_MASK) =>
            {
                self.submit();
                return glib::Propagation::Stop;
            }
            gdk::Key::Up if at.line() == 0 => {
                if let Some(t) = self.doc.edit_tutor(|t| t.history_up(&text)) {
                    self.set_composer(&t);
                    return glib::Propagation::Stop;
                }
            }
            gdk::Key::Down if at.line() == buffer.line_count() - 1 => {
                if let Some(t) = self.doc.edit_tutor(|t| t.history_down()) {
                    self.set_composer(&t);
                    return glib::Propagation::Stop;
                }
            }
            gdk::Key::Tab if !shift => {
                if let Some(c) = TutorModel::matching_commands(&text).first() {
                    self.set_composer(&format!("{} ", c.name));
                    return glib::Propagation::Stop;
                }
            }
            gdk::Key::ISO_Left_Tab | gdk::Key::Tab if shift => {
                self.doc.edit_tutor(|t| t.cycle_mode());
                return glib::Propagation::Stop;
            }
            gdk::Key::Escape => {
                if self.doc.tutor().busy {
                    self.doc.tutor().stop();
                } else if self
                    .last_escape
                    .get()
                    .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(600))
                {
                    self.doc.tutor_submit("/rewind");
                } else if !text.is_empty() && !TutorModel::matching_commands(&text).is_empty() {
                    self.composer.buffer().set_text("");
                }
                self.last_escape.set(Some(std::time::Instant::now()));
                return glib::Propagation::Stop;
            }
            _ => {}
        }
        glib::Propagation::Proceed
    }

    fn composer_changed(&self) {
        let text = self.composer_text();
        self.placeholder.set_visible(text.is_empty());
        while let Some(c) = self.commands.first_child() {
            self.commands.remove(&c);
        }
        let found = TutorModel::matching_commands(&text);
        self.commands.set_visible(!found.is_empty());
        for c in found {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let name = gtk::Label::new(Some(c.name));
            name.add_css_class("monospace");
            row.append(&name);
            row.append(&caption(c.about));
            let b = gtk::Button::builder().child(&row).build();
            b.add_css_class("flat");
            let weak = self.composer.downgrade();
            let name = c.name;
            b.connect_clicked(move |_| {
                if let Some(c) = weak.upgrade() {
                    c.buffer().set_text(&format!("{name} "));
                    c.grab_focus();
                }
            });
            self.commands.append(&b);
        }
        // Walking back through the history ends when the person edits.
        let _ = self.doc.tutor().walking_history();
    }

    // MARK: Pictures

    fn paste_picture(self: &Rc<Self>) -> bool {
        let clipboard = self.composer.clipboard();
        let formats = clipboard.formats();
        if formats.contains_type(glib::GString::static_type())
            || !formats.contains_type(gdk::Texture::static_type())
        {
            return false;
        }
        let weak = Rc::downgrade(self);
        clipboard.read_texture_async(gio::Cancellable::NONE, move |t| {
            if let (Some(v), Ok(Some(texture))) = (weak.upgrade(), t) {
                let png = texture.save_to_png_bytes();
                v.doc
                    .edit_tutor(|m| m.attach(fit(&png), "image/png", "Pasted picture"));
            }
        });
        true
    }

    fn attach_file(self: &Rc<Self>, file: &gio::File) -> bool {
        let Some(path) = file.path() else {
            return false;
        };
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let Ok(data) = std::fs::read(&path) else {
            return false;
        };
        let media = match name.rsplit('.').next().map(str::to_lowercase).as_deref() {
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            _ => {
                // Other pictures (TIFF, BMP...) are turned into PNG.
                return match gdk::Texture::from_file(file) {
                    Ok(t) => {
                        let png = t.save_to_png_bytes();
                        self.doc
                            .edit_tutor(|m| m.attach(fit(&png), "image/png", &name));
                        true
                    }
                    Err(_) => {
                        self.doc
                            .edit_tutor(|m| m.error = Some(format!("{name} is not a picture.")));
                        false
                    }
                };
            }
        };
        let (data, media) = fit_media(data, media);
        self.doc.edit_tutor(|m| m.attach(data, media, &name));
        true
    }

    // MARK: Updating

    fn refresh(self: &Rc<Self>, tutor_changed: bool) {
        let t = self.doc.tutor();
        // The composer's state.
        self.placeholder.set_label(if t.busy {
            "Esc stops it"
        } else {
            "Ask about the ROM, or / for commands"
        });
        self.send.set_sensitive(t.can_send(&self.composer_text()));
        self.chip_update(&t);
        self.attachments_update(&t);
        self.status_update(&t);
        self.title.set_subtitle(t.title.as_deref().unwrap_or(""));
        // The banner for what was just earned.
        match &t.banner {
            Some(b) => {
                let mut text = b.lines.join("\n");
                if b.points > 0 {
                    text += &format!("   +{}", b.points);
                }
                self.banner.set_label(&text);
                self.banner.set_visible(true);
            }
            None => self.banner.set_visible(false),
        }
        drop(t);
        if tutor_changed {
            self.history();
            self.schedule_live();
            self.notes_update();
        }
        if self.doc.tutor().sheet.is_some() && !self.sheet_open.get() {
            self.show_sheet();
        }
        if self.following.get() {
            self.to_end();
        }
    }

    fn chip_update(&self, t: &TutorModel) {
        let chip = self.doc.selected_address().map(|a| {
            let name = self
                .doc
                .details()
                .label
                .map_or_else(String::new, |l| format!(" {}", l.name));
            format!("{}{name}", tutor::address(a))
        });
        self.chip.set_visible(chip.is_some());
        if let Some(c) = chip {
            self.chip.set_label(&c);
            if self.chip.is_active() != t.include_selection {
                self.chip.set_active(t.include_selection);
            }
            self.chip.set_tooltip_text(Some(if t.include_selection {
                "The question goes with this selection; click to leave it out"
            } else {
                "Sent without the selection; click to send it"
            }));
        }
        let has = self.doc.graphics().has_recording();
        self.frame_button.set_visible(has);
        self.frame_button
            .set_label(&format!("Frame {}", self.doc.graphics().frame()));
    }

    fn attachments_update(&self, t: &TutorModel) {
        while let Some(c) = self.attachments.first_child() {
            self.attachments.remove(&c);
        }
        for (i, a) in t.attachments.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            if let Ok(tex) = gdk::Texture::from_bytes(&glib::Bytes::from(&a.data)) {
                let pic = gtk::Picture::for_paintable(&tex);
                pic.set_height_request(22);
                pic.set_content_fit(gtk::ContentFit::ScaleDown);
                row.append(&pic);
            }
            let x = gtk::Button::from_icon_name("window-close-symbolic");
            x.add_css_class("flat");
            x.add_css_class("circular");
            row.set_tooltip_text(Some(&a.name));
            let doc = Rc::clone(&self.doc);
            x.connect_clicked(move |_| {
                doc.edit_tutor(|t| {
                    if i < t.attachments.len() {
                        t.attachments.remove(i);
                    }
                });
            });
            row.append(&x);
            self.attachments.append(&row);
        }
    }

    fn status_update(&self, t: &TutorModel) {
        let s = &self.status;
        let settings = crate::settings::tutor();
        let st = settings.borrow();
        let default = st.default_endpoint();
        let model = t
            .model_name
            .clone()
            .or_else(|| st.model(&default))
            .unwrap_or_default();
        let endpoint = t.endpoint_name.clone().unwrap_or(default.name);
        s.model.set_label(&format!("{endpoint} · {model}"));
        s.model
            .set_tooltip_text(Some("Change the provider or model (/model)"));
        s.mode.set_label(t.mode.title());
        s.mode
            .set_tooltip_text(Some("What the tutor may change: Shift+Tab cycles"));
        s.context.set_visible(t.context_used > 0.0);
        s.context
            .set_text(&format!("context {}%", (t.context_used * 100.0) as i32));
        s.context.set_tooltip_text(Some(
            "How full the model's context was at the last answer; past 80% it summarises first",
        ));
        // A cost that rounds to nothing is shown as nothing, never $-0.000.
        s.cost.set_text(&format!("${:.3}", t.cost.max(0.0)));
        s.cost
            .set_tooltip_text(Some("What this conversation has cost"));
        let due = t.due_count();
        s.due.set_visible(due > 0);
        s.due.set_label(&format!("{due} to review"));
        s.due
            .set_tooltip_text(Some(&format!("{due} ready to review (/review)")));
        s.xp.set_visible(st.stored().show_progress && t.progress.is_some());
        if let Some(p) = &t.progress {
            s.xp.set_label(&format!("{} XP", p.xp));
            s.xp.set_tooltip_text(Some(&format!(
                "{}: your points, rank and achievements (/progress)",
                p.rank
            )));
        }
        s.conversations.set_sensitive(!t.busy);
        if s.explain.is_active() != t.explain {
            s.explain.set_active(t.explain);
        }
        let work = st.stored().show_work;
        if s.details.is_active() != work {
            s.details.set_active(work);
        }
        s.stop.set_visible(t.busy);
    }

    fn notes_update(&self) {
        while let Some(c) = self.notes.first_child() {
            self.notes.remove(&c);
        }
        let t = self.doc.tutor();
        if let Some(n) = &t.cost_note {
            self.notes.append(&caption(n));
        }
        if let Some(e) = &t.error {
            let l = gtk::Label::builder()
                .label(e)
                .xalign(0.0)
                .wrap(true)
                .selectable(true)
                .build();
            l.add_css_class("warning");
            self.notes.append(&l);
        }
    }

    // MARK: The transcript

    fn history(self: &Rc<Self>) {
        let work = crate::settings::tutor().borrow().stored().show_work;
        let t = self.doc.tutor();
        let key = format!(
            "{}|{:?}|{work}|{}|{}",
            t.turns.len(),
            t.turns
                .last()
                .map(|x| (x.index, x.cost.to_bits(), x.blocks.len())),
            t.live.is_some(),
            t.lesson_steps.len()
        );
        if *self.history_key.borrow() == key {
            return;
        }
        *self.history_key.borrow_mut() = key;
        let rows = tutor::shown(tutor::rows(&t.turns), work);
        let empty = rows.is_empty() && t.live.is_none();
        drop(t);
        while let Some(c) = self.history_box.first_child() {
            self.history_box.remove(&c);
        }
        if empty {
            self.history_box.append(&self.empty());
        }
        for r in &rows {
            self.history_box.append(&self.row(r, work));
        }
    }

    fn empty(self: &Rc<Self>) -> gtk::Widget {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let t = gtk::Label::builder()
            .label("Ask about this ROM")
            .xalign(0.0)
            .build();
        t.add_css_class("title-3");
        b.append(&t);
        let about = gtk::Label::builder()
            .label(
                "Select an instruction, a routine or a frame in the window, then ask: what it \
                 does, why it is written that way, what would change if… The tutor reads Romlens's \
                 analysis with its tools and cites every address.",
            )
            .xalign(0.0)
            .wrap(true)
            .build();
        about.add_css_class("dim-label");
        b.append(&about);
        let keys = caption(
            "Return sends, Shift+Return starts a line, Up brings back what you asked, Shift+Tab \
             changes what it may edit, Esc stops it, / lists the commands.",
        );
        keys.set_wrap(true);
        b.append(&keys);
        let ready = {
            let s = crate::settings::tutor();
            let s = s.borrow();
            s.ready(&s.default_endpoint())
        };
        if !ready {
            let button = gtk::Button::with_label("Add a Key in Settings…");
            button.set_halign(gtk::Align::Start);
            button.connect_clicked(|b| {
                if let Some(app) = b
                    .root()
                    .and_downcast::<gtk::Window>()
                    .and_then(|w| w.application())
                    .and_downcast::<adw::Application>()
                {
                    crate::settings::show(&app);
                }
            });
            b.append(&button);
        }
        b.upcast()
    }

    fn picture(&self, id: &str, height: i32, diagram: bool) -> gtk::Widget {
        let data = self
            .doc
            .tutor()
            .session()
            .and_then(|s| s.picture(id.to_owned()));
        match data.and_then(|d| gdk::Texture::from_bytes(&glib::Bytes::from_owned(d)).ok()) {
            Some(tex) => {
                let pic = gtk::Picture::for_paintable(&tex);
                pic.set_content_fit(gtk::ContentFit::ScaleDown);
                pic.set_halign(gtk::Align::Start);
                pic.add_css_class("card");
                if !diagram {
                    pic.set_height_request(height.min(tex.height()));
                }
                pic.upcast()
            }
            None => gtk::Image::from_icon_name("image-missing-symbolic").upcast(),
        }
    }

    fn row(self: &Rc<Self>, r: &Row, work: bool) -> gtk::Widget {
        match r {
            Row::Question {
                text,
                images,
                selection,
                ..
            } => {
                let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
                b.set_halign(gtk::Align::End);
                b.append(&self.bubble(text, images));
                if *selection {
                    let c = caption("with the selection");
                    c.set_halign(gtk::Align::End);
                    b.append(&c);
                }
                b.upcast()
            }
            Row::Answer {
                text,
                reasoning,
                tools,
                model,
                cost,
                ..
            } => {
                let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
                if work {
                    if !reasoning.is_empty() {
                        b.append(&thinking(reasoning));
                    }
                    if !tools.is_empty() {
                        b.append(&self.tool_log(tools));
                    }
                }
                b.append(&self.drawn(tools));
                if !text.is_empty() {
                    b.append(&messageview::message(&self.doc, text, &self.link_handler()));
                }
                if let Some(f) = messageview::footer(model.as_deref(), *cost) {
                    b.append(&f);
                }
                b.upcast()
            }
            Row::Note { text, .. } => {
                let c = caption(text);
                c.set_halign(gtk::Align::Center);
                c.upcast()
            }
            Row::Lesson { id, .. } => crate::lessonview::card(&self.doc, id, &self.link_handler()),
        }
    }

    /// Pictures drawn for the answer come first and large; ones a tool read
    /// from the ROM or the recording sit in the log.
    fn drawn(&self, tools: &[ToolRow]) -> gtk::Box {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
        for t in tools {
            let Some(cap) = tutor::drawing_caption(&t.name) else {
                continue;
            };
            for id in &t.images {
                let one = gtk::Box::new(gtk::Orientation::Vertical, 2);
                one.append(&self.picture(id, 360, true));
                one.append(&caption(cap));
                b.append(&one);
            }
        }
        b
    }

    fn bubble(&self, text: &str, images: &[String]) -> gtk::Box {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
        b.set_halign(gtk::Align::End);
        if !images.is_empty() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            for id in images {
                row.append(&self.picture(id, 90, false));
            }
            b.append(&row);
        }
        if !text.is_empty() {
            let l = gtk::Label::builder()
                .label(text)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .selectable(true)
                .xalign(0.0)
                .margin_start(10)
                .margin_end(10)
                .margin_top(7)
                .margin_bottom(7)
                .build();
            let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
            frame.add_css_class("question-bubble");
            frame.append(&l);
            frame.set_halign(gtk::Align::End);
            b.append(&frame);
        }
        b
    }

    fn tool_log(&self, tools: &[ToolRow]) -> gtk::Widget {
        let body = gtk::Box::new(gtk::Orientation::Vertical, 4);
        body.set_margin_start(4);
        for t in tools {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let icon = gtk::Image::from_icon_name(if !t.done {
                "content-loading-symbolic"
            } else if t.is_error {
                "dialog-error-symbolic"
            } else {
                "object-select-symbolic"
            });
            icon.set_valign(gtk::Align::Start);
            if t.is_error {
                icon.add_css_class("warning");
            }
            let col = gtk::Box::new(gtk::Orientation::Vertical, 1);
            let head = gtk::Label::builder()
                .use_markup(true)
                .label(format!(
                    "<b>{}</b>  <tt>{}</tt>",
                    tutor::tool_title(&t.name),
                    glib::markup_escape_text(&args(&t.input))
                ))
                .xalign(0.0)
                .wrap(true)
                .selectable(true)
                .build();
            head.add_css_class("caption");
            col.append(&head);
            if let Some(s) = t.summary.as_deref().filter(|s| !s.is_empty()) {
                let l = caption(s);
                l.set_wrap(true);
                l.set_lines(2);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                col.append(&l);
            }
            if tutor::drawing_caption(&t.name).is_none() && !t.images.is_empty() {
                let pics = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                for id in &t.images {
                    pics.append(&self.picture(id, 120, false));
                }
                col.append(&pics);
            }
            row.append(&icon);
            row.append(&col);
            body.append(&row);
        }
        let label = if tools.len() == 1 {
            "1 tool call".to_owned()
        } else {
            format!("{} tool calls", tools.len())
        };
        gtk::Expander::builder()
            .label(label)
            .expanded(true)
            .child(&body)
            .build()
            .upcast()
    }

    // MARK: The turn streaming in

    fn schedule_live(self: &Rc<Self>) {
        if self.live_scheduled.replace(true) {
            return;
        }
        // Fifteen times a second at most, however fast the text arrives.
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_millis(66), move || {
            if let Some(v) = weak.upgrade() {
                v.live_scheduled.set(false);
                v.live();
            }
        });
    }

    fn live(self: &Rc<Self>) {
        while let Some(c) = self.live_box.first_child() {
            self.live_box.remove(&c);
        }
        let Some(live) = self.doc.tutor().live.clone() else {
            return;
        };
        let work = crate::settings::tutor().borrow().stored().show_work;
        let Live {
            question,
            pictures,
            text,
            reasoning,
            tools,
            cards,
            status,
            lesson,
        } = live;
        if !question.is_empty() || !pictures.is_empty() {
            let shown: Vec<String> = Vec::new();
            let b = self.bubble(&question, &shown);
            for p in pictures.iter().rev() {
                if let Ok(tex) = gdk::Texture::from_bytes(&glib::Bytes::from(p)) {
                    let pic = gtk::Picture::for_paintable(&tex);
                    pic.set_height_request(90.min(tex.height()));
                    pic.set_content_fit(gtk::ContentFit::ScaleDown);
                    b.prepend(&pic);
                }
            }
            self.live_box.append(&b);
        }
        if work {
            if !reasoning.is_empty() {
                self.live_box.append(&thinking(&reasoning));
            }
            if !tools.is_empty() {
                self.live_box.append(&self.tool_log(&tools));
            }
        }
        for c in &cards {
            self.live_box.append(&self.edit_card(&c.proposal, c.state));
        }
        if !text.is_empty() {
            self.live_box.append(&messageview::message(
                &self.doc,
                &text,
                &self.link_handler(),
            ));
        }
        if let Some(l) = lesson {
            self.live_box.append(&crate::lessonview::card(
                &self.doc,
                &l,
                &self.link_handler(),
            ));
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&adw::Spinner::new());
        row.append(&caption(status.as_deref().unwrap_or("Thinking…")));
        self.live_box.append(&row);
    }

    /// An edit the tutor wants to make, in "ask before edits" (docs/24).
    fn edit_card(&self, p: &ProposalInfo, state: CardState) -> gtk::Widget {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
        b.add_css_class("card");
        b.set_margin_top(2);
        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_start(10)
            .margin_end(10)
            .margin_top(10)
            .margin_bottom(10)
            .build();
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&gtk::Image::from_icon_name("document-edit-symbolic"));
        let title = gtk::Label::builder()
            .label(&p.summary)
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .selectable(true)
            .build();
        title.add_css_class("heading");
        head.append(&title);
        match state {
            CardState::Accepted => {
                let l = caption("Made");
                l.add_css_class("success");
                head.append(&l);
            }
            CardState::Declined => head.append(&caption("Declined")),
            CardState::Waiting => {}
        }
        inner.append(&head);
        if !p.reason.is_empty() {
            let r = gtk::Label::builder()
                .label(&p.reason)
                .xalign(0.0)
                .wrap(true)
                .build();
            r.add_css_class("dim-label");
            inner.append(&r);
        }
        if p.before.is_some() || p.after.is_some() {
            let diff = gtk::Label::builder()
                .use_markup(true)
                .xalign(0.0)
                .wrap(true)
                .selectable(true)
                .build();
            let mut m = String::new();
            if let Some(b) = &p.before {
                let esc = glib::markup_escape_text(b);
                m += &if p.after.is_some() {
                    format!("now:   <s>{esc}</s>\n")
                } else {
                    format!("now:   {esc}\n")
                };
            }
            if let Some(a) = &p.after {
                m += &format!("after: {}", glib::markup_escape_text(a));
            }
            diff.set_markup(m.trim_end());
            diff.add_css_class("monospace");
            diff.add_css_class("caption");
            inner.append(&diff);
        }
        if state == CardState::Waiting {
            let why = gtk::Entry::builder()
                .placeholder_text("Why not? (optional)")
                .hexpand(true)
                .build();
            let (decline, accept, all) = (
                gtk::Button::with_label("Decline"),
                gtk::Button::with_label("Accept"),
                gtk::Button::with_label("Accept All"),
            );
            accept.add_css_class("suggested-action");
            all.set_tooltip_text(Some(
                "Accept this and the tutor's other edits in this answer",
            ));
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            for w in [
                why.upcast_ref::<gtk::Widget>(),
                decline.upcast_ref(),
                accept.upcast_ref(),
                all.upcast_ref(),
            ] {
                row.append(w);
            }
            let id = p.id.clone();
            let (doc, w, i) = (Rc::clone(&self.doc), why.clone(), id.clone());
            decline.connect_clicked(move |_| {
                let text = w.text().to_string();
                doc.edit_tutor(|t| {
                    t.answer_card(&i, false, (!text.is_empty()).then_some(text), false)
                });
            });
            let (doc, i) = (Rc::clone(&self.doc), id.clone());
            accept
                .connect_clicked(move |_| doc.edit_tutor(|t| t.answer_card(&i, true, None, false)));
            let (doc, i) = (Rc::clone(&self.doc), id);
            all.connect_clicked(move |_| doc.edit_tutor(|t| t.answer_card(&i, true, None, true)));
            inner.append(&row);
        }
        b.append(&inner);
        b.upcast()
    }

    // MARK: Sheets

    fn show_sheet(self: &Rc<Self>) {
        let Some(sheet) = self.doc.tutor().sheet else {
            return;
        };
        self.sheet_open.set(true);
        let weak = Rc::downgrade(self);
        let composer = {
            let weak = Rc::downgrade(self);
            Rc::new(move |t: &str| {
                if let Some(v) = weak.upgrade() {
                    v.set_composer(t);
                }
            })
        };
        let dialog = tutorsheets::build(&self.doc, sheet, composer, self.link_handler());
        dialog.connect_closed(move |_| {
            if let Some(v) = weak.upgrade() {
                v.sheet_open.set(false);
                v.doc.edit_tutor(|t| t.sheet = None);
            }
        });
        dialog.present(Some(&self.root));
    }
}

fn thinking(text: &str) -> gtk::Widget {
    let l = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    l.add_css_class("dim-label");
    gtk::Expander::builder()
        .label("How the tutor got here")
        .expanded(true)
        .child(&l)
        .build()
        .upcast()
}

/// `{"address":"$80:8000","lines":30}` as `address $80:8000, lines 30`.
pub fn args(json: &str) -> String {
    let Ok(serde_json::Value::Object(o)) = serde_json::from_str::<serde_json::Value>(json) else {
        return json.chars().take(80).collect();
    };
    let mut keys: Vec<&String> = o.keys().collect();
    keys.sort();
    let parts: Vec<String> = keys
        .into_iter()
        .filter(|k| !matches!(k.as_str(), "reason" | "text"))
        .filter_map(|k| {
            let v = &o[k];
            (!v.is_null()).then(|| match v {
                serde_json::Value::String(s) => format!("{k} {s}"),
                other => format!("{k} {other}"),
            })
        })
        .collect();
    parts.join(", ").chars().take(120).collect()
}

/// A picture as it can be sent: one too big in pixels or bytes for Claude or
/// OpenAI is scaled down; the rest as it is.
fn fit(png: &glib::Bytes) -> Vec<u8> {
    fit_media(png.to_vec(), "image/png").0
}

fn fit_media(data: Vec<u8>, media: &'static str) -> (Vec<u8>, &'static str) {
    let Ok(tex) = gdk::Texture::from_bytes(&glib::Bytes::from(&data)) else {
        return (data, media);
    };
    let long = tex.width().max(tex.height()) as u32;
    if long <= tutor::MAX_SIDE && data.len() <= tutor::MAX_BYTES {
        return (data, media);
    }
    // Scaled down with cairo, and sent as PNG: smaller sides until it fits.
    let (tw, th) = (tex.width(), tex.height());
    let stride = tw as usize * 4;
    let mut pixels = vec![0u8; stride * th as usize];
    tex.download(&mut pixels, stride);
    let Ok(src) = gtk::cairo::ImageSurface::create_for_data(
        pixels,
        gtk::cairo::Format::ARgb32,
        tw,
        th,
        stride as i32,
    ) else {
        return (data, media);
    };
    let mut side = tutor::MAX_SIDE.min(long);
    loop {
        let scale = f64::from(side) / f64::from(long);
        let (w, h) = (
            ((f64::from(tw) * scale).round() as i32).max(1),
            ((f64::from(th) * scale).round() as i32).max(1),
        );
        let Ok(mut dst) = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w, h) else {
            return (data, media);
        };
        if let Ok(cr) = gtk::cairo::Context::new(&dst) {
            cr.scale(scale, scale);
            let _ = cr.set_source_surface(&src, 0.0, 0.0);
            cr.source().set_filter(gtk::cairo::Filter::Good);
            let _ = cr.paint();
        }
        let out_stride = dst.stride() as usize;
        let Ok(bytes) = dst.data().map(|d| glib::Bytes::from(&d[..])) else {
            return (data, media);
        };
        let scaled = gdk::MemoryTexture::new(
            w,
            h,
            gdk::MemoryFormat::B8g8r8a8Premultiplied,
            &bytes,
            out_stride,
        );
        let out = scaled.save_to_png_bytes().to_vec();
        if out.len() <= tutor::MAX_BYTES || side <= 256 {
            return (out, "image/png");
        }
        side = side * 3 / 4;
    }
}

#[allow(dead_code)]
fn _modes(_: ModePreference, _: Sheet) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_calls_arguments_read_as_a_short_line() {
        assert_eq!(
            args(r#"{"address":"$80:8000","lines":30}"#),
            "address $80:8000, lines 30"
        );
        assert_eq!(
            args(r#"{"name":"Reset","reason":"why","text":"x"}"#),
            "name Reset",
            "the reason and text are shown elsewhere"
        );
        assert_eq!(args(r#"{"a":null,"b":true}"#), "b true");
        assert_eq!(args("not json at all"), "not json at all");
        assert!(
            args(&format!(r#"{{"k":"{}"}}"#, "x".repeat(300)))
                .chars()
                .count()
                <= 120
        );
    }
}
