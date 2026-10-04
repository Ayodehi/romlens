//! The Source tab (docs/22, S2): the program's source on the left, the
//! disassembly on the right. A line that made bytes is marked in its number;
//! clicking one selects its bytes, and selecting bytes anywhere shows and
//! highlights the line that made them. The macOS twin is `SourceSplitView`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{cairo, glib};

use crate::asmview;
use crate::canvas::{set_source, with_alpha};
use crate::model::{Change, Document};

struct Pane {
    doc: Rc<Document>,
    view: gtk::TextView,
    scroll: gtk::ScrolledWindow,
    gutter: gtk::DrawingArea,
    files: gtk::DropDown,
    names: gtk::StringList,
    status: gtk::Label,
    stack: gtk::Stack,
    missing: adw::StatusPage,
    shown_generation: Cell<u64>,
    /// The lines (1-based) the selection came from.
    highlighted: RefCell<Vec<u32>>,
    /// The dropdown is being filled: its selection is not a choice.
    filling: Cell<bool>,
    /// A click here moved the selection: no need to scroll to it.
    clicked: Cell<bool>,
}

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let asm = asmview::build(doc);
    let (root, pane) = Pane::build(doc);
    root.connect_destroy(move |_| {
        let _ = &pane;
    });
    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&root)
        .end_child(&asm.widget)
        .resize_start_child(true)
        .resize_end_child(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .build();
    let placed = Rc::new(Cell::new(false));
    paned.connect_notify_local(Some("max-position"), move |p, _| {
        if !placed.get() && p.width() > 400 {
            placed.set(true);
            p.set_position(p.width() / 2);
        }
    });
    paned.upcast()
}

impl Pane {
    fn build(doc: &Rc<Document>) -> (gtk::Widget, Rc<Self>) {
        let view = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::None)
            .top_margin(6)
            .bottom_margin(6)
            .left_margin(8)
            .right_margin(8)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .child(&view)
            .build();
        let gutter = gtk::DrawingArea::builder().vexpand(true).build();
        let names = gtk::StringList::new(&[]);
        let files = gtk::DropDown::builder()
            .model(&names)
            .hexpand(true)
            .tooltip_text("The source files of the imported debug information")
            .build();
        let status = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .margin_start(8)
            .margin_end(8)
            .build();
        status.add_css_class("caption");
        status.add_css_class("dim-label");
        status.set_visible(false);
        let missing = adw::StatusPage::builder()
            .icon_name("document-open-symbolic")
            .title("Source file not found")
            .build();
        let retry = gtk::Button::builder()
            .label("Try Again")
            .halign(gtk::Align::Center)
            .build();
        retry.add_css_class("pill");
        missing.set_child(Some(&retry));

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.append(&gutter);
        body.append(&scroll);
        let stack = gtk::Stack::new();
        stack.add_named(&body, Some("text"));
        stack.add_named(&missing, Some("missing"));

