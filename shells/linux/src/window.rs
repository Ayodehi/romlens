//! One window per document, and the welcome window when none is open.
//! Navigator on the left, the editor in the middle, the inspector on the
//! right, as on macOS; the sidebars are `AdwOverlaySplitView`s so they
//! collapse into overlays on a narrow window. The macOS twin is
//! `RomWindowController` and `DocumentView`.
//!
//! The toolbar holds only controls: the editor tabs in the centre, and
//! navigation and the address style trailing. What the analyzer found is not
//! a control, so it is in the header band with the overview strip.

use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use crate::hex::AddressStyle;
use crate::model::{Change, Document, Tab};
use crate::{
    actions, asmview, atlasview, compareview, cview, graphview, headerband, hexview, inspector,
    lockstep, menu, navigatorview, results, sheets, sourceview,
};

/// The macOS toolbar switches the editor tabs to a menu below this width,
/// because a control that does not fit goes to an overflow menu as blanks.
const COMPACT_WIDTH: f64 = 1180.0;

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

    let (header, tab_group, tab_menu) = build_header(&doc);
    let editor = editor_stack(&doc);

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
        .sidebar(&navigatorview::build(&doc))
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

    // Below the compact width the tab buttons give way to a menu.
    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        COMPACT_WIDTH,
        adw::LengthUnit::Px,
    ));
    breakpoint.add_setter(&tab_group, "visible", Some(&false.to_value()));
    breakpoint.add_setter(&tab_menu, "visible", Some(&true.to_value()));
    window.add_breakpoint(breakpoint);

    // Analysis runs off the main thread; the rows refetch when it lands,
    // which is when the region lane first has something to show.
    doc.start_analysis();

    window.present();
    window
}

/// The header bar, and the two forms of the tab selector (the breakpoint
/// swaps them).
fn build_header(doc: &Rc<Document>) -> (adw::HeaderBar, gtk::Box, gtk::MenuButton) {
    let header = adw::HeaderBar::new();

    let navigator = toggle_button(
        "sidebar-show-symbolic",
        "win.toggle-navigator",
        "Show or hide the navigator (Ctrl+0)",
    );
    let focus = toggle_button(
        "view-fullscreen-symbolic",
        "win.focus-on-code",
        "Focus on Code: hide the panels (Ctrl+Alt+F)",
    );
    header.pack_start(&navigator);
    header.pack_start(&focus);
    let title = adw::WindowTitle::new(&doc.display_name(), &doc.subtitle());
    header.pack_start(&title);
    titles(&title, doc);

    let tab_group = tab_buttons(doc);
    let tab_menu = tab_menu_button(doc);
    tab_menu.set_visible(false);
    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tabs.append(&tab_group);
    tabs.append(&tab_menu);
    header.set_title_widget(Some(&tabs));

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
    header.pack_end(&address_style_button(doc));
    header.pack_end(&back_forward());
    (header, tab_group, tab_menu)
}

/// The window's title follows the project: its name, with a dot while it has
/// changes not yet saved.
fn titles(title: &adw::WindowTitle, doc: &Rc<Document>) {
    let update = {
        let (title, doc) = (title.clone(), Rc::clone(doc));
        move || {
            let dirty = if doc.session.is_dirty() { "• " } else { "" };
            title.set_title(&format!("{dirty}{}", doc.display_name()));
            if let Some(w) = title.root().and_downcast::<gtk::Window>() {
                w.set_title(Some(&format!("{dirty}{}", doc.display_name())));
            }
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

/// The centre of the window: one page per editor tab, with the listing
/// replaced by a note until the first analysis has landed.
fn editor_stack(doc: &Rc<Document>) -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.add_named(&hexview::build(doc).widget, Some("hex"));
    stack.add_named(&asmview::build(doc).widget, Some("disassembly"));
    stack.add_named(&lockstep::build(doc), Some("both"));
    stack.add_named(&cview::build(doc), Some("c"));
    stack.add_named(&graphview::build(doc), Some("graph"));
    stack.add_named(&atlasview::build(doc), Some("atlas"));
    stack.add_named(&sourceview::build(doc), Some("source"));
    stack.add_named(&compareview::build(doc), Some("compare"));
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
        let (stack, doc) = (stack.clone(), Rc::clone(doc));
        move || {
            let tab = doc.tab();
            let page = if tab.needs_disassembly() && !doc.has_disassembly() {
                "analyzing"
            } else {
                tab.id()
            };
            stack.set_visible_child_name(page);
        }
    };
    show();
    doc.subscribe(move |c| {
        if matches!(c, Change::Layout | Change::Rows | Change::Status) {
            show();
        }
    });
    stack
}

/// The editor tabs as one linked group, the GNOME counterpart of the macOS
/// segmented control. Source and Compare appear only when there is something
/// to show.
fn tab_buttons(doc: &Rc<Document>) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    group.add_css_class("linked");
    let mut buttons = Vec::new();
    for tab in Tab::BUILT {
        let button = gtk::ToggleButton::builder().label(tab.title()).build();
        button.set_action_name(Some("win.show-tab"));
        button.set_action_target_value(Some(&tab.id().to_variant()));
        button.set_tooltip_text(Some(&tab_tooltip()));
        button.set_visible(doc.tab_available(tab));
        group.append(&button);
        buttons.push((tab, button));
    }
    doc.subscribe({
        let doc = Rc::clone(doc);
        move |c| {
            if matches!(c, Change::Source | Change::Compare | Change::Layout) {
                for (tab, button) in &buttons {
                    button.set_visible(doc.tab_available(*tab));
                }
            }
        }
    });
    group
}

fn tab_tooltip() -> String {
    "Hex (Alt+1), Disassembly (Alt+2), Both (Alt+3), C (Alt+8), Graph (Alt+9) or Atlas (Alt+Shift+A)"
        .to_owned()
}

/// The same choice as a menu, for a window too narrow for the buttons.
fn tab_menu_button(doc: &Rc<Document>) -> gtk::MenuButton {
    let menu = gio::Menu::new();
    let fill = {
        let (menu, doc) = (menu.clone(), Rc::clone(doc));
        move || {
            menu.remove_all();
            for tab in Tab::BUILT.into_iter().filter(|t| doc.tab_available(*t)) {
                menu.append(
                    Some(tab.title()),
                    Some(&format!("win.show-tab::{}", tab.id())),
                );
            }
        }
    };
    fill();
    let button = gtk::MenuButton::builder()
        .menu_model(&menu)
        .label(doc.tab().title())
        .tooltip_text(tab_tooltip())
        .build();
    doc.subscribe({
        let (doc, button) = (Rc::clone(doc), button.clone());
        move |c| {
            if matches!(c, Change::Source | Change::Compare) {
                fill();
            }
            if c == Change::Layout {
                button.set_label(doc.tab().title());
            }
        }
    });
    button
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

/// The address columns menu. A `MenuButton` whose label follows the current
/// value, as the macOS button does.
fn address_style_button(doc: &Rc<Document>) -> gtk::MenuButton {
    let menu = gio::Menu::new();
    for style in AddressStyle::ALL {
        menu.append(
            Some(style.label()),
            Some(&format!("win.address-style::{}", actions::style_id(style))),
        );
    }
    let button = gtk::MenuButton::builder()
        .menu_model(&menu)
        .label(doc.address_style().short_label())
        .tooltip_text("Which address columns the editor shows")
        .build();
    doc.subscribe({
        let (doc, button) = (Rc::clone(doc), button.clone());
        move |c| {
            if c == Change::AddressStyle {
                button.set_label(doc.address_style().short_label());
            }
        }
    });
    button
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
