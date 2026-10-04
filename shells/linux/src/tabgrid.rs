//! The editor area (docs/29): tab groups laid out in a grid. Each group is a
//! libadwaita tab bar over a tab view; the grid is nested `gtk::Paned`s built
//! from the workspace's tree. The document's workspace is the truth: a click,
//! a close or a drag in a tab bar tells the document, and the grid makes the
//! widgets match whatever the workspace then says. The macOS twins are
//! `EditorGridView`, `TabBarView` and `EditorItemBody`.
//!
//! Each tab's view is made once and kept while the tab exists, hidden when
//! another tab shows, so its scroll and zoom survive switching.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::model::workspace::{
    CodeRep, DropEdge, DropTarget, DropZone, EditorContent, EditorItem, Id, LayoutNode, Rect,
    SplitAxis, TabDrop, drop_zone,
};
use crate::model::{Change, Document};
use crate::{
    asmview, atlasview, audioview, compareview, cview, graphicsview, graphview, hexview, lockstep,
    sourceview,
};

/// Below this window width only the focused group shows; the layout is kept
/// and comes back when the window widens.
pub const COMPACT_WIDTH: f64 = 1100.0;

/// No group is squeezed narrower or shorter than this.
const MIN_PANE: i32 = 160;

// MARK: A tab's view

/// The view a tab shows, made once for the tab.
fn content_view(doc: &Rc<Document>, item: &EditorItem) -> gtk::Widget {
    let id = Some(item.id);
    let view: gtk::Widget = match item.content {
        EditorContent::Code(CodeRep::Hex) => hexview::build(doc, id).widget,
        EditorContent::Code(CodeRep::Assembly) => asmview::build(doc, id).widget,
        EditorContent::Code(CodeRep::Both) => lockstep::build(doc, id),
        EditorContent::Code(CodeRep::C) => cview::build(doc, id),
        EditorContent::Code(CodeRep::Graph) => graphview::build(doc, id),
        EditorContent::Atlas => atlasview::build(doc),
        EditorContent::Source => sourceview::build(doc),
        EditorContent::Compare => compareview::build(doc),
        EditorContent::Graphics(t) => graphicsview::build(doc, t),
        EditorContent::Audio(t) => audioview::build(doc, t),
        EditorContent::Tutor => adw::StatusPage::builder()
            .icon_name("help-about-symbolic")
            .title("Tutor")
            .description("The tutor opens in its own window for now: View › Tutor (Alt+Shift+T).")
            .build()
            .upcast(),
        EditorContent::Header => adw::StatusPage::builder()
            .title("Header and Vectors")
            .description("The inspector shows the header and vectors.")
            .build()
            .upcast(),
    };
    let view = if needs_analysis(item.content) {
        with_analyzing(doc, view)
    } else {
        view
    };
    // A group may be narrower than a view's controls want: the view scrolls
    // sideways rather than being cut off or holding the divider back.
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .child(&view)
        .hexpand(true)
        .vexpand(true)
        .build()
        .upcast()
}

fn needs_analysis(content: EditorContent) -> bool {
    matches!(
        content,
        EditorContent::Code(CodeRep::Assembly | CodeRep::C | CodeRep::Graph | CodeRep::Both)
            | EditorContent::Source
    )
}

/// A listing, with a note in its place until the first analysis lands.
fn with_analyzing(doc: &Rc<Document>, view: gtk::Widget) -> gtk::Widget {
    let stack = gtk::Stack::new();
    stack.add_named(&view, Some("view"));
    stack.add_named(
        &adw::StatusPage::builder()
            .icon_name("system-run-symbolic")
            .title("Analyzing…")
            .description(
                "The disassembly appears when the first analysis finishes. \
                 The Hex tab works meanwhile.",
            )
            .build(),
        Some("analyzing"),
    );
    let show = {
        let (stack, doc) = (stack.clone(), Rc::downgrade(doc));
        move || {
            let Some(doc) = doc.upgrade() else { return };
            stack.set_visible_child_name(if doc.has_disassembly() {
                "view"
            } else {
                "analyzing"
            });
        }
    };
    show();
    doc.subscribe(move |c| {
        if matches!(c, Change::Rows | Change::Status) {
            show();
        }
    });
    stack.upcast()
}

