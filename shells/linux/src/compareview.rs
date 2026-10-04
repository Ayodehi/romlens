//! The Compare tab (docs/22, D2): what changed from another version of the
//! ROM to this one. The list on the left, the chosen change on the right: a
//! changed routine's instructions side by side, aligned by the pairing. The
//! macOS twin is `CompareView`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use romlens_ffi::compare::{
    DataChangeInfo, DiffLineInfo, DiffLineOp, RoutinePairInfo, RoutinePairing,
};
use romlens_ffi::format_file_offset;

use crate::files::alert;
use crate::model::compare::{self, CompareState, Item, bytes};
use crate::model::{Change, Document, Tab};
use crate::style::chip;

struct View {
    doc: Rc<Document>,
    stack: gtk::Stack,
    loading: gtk::Label,
    failed: adw::StatusPage,
    title: gtk::Label,
    summary: gtk::Label,
    carry: gtk::Button,
    list: gtk::ListBox,
    detail: gtk::Box,
    /// The list row of each item.
    rows: RefCell<Vec<(Item, gtk::ListBoxRow)>>,
    shown_revision: Cell<u64>,
    /// The list is being rebuilt or the selection set from the model.
    syncing: Cell<bool>,
}

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let (root, view) = View::build(doc);
    root.connect_destroy(move |_| {
        let _ = &view;
    });
    root
}

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).build();
    l.add_css_class("heading");
    l
}

fn caption(text: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build();
    l.add_css_class("caption");
    l.add_css_class("dim-label");
    l
}

fn mono(text: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    l.add_css_class("monospace");
    l
}

fn pairing_class(p: RoutinePairing) -> &'static str {
    match p {
        RoutinePairing::Same => "chip-gray",
        RoutinePairing::Moved => "chip-blue",
        RoutinePairing::Changed => "chip-orange",
        RoutinePairing::Added => "chip-green",
        RoutinePairing::Removed => "chip-red",
    }
}

impl View {
    fn build(doc: &Rc<Document>) -> (gtk::Widget, Rc<Self>) {
        let idle = adw::StatusPage::builder()
            .icon_name("view-dual-symbolic")
            .title("Nothing to compare")
            .description(
                "File › Compare With… opens another version of this ROM, or its \
                 saved project, beside this one.",
            )
            .build();
        let loading = gtk::Label::new(None);
        let spinner = adw::Spinner::new();
        spinner.set_size_request(32, 32);
        let wait = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        wait.append(&spinner);
        wait.append(&loading);
        let failed = adw::StatusPage::builder()
            .icon_name("dialog-warning-symbolic")
            .title("The comparison failed")
            .build();

        let title = heading("");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_hexpand(true);
        let close = gtk::Button::builder()
            .label("Close")
            .tooltip_text("Stop comparing")
            .action_name("win.close-compare")
            .build();
        let carry = gtk::Button::builder()
            .tooltip_text(
                "Name the routines here that the other version names and this one \
                 does not, as one step you can undo",
            )
            .halign(gtk::Align::Start)
            .build();
        let summary = caption("");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        top.append(&title);
        top.append(&close);
        let header = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_start(10)
            .margin_end(10)
            .margin_top(8)
            .margin_bottom(8)
            .build();
        header.append(&top);
        header.append(&summary);
        header.append(&carry);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        list.add_css_class("navigation-sidebar");
        let list_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .build();
        let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
        left.append(&header);
        left.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        left.append(&list_scroll);
        left.set_width_request(260);

        let detail = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let split = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&left)
            .end_child(&detail)
            .resize_start_child(false)
            .resize_end_child(true)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .position(340)
            .build();

        let stack = gtk::Stack::new();
        stack.add_named(&idle, Some("idle"));
        stack.add_named(&wait, Some("loading"));
        stack.add_named(&failed, Some("failed"));
        stack.add_named(&split, Some("ready"));

        let view = Rc::new(Self {
            doc: Rc::clone(doc),
            stack,
            loading,
            failed,
            title,
            summary,
            carry: carry.clone(),
            list: list.clone(),
            detail,
            rows: RefCell::new(Vec::new()),
            shown_revision: Cell::new(u64::MAX),
            syncing: Cell::new(false),
        });

