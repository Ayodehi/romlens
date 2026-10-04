//! Romlens for Linux: GTK4 and libadwaita over the Rust core.

mod actions;
mod aramview;
mod asm;
mod asmview;
mod atlasview;
mod audio_out;
mod audioview;
mod canvas;
mod compareview;
mod config;
mod cview;
mod echoview;
mod editor_keys;
mod files;
mod frameview;
mod gfxdraw;
mod glib_runtime;
mod glossary;
mod graphicsview;
mod graphview;
mod hex;
mod hexview;
mod inspector;
mod jumpbar;
mod layersview;
mod lessonsheets;
mod lessonview;
mod lists;
mod locator;
mod lockstep;
mod menu;
mod messageview;
mod model;
mod oamview;
mod package;
mod palette;
mod paletteview;
mod pixels;
mod portsview;
mod progressview;
mod quizview;
mod recording;
mod results;
mod samplesview;
mod scopeview;
mod secrets;
mod settings;
mod sheets;
mod shortcuts;
mod sidebar;
mod snapshot;
mod sourceview;
mod statusbar;
mod stripview;
mod style;
mod tabgrid;
mod tilemapview;
mod tilesview;
mod timelineview;
mod transferview;
mod tutorsheets;
mod tutorview;
mod voicesview;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "io.github.ayodehi.Romlens";

fn write_fixture(kind: &str, out: &std::path::Path) -> std::io::Result<()> {
    let into = |files: Vec<(String, Vec<u8>)>| -> std::io::Result<()> {
        std::fs::create_dir_all(out)?;
        files
            .into_iter()
            .try_for_each(|(name, bytes)| std::fs::write(out.join(name), bytes))
    };
    match kind {
        "routines" => std::fs::write(out, romlens_ffi::make_routines_test_rom()),
        "explain" => std::fs::write(out, romlens_ffi::make_explain_test_rom()),
        "graphics" => std::fs::write(out, romlens_ffi::make_graphics_test_rom()),
        "sound" => std::fs::write(out, romlens_ffi::make_sound_test_rom()),
        "soundrec" => std::fs::write(out, romlens_ffi::make_sound_test_recording(40)),
        "recording" => std::fs::write(out, romlens_ffi::make_test_recording(40)),
        "compare" => {
            let roms = romlens_ffi::make_compare_test_roms();
            into(vec![
                ("old.sfc".into(), roms[0].clone()),
                ("new.sfc".into(), roms[1].clone()),
            ])
        }
        "ca65" => into(
            romlens_ffi::make_ca65_test_program()
                .into_iter()
                .map(|f| (f.name, f.bytes))
                .collect(),
        ),
        other => Err(std::io::Error::other(format!("unknown fixture {other}"))),
    }
}

fn main() -> glib::ExitCode {
    // Development aid: `romlens --write-fixture <kind> <out>` writes one of the
    // core's test programs: `routines` (a ROM with loops and calls to
    // decompile), `explain` (a reset that sets the screen up), `graphics` (tiles, a
    // palette, OAM and a tilemap) or `recording` (40 frames to go with it), `compare` (`<out>` is a folder getting old.sfc and new.sfc)
    // or `ca65` (a folder getting a ROM, its .dbg and the sources it names).
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, kind, out] = args.as_slice()
        && flag == "--write-fixture"
    {
        return match write_fixture(kind, std::path::Path::new(out)) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{out}: {e}");
                glib::ExitCode::FAILURE
            }
        };
    }

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_startup(|app| {
        let open = gio::ActionEntry::builder("open")
            .activate(|app: &adw::Application, _, _| files::choose_rom(app))
            .build();
        let open_project = gio::ActionEntry::builder("open-project")
            .activate(|app: &adw::Application, _, _| files::choose_project(app))
            .build();
        let quit = gio::ActionEntry::builder("quit")
            .activate(|app: &adw::Application, _, _| files::quit(app))
            .build();
        let about = gio::ActionEntry::builder("about")
            .activate(|app: &adw::Application, _, _| show_about(app))
            .build();
        let settings = gio::ActionEntry::builder("settings")
            .activate(|app: &adw::Application, _, _| settings::show(app))
            .build();
        let shortcuts = gio::ActionEntry::builder("shortcuts")
            .activate(|app: &adw::Application, _, _| shortcuts::show(app))
            .build();
        app.add_action_entries([open, open_project, quit, about, settings, shortcuts]);
        actions::set_accels(app);
        style::install();
    });
    app.connect_activate(|app| {
        if app.windows().is_empty() {
            window::open_welcome(app);
        }
    });
    app.connect_open(|app, files, _| {
        for file in files {
            if let Some(path) = file.path() {
                files::open_path(app, &path);
            }
        }
    });
    app.run()
}

/// The About box carries the core API version the shell was built against, so
/// a bug report has it (docs/15 row 0.12, docs/08 rule 6). It comes from the
/// core at runtime, never from a constant here.
fn show_about(app: &adw::Application) {
    let dialog = adw::AboutDialog::builder()
        .application_name("Romlens")
        // The installed icon (packages, Flatpak); a plain run has none.
        .application_icon(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .comments(about_credits(&romlens_ffi::api_version()))
        .website("https://github.com/Ayodehi/romlens")
        .license_type(gtk::License::Custom)
        .license("0BSD. See LICENSE and THIRD-PARTY-NOTICES.md.")
        .build();
    dialog.present(app.active_window().as_ref());
}

fn about_credits(core_version: &str) -> String {
    format!("Core API {core_version}\nA study tool for SNES ROM images. Ships no ROM data.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_about_credits_carry_the_core_api_version() {
        let text = about_credits(&romlens_ffi::api_version());
        assert!(text.starts_with("Core API "));
        assert!(text.contains(&romlens_ffi::api_version()));
        assert!(text.contains("Ships no ROM data"));
    }
}