/// A tab's icon, from the desktop's symbolic set.
pub fn icon_name(content: EditorContent) -> &'static str {
    match content {
        EditorContent::Code(CodeRep::Assembly) => "format-justify-left-symbolic",
        EditorContent::Code(CodeRep::C) => "text-x-script-symbolic",
        EditorContent::Code(CodeRep::Graph) => "view-paged-symbolic",
        EditorContent::Code(CodeRep::Hex) => "view-grid-symbolic",
        EditorContent::Code(CodeRep::Both) => "view-dual-symbolic",
        EditorContent::Header => "dialog-information-symbolic",
        EditorContent::Atlas => "view-app-grid-symbolic",
        EditorContent::Compare => "edit-copy-symbolic",
        EditorContent::Source => "text-x-generic-symbolic",
        EditorContent::Graphics(_) => "image-x-generic-symbolic",
        EditorContent::Audio(_) => "audio-x-generic-symbolic",
        EditorContent::Tutor => "help-about-symbolic",
    }
}

/// The tab a page shows, kept in its view's widget name.
fn item_of(page: &adw::TabPage) -> Option<Id> {
    page.child().widget_name().parse().ok()
}

// MARK: A group

struct Group {
    id: Id,
    root: gtk::Box,
    view: adw::TabView,
    bar: adw::TabBar,
    body: gtk::Stack,
    /// Where a drop would land, shown over the group's view while dragging.
    preview: gtk::Box,
    /// The tab the tab menu was opened on.
    menu_item: Cell<Option<Id>>,
    follow: gio::SimpleAction,
}

impl Group {
    fn build(grid: &Rc<Grid>, id: Id) -> Rc<Self> {
        let view = adw::TabView::new();
        // Alt+digit are the view keys and the menus own the tab keys.
        view.set_shortcuts(adw::TabViewShortcuts::NONE);
        view.set_vexpand(true);
        let bar = adw::TabBar::builder()
            .view(&view)
            .autohide(false)
            // Tabs share the bar, squeezing before it scrolls.
            .expand_tabs(true)
            .build();
        let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        for (icon, tip, edge) in [
            (
                "view-dual-symbolic",
                "Split Right (Ctrl+\\)",
                DropEdge::Right,
            ),
            (
                "view-paged-symbolic",
                "Split Down (Ctrl+Shift+\\)",
                DropEdge::Bottom,
            ),
        ] {
            let b = gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tip)
                .build();
            b.add_css_class("flat");
            let (g, doc) = (Rc::downgrade(grid), Rc::downgrade(&grid.doc));
            b.connect_clicked(move |_| {
                let (Some(_), Some(doc)) = (g.upgrade(), doc.upgrade()) else {
                    return;
                };
                doc.focus_group(id);
                doc.split_focused(edge);
            });
            split.append(&b);
        }
        bar.set_end_action_widget(Some(&split));

        let empty = adw::StatusPage::builder()
            .icon_name("view-grid-symbolic")
            .description("Choose a view in the View menu, or drag a tab here.")
            .build();
        empty.add_css_class("compact");
        let body = gtk::Stack::new();
        body.add_named(&view, Some("tabs"));
        body.add_named(&empty, Some("empty"));
        body.set_vexpand(true);

