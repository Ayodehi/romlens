//! One window per document, and the welcome window when none is open.
//! Navigator on the left, the editor in the middle, the inspector on the
//! right, as on macOS; the sidebars are `AdwOverlaySplitView`s so they
//! collapse into overlays on a narrow window. The macOS twin is
//! `RomWindowController` and `DocumentView`.
//!
//! The editor is tab groups in a grid (docs/29, `tabgrid.rs`), and the
//! header bar holds commands only: the panels, Back and Forward, and the
//! menu. Views are chosen from the View menu and their keys. What the
//! analyzer found is not a control, so it is in the header band with the
//! overview strip.

use std::rc::Rc;

use adw::prelude::*;

use crate::model::{Change, Document};
use crate::tabgrid::{self, Grid};
use crate::{actions, headerband, inspector, jumpbar, menu, results, sheets, sidebar};

pub fn open_document(app: &adw::Application, doc: Rc<Document>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(doc.display_name())
        .default_width(1440)
        .default_height(860)
        .width_request(900)
        .height_request(480)
        .build();
    actions::install(&window, &doc);
    // A sheet asked for by a key, a menu item or a button.
    doc.subscribe({
        let (window, doc) = (window.clone(), Rc::clone(&doc));
        move |c| {
            if c == Change::Sheet
                && let Some(sheet) = doc.active_sheet()
            {
                sheets::present(&window, &doc, sheet);
            }
        }
    });
    // The switch View › Show Explanations remembered from the last window.
    let saved = crate::config::Settings::load();
    doc.set_explanations(!saved.hide_explanations);
    if let Some(style) = crate::model::decompile::number_style_named(&saved.c_numbers) {
        doc.set_c_numbers(style);
    }

    let header = build_header(&doc);
    titles(&window, &doc);
    let grid = Grid::build(&doc);
    let editor = grid.root.clone();

    let center = gtk::Box::new(gtk::Orientation::Vertical, 0);
    center.append(&headerband::build(&doc));
    center.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    editor.set_vexpand(true);
    center.append(&editor);
    center.append(&results::build(&doc));

    let inner = adw::OverlaySplitView::builder()
        .sidebar_position(gtk::PackType::End)
        .sidebar_width_fraction(0.22)
        .min_sidebar_width(280.0)
        .max_sidebar_width(420.0)
        .content(&center)
        .sidebar(&inspector::build(&doc))
        .show_sidebar(true)
        .build();
    let outer = adw::OverlaySplitView::builder()
        .sidebar_width_fraction(0.18)
        .min_sidebar_width(200.0)
        .max_sidebar_width(360.0)
        .content(&inner)
        .sidebar(&sidebar::build(&doc))
        .show_sidebar(true)
        .build();
    sync_panes(&doc, &outer, &inner);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.set_vexpand(true);
    body.append(&outer);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&body));
    window.set_content(Some(&view));

    // A narrow window shows only the focused group.
    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        tabgrid::COMPACT_WIDTH,
        adw::LengthUnit::Px,
    ));
    let g = Rc::clone(&grid);
    breakpoint.connect_apply(move |_| g.set_compact(true));
    let g = Rc::clone(&grid);
    breakpoint.connect_unapply(move |_| g.set_compact(false));
    window.add_breakpoint(breakpoint);

    // Analysis runs off the main thread; the rows refetch when it lands,
    // which is when the region lane first has something to show.
    doc.start_analysis();

    window.present();
    window
}

/// The header bar: commands only.
fn build_header(doc: &Rc<Document>) -> adw::HeaderBar {
    let header = adw::HeaderBar::new();

    let navigator = toggle_button(
        "sidebar-show-symbolic",
        "win.toggle-navigator",
        "Show or hide the sidebar (Ctrl+0)",
    );
    header.pack_start(&navigator);
    header.pack_start(&back_forward());
    header.set_title_widget(Some(&jumpbar::build(doc)));

    let primary = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu::primary_menu())
        .primary(true)
        .tooltip_text("Main menu")
        .build();
    header.pack_end(&primary);
    header.pack_end(&toggle_button(
        "sidebar-show-right-symbolic",
        "win.toggle-inspector",
        "Show or hide the inspector (Alt+0)",
    ));
    header.pack_end(
        &gtk::Button::builder()
            .icon_name("system-search-symbolic")
            .action_name("win.open-quickly")
            .tooltip_text("Open Quickly (Ctrl+P)")
            .build(),
    );
    header
}