        let this = Rc::downgrade(&view);
        list.connect_row_selected(move |_, row| {
            let Some(v) = this.upgrade() else { return };
            if v.syncing.get() {
                return;
            }
            let item = row.and_then(|r| {
                v.rows
                    .borrow()
                    .iter()
                    .find(|(_, x)| x == r)
                    .map(|(i, _)| *i)
            });
            v.doc.select_compare_item(item);
        });
        let this = Rc::downgrade(&view);
        carry.connect_clicked(move |b| {
            let Some(v) = this.upgrade() else { return };
            if let Err(e) = v.doc.carry_names() {
                alert(
                    b.root().and_downcast_ref::<gtk::Window>(),
                    "Can't Carry Over Names",
                    &e.to_string(),
                );
            }
        });
        let this = Rc::downgrade(&view);
        doc.subscribe(move |c| {
            if matches!(c, Change::Compare | Change::Layout)
                && let Some(v) = this.upgrade()
            {
                v.update();
            }
        });
        view.update();
        (view.stack.clone().upcast(), view)
    }

    fn update(self: &Rc<Self>) {
        let (state, revision) = {
            let c = self.doc.compare();
            (c.state.clone(), c.revision)
        };
        match state {
            CompareState::Idle => self.stack.set_visible_child_name("idle"),
            CompareState::Loading(what) => {
                self.loading.set_text(&what);
                self.stack.set_visible_child_name("loading");
            }
            CompareState::Failed(message) => {
                self.failed.set_description(Some(&message));
                self.stack.set_visible_child_name("failed");
            }
            CompareState::Ready => {
                if revision != self.shown_revision.get() {
                    self.shown_revision.set(revision);
                    self.fill_list();
                }
                self.sync_selection();
                self.show_detail();
                self.stack.set_visible_child_name("ready");
            }
        }
    }

    // MARK: The list

    fn fill_list(self: &Rc<Self>) {
        self.syncing.set(true);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.rows.borrow_mut().clear();
        let c = self.doc.compare();
        let Some(info) = c.info.as_ref() else {
            self.syncing.set(false);
            return;
        };
        self.title.set_text(&format!(
            "{} → this one",
            c.other_name.as_deref().unwrap_or("The other version")
        ));
        self.summary.set_text(&format!(
            "{} changed, {} inserted, {} deleted, {} the same",
            bytes(info.changed_bytes),
            bytes(info.inserted_bytes),
            bytes(info.deleted_bytes),
            bytes(info.same_bytes)
        ));
        let n = info.names_to_carry.len();
        self.carry.set_visible(n > 0);
        self.carry.set_label(&format!(
            "Carry Over {n} Name{}",
            if n == 1 { "" } else { "s" }
        ));

        self.section(&format!(
            "Routines: {} the same, {} not",
            info.same_routines,
            info.routines.len()
        ));
        for (i, r) in c.routines().iter().enumerate() {
            self.item(Item::Routine(i), routine_row(r));
        }
        if !info.data.is_empty() {
            self.section(&format!("Data with changed bytes: {}", info.data.len()));
            for (i, d) in c.data().iter().enumerate() {
                self.item(Item::Data(i), data_row(d));
            }
        }
        if !info.moves.is_empty() {
            self.section(&format!("Moved blocks: {}", info.moves.len()));
            for (i, m) in c.moves().iter().enumerate() {
                let text = format!(
                    "{} → {}, {}",
                    format_file_offset(m.a),
                    format_file_offset(m.b),
                    bytes(u64::from(m.len))
                );
                self.item(Item::Move(i), mono(&text).upcast());
            }
        }
        self.section(&format!("Bytes that differ: {} stretches", info.runs.len()));
        for (i, r) in c.runs().iter().enumerate() {
            let len = r
                .a_end
                .saturating_sub(r.a_start)
                .max(r.b_end.saturating_sub(r.b_start));
            let text = format!(
                "{} {}, {}",
                compare::run_title(r.kind),
                format_file_offset(r.b_start),
                bytes(u64::from(len))
            );
            self.item(Item::Run(i), mono(&text).upcast());
        }
        if info.runs.len() > c.runs().len() {
            let more = caption(&format!("and {} more", info.runs.len() - c.runs().len()));
            more.set_margin_start(12);
            more.set_margin_top(4);
            let row = gtk::ListBoxRow::builder()
                .selectable(false)
                .activatable(false)
                .child(&more)
                .build();
            self.list.append(&row);
        }
        self.syncing.set(false);
    }

    fn section(&self, text: &str) {
        let l = heading(text);
        l.set_margin_start(6);
        l.set_margin_top(8);
        l.set_margin_bottom(2);
        let row = gtk::ListBoxRow::builder()
            .selectable(false)
            .activatable(false)
            .child(&l)
            .build();
        self.list.append(&row);
    }

    fn item(&self, item: Item, content: gtk::Widget) {
        content.set_margin_start(6);
        content.set_margin_end(6);
        content.set_margin_top(3);
        content.set_margin_bottom(3);
        let row = gtk::ListBoxRow::builder().child(&content).build();
        self.list.append(&row);
        self.rows.borrow_mut().push((item, row));
    }

    fn sync_selection(&self) {
        let selected = self.doc.compare().selected;
        self.syncing.set(true);
        match selected.and_then(|s| {
            self.rows
                .borrow()
                .iter()
                .find(|(i, _)| *i == s)
                .map(|(_, r)| r.clone())
        }) {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
        self.syncing.set(false);
    }

    // MARK: The detail

    fn show_detail(&self) {
        while let Some(child) = self.detail.first_child() {
            self.detail.remove(&child);
        }
        let c = self.doc.compare();
        let Some(info) = c.info.as_ref() else { return };
        let other = c
            .other_name
            .clone()
            .unwrap_or_else(|| "The other version".into());
        let page = match c.selected {
            Some(Item::Routine(i)) if i < c.routines().len() => {
                self.routine_detail(&c.routines()[i], &other)
            }
            Some(Item::Data(i)) if i < info.data.len() => {
                let d = &info.data[i];
                self.place(
                    d.name.as_deref().unwrap_or(&d.kind),
                    &format!(
                        "{} changed in this {} region of {}, {}-{} in {}.",
                        bytes(u64::from(d.changed)),
                        d.kind,
                        bytes(u64::from(d.len)),
                        format_file_offset(d.start),
                        format_file_offset(d.start + d.len),
                        if d.in_b {
                            "this version"
                        } else {
                            "the other version"
                        }
                    ),
                    d.in_b.then_some(d.start),
                )
            }
            Some(Item::Run(i)) if i < c.runs().len() => {
                let r = &c.runs()[i];
                self.place(
                    &format!("Bytes {}", compare::run_title(r.kind)),
                    &format!(
                        "The other version's {}-{} is this one's {}-{}.",
                        format_file_offset(r.a_start),
                        format_file_offset(r.a_end),
                        format_file_offset(r.b_start),
                        format_file_offset(r.b_end)
                    ),
                    (r.kind != romlens_ffi::compare::ByteRunKind::Deleted).then_some(r.b_start),
                )
            }
            Some(Item::Move(i)) if i < info.moves.len() => {
                let m = &info.moves[i];
                self.place(
                    "A block moved",
                    &format!(
                        "{} at {} in the other version are at {} in this one.",
                        bytes(u64::from(m.len)),
                        format_file_offset(m.a),
                        format_file_offset(m.b)
                    ),
                    Some(m.b),
                )
            }
            _ => {
                let page = adw::StatusPage::builder()
                    .icon_name("view-list-symbolic")
                    .title("Choose a change")
                    .description(
                        "A changed routine shows both versions' instructions side by side.",
                    )
                    .vexpand(true)
                    .build();
                page.upcast()
            }
        };
        self.detail.append(&page);
    }

    fn place(&self, title: &str, text: &str, offset: Option<u32>) -> gtk::Widget {
        let b = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .margin_start(16)
            .margin_end(16)
            .margin_top(16)
            .build();
        let t = heading(title);
        t.add_css_class("title-3");
        b.append(&t);
        let body = gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .build();
        b.append(&body);
        if let Some(offset) = offset {
            let show = gtk::Button::builder()
                .label("Show in Hex")
                .halign(gtk::Align::Start)
                .build();
            let doc = Rc::clone(&self.doc);
            show.connect_clicked(move |_| {
                doc.set_tab(Tab::Hex);
                doc.jump_to(offset);
            });
            b.append(&show);
        }
        b.upcast()
    }

    fn routine_detail(&self, r: &RoutinePairInfo, other: &str) -> gtk::Widget {
        let name =
            r.b.as_ref()
                .or(r.a.as_ref())
                .map_or_else(String::new, |s| s.name.clone());
        let title = heading(&name);
        title.add_css_class("title-3");
        let names = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();
        names.append(&title);
        names.append(&caption(&compare::pair_summary(r)));
        let top = gtk::Box::builder()
            .spacing(8)
            .margin_start(10)
            .margin_end(10)
            .margin_top(10)
            .margin_bottom(10)
            .build();
        top.append(&names);
        if let Some(b) = &r.b {
            let show = gtk::Button::builder()
                .label("Show in Listing")
                .valign(gtk::Align::Center)
                .build();
            let (doc, offset) = (Rc::clone(&self.doc), b.offset);
            show.connect_clicked(move |_| {
                doc.set_tab(Tab::Disassembly);
                doc.jump_to(offset);
            });
            top.append(&show);
        }
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.set_vexpand(true);
        page.append(&top);
        page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        if r.lines.is_empty() {
            let l = gtk::Label::builder()
                .label(compare::pair_summary(r))
                .xalign(0.0)
                .margin_start(16)
                .margin_top(16)
                .build();
            l.add_css_class("dim-label");
            page.append(&l);
            return page.upcast();
        }
        let heads = gtk::Grid::builder()
            .column_homogeneous(true)
            .column_spacing(12)
            .margin_start(10)
            .margin_end(10)
            .margin_top(4)
            .margin_bottom(4)
            .build();
        heads.attach(&caption(other), 0, 0, 1, 1);
        heads.attach(&caption("This version"), 1, 0, 1, 1);
        page.append(&heads);
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        rows.set_margin_start(10);
        rows.set_margin_end(10);
        for l in &r.lines {
            rows.append(&self.line_row(l));
        }
        page.append(
            &gtk::ScrolledWindow::builder()
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .child(&rows)
                .build(),
        );
        page.upcast()
    }

    fn line_row(&self, l: &DiffLineInfo) -> gtk::Widget {
        let row = gtk::Grid::builder()
            .column_homogeneous(true)
            .column_spacing(12)
            .build();
        match l.op {
            DiffLineOp::Same => {}
            DiffLineOp::Changed => row.add_css_class("diff-changed"),
            DiffLineOp::Added => row.add_css_class("diff-added"),
            DiffLineOp::Removed => row.add_css_class("diff-removed"),
        }
        row.attach(&cell(l.a_offset, l.a_text.as_deref()), 0, 0, 1, 1);
        let b = cell(l.b_offset, l.b_text.as_deref());
        if let Some(offset) = l.b_offset {
            // A click on this version's side selects that instruction.
            let click = gtk::GestureClick::new();
            let doc = Rc::clone(&self.doc);
            click.connect_pressed(move |_, _, _, _| doc.select(Some(offset)));
            b.add_controller(click);
        }
        row.attach(&b, 1, 0, 1, 1);
        row.upcast()
    }
}