        // The drop preview: translucent accent over the whole view, or the
        // half on the side a tab would split to. It takes no events.
        let preview = gtk::Box::new(gtk::Orientation::Vertical, 0);
        preview.add_css_class("romlens-drop-preview");
        preview.set_can_target(false);
        preview.set_visible(false);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&body));
        overlay.add_overlay(&preview);
        overlay.set_vexpand(true);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("romlens-group");
        root.append(&bar);
        root.append(&overlay);
        root.set_size_request(MIN_PANE, MIN_PANE);

        let follow = gio::SimpleAction::new_stateful("follow", None, &true.to_variant());
        let group = Rc::new(Self {
            id,
            root,
            view,
            bar,
            body,
            preview,
            menu_item: Cell::new(None),
            follow,
        });
        group.wire(grid);
        group
    }

    fn wire(self: &Rc<Self>, grid: &Rc<Grid>) {
        let id = self.id;
        let weak_grid = Rc::downgrade(grid);

        // A click anywhere in the group gives it focus, before the click does
        // whatever it does.
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let g = weak_grid.clone();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(grid) = g.upgrade()
                && grid.doc.workspace().focused_group() != id
            {
                grid.doc.focus_group(id);
            }
        });
        self.root.add_controller(click);

        let g = weak_grid.clone();
        self.view.connect_selected_page_notify(move |v| {
            let Some(grid) = g.upgrade() else { return };
            if grid.syncing.get() {
                return;
            }
            if let Some(item) = v.selected_page().as_ref().and_then(item_of) {
                grid.doc.focus_item(item);
            }
        });

        // The close button and a middle click. The page goes now; the
        // document closes the tab after this signal returns.
        let g = weak_grid.clone();
        self.view.connect_close_page(move |v, page| {
            v.close_page_finish(page, true);
            let Some(grid) = g.upgrade() else {
                return glib::Propagation::Stop;
            };
            if !grid.syncing.get()
                && let Some(item) = item_of(page)
            {
                grid.forget(item);
                let doc = Rc::downgrade(&grid.doc);
                glib::idle_add_local_once(move || {
                    if let Some(doc) = doc.upgrade() {
                        doc.close_item(item);
                    }
                });
            }
            glib::Propagation::Stop
        });

        // A tab dragged within the bar: the same move in the layout. Its
        // position here is where it ended up, which the layout's move counts
        // before taking the tab out.
        let g = weak_grid.clone();
        self.view.connect_page_reordered(move |_, page, position| {
            let Some(grid) = g.upgrade() else { return };
            if grid.syncing.get() {
                return;
            }
            let Some(item) = item_of(page) else { return };
            let from = grid
                .doc
                .workspace()
                .layout()
                .group(id)
                .and_then(|grp| grp.items.iter().position(|i| i.id == item));
            let to = position.max(0) as usize;
            let index = if from.is_some_and(|f| to > f) {
                to + 1
            } else {
                to
            };
            grid.defer_drop(TabDrop::Item(item), id, DropTarget::TabBar(index));
        });

        // A tab dragged here from another group's bar.
        let g = weak_grid.clone();
        self.view.connect_page_attached(move |_, page, position| {
            let Some(grid) = g.upgrade() else { return };
            if grid.syncing.get() {
                return;
            }
            let Some(item) = item_of(page) else { return };
            grid.page_group.borrow_mut().insert(item, id);
            grid.defer_drop(
                TabDrop::Item(item),
                id,
                DropTarget::TabBar(position.max(0) as usize),
            );
        });

        // A tab is never torn off into a window of its own.
        self.view.connect_create_window(|_| None);

        self.wire_drops(grid);

        // The tab menu, for the tab it was opened on.
        let menu = gio::Menu::new();
        let close = gio::Menu::new();
        close.append(Some("Close Tab"), Some("tab.close"));
        close.append(Some("Close Other Tabs"), Some("tab.close-others"));
        let split = gio::Menu::new();
        split.append(Some("Split Right"), Some("tab.split-right"));
        split.append(Some("Split Down"), Some("tab.split-down"));
        let follow = gio::Menu::new();
        follow.append(Some("Follow Selection"), Some("tab.follow"));
        menu.append_section(None, &close);
        menu.append_section(None, &split);
        menu.append_section(None, &follow);
        self.view.set_menu_model(Some(&menu));
        let (g, me) = (weak_grid.clone(), Rc::downgrade(self));
        self.view.connect_setup_menu(move |_, page| {
            let (Some(grid), Some(me)) = (g.upgrade(), me.upgrade()) else {
                return;
            };
            let item = page.and_then(item_of);
            me.menu_item.set(item);
            let follows = item
                .and_then(|i| grid.doc.workspace().layout().item(i))
                .is_some_and(|i| i.follows_selection);
            me.follow.set_state(&follows.to_variant());
        });
        let actions = gio::SimpleActionGroup::new();
        let add = |name: &str, run: fn(&Rc<Document>, Id, Id)| {
            let a = gio::SimpleAction::new(name, None);
            let (g, me) = (weak_grid.clone(), Rc::downgrade(self));
            a.connect_activate(move |_, _| {
                let (Some(grid), Some(me)) = (g.upgrade(), me.upgrade()) else {
                    return;
                };
                if let Some(item) = me.menu_item.get() {
                    run(&grid.doc, me.id, item);
                }
            });
            actions.add_action(&a);
        };
        add("close", |doc, _, item| doc.close_item(item));
        add("close-others", |doc, group, item| {
            let others: Vec<Id> = doc
                .workspace()
                .layout()
                .group(group)
                .map(|g| {
                    g.items
                        .iter()
                        .map(|i| i.id)
                        .filter(|i| *i != item)
                        .collect()
                })
                .unwrap_or_default();
            for o in others {
                doc.close_item(o);
            }
        });
        add("split-right", |doc, _, item| {
            doc.focus_item(item);
            doc.split_focused(DropEdge::Right);
        });
        add("split-down", |doc, _, item| {
            doc.focus_item(item);
            doc.split_focused(DropEdge::Bottom);
        });
        let (g, me) = (weak_grid, Rc::downgrade(self));
        self.follow.connect_activate(move |a, _| {
            let (Some(grid), Some(me)) = (g.upgrade(), me.upgrade()) else {
                return;
            };
            let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
            a.set_state(&on.to_variant());
            if let Some(item) = me.menu_item.get() {
                grid.doc.set_follows_selection(item, on);
            }
        });
        actions.add_action(&self.follow);
        self.root.insert_action_group("tab", Some(&actions));
    }
}

