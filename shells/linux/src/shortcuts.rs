//! The Keyboard Shortcuts window, built from the one table in `actions.rs`.

use adw::prelude::*;

use crate::actions::{SHORTCUT_GROUPS, accels_for};

pub fn show(app: &adw::Application) {
    let page = adw::PreferencesPage::new();
    for (title, rows) in SHORTCUT_GROUPS {
        let group = adw::PreferencesGroup::builder().title(*title).build();
        for (name, action) in *rows {
            let row = adw::ActionRow::builder().title(*name).build();
            // A command with two shortcuts shows both, side by side.
            let keys = accels_for(action);
            let labels = gtk::Box::builder()
                .spacing(12)
                .valign(gtk::Align::Center)
                .build();
            for accel in keys {
                labels.append(&gtk::ShortcutLabel::new(accel));
            }
            row.add_suffix(&labels);
            group.add(&row);
        }
        page.add(&group);
    }
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title("Keyboard Shortcuts")
        .content_width(520)
        .content_height(640)
        .child(&view)
        .build();
    dialog.present(
        app.active_window()
            .or_else(|| app.windows().into_iter().next())
            .as_ref(),
    );
}
