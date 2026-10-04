//! The C tab: the disassembly on the left, the routine at the cursor as
//! pseudo-C on the right (docs/18). Selecting a line on either side
//! highlights its counterpart on the other. The macOS twin is `CSplitView`
//! and `CPaneController`.
//!
//! The right-click menu and the version picker carry the C's annotations
//! (docs/24, U10): name a local, comment a line, note the routine, or write a
//! C version beside the generated C.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{cairo, gdk, gio, glib};
use romlens_ffi::cnotes::CVersionInfo;
use romlens_ffi::{CTokenInfo, CTokenKind, Command};

use crate::asmview;
use crate::canvas::{set_source, with_alpha};
use crate::files::alert;
use crate::model::cfold::{self, Folder};
use crate::model::decompile::{
    DecompileState, LEVELS, NUMBER_STYLES, bases, literal_value, utf16_to_chars,
};
use crate::model::{CEdit, Change, Document};
use crate::palette;

const GUTTER_WIDTH: i32 = 16;

const KINDS: [CTokenKind; 11] = [
    CTokenKind::Keyword,
    CTokenKind::Type,
    CTokenKind::Number,
    CTokenKind::Comment,
    CTokenKind::Function,
    CTokenKind::Variable,
    CTokenKind::Register,
    CTokenKind::Label,
    CTokenKind::Helper,
    CTokenKind::Local,
    CTokenKind::GotoLabel,
];

fn tag_name(kind: CTokenKind) -> String {
    format!("c-{kind:?}")
}

/// An entry of the version picker.
#[derive(Debug, Clone, PartialEq, Eq)]
enum VersionItem {
    Generated,
    Named(String),
    New,
    Edit,
    Delete,
}

/// The annotations the right-click menu offers at the click.
#[derive(Debug, Clone, Default)]
struct MenuTarget {
    /// The local under the click: its name in the C, and its original.
    local: Option<(String, String)>,
    /// The instruction the clicked line came from.
    address: Option<u32>,
}

struct Pane {
    doc: Rc<Document>,
    view: gtk::TextView,
    scroll: gtk::ScrolledWindow,
    gutter: gtk::DrawingArea,
    title: gtk::Label,
    status: gtk::Label,
    levels: Vec<gtk::ToggleButton>,
    numbers: Vec<gtk::ToggleButton>,
    versions: gtk::DropDown,
    /// What each entry of the picker does.
    version_items: RefCell<Vec<VersionItem>>,
    versions_key: RefCell<String>,
    /// The picker is being rebuilt: its changes are not choices.
    building_versions: Cell<bool>,
    /// The version's text as shown, when one is.
    shown_version_text: RefCell<Option<String>>,
    /// What the right-click menu was built for.
    menu_target: RefCell<MenuTarget>,
    folder: RefCell<Folder>,
    /// Char offset where each line starts, plus the end.
    line_starts: RefCell<Vec<usize>>,
    tokens: RefCell<Vec<(Range<usize>, CTokenInfo)>>,
    shown_generation: Cell<u64>,
    shown_entry: Cell<Option<u32>>,
    highlighted: RefCell<Vec<usize>>,
    /// The instruction the highlight is for: a block opens when the selection
    /// moves into it, not when the same C comes back.
    highlighted_for: Cell<Option<u32>>,
    /// A click in the C moved the selection: the C need not scroll to it.
    selecting_from_text: Cell<bool>,
    /// The text is being replaced: the selection moves, but nobody clicked.
    replacing: Cell<bool>,
    overlays: RefCell<Vec<gtk::Widget>>,
}

/// The whole tab: disassembly on the left, the C pane on the right.
pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let asm = asmview::build(doc);
    let (root, pane) = Pane::build(doc);
    // The closures the pane wires hold it weakly; the widget keeps it alive.
    root.connect_destroy(move |_| {
        let _ = &pane;
    });
    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&asm.widget)
        .end_child(&root)
        .resize_start_child(true)
        .resize_end_child(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .build();
    // Half and half the first time there is room to say so.
    let placed = Rc::new(Cell::new(false));
    paned.connect_notify_local(Some("max-position"), move |p, _| {
        if !placed.get() && p.width() > 400 {
            placed.set(true);
            p.set_position(p.width() / 2);
        }
    });
    paned.upcast()
}

fn toggle_group(labels: &[&str], tip: &str) -> (gtk::Box, Vec<gtk::ToggleButton>) {
    let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    group.add_css_class("linked");
    group.set_tooltip_text(Some(tip));
    let buttons = labels
        .iter()
        .map(|l| {
            let b = gtk::ToggleButton::with_label(l);
            group.append(&b);
            b
        })
        .collect();
    (group, buttons)
}