impl Group {
    /// Drops on the group's view (a tab, or a row of the sidebar to open):
    /// the middle moves it here, the outer third of a side splits there, as
    /// the preview shows. Rows of the sidebar dropped on the tab bar open at
    /// that place; tabs dragged between bars are the bar's own.
    fn wire_drops(self: &Rc<Self>, grid: &Rc<Grid>) {
        let id = self.id;
        let target = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::MOVE);
        target.set_types(&[adw::TabPage::static_type(), String::static_type()]);
        target.set_actions(gdk::DragAction::MOVE | gdk::DragAction::COPY);
        let me = Rc::downgrade(self);
        target.connect_motion(move |t, x, y| {
            let Some(me) = me.upgrade() else {
                return gdk::DragAction::empty();
            };
            let zone = me.zone_at(t, x, y);
            me.show_preview(zone);
            gdk::DragAction::MOVE
        });
        let me = Rc::downgrade(self);
        target.connect_leave(move |_| {
            if let Some(me) = me.upgrade() {
                me.preview.set_visible(false);
            }
        });
        let (me, g) = (Rc::downgrade(self), Rc::downgrade(grid));
        target.connect_drop(move |t, value, x, y| {
            let (Some(me), Some(grid)) = (me.upgrade(), g.upgrade()) else {
                return false;
            };
            me.preview.set_visible(false);
            let Some(drop) = tab_drop(value) else {
                return false;
            };
            let zone = me.zone_at(t, x, y);
            grid.defer_drop(drop, id, DropTarget::Zone(zone));
            true
        });
        if let Some(over) = self.preview.parent() {
            over.add_controller(target);
        }