/// The window's title follows the project: its name, with a dot while it has
/// changes not yet saved. The header's centre is the jump bar, so the name
/// shows where the desktop lists windows.
fn titles(window: &adw::ApplicationWindow, doc: &Rc<Document>) {
    let update = {
        let (window, doc) = (window.downgrade(), Rc::downgrade(doc));
        move || {
            let (Some(window), Some(doc)) = (window.upgrade(), doc.upgrade()) else {
                return;
            };
            let dirty = if doc.session.is_dirty() { "• " } else { "" };
            window.set_title(Some(&format!("{dirty}{}", doc.display_name())));
        }
    };
    update();
    doc.subscribe(move |c| {
        if c == Change::Status {
            update();
        }
    });
}

fn toggle_button(icon: &str, action: &str, tip: &str) -> gtk::ToggleButton {
    gtk::ToggleButton::builder()
        .icon_name(icon)
        .action_name(action)
        .tooltip_text(tip)
        .build()
}

/// Keep the sidebars in step with the document: it is the truth, and a
/// sidebar closed by its own gesture (an overlay dismissed) reports back.
fn sync_panes(
    doc: &Rc<Document>,
    navigator: &adw::OverlaySplitView,
    inspector: &adw::OverlaySplitView,
) {
    let apply = {
        let (doc, navigator, inspector) = (Rc::clone(doc), navigator.clone(), inspector.clone());
        move || {
            let p = doc.panes();
            navigator.set_show_sidebar(p.navigator);
            inspector.set_show_sidebar(p.inspector);
        }
    };
    apply();
    doc.subscribe(move |c| {
        if c == Change::Layout {
            apply();
        }
    });
    navigator.connect_show_sidebar_notify({
        let doc = Rc::clone(doc);
        move |s| {
            if s.shows_sidebar() != doc.panes().navigator {
                doc.set_pane(|p| &mut p.navigator, s.shows_sidebar());
            }
        }
    });
    inspector.connect_show_sidebar_notify({
        let doc = Rc::clone(doc);
        move |s| {
            if s.shows_sidebar() != doc.panes().inspector {
                doc.set_pane(|p| &mut p.inspector, s.shows_sidebar());
            }
        }
    });
}

fn back_forward() -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    group.add_css_class("linked");
    for (icon, action, tip) in [
        ("go-previous-symbolic", "win.go-back", "Back (Ctrl+[)"),
        ("go-next-symbolic", "win.go-forward", "Forward (Ctrl+])"),
    ] {
        group.append(
            &gtk::Button::builder()
                .icon_name(icon)
                .action_name(action)
                .tooltip_text(tip)
                .build(),
        );
    }
    group
}

/// The name of the welcome window, so opening a document can retire it.
pub const WELCOME: &str = "romlens-welcome";

pub fn open_welcome(app: &adw::Application) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(560)
        .default_height(520)
        .name(WELCOME)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .valign(gtk::Align::Center)
        .build();
    let page = adw::StatusPage::builder()
        .icon_name("document-open-symbolic")
        .title("Romlens")
        .description("Open a SNES ROM or a Romlens project.")
        .build();
    let buttons = gtk::Box::builder()
        .spacing(8)
        .halign(gtk::Align::Center)
        .build();
    for (label, action, primary) in [
        ("Open ROM…", "app.open", true),
        ("Open Project…", "app.open-project", false),
    ] {
        let b = gtk::Button::builder()
            .label(label)
            .action_name(action)
            .build();
        b.add_css_class("pill");
        if primary {
            b.add_css_class("suggested-action");
        }
        buttons.append(&b);
    }
    page.set_child(Some(&buttons));
    content.append(&page);

    let recent = crate::files::recent(6);
    if !recent.is_empty() {
        let group = adw::PreferencesGroup::builder()
            .title("Recent")
            .margin_start(24)
            .margin_end(24)
            .margin_bottom(24)
            .build();
        for (name, path) in recent {
            let row = adw::ActionRow::builder()
                .title(&name)
                .subtitle(
                    path.parent()
                        .map_or_else(String::new, |p| p.display().to_string()),
                )
                .activatable(true)
                .build();
            let app = app.clone();
            row.connect_activated(move |_| crate::files::open_path(&app, &path));
            group.add(&row);
        }
        content.append(&group);
    }
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&content));
    window.set_content(Some(&view));
    crate::files::accept_drops(app, window.upcast_ref());
    window.present();
}
