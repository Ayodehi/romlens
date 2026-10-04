//! Settings (Ctrl+,). The macOS window holds the tutor's providers, defaults,
//! images and privacy; those arrive with the tutor (L5). What is here now is
//! what the app already remembers.

use crate::config::Settings;
use adw::prelude::*;

pub fn show(app: &adw::Application) {
    let dialog = adw::PreferencesDialog::builder().title("Settings").build();
    let page = adw::PreferencesPage::builder()
        .title("General")
        .icon_name("preferences-system-symbolic")
        .build();
    let group = adw::PreferencesGroup::builder().title("Listing").build();
    let explanations = adw::SwitchRow::builder()
        .title("Show explanations")
        .subtitle("Explained comments on hardware writes, and a note above each idiom, in the listing and the C")
        .active(!Settings::load().hide_explanations)
        .build();
    explanations.connect_active_notify({
        let app = app.clone();
        move |row| {
            let show = row.is_active();
            // The open windows follow, each through its own action, which also
            // remembers the choice.
            let mut any = false;
            for w in app.windows() {
                if let Some(a) = w
                    .downcast_ref::<adw::ApplicationWindow>()
                    .and_then(|w| w.lookup_action("toggle-explanations"))
                    && a.state().and_then(|s| s.get::<bool>()) != Some(show)
                {
                    a.activate(None);
                    any = true;
                }
            }
            if !any {
                let mut saved = Settings::load();
                saved.hide_explanations = !show;
                saved.save();
            }
        }
    });
    group.add(&explanations);
    page.add(&group);
    dialog.add(&page);
    dialog.present(
        app.active_window()
            .or_else(|| app.windows().into_iter().next())
            .as_ref(),
    );
}