        let header = gtk::Box::builder()
            .spacing(8)
            .margin_start(8)
            .margin_end(8)
            .margin_top(4)
            .margin_bottom(4)
            .build();
        header.append(&files);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&header);
        root.append(&status);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&stack);

        let pane = Rc::new(Self {
            doc: Rc::clone(doc),
            view,
            scroll,
            gutter,
            files,
            names,
            status,
            stack,
            missing,
            shown_generation: Cell::new(u64::MAX),
            highlighted: RefCell::new(Vec::new()),
            filling: Cell::new(false),
            clicked: Cell::new(false),
        });
        pane.create_tags();
        pane.wire(&retry);
        pane.update();
        (root.upcast(), pane)
    }

    fn create_tags(&self) {
        let table = self.view.buffer().tag_table();
        table.add(&gtk::TextTag::builder().name("highlight").build());
        self.apply_colors();
    }

    fn apply_colors(&self) {
        let accent = adw::StyleManager::default().accent_color_rgba();
        if let Some(tag) = self.view.buffer().tag_table().lookup("highlight") {
            tag.set_background_rgba(Some(&with_alpha(&accent, 0.3)));
        }
    }

    fn wire(self: &Rc<Self>, retry: &gtk::Button) {
        let this = Rc::downgrade(self);
        self.gutter.set_draw_func(move |_, cr, w, h| {
            if let Some(p) = this.upgrade() {
                p.draw_gutter(cr, f64::from(w), f64::from(h));
            }
        });
        let gutter = self.gutter.clone();
        self.scroll
            .vadjustment()
            .connect_value_changed(move |_| gutter.queue_draw());

        let this = Rc::downgrade(self);
        self.files.connect_selected_notify(move |d| {
            if let Some(p) = this.upgrade()
                && !p.filling.get()
            {
                p.doc.show_source_file(d.selected() as usize);
            }
        });
        let doc = Rc::clone(&self.doc);
        retry.connect_clicked(move |_| doc.retry_source());

        // A click on a line that made bytes selects them.
        let click = gtk::GestureClick::builder().button(1).build();
        let this = Rc::downgrade(self);
        click.connect_released(move |g, n, x, y| {
            if n != 1 {
                return;
            }
            let Some(p) = this.upgrade() else { return };
            // Dragging to select text is not a click.
            if p.view.buffer().has_selection() {
                return;
            }
            g.set_state(gtk::EventSequenceState::None);
            if let Some(line) = p.line_at(x, y) {
                p.clicked.set(true);
                p.doc.select_source_line(line);
                p.clicked.set(false);
            }
        });
        self.view.add_controller(click);

        let this = Rc::downgrade(self);
        self.doc.subscribe(move |c| {
            if matches!(
                c,
                Change::Source | Change::Selection | Change::Rows | Change::Layout
            ) && let Some(p) = this.upgrade()
            {
                p.update();
            }
        });
        let this = Rc::downgrade(self);
        adw::StyleManager::default().connect_accent_color_notify(move |_| {
            if let Some(p) = this.upgrade() {
                p.apply_colors();
            }
        });
    }

    /// The 1-based line under a point of the view, if it made bytes.
    fn line_at(&self, x: f64, y: f64) -> Option<u32> {
        let (bx, by) =
            self.view
                .window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let iter = self.view.iter_at_location(bx, by)?;
        let line = iter.line() as u32 + 1;
        self.doc
            .source()
            .by_line
            .contains_key(&line)
            .then_some(line)
    }

    fn update(self: &Rc<Self>) {
        if self.doc.tab() != crate::model::Tab::Source {
            return;
        }
        // The file of the selected byte comes up on its own.
        if !self.clicked.get() {
            self.doc.follow_selection_in_source();
        }
        let (generation, text, problem, names, shown) = {
            let s = self.doc.source();
            (
                s.generation,
                s.text.as_ref().map(|t| t.join("\n")),
                s.problem.clone(),
                s.files
                    .iter()
                    .map(|f| {
                        if f.lines == 0 {
                            format!("{} (no code)", f.name)
                        } else {
                            f.name.clone()
                        }
                    })
                    .collect::<Vec<_>>(),
                s.shown,
            )
        };
        if generation != self.shown_generation.get() {
            self.shown_generation.set(generation);
            self.filling.set(true);
            self.names.splice(
                0,
                self.names.n_items(),
                &names.iter().map(String::as_str).collect::<Vec<_>>(),
            );
            if let Some(i) = shown {
                self.files.set_selected(i as u32);
            }
            self.filling.set(false);
            self.highlighted.borrow_mut().clear();
            match &text {
                Some(t) => {
                    self.view.buffer().set_text(t);
                    self.stack.set_visible_child_name("text");
                }
                None => {
                    self.view.buffer().set_text("");
                    self.missing
                        .set_description(Some(problem.as_deref().unwrap_or("")));
                    self.stack.set_visible_child_name("missing");
                }
            }
            // A warning (the file changed) shows over the text; a missing
            // file's message is the page's.
            let warn = text.is_some().then_some(problem).flatten();
            self.status.set_visible(warn.is_some());
            self.status.set_text(warn.as_deref().unwrap_or(""));
            self.size_gutter();
        }
        self.highlight();
        self.gutter.queue_draw();
    }

    fn size_gutter(&self) {
        let lines = self.doc.source().text.as_ref().map_or(0, Vec::len).max(100);
        let digits = lines.to_string().len() as i32;
        let layout = self.view.create_pango_layout(Some("0"));
        let (w, _) = layout.pixel_size();
        self.gutter.set_content_width(w * digits + 16);
    }

    /// Mark the lines that made the selected byte, and bring the first into
    /// view.
    fn highlight(&self) {
        let buffer = self.view.buffer();
        let (start, end) = buffer.bounds();
        buffer.remove_tag_by_name("highlight", &start, &end);
        let Some(file) = self.doc.source().shown_file().map(|f| (f.map, f.file)) else {
            return;
        };
        let lines: Vec<u32> = self
            .doc
            .source_lines_at_selection()
            .into_iter()
            .filter(|l| (l.map, l.file) == file)
            .map(|l| l.line)
            .collect();
        for &line in &lines {
            let Some(from) = buffer.iter_at_line(line as i32 - 1) else {
                continue;
            };
            let mut to = from;
            if !to.ends_line() {
                to.forward_to_line_end();
            }
            buffer.apply_tag_by_name("highlight", &from, &to);
        }
        let first = lines.iter().min().copied();
        if let Some(line) = first
            && !self.clicked.get()
            && *self.highlighted.borrow() != lines
        {
            let view = self.view.clone();
            // After the text has been laid out, or the line lands at the edge.
            glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
                if let Some(mut iter) = view.buffer().iter_at_line(line as i32 - 1) {
                    view.scroll_to_iter(&mut iter, 0.1, true, 0.0, 0.3);
                }
            });
        }
        *self.highlighted.borrow_mut() = lines;
    }

    fn draw_gutter(&self, cr: &cairo::Context, width: f64, height: f64) {
        let fg = self.view.color();
        let accent = adw::StyleManager::default().accent_color_rgba();
        let layout = self.view.create_pango_layout(None);
        let source = self.doc.source();
        // Buffer coordinates of the top of the viewport; the view's own top
        // margin is accounted for by the conversion.
        let (_, top) = self
            .view
            .window_to_buffer_coords(gtk::TextWindowType::Widget, 0, 0);
        let mut line = self.view.line_at_y(top).0.line();
        let highlighted = self.highlighted.borrow();
        loop {
            let Some(iter) = self.view.buffer().iter_at_line(line) else {
                break;
            };
            let (y, h) = self.view.line_yrange(&iter);
            let screen = f64::from(y - top);
            if screen > height {
                break;
            }
            let number = line as u32 + 1;
            let mapped = source.by_line.contains_key(&number);
            let chosen = highlighted.contains(&number);
            if chosen {
                set_source(cr, &with_alpha(&accent, 0.3));
                cr.rectangle(0.0, screen, width, f64::from(h));
                let _ = cr.fill();
            }
            // A line that made bytes is full strength; the rest are quiet.
            set_source(cr, &with_alpha(&fg, if mapped { 0.9 } else { 0.35 }));
            layout.set_text(&number.to_string());
            let (w, _) = layout.pixel_size();
            cr.move_to(width - f64::from(w) - 8.0, screen);
            pangocairo::functions::show_layout(cr, &layout);
            line += 1;
        }
    }
}