impl Pane {
    fn build(doc: &Rc<Document>) -> (gtk::Widget, Rc<Self>) {
        let view = gtk::TextView::builder()
            .editable(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::None)
            .top_margin(6)
            .bottom_margin(6)
            .left_margin(8)
            .right_margin(8)
            .has_tooltip(true)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .child(&view)
            .build();
        let gutter = gtk::DrawingArea::builder()
            .content_width(GUTTER_WIDTH)
            .vexpand(true)
            .build();

        let title = gtk::Label::builder().xalign(0.0).build();
        title.add_css_class("heading");
        let status = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        status.add_css_class("dim-label");
        status.add_css_class("caption");
        let (level_box, levels) = toggle_group(
            &LEVELS.map(|(_, n)| n),
            "Lift: every instruction in full. Clean: after data flow. Full: with if, loops and signatures.",
        );
        let (number_box, numbers) = toggle_group(
            &NUMBER_STYLES.map(|(_, n, _)| n),
            "How numbers print: small ones in decimal and the rest in hex, or all in hex, decimal or binary. Addresses stay hex. Hover over a number to see it in every base.",
        );
        let versions = gtk::DropDown::new(None::<gtk::StringList>, gtk::Expression::NONE);
        versions.set_tooltip_text(Some(
            "The C Romlens generates, or a C version written by you or the tutor",
        ));
        let export = gtk::Button::builder()
            .label("Export C…")
            .action_name("win.export-c")
            .build();

        // Two lines, so a narrow pane wraps the controls instead of widening
        // the window: what the routine is, then how it is shown.
        let names = gtk::Box::builder()
            .spacing(8)
            .margin_start(8)
            .margin_end(8)
            .margin_top(4)
            .build();
        names.append(&title);
        names.append(&status);
        let controls = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(8)
            .column_spacing(8)
            .row_spacing(4)
            .margin_start(8)
            .margin_end(8)
            .margin_bottom(4)
            .homogeneous(false)
            .build();
        for w in [
            versions.upcast_ref::<gtk::Widget>(),
            number_box.upcast_ref(),
            level_box.upcast_ref(),
            export.upcast_ref(),
        ] {
            w.set_halign(gtk::Align::Start);
            controls.append(w);
        }
        let header = gtk::Box::new(gtk::Orientation::Vertical, 0);
        header.append(&names);
        header.append(&controls);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.append(&gutter);
        body.append(&scroll);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&header);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&body);

        let pane = Rc::new(Self {
            doc: Rc::clone(doc),
            view,
            scroll,
            gutter,
            title,
            status,
            levels,
            numbers,
            versions,
            version_items: RefCell::new(Vec::new()),
            versions_key: RefCell::new(String::new()),
            building_versions: Cell::new(false),
            shown_version_text: RefCell::new(None),
            menu_target: RefCell::new(MenuTarget::default()),
            folder: RefCell::new(Folder::default()),
            line_starts: RefCell::new(vec![0]),
            tokens: RefCell::new(Vec::new()),
            shown_generation: Cell::new(u64::MAX),
            shown_entry: Cell::new(None),
            highlighted: RefCell::new(Vec::new()),
            highlighted_for: Cell::new(None),
            selecting_from_text: Cell::new(false),
            replacing: Cell::new(false),
            overlays: RefCell::new(Vec::new()),
        });
        pane.create_tags();
        pane.wire();
        pane.update();
        // Development aid: `ROMLENS_CFOLD=all` folds every block once the C is up.
        #[cfg(debug_assertions)]
        if std::env::var("ROMLENS_CFOLD").is_ok() {
            let this = Rc::downgrade(&pane);
            glib::timeout_add_local_once(std::time::Duration::from_millis(1800), move || {
                if let Some(p) = this.upgrade() {
                    p.fold_key(FoldKey::FoldAll);
                }
            });
        }
        (root.upcast(), pane)
    }

    fn buffer(&self) -> gtk::TextBuffer {
        self.view.buffer()
    }

    // MARK: Tags and colours

    fn create_tags(&self) {
        let buffer = self.buffer();
        let table = buffer.tag_table();
        for kind in KINDS {
            table.add(&gtk::TextTag::builder().name(tag_name(kind)).build());
        }
        table.add(&gtk::TextTag::builder().name("highlight").build());
        table.add(
            &gtk::TextTag::builder()
                .name("folded")
                .invisible(true)
                .build(),
        );
        self.apply_colors();
    }

    /// Token colours follow light and dark and the accent colour.
    fn apply_colors(&self) {
        let style = adw::StyleManager::default();
        let (dark, accent) = (style.is_dark(), style.accent_color_rgba());
        let fg = self.view.color();
        let table = self.buffer().tag_table();
        for kind in KINDS {
            if let Some(tag) = table.lookup(&tag_name(kind)) {
                tag.set_foreground_rgba(Some(&palette::c_token_color(kind, dark, &fg, &accent)));
            }
        }
        if let Some(tag) = table.lookup("highlight") {
            tag.set_background_rgba(Some(&with_alpha(&accent, 0.3)));
        }
    }

    // MARK: Wiring

    fn wire(self: &Rc<Self>) {
        // Level and number-style buttons.
        for (i, button) in self.levels.iter().enumerate() {
            let doc = Rc::clone(&self.doc);
            button.connect_clicked(move |b| {
                // Radio behaviour without a group: a click always selects.
                b.set_active(true);
                doc.set_decompile_level(LEVELS[i].0);
            });
        }
        for (i, button) in self.numbers.iter().enumerate() {
            let doc = Rc::clone(&self.doc);
            button.connect_clicked(move |b| {
                b.set_active(true);
                doc.set_c_numbers(NUMBER_STYLES[i].0);
            });
        }

        // The version picker.
        let this = Rc::downgrade(self);
        self.versions.connect_selected_notify(move |d| {
            if let Some(p) = this.upgrade()
                && !p.building_versions.get()
            {
                p.version_chosen(d.selected() as usize);
            }
        });

        // The fold gutter.
        let this = Rc::downgrade(self);
        self.gutter.set_draw_func(move |_, cr, w, h| {
            if let Some(p) = this.upgrade() {
                p.draw_gutter(cr, f64::from(w), f64::from(h));
            }
        });
        let click = gtk::GestureClick::new();
        let this = Rc::downgrade(self);
        click.connect_pressed(move |_, _, _, y| {
            if let Some(p) = this.upgrade() {
                p.click_gutter(y);
            }
        });
        self.gutter.add_controller(click);
        let gutter = self.gutter.clone();
        self.scroll
            .vadjustment()
            .connect_value_changed(move |_| gutter.queue_draw());

        // A click or key in the C selects the instructions its line came from.
        let this = Rc::downgrade(self);
        self.buffer()
            .connect_notify_local(Some("cursor-position"), move |_, _| {
                if let Some(p) = this.upgrade() {
                    p.cursor_moved();
                }
            });

        // Double-click on a routine's or a label's name goes there.
        let double = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        let this = Rc::downgrade(self);
        double.connect_pressed(move |_, n, x, y| {
            if n == 2
                && let Some(p) = this.upgrade()
            {
                p.follow_token(x, y);
            }
        });
        self.view.add_controller(double);

        // Numbers show themselves in every base.
        let this = Rc::downgrade(self);
        self.view.connect_query_tooltip(move |_, x, y, _, tip| {
            let Some(text) = this.upgrade().and_then(|p| p.number_tooltip(x, y)) else {
                return false;
            };
            tip.set_text(Some(&text));
            true
        });

        // Fold keys, from the one shortcut table.
        let keys = gtk::ShortcutController::new();
        for (name, action) in [
            ("local.c-fold", FoldKey::Fold),
            ("local.c-unfold", FoldKey::Unfold),
            ("local.c-fold-all", FoldKey::FoldAll),
            ("local.c-unfold-all", FoldKey::UnfoldAll),
        ] {
            for accel in crate::actions::accels_for(name) {
                let Some(trigger) = gtk::ShortcutTrigger::parse_string(accel) else {
                    continue;
                };
                let this = Rc::downgrade(self);
                let cb = gtk::CallbackAction::new(move |_, _| {
                    this.upgrade().is_some_and(|p| p.fold_key(action)).into()
                });
                keys.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(cb)));
            }
        }
        self.view.add_controller(keys);

        // The context menu: the C's annotations, then the fold items.
        let actions = gio::SimpleActionGroup::new();
        for name in [
            "name-local",
            "comment",
            "note",
            "new-version",
            "edit-version",
        ] {
            let a = gio::SimpleAction::new(name, None);
            let this = Rc::downgrade(self);
            a.connect_activate(move |_, _| {
                if let Some(p) = this.upgrade() {
                    p.annotate(name);
                }
            });
            actions.add_action(&a);
        }
        let right = gtk::GestureClick::builder()
            .button(gdk::BUTTON_SECONDARY)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        let this = Rc::downgrade(self);
        right.connect_pressed(move |_, _, x, y| {
            if let Some(p) = this.upgrade() {
                p.build_menu(x, y);
            }
        });
        self.view.add_controller(right);
        for (name, key) in [
            ("fold", FoldKey::Fold),
            ("unfold", FoldKey::Unfold),
            ("fold-all", FoldKey::FoldAll),
            ("unfold-all", FoldKey::UnfoldAll),
        ] {
            let a = gio::SimpleAction::new(name, None);
            let this = Rc::downgrade(self);
            a.connect_activate(move |_, _| {
                if let Some(p) = this.upgrade() {
                    p.fold_key(key);
                }
            });
            actions.add_action(&a);
        }
        self.view.insert_action_group("c", Some(&actions));

        // Redraw and refresh when the document or the colour scheme changes.
        let this = Rc::downgrade(self);
        self.doc.subscribe(move |c| {
            if matches!(
                c,
                Change::Decompile | Change::Selection | Change::Rows | Change::Layout
            ) && let Some(p) = this.upgrade()
            {
                p.update();
            }
        });
        let this = Rc::downgrade(self);
        adw::StyleManager::default().connect_dark_notify(move |_| {
            if let Some(p) = this.upgrade() {
                p.apply_colors();
                p.gutter.queue_draw();
            }
        });
    }

    // MARK: Updating

    fn update(self: &Rc<Self>) {
        let doc = &self.doc;
        let (state, level, numbers, generation, result) = {
            let d = doc.decompile();
            (
                d.state.clone(),
                d.level,
                d.numbers,
                d.result_generation,
                d.result.clone(),
            )
        };
        for (i, b) in self.levels.iter().enumerate() {
            b.set_active(LEVELS[i].0 == level);
        }
        for (i, b) in self.numbers.iter().enumerate() {
            b.set_active(NUMBER_STYLES[i].0 == numbers);
        }
        match &state {
            DecompileState::Idle => {
                self.title.set_text("No routine");
                self.status
                    .set_text("Select an instruction to see its routine as C.");
            }
            DecompileState::Loading => self.status.set_text("Decompiling…"),
            DecompileState::Ready => {
                if let Some(r) = &result {
                    self.title.set_text(&format!(
                        "{}  {}",
                        r.name,
                        romlens_ffi::format_snes_address(r.entry)
                    ));
                    self.status.set_text(&if r.warnings.is_empty() {
                        format!(
                            "{} instructions, {} statements, {} gotos",
                            r.instructions, r.statements, r.gotos
                        )
                    } else {
                        let n = r.warnings.len();
                        format!(
                            "{n} note{}: {}",
                            if n == 1 { "" } else { "s" },
                            r.warnings[0]
                        )
                    });
                    self.status.set_tooltip_text(Some(&r.warnings.join("\n")));
                }
            }
            DecompileState::NotInRoutine => {
                self.title.set_text("No routine");
                self.status
                    .set_text("The selection is not inside a routine the analysis found.");
            }
            DecompileState::Failed(m) => self.status.set_text(m),
        }

        self.refresh_versions();
        if let Some(version) = self.shown_version_info() {
            let name = doc.shown_version().unwrap_or_default();
            if self.shown_version_text.borrow().as_deref() != Some(version.text.as_str())
                || self.shown_generation.get() != generation
            {
                self.shown_generation.set(generation);
                self.set_version_text(&name, &version);
            }
            self.highlight_version(&version);
            self.selecting_from_text.set(false);
            return;
        }
        if self.shown_generation.get() != generation || self.shown_version_text.borrow().is_some() {
            self.shown_generation.set(generation);
            if self.shown_version_text.borrow_mut().take().is_some() {
                // Back from a version: the generated text goes in whole.
                self.shown_entry.set(None);
            }
            self.set_text(result.as_ref());
        }
        let start = doc
            .details()
            .instruction
            .map(|i| i.file_offset)
            .or_else(|| doc.selected());
        let lines = start
            .map(|s| doc.decompile().lines_for_instruction(s))
            .unwrap_or_default();
        if lines != *self.highlighted.borrow() {
            self.set_highlight(&lines, start != self.highlighted_for.get());
            self.highlighted_for.set(start);
            if !self.selecting_from_text.get()
                && let Some(first) = lines.first()
            {
                self.scroll_to_line(*first);
            }
        }
        self.selecting_from_text.set(false);
    }

    fn set_text(self: &Rc<Self>, result: Option<&romlens_ffi::DecompiledInfo>) {
        let text = result.map_or("", |r| r.text.as_str());
        let buffer = self.buffer();
        let current = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
        // A live session re-analyses every few seconds and the routine's C
        // usually comes back the same: leave the caret and the scroll alone.
        if text == current.as_str() && result.map(|r| r.entry) == self.shown_entry.get() {
            return;
        }
        let same_routine = result.is_some() && result.map(|r| r.entry) == self.shown_entry.get();
        self.shown_entry.set(result.map(|r| r.entry));
        self.install(
            text,
            result.map_or(&[], |r| r.tokens.as_slice()),
            same_routine,
        );
    }

    /// Put `text` in the pane, coloured by `tokens`. What was folded stays
    /// folded and the scroll stays where it was when `same_routine`.
    fn install(self: &Rc<Self>, text: &str, tokens: &[CTokenInfo], same_routine: bool) {
        let buffer = self.buffer();
        let (hadj, vadj) = (
            self.scroll.hadjustment().value(),
            self.scroll.vadjustment().value(),
        );
        // What was folded stays folded, by line, if it is the same routine.
        let folded_lines: Vec<usize> = if same_routine {
            let f = self.folder.borrow();
            f.folded_opens().iter().map(|o| self.line_of(*o)).collect()
        } else {
            Vec::new()
        };

        self.replacing.set(true);
        buffer.set_text(text);
        buffer.place_cursor(&buffer.start_iter());
        self.replacing.set(false);

        // Line starts, in chars.
        let mut starts = vec![0usize];
        for (i, c) in text.chars().enumerate() {
            if c == '\n' {
                starts.push(i + 1);
            }
        }
        if *starts.last().unwrap() != text.chars().count() {
            starts.push(text.chars().count());
        }
        *self.line_starts.borrow_mut() = starts;

        // Tokens, with the core's UTF-16 offsets mapped to chars.
        let map = utf16_to_chars(text);
        let mut placed = Vec::new();
        for t in tokens {
            let (s, e) = (t.start as usize, (t.start + t.len) as usize);
            let (Some(&cs), Some(&ce)) = (map.get(s), map.get(e)) else {
                continue;
            };
            let (a, b) = (
                buffer.iter_at_offset(cs as i32),
                buffer.iter_at_offset(ce as i32),
            );
            buffer.apply_tag_by_name(&tag_name(t.kind), &a, &b);
            placed.push((cs..ce, t.clone()));
        }
        *self.tokens.borrow_mut() = placed;
        self.highlighted.borrow_mut().clear();

        let folds = cfold::find(text);
        let mut f = self.folder.borrow_mut();
        f.clear();
        let keep = folds
            .iter()
            .filter(|fold| folded_lines.contains(&self.line_of(fold.open)))
            .map(|fold| fold.open)
            .collect();
        f.set(folds, keep);
        drop(f);
        self.apply_folds();
        if same_routine {
            // Put the scroll back once the new text has laid out.
            let (h, v) = (self.scroll.hadjustment(), self.scroll.vadjustment());
            glib::idle_add_local_once(move || {
                h.set_value(hadj);
                v.set_value(vadj);
            });
        }
    }

    /// The line holding char offset `index`.
    fn line_of(&self, index: usize) -> usize {
        let starts = self.line_starts.borrow();
        starts.partition_point(|s| *s <= index).saturating_sub(1)
    }

    fn line_range(&self, line: usize) -> Option<Range<usize>> {
        let starts = self.line_starts.borrow();
        (line + 1 < starts.len()).then(|| starts[line]..starts[line + 1])
    }

    // MARK: Highlight and scrolling

    fn set_highlight(self: &Rc<Self>, lines: &[usize], reveal: bool) {
        let buffer = self.buffer();
        // The selection moved into a folded block: open it, as an IDE does.
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
        let chars: Vec<char> = text.chars().collect();
        let mut opened = false;
        if reveal {
            for l in lines {
                let Some(r) = self.line_range(*l) else {
                    continue;
                };
                if let Some(first) =
                    (r.start..r.end).find(|i| chars.get(*i).is_some_and(|c| !c.is_whitespace()))
                {
                    opened |= self.folder.borrow_mut().reveal(first..first + 1);
                }
            }
        }
        if opened {
            self.apply_folds();
        }
        buffer.remove_tag_by_name("highlight", &buffer.start_iter(), &buffer.end_iter());
        for l in lines {
            if let Some(r) = self.line_range(*l) {
                let (a, b) = (
                    buffer.iter_at_offset(r.start as i32),
                    buffer.iter_at_offset(r.end as i32),
                );
                buffer.apply_tag_by_name("highlight", &a, &b);
            }
        }
        *self.highlighted.borrow_mut() = lines.to_vec();
    }

    fn scroll_to_line(&self, line: usize) {
        let Some(r) = self.line_range(line) else {
            return;
        };
        let buffer = self.buffer();
        let rect = self
            .view
            .iter_location(&buffer.iter_at_offset(r.start as i32));
        let visible = self.view.visible_rect();
        let inside =
            rect.y() >= visible.y() && rect.y() + rect.height() <= visible.y() + visible.height();
        if !inside {
            let vadj = self.scroll.vadjustment();
            let target = f64::from(rect.y()) - vadj.page_size() / 2.0;
            vadj.set_value(target.clamp(0.0, (vadj.upper() - vadj.page_size()).max(0.0)));
        }
    }

    // MARK: Selection from the C

    fn cursor_moved(&self) {
        if self.replacing.get() || self.line_starts.borrow().len() < 2 {
            return;
        }
        let buffer = self.buffer();
        let at = buffer.iter_at_mark(&buffer.get_insert()).offset() as usize;
        if let Some(version) = self.shown_version_info() {
            self.follow_version_line(&version, self.line_of(at));
            return;
        }
        let offsets = self.doc.decompile().offsets_for_line(self.line_of(at));
        let Some(first) = offsets.iter().min().copied() else {
            return;
        };
        let current = self
            .doc
            .details()
            .instruction
            .map(|i| i.file_offset)
            .or_else(|| self.doc.selected());
        if Some(first) == current {
            return;
        }
        self.selecting_from_text.set(true);
        self.doc.select(Some(first));
        self.doc.request_scroll(first);
    }

    fn char_at(&self, x: f64, y: f64) -> Option<usize> {
        let (bx, by) =
            self.view
                .window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        self.view
            .iter_at_location(bx, by)
            .map(|i| i.offset() as usize)
    }

    fn token_at(&self, index: usize) -> Option<(Range<usize>, CTokenInfo)> {
        self.tokens
            .borrow()
            .iter()
            .find(|(r, _)| r.contains(&index))
            .cloned()
    }

    fn follow_token(&self, x: f64, y: f64) {
        let Some((_, t)) = self.char_at(x, y).and_then(|i| self.token_at(i)) else {
            return;
        };
        if matches!(t.kind, CTokenKind::Function | CTokenKind::Label)
            && let Some(address) = t.address
        {
            self.doc.jump_to_snes(address);
        }
    }

    fn number_tooltip(&self, x: i32, y: i32) -> Option<String> {
        let (range, t) = self
            .char_at(f64::from(x), f64::from(y))
            .and_then(|i| self.token_at(i))?;
        if t.kind != CTokenKind::Number {
            return None;
        }
        let buffer = self.buffer();
        let literal = buffer.text(
            &buffer.iter_at_offset(range.start as i32),
            &buffer.iter_at_offset(range.end as i32),
            false,
        );
        literal_value(&literal).map(bases)
    }

    // MARK: C versions and the C's annotations (docs/24, U10)

    /// The routine shown.
    fn entry(&self) -> Option<u32> {
        self.doc.decompile().result.as_ref().map(|r| r.entry)
    }

    /// The version picked, if it still exists for the routine shown.
    fn shown_version_info(&self) -> Option<CVersionInfo> {
        let name = self.doc.shown_version()?;
        self.doc
            .c_versions(self.entry()?)
            .into_iter()
            .find(|n| n.name == name)
            .map(|n| n.version)
    }

    /// The picker: Generated, each version, then what can be done.
    fn refresh_versions(&self) {
        let entry = self.entry();
        let names: Vec<String> = entry
            .map(|e| self.doc.c_versions(e).into_iter().map(|n| n.name).collect())
            .unwrap_or_default();
        let shown = self.doc.shown_version().filter(|v| names.contains(v));
        let key = format!("{entry:?}|{}|{shown:?}", names.join("|"));
        if *self.versions_key.borrow() == key {
            return;
        }
        *self.versions_key.borrow_mut() = key;
        let mut items = vec![VersionItem::Generated];
        items.extend(names.iter().cloned().map(VersionItem::Named));
        items.push(VersionItem::New);
        if shown.is_some() {
            items.push(VersionItem::Edit);
            items.push(VersionItem::Delete);
        }
        let labels: Vec<&str> = items
            .iter()
            .map(|i| match i {
                VersionItem::Generated => "Generated",
                VersionItem::Named(n) => n.as_str(),
                VersionItem::New => "New Version…",
                VersionItem::Edit => "Edit Version…",
                VersionItem::Delete => "Delete Version",
            })
            .collect();
        self.building_versions.set(true);
        self.versions
            .set_model(Some(&gtk::StringList::new(&labels)));
        let at = shown
            .and_then(|s| {
                items
                    .iter()
                    .position(|i| *i == VersionItem::Named(s.clone()))
            })
            .unwrap_or(0);
        self.versions.set_selected(at as u32);
        self.versions.set_sensitive(entry.is_some());
        self.building_versions.set(false);
        *self.version_items.borrow_mut() = items;
    }

    fn version_chosen(self: &Rc<Self>, index: usize) {
        let Some(item) = self.version_items.borrow().get(index).cloned() else {
            return;
        };
        let Some(entry) = self.entry() else { return };
        let doc = &self.doc;
        match item {
            VersionItem::Generated => doc.show_c_version(None),
            VersionItem::Named(n) => doc.show_c_version(Some(n)),
            VersionItem::New => doc.begin_c_edit(CEdit::Version {
                routine: entry,
                name: None,
            }),
            VersionItem::Edit => doc.begin_c_edit(CEdit::Version {
                routine: entry,
                name: doc.shown_version(),
            }),
            VersionItem::Delete => {
                if let Some(v) = doc.shown_version() {
                    let _ = doc.run_c_command(Command::SetCVersion {
                        routine: entry,
                        name: v,
                        version: None,
                    });
                    doc.show_c_version(None);
                }
            }
        }
        // The picker shows what is shown, whatever was asked.
        self.versions_key.borrow_mut().clear();
        self.update();
    }

    /// A version's text, coloured by the lexer.
    fn set_version_text(self: &Rc<Self>, name: &str, version: &CVersionInfo) {
        let tokens = self.doc.workbench().lex_c(version.text.clone());
        *self.shown_version_text.borrow_mut() = Some(version.text.clone());
        self.shown_entry.set(None);
        self.install(&version.text, &tokens, false);
        let routine = self.title.text();
        let routine = routine.split(' ').next().unwrap_or_default();
        self.title.set_text(&format!("{routine}  version “{name}”"));
        self.status
            .set_text(if version.author == romlens_ffi::cnotes::CAuthor::Tutor {
                "Written by the tutor; not checked against the code"
            } else {
                "Written by you; not checked against the code"
            });
        self.status.set_tooltip_text(None);
    }

    /// The version's lines anchored to the selected instruction.
    fn highlight_version(self: &Rc<Self>, version: &CVersionInfo) {
        let Some(a) = self.doc.selected_address() else {
            return;
        };
        let lines: Vec<usize> = version
            .anchors
            .iter()
            .filter(|x| x.start <= a && a <= x.end)
            .flat_map(|x| x.first.saturating_sub(1) as usize..=x.last.saturating_sub(1) as usize)
            .collect();
        if lines != *self.highlighted.borrow() {
            self.set_highlight(&lines, true);
            if !self.selecting_from_text.get()
                && let Some(&first) = lines.first()
            {
                // The new text has not laid out yet.
                let this = Rc::downgrade(self);
                glib::idle_add_local_once(move || {
                    if let Some(p) = this.upgrade() {
                        p.scroll_to_line(first);
                    }
                });
            }
        }
    }

    fn follow_version_line(&self, version: &CVersionInfo, line: usize) {
        let Some(a) = version.anchors.iter().find(|a| {
            (a.first.saturating_sub(1) as usize..=a.last.saturating_sub(1) as usize).contains(&line)
        }) else {
            return;
        };
        if Some(a.start) == self.doc.selected_address() {
            return;
        }
        self.selecting_from_text.set(true);
        self.doc.jump_to_snes(a.start);
    }

    /// Right-click in the C: name a local, comment the line, note the routine,
    /// or write a version. Built for each click.
    fn build_menu(&self, x: f64, y: f64) {
        let menu = gio::Menu::new();
        let mut target = MenuTarget::default();
        if self.entry().is_some() {
            let notes = gio::Menu::new();
            if self.shown_version_info().is_none() {
                if let Some(index) = self.char_at(x, y) {
                    if let Some((range, t)) = self.token_at(index)
                        && t.kind == CTokenKind::Local
                    {
                        let buffer = self.buffer();
                        let word = buffer
                            .text(
                                &buffer.iter_at_offset(range.start as i32),
                                &buffer.iter_at_offset(range.end as i32),
                                false,
                            )
                            .to_string();
                        let original = self
                            .entry()
                            .and_then(|e| {
                                self.doc
                                    .workbench()
                                    .local_names(e)
                                    .into_iter()
                                    .find(|n| n.name == word)
                            })
                            .map_or_else(|| word.clone(), |n| n.local);
                        notes.append(Some(&format!("Name “{word}”…")), Some("c.name-local"));
                        target.local = Some((word, original));
                    }
                    let offsets = self.doc.decompile().offsets_for_line(self.line_of(index));
                    if let Some(a) = offsets.iter().min().and_then(|o| self.doc.snes_address(*o)) {
                        target.address = Some(a);
                        notes.append(Some("C Comment Here…"), Some("c.comment"));
                    }
                }
                notes.append(Some("Routine Note…"), Some("c.note"));
            }
            menu.append_section(None, &notes);
            let versions = gio::Menu::new();
            versions.append(Some("New C Version…"), Some("c.new-version"));
            if self.shown_version_info().is_some() {
                versions.append(Some("Edit C Version…"), Some("c.edit-version"));
            }
            menu.append_section(None, &versions);
        }
        let folds = gio::Menu::new();
        for (label, name) in [
            ("Fold", "c.fold"),
            ("Unfold", "c.unfold"),
            ("Fold All", "c.fold-all"),
            ("Unfold All", "c.unfold-all"),
        ] {
            folds.append(Some(label), Some(name));
        }
        menu.append_section(None, &folds);
        *self.menu_target.borrow_mut() = target;
        self.view.set_extra_menu(Some(&menu));
    }

    fn annotate(&self, what: &str) {
        let Some(routine) = self.entry() else { return };
        let target = self.menu_target.borrow().clone();
        let edit = match what {
            "name-local" => target
                .local
                .map(|(_, local)| CEdit::Local { routine, local }),
            "comment" => target.address.map(|address| CEdit::Comment { address }),
            "note" => Some(CEdit::Note { routine }),
            "new-version" => Some(CEdit::Version {
                routine,
                name: None,
            }),
            "edit-version" => Some(CEdit::Version {
                routine,
                name: self.doc.shown_version(),
            }),
            _ => None,
        };
        if let Some(e) = edit {
            self.doc.begin_c_edit(e);
        }
    }

    // MARK: Folding

    /// Hide the folded blocks' bodies and put a `…` where each was.
    fn apply_folds(self: &Rc<Self>) {
        let buffer = self.buffer();
        let (start, end) = (buffer.start_iter(), buffer.end_iter());
        buffer.remove_tag_by_name("folded", &start, &end);
        for w in self.overlays.borrow_mut().drain(..) {
            self.view.remove(&w);
        }
        let hidden = self.folder.borrow().hidden();
        for r in &hidden {
            if let Some(lines) = self.hidden_lines(r) {
                let (a, b) = (
                    buffer.iter_at_offset(lines.start as i32),
                    buffer.iter_at_offset(lines.end as i32),
                );
                buffer.apply_tag_by_name("folded", &a, &b);
            }
        }
        self.gutter.queue_draw();
        // The `…` buttons sit where the layout puts the brace, which is only
        // known once it has been laid out.
        let this = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_millis(30), move || {
            if let Some(p) = this.upgrade() {
                p.place_ellipses(&hidden);
            }
        });
    }

    fn place_ellipses(self: &Rc<Self>, hidden: &[Range<usize>]) {
        // A newer fold has replaced the one this was scheduled for.
        if *hidden != self.folder.borrow().hidden() {
            return;
        }
        for w in self.overlays.borrow_mut().drain(..) {
            self.view.remove(&w);
        }
        for r in hidden {
            // At the end of the line the block opens on, where its body was.
            let open_line = self.line_of(r.start.saturating_sub(1));
            let Some(line) = self.line_range(open_line) else {
                continue;
            };
            let mut iter = self.buffer().iter_at_offset(line.start as i32);
            if !iter.ends_line() {
                iter.forward_to_line_end();
            }
            let rect = self.view.iter_location(&iter);
            let button = gtk::Button::with_label("…");
            button.add_css_class("flat");
            button.add_css_class("monospace");
            button.set_tooltip_text(Some("Unfold"));
            let (this, start) = (Rc::downgrade(self), r.start);
            button.connect_clicked(move |_| {
                if let Some(p) = this.upgrade() {
                    let fold = p.folder.borrow().placeholder_at(start);
                    if let Some(f) = fold {
                        p.folder.borrow_mut().toggle(&f);
                        p.apply_folds();
                    }
                }
            });
            self.view.add_overlay(&button, rect.x(), rect.y());
            self.overlays.borrow_mut().push(button.upcast());
        }
    }

    /// The whole lines a folded block hides: those strictly between the line
    /// that opens it and the line that closes it, so the layout never has to
    /// join a line to its neighbour. The closing brace stays on its own line.
    fn hidden_lines(&self, r: &Range<usize>) -> Option<Range<usize>> {
        let open_line = self.line_of(r.start.checked_sub(1)?);
        let close_line = self.line_of(r.end);
        if close_line <= open_line + 1 {
            return None;
        }
        let starts = self.line_starts.borrow();
        Some(*starts.get(open_line + 1)?..*starts.get(close_line)?)
    }

    fn fold_key(self: &Rc<Self>, key: FoldKey) -> bool {
        let buffer = self.buffer();
        let at = buffer.iter_at_mark(&buffer.get_insert()).offset() as usize;
        let line = self.line_range(self.line_of(at)).unwrap_or(at..at);
        let changed = {
            let mut f = self.folder.borrow_mut();
            match key {
                FoldKey::Fold => f.fold(at, line),
                FoldKey::Unfold => f.unfold(at, line),
                FoldKey::FoldAll => {
                    f.fold_all();
                    true
                }
                FoldKey::UnfoldAll => {
                    f.unfold_all();
                    true
                }
            }
        };
        if changed {
            self.apply_folds();
        }
        changed
    }

    /// Each block whose first line shows, with that line's y in the gutter.
    fn arrows(&self) -> Vec<(cfold::Fold, f64, f64)> {
        let buffer = self.buffer();
        let folder = self.folder.borrow();
        let vadj = self.scroll.vadjustment().value();
        let mut seen_line = i32::MIN;
        let mut out = Vec::new();
        for f in folder.folds() {
            if folder.is_hidden(f.open) {
                continue;
            }
            let rect = self
                .view
                .iter_location(&buffer.iter_at_offset(f.open as i32));
            // Two blocks opening on one line: the arrow is the first's.
            if rect.y() == seen_line {
                continue;
            }
            seen_line = rect.y();
            let top = f64::from(rect.y()) + f64::from(self.view.top_margin()) - vadj;
            out.push((*f, top, f64::from(rect.height())));
        }
        out
    }

    fn draw_gutter(&self, cr: &cairo::Context, width: f64, height: f64) {
        let fg = self.view.color();
        for (fold, top, h) in self.arrows() {
            if top + h < 0.0 || top > height {
                continue;
            }
            let folded = self.folder.borrow().is_folded(&fold);
            set_source(cr, &with_alpha(&fg, if folded { 0.7 } else { 0.4 }));
            cr.set_line_width(1.5);
            let (cx, cy) = (width / 2.0, top + h / 2.0);
            if folded {
                cr.move_to(cx - 1.5, cy - 3.5);
                cr.line_to(cx + 2.0, cy);
                cr.line_to(cx - 1.5, cy + 3.5);
            } else {
                cr.move_to(cx - 3.5, cy - 1.5);
                cr.line_to(cx, cy + 2.0);
                cr.line_to(cx + 3.5, cy - 1.5);
            }
            let _ = cr.stroke();
        }
    }

    fn click_gutter(self: &Rc<Self>, y: f64) {
        let hit = self
            .arrows()
            .into_iter()
            .find(|(_, top, h)| *top <= y && y < top + h);
        if let Some((fold, _, _)) = hit {
            self.folder.borrow_mut().toggle(&fold);
            self.apply_folds();
        }
    }
}