fn cell(offset: Option<u32>, text: Option<&str>) -> gtk::Box {
    let b = gtk::Box::builder().spacing(8).hexpand(true).build();
    let o = mono(&offset.map_or_else(String::new, format_file_offset));
    o.add_css_class("dim-label");
    o.set_width_chars(8);
    b.append(&o);
    b.append(&mono(text.unwrap_or("")));
    b
}

fn routine_row(r: &RoutinePairInfo) -> gtk::Widget {
    let b = gtk::Box::builder().spacing(8).build();
    let tag = chip(compare::pairing_title(r.pairing), pairing_class(r.pairing));
    tag.set_width_chars(8);
    tag.set_valign(gtk::Align::Start);
    b.append(&tag);
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    let name = gtk::Label::builder()
        .label(
            r.b.as_ref()
                .or(r.a.as_ref())
                .map_or("", |s| s.name.as_str()),
        )
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    text.append(&name);
    let at = mono(&compare::routine_where(r));
    at.add_css_class("caption");
    at.add_css_class("dim-label");
    text.append(&at);
    b.append(&text);
    b.upcast()
}

fn data_row(d: &DataChangeInfo) -> gtk::Widget {
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    text.append(
        &gtk::Label::builder()
            .label(d.name.as_deref().unwrap_or(&d.kind))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    let at = mono(&format!(
        "{}, {} of {}{}",
        format_file_offset(d.start),
        bytes(u64::from(d.changed)),
        bytes(u64::from(d.len)),
        if d.in_b { ", only in this one" } else { "" }
    ));
    at.add_css_class("caption");
    at.add_css_class("dim-label");
    text.append(&at);
    text.upcast()
}