        // Sidebar rows dropped between two tabs of the bar.
        self.bar.setup_extra_drop_target(
            gdk::DragAction::COPY | gdk::DragAction::MOVE,
            &[String::static_type()],
        );
        let g = Rc::downgrade(grid);
        let view = self.view.clone();
        self.bar.connect_extra_drag_drop(move |_, page, value| {
            let Some(grid) = g.upgrade() else {
                return false;
            };
            let Some(drop) = value
                .get::<String>()
                .ok()
                .and_then(|s| TabDrop::from_json(&s))
            else {
                return false;
            };
            let at = view.page_position(page).max(0) as usize;
            grid.defer_drop(drop, id, DropTarget::TabBar(at));
            true
        });
    }

    /// The zone of the group's view a point in the drop target is in.
    fn zone_at(&self, target: &gtk::DropTarget, x: f64, y: f64) -> DropZone {
        let Some(w) = target.widget() else {
            return DropZone::Center;
        };
        drop_zone(
            x,
            y,
            Rect {
                x: 0.0,
                y: 0.0,
                width: f64::from(w.width()),
                height: f64::from(w.height()),
            },
        )
    }

    /// The preview for a zone: the whole view or the half by that edge,
    /// inset 4 px.
    fn show_preview(&self, zone: DropZone) {
        let Some(over) = self.preview.parent() else {
            return;
        };
        let (w, h) = (over.width(), over.height());
        let (mut top, mut bottom, mut start, mut end) = (4, 4, 4, 4);
        match zone {
            DropZone::Center => {}
            DropZone::Edge(DropEdge::Left) => end = w / 2,
            DropZone::Edge(DropEdge::Right) => start = w / 2,
            DropZone::Edge(DropEdge::Top) => bottom = h / 2,
            DropZone::Edge(DropEdge::Bottom) => top = h / 2,
        }
        self.preview.set_margin_top(top);
        self.preview.set_margin_bottom(bottom);
        self.preview.set_margin_start(start);
        self.preview.set_margin_end(end);
        self.preview.set_visible(true);
    }
}

/// What a drop carries: a tab dragged from a bar, or a sidebar row's
/// `TabDrop` as JSON.
fn tab_drop(value: &glib::Value) -> Option<TabDrop> {
    if let Ok(page) = value.get::<adw::TabPage>() {
        return item_of(&page).map(TabDrop::Item);
    }
    value
        .get::<String>()
        .ok()
        .and_then(|s| TabDrop::from_json(&s))
}

/// A drag source carrying `drop`, for a row of the sidebar.
pub fn drag_source(drop: impl Fn() -> Option<TabDrop> + 'static) -> gtk::DragSource {
    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::COPY);
    source.connect_prepare(move |_, _, _| {
        drop().map(|d| gdk::ContentProvider::for_value(&d.to_json().to_value()))
    });
    source
}

// MARK: The grid

pub struct Grid {
    doc: Rc<Document>,
    /// Holds the tree of panes.
    pub root: gtk::Box,
    groups: RefCell<HashMap<Id, Rc<Group>>>,
    /// Each tab's page, and the group whose view holds it.
    pages: RefCell<HashMap<Id, adw::TabPage>>,
    page_group: RefCell<HashMap<Id, Id>>,
    /// The panes of each split, in order: one fewer than its children.
    panes: RefCell<HashMap<Id, Vec<gtk::Paned>>>,
    /// The tree's shape as last built, so a change of focus or of a tab does
    /// not rebuild it.
    shape: RefCell<String>,
    /// The widgets are being made to match the workspace: their signals are
    /// not the person's doing.
    syncing: Cell<bool>,
    /// Only the focused group shows (a narrow window).
    compact: Cell<bool>,
}