#[derive(Clone, Copy)]
enum FoldKey {
    Fold,
    Unfold,
    FoldAll,
    UnfoldAll,
}

/// Export C…: the routine's translation unit and the `snes.h` it includes,
/// side by side.
pub fn export(window: &adw::ApplicationWindow, doc: &Rc<Document>) {
    let Some((name, text)) = doc
        .decompile()
        .result
        .as_ref()
        .map(|r| (r.name.clone(), r.text.clone()))
    else {
        return;
    };
    let folder = doc
        .project_path()
        .or_else(|| doc.rom_path())
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf));
    let dialog = gtk::FileDialog::builder()
        .title("Export C")
        .initial_name(format!("{name}.c"))
        .accept_label("Export")
        .build();
    if let Some(f) = folder {
        dialog.set_initial_folder(Some(&gio::File::for_path(f)));
    }
    let w = window.clone();
    dialog.save(Some(window), gio::Cancellable::NONE, move |picked| {
        let Some(path) = picked.ok().and_then(|f| f.path()) else {
            return;
        };
        if let Err(e) = write_c(&path, &text) {
            alert(
                Some(w.upcast_ref()),
                "Could not export",
                &format!("{}\n\n{e}", path.display()),
            );
        }
    });
}

/// The unit, and `snes.h` beside it.
pub fn write_c(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)?;
    let header = path
        .parent()
        .map_or_else(|| "snes.h".into(), |d| d.join("snes.h"));
    std::fs::write(header, romlens_ffi::workbench::snes_header())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_writes_the_unit_and_snes_h_beside_it() {
        let dir = std::env::temp_dir().join(format!("romlens-exportc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_c(&dir.join("SUB_008020.c"), "void f(void) {}\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("SUB_008020.c")).unwrap(),
            "void f(void) {}\n"
        );
        let header = std::fs::read_to_string(dir.join("snes.h")).unwrap();
        assert!(header.contains("typedef"), "{header}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn every_token_kind_has_a_tag() {
        let names: std::collections::HashSet<_> = KINDS.iter().map(|k| tag_name(*k)).collect();
        assert_eq!(names.len(), KINDS.len());
    }
}