impl Grid {
    pub fn build(doc: &Rc<Document>) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_vexpand(true);
        root.set_hexpand(true);
        let grid = Rc::new(Self {
            doc: Rc::clone(doc),
            root,
            groups: RefCell::new(HashMap::new()),
            pages: RefCell::new(HashMap::new()),
            page_group: RefCell::new(HashMap::new()),
            panes: RefCell::new(HashMap::new()),
            shape: RefCell::new(String::new()),
            syncing: Cell::new(false),
            compact: Cell::new(false),
        });
        grid.sync();
        let weak: Weak<Self> = Rc::downgrade(&grid);
        doc.subscribe(move |c| {
            let Some(grid) = weak.upgrade() else { return };
            match c {
                Change::Layout => grid.sync(),
                // Twin tabs are named by their routine, which moves with the
                // selection; and titles say when a view is waiting.
                Change::Selection | Change::Navigator => grid.retitle(),
                _ => {}
            }
        });
        grid
    }

    /// A narrow window shows only the focused group.
    pub fn set_compact(self: &Rc<Self>, compact: bool) {
        if self.compact.replace(compact) != compact {
            self.sync();
        }
    }

    /// A drop's change, after the tab bar has finished with its own signal.
    fn defer_drop(self: &Rc<Self>, drop: TabDrop, group: Id, target: DropTarget) {
        let doc = Rc::downgrade(&self.doc);
        glib::idle_add_local_once(move || {
            if let Some(doc) = doc.upgrade() {
                doc.drop_tab(drop, group, target);
            }
        });
    }

    /// A page already gone from its view.
    fn forget(&self, item: Id) {
        self.pages.borrow_mut().remove(&item);
        self.page_group.borrow_mut().remove(&item);
    }

    /// Make the widgets match the workspace.
    pub fn sync(self: &Rc<Self>) {
        self.syncing.set(true);
        let (layout, focused) = {
            let w = self.doc.workspace();
            (w.layout().clone(), w.focused_group())
        };

        // Every group in the layout has its widgets.
        for g in layout.groups() {
            if !self.groups.borrow().contains_key(&g.id) {
                let group = Group::build(self, g.id);
                self.groups.borrow_mut().insert(g.id, group);
            }
        }

        // Each group's pages, in order: made, moved in from another group, or
        // put in place.
        for g in layout.groups() {
            let group = Rc::clone(&self.groups.borrow()[&g.id]);
            for (position, item) in g.items.iter().enumerate() {
                let position = position as i32;
                let existing = self.pages.borrow().get(&item.id).cloned();
                match existing {
                    None => {
                        let child = content_view(&self.doc, item);
                        child.set_widget_name(&item.id.to_string());
                        let page = group.view.insert(&child, position);
                        self.pages.borrow_mut().insert(item.id, page);
                        self.page_group.borrow_mut().insert(item.id, g.id);
                    }
                    Some(page) => {
                        let from = self.page_group.borrow().get(&item.id).copied();
                        if from != Some(g.id) {
                            let source = from.and_then(|f| self.groups.borrow().get(&f).cloned());
                            if let Some(source) = source
                                && source.view.page_position(&page) >= 0
                            {
                                source.view.transfer_page(&page, &group.view, position);
                            }
                            self.page_group.borrow_mut().insert(item.id, g.id);
                        } else if group.view.page_position(&page) != position {
                            group.view.reorder_page(&page, position);
                        }
                    }
                }
            }
            if let Some(page) = g
                .selected
                .and_then(|s| self.pages.borrow().get(&s).cloned())
            {
                group.view.set_selected_page(&page);
            }
            group
                .body
                .set_visible_child_name(if g.items.is_empty() { "empty" } else { "tabs" });
            if g.id == focused {
                group.root.add_css_class("focused");
            } else {
                group.root.remove_css_class("focused");
            }
        }

        // Tabs closed: their pages go.
        let kept: Vec<Id> = layout.items().iter().map(|i| i.id).collect();
        let gone: Vec<Id> = self
            .pages
            .borrow()
            .keys()
            .copied()
            .filter(|i| !kept.contains(i))
            .collect();
        for item in gone {
            let page = self.pages.borrow_mut().remove(&item);
            let group = self.page_group.borrow_mut().remove(&item);
            if let (Some(page), Some(group)) = (page, group)
                && let Some(g) = self.groups.borrow().get(&group)
                && g.view.page_position(&page) >= 0
            {
                g.view.close_page(&page);
            }
        }

        // Groups gone from the layout.
        let groups: Vec<Id> = layout.groups().iter().map(|g| g.id).collect();
        self.groups.borrow_mut().retain(|id, _| groups.contains(id));

        // The tree, rebuilt only when its shape changed.
        let shape = format!(
            "{}{}",
            shape_of(&layout.root),
            if self.compact.get() {
                format!("c{focused}")
            } else {
                String::new()
            }
        );
        if *self.shape.borrow() != shape {
            *self.shape.borrow_mut() = shape;
            self.rebuild(&layout.root, focused);
        }
        self.apply_fractions(&layout.root);
        self.retitle();
        self.syncing.set(false);
    }

    /// Every tab's title and icon, as the document names it now.
    fn retitle(&self) {
        let items = self.doc.workspace().layout().items();
        for item in items {
            if let Some(page) = self.pages.borrow().get(&item.id) {
                let title = self.doc.title_of(&item);
                if page.title() != title {
                    page.set_title(&title);
                    page.set_tooltip(&title);
                }
                if page.icon().is_none() {
                    page.set_icon(Some(&gio::ThemedIcon::new(icon_name(item.content))));
                }
            }
        }
    }

    fn rebuild(self: &Rc<Self>, root: &LayoutNode, focused: Id) {
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        // Take every group out of the old tree first, so each can be put in
        // the new one.
        for g in self.groups.borrow().values() {
            unparent(g.root.upcast_ref());
        }
        self.panes.borrow_mut().clear();
        let tree = if self.compact.get() {
            self.groups
                .borrow()
                .get(&focused)
                .map(|g| g.root.clone().upcast::<gtk::Widget>())
        } else {
            Some(self.build_node(root))
        };
        if let Some(tree) = tree {
            tree.set_vexpand(true);
            tree.set_hexpand(true);
            self.root.append(&tree);
        }
    }

    fn build_node(self: &Rc<Self>, node: &LayoutNode) -> gtk::Widget {
        match node {
            LayoutNode::Group(g) => self.groups.borrow()[&g.id].root.clone().upcast(),
            LayoutNode::Split(s) => {
                let children: Vec<gtk::Widget> =
                    s.children.iter().map(|c| self.build_node(c)).collect();
                let orientation = match s.axis {
                    SplitAxis::Horizontal => gtk::Orientation::Horizontal,
                    SplitAxis::Vertical => gtk::Orientation::Vertical,
                };
                // n children as a chain of panes, each holding one child and
                // the rest.
                let mut rest = children.last().cloned().expect("a split has children");
                let mut panes = Vec::new();
                for child in children.iter().rev().skip(1) {
                    let paned = gtk::Paned::builder()
                        .orientation(orientation)
                        .start_child(child)
                        .end_child(&rest)
                        .resize_start_child(true)
                        .resize_end_child(true)
                        // A view's controls may want more room than a
                        // group has: the divider decides, and the view
                        // scrolls (`content_view`).
                        .shrink_start_child(true)
                        .shrink_end_child(true)
                        .wide_handle(false)
                        .build();
                    panes.push(paned.clone());
                    rest = paned.upcast();
                }
                panes.reverse();
                for (i, paned) in panes.iter().enumerate() {
                    self.wire_paned(paned, s.id, i);
                }
                self.panes.borrow_mut().insert(s.id, panes);
                rest
            }
        }
    }

    /// A pane's divider: placed from the split's fractions once there is room,
    /// written back when dragged, and equalized on a double-click.
    fn wire_paned(self: &Rc<Self>, paned: &gtk::Paned, split: Id, index: usize) {
        let weak = Rc::downgrade(self);
        paned.connect_notify_local(Some("max-position"), move |_, _| {
            if let Some(grid) = weak.upgrade() {
                grid.place(split);
            }
        });
        let weak = Rc::downgrade(self);
        // GTK places a new divider itself before it has room; only a divider
        // already placed from the fractions, and moved since, was dragged.
        paned.connect_notify_local(Some("position"), move |p, _| {
            let Some(grid) = weak.upgrade() else { return };
            if !grid.syncing.get() && p.is_position_set() {
                grid.dragged(split);
            }
        });
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |gesture, n, x, y| {
            let Some(grid) = weak.upgrade() else { return };
            let Some(paned) = gesture.widget().and_downcast::<gtk::Paned>() else {
                return;
            };
            let along = if paned.orientation() == gtk::Orientation::Horizontal {
                x
            } else {
                y
            };
            if n == 2 && (along - f64::from(paned.position())).abs() < 8.0 {
                grid.doc.equalize_split(split);
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        paned.add_controller(click);
        let _ = index;
    }

    /// Put each split's dividers where its fractions say.
    fn apply_fractions(&self, node: &LayoutNode) {
        if let LayoutNode::Split(s) = node {
            self.place(s.id);
            s.children.iter().for_each(|c| self.apply_fractions(c));
        }
    }

    fn place(&self, split: Id) {
        let fractions = match self.doc.workspace().layout().split_node(split) {
            Some(s) => s.fractions.clone(),
            None => return,
        };
        let panes = self.panes.borrow();
        let Some(panes) = panes.get(&split) else {
            return;
        };
        let was = self.syncing.replace(true);
        for (i, paned) in panes.iter().enumerate() {
            // Before it has room a pane's range is unbounded.
            let size = paned.max_position();
            if size <= 0 || size >= i32::MAX / 2 {
                continue;
            }
            let rest: f64 = fractions[i..].iter().sum();
            let share = if rest > 0.0 { fractions[i] / rest } else { 0.5 };
            let at = ((share * f64::from(size)).round() as i32)
                .clamp(MIN_PANE.min(size / 2), (size - MIN_PANE).max(size / 2));
            if (paned.position() - at).abs() > 1 {
                paned.set_position(at);
            }
        }
        self.syncing.set(was);
    }

    /// A divider was dragged: the split's fractions from where its dividers
    /// are now.
    fn dragged(&self, split: Id) {
        let panes = self.panes.borrow();
        let Some(panes) = panes.get(&split) else {
            return;
        };
        let mut fractions = Vec::new();
        let mut remaining = 1.0;
        for paned in panes {
            let size = f64::from(paned.max_position());
            if size <= 0.0 || size >= f64::from(i32::MAX / 2) {
                return;
            }
            let share = (f64::from(paned.position()) / size).clamp(0.0, 1.0);
            fractions.push(remaining * share);
            remaining *= 1.0 - share;
        }
        fractions.push(remaining);
        self.doc.set_split_fractions(split, &fractions);
    }
}

/// The tree's shape: groups and splits by id, with each split's axis.
fn shape_of(node: &LayoutNode) -> String {
    match node {
        LayoutNode::Group(g) => format!("g{}", g.id),
        LayoutNode::Split(s) => format!(
            "{}{}({})",
            if s.axis == SplitAxis::Horizontal {
                "H"
            } else {
                "V"
            },
            s.id,
            s.children
                .iter()
                .map(shape_of)
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

/// Take a group out of whatever pane or box holds it.
fn unparent(widget: &gtk::Widget) {
    let Some(parent) = widget.parent() else {
        return;
    };
    if let Some(paned) = parent.downcast_ref::<gtk::Paned>() {
        if paned.start_child().as_ref() == Some(widget) {
            paned.set_start_child(gtk::Widget::NONE);
        } else {
            paned.set_end_child(gtk::Widget::NONE);
        }
    } else if let Some(b) = parent.downcast_ref::<gtk::Box>() {
        b.remove(widget);
    }
}
