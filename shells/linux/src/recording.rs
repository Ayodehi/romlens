//! Recordings in the shell: File > Open Recording, Import Snapshot and Export
//! Frame Region, and Save Mesen Recorder Script. The macOS twin is
//! `RecordingController`.
//!
//! A recording holds VRAM, CGRAM and OAM, which are the game's assets, so it is
//! read where it is and never copied into the project (docs/12, rule 4); the
//! project keeps its path and fingerprint. One made from a different ROM is
//! refused with the core's message, as a mismatched project package is, and
//! one the validator finds broken is refused with the validator's words.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use adw::prelude::*;
use gtk::{gio, glib};
use romlens_ffi::{RecordingSession, StateRegion};

use crate::files::alert;
use crate::model::Document;

fn filters(items: &[(&str, &[&str])]) -> gio::ListStore {
    let store = gio::ListStore::new::<gtk::FileFilter>();
    for (name, suffixes) in items {
        let f = gtk::FileFilter::new();
        f.set_name(Some(name));
        for s in *suffixes {
            f.add_suffix(s);
        }
        store.append(&f);
    }
    let all = gtk::FileFilter::new();
    all.set_name(Some("All files"));
    all.add_pattern("*");
    store.append(&all);
    store
}

/// Where Mesen's recorder script writes its streams, if Mesen is installed
/// the usual way: its settings folder's `LuaScriptData`.
pub fn mesen_script_data() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    let dir = config.join("Mesen2").join("LuaScriptData");
    dir.is_dir().then_some(dir)
}

/// Where packed recordings are kept: the shell's own Recordings folder.
pub fn packed_folder() -> PathBuf {
    crate::config::data_dir().join("Recordings")
}

// MARK: Open

/// File > Open Recording…
pub fn open(window: &adw::ApplicationWindow, doc: &Rc<Document>) {
    let dialog = gtk::FileDialog::builder()
        .title("Open Recording")
        .filters(&filters(&[(
            "Recordings and recorder streams",
            &["romrec", "rlstream"],
        )]))
        .build();
    if let Some(dir) = mesen_script_data() {
        dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
    }
    let (window, doc) = (window.clone(), Rc::clone(doc));
    dialog.open(
        Some(&window.clone()),
        gio::Cancellable::NONE,
        move |picked| {
            let Some(path) = picked.ok().and_then(|f| f.path()) else {
                return;
            };
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("rlstream"))
            {
                pack(&window, &doc, &path);
            } else {
                attach(&window, &doc, &path, false);
            }
        },
    );
}

/// Pack the recorder's stream into a recording in the shell's own folder, off
/// the main thread, then open it.
pub fn pack(window: &adw::ApplicationWindow, doc: &Rc<Document>, stream: &Path) {
    let folder = packed_folder();
    let name = stream.file_stem().map_or_else(
        || "recording".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let out = folder.join(format!("{name}-{stamp}.romrec"));
    let shown = stream
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    doc.edit_graphics(|g| {
        g.packing = Some(shown);
        None
    });
    let (window, doc_done, out_done) = (window.clone(), Rc::clone(doc), out.clone());
    doc.pack_recording(stream.to_path_buf(), out, move |result| {
        doc_done.edit_graphics(|g| {
            g.packing = None;
            None
        });
        match result {
            Ok(summary) => {
                if attach(&window, &doc_done, &out_done, false) && summary.truncated {
                    alert(
                        Some(window.upcast_ref()),
                        "The recording was cut short",
                        &format!(
                            "Mesen closed before the recorder finished; every whole frame, {} of them, was kept.",
                            summary.frames
                        ),
                    );
                }
            }
            Err(e) => alert(
                Some(window.upcast_ref()),
                "The recorder's stream could not be read",
                &e.to_string(),
            ),
        }
    });
}

/// Open, validate and attach, reporting any refusal. True if attached.
pub fn attach(
    window: &adw::ApplicationWindow,
    doc: &Rc<Document>,
    path: &Path,
    recover: bool,
) -> bool {
    let parent = Some(window.upcast_ref::<gtk::Window>());
    let session = match RecordingSession::open(path.to_string_lossy().into_owned(), recover) {
        Ok(s) => s,
        Err(e) => {
            let text = e.to_string();
            if !recover && text.contains("no footer") {
                offer_recovery(window, doc, path, &text);
            } else {
                alert(parent, "The recording could not be opened", &text);
            }
            return false;
        }
    };
    if let Ok(v) = session.validate(8)
        && (v.errors > 0 || v.warnings > 0)
    {
        if v.errors > 0 {
            alert(parent, "The recording is damaged", &v.lines.join("\n"));
            return false;
        }
        alert(
            parent,
            "The recording opened with warnings",
            &v.lines.join("\n"),
        );
    }
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    match doc.attach_recording(Arc::clone(&session), &name) {
        Ok(()) => true,
        Err(e) => {
            alert(parent, "The recording could not be opened", &e.to_string());
            false
        }
    }
}

fn offer_recovery(window: &adw::ApplicationWindow, doc: &Rc<Document>, path: &Path, reason: &str) {
    let dialog = adw::AlertDialog::new(
        Some("The recording was not finished"),
        Some(&format!(
            "{reason}\n\nThe frames that reached the disk can still be read."
        )),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("recover", "Open What Was Recorded");
    dialog.set_close_response("cancel");
    let (window, doc, path) = (window.clone(), Rc::clone(doc), path.to_path_buf());
    dialog.choose(
        Some(&window.clone()),
        gio::Cancellable::NONE,
        move |response| {
            if response == "recover" {
                attach(&window, &doc, &path, true);
            }
        },
    );
}

// MARK: Import Snapshot

/// File > Import Snapshot…: loose VRAM, CGRAM and OAM dumps (a PPU register
/// block too, if there is one) made into a one-frame recording, saved where
/// the person says and attached. Which dump is which comes from its size, and
/// for the two 512-byte kinds from its name.
pub fn import_snapshot(window: &adw::ApplicationWindow, doc: &Rc<Document>) {
    let dialog = gtk::FileDialog::builder().title("Import Snapshot").build();
    let (window, doc) = (window.clone(), Rc::clone(doc));
    dialog.open_multiple(
        Some(&window.clone()),
        gio::Cancellable::NONE,
        move |picked| {
            let files: Vec<PathBuf> = picked
                .ok()
                .map(|l| {
                    l.iter::<glib::Object>()
                        .flatten()
                        .filter_map(|o| o.downcast::<gio::File>().ok()?.path())
                        .collect()
                })
                .unwrap_or_default();
            if files.is_empty() {
                return;
            }
            let dumps = Dumps::classify(files.iter().filter_map(|p| {
                let name = p.file_name()?.to_string_lossy().to_lowercase();
                Some((name, std::fs::read(p).ok()?))
            }));
            if dumps.none() {
                alert(
                    Some(window.upcast_ref()),
                    "No dumps recognised",
                    "None of the files is the size of VRAM, CGRAM or OAM.",
                );
                return;
            }
            let save = gtk::FileDialog::builder()
                .title("Save Snapshot Recording")
                .initial_name("snapshot.romrec")
                .build();
            let (window, doc) = (window.clone(), Rc::clone(&doc));
            save.save(Some(&window.clone()), gio::Cancellable::NONE, move |out| {
                let Some(out) = out.ok().and_then(|f| f.path()) else {
                    return;
                };
                let written = romlens_ffi::write_snapshot_recording(
                    Arc::clone(&doc.rom),
                    dumps.vram.clone(),
                    dumps.cgram.clone(),
                    dumps.oam.clone(),
                    dumps.ppu.clone(),
                    out.to_string_lossy().into_owned(),
                );
                match written {
                    Ok(()) => {
                        attach(&window, &doc, &out, false);
                    }
                    Err(e) => alert(
                        Some(window.upcast_ref()),
                        "The snapshot could not be imported",
                        &e.to_string(),
                    ),
                }
            });
        },
    );
}

/// Loose memory dumps, told apart by size.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Dumps {
    pub vram: Option<Vec<u8>>,
    pub cgram: Option<Vec<u8>>,
    pub oam: Option<Vec<u8>>,
    pub ppu: Option<Vec<u8>>,
}

impl Dumps {
    /// `files` are lower-case names with their bytes.
    pub fn classify(files: impl IntoIterator<Item = (String, Vec<u8>)>) -> Self {
        let mut d = Dumps::default();
        for (name, data) in files {
            match data.len() {
                0x10000 => d.vram = Some(data),
                544 => d.oam = Some(data),
                256 => d.ppu = Some(data),
                512 if name.contains("oam") || name.contains("sprite") => d.oam = Some(data),
                512 => d.cgram = Some(data),
                _ => {}
            }
        }
        d
    }

    pub fn none(&self) -> bool {
        self.vram.is_none() && self.cgram.is_none() && self.oam.is_none()
    }
}

// MARK: Export Frame Region

pub const EXPORTABLE: [(StateRegion, &str); 8] = [
    (StateRegion::Vram, "VRAM"),
    (StateRegion::Cgram, "CGRAM"),
    (StateRegion::Oam, "OAM"),
    (StateRegion::Wram, "WRAM"),
    (StateRegion::Ppu, "PPU registers"),
    (StateRegion::Cpu, "CPU registers"),
    (StateRegion::Io, "I/O registers"),
    (StateRegion::Timing, "Timing"),
];

/// The regions a recording has, with their names.
pub fn exportable(doc: &Document) -> Vec<(StateRegion, &'static str)> {
    let g = doc.graphics();
    let present = g
        .recording_info()
        .map(|i| i.regions.clone())
        .unwrap_or_default();
    EXPORTABLE
        .into_iter()
        .filter(|(r, _)| present.contains(r))
        .collect()
}

/// File > Export Frame Region…: one region of the current frame, as raw
/// bytes. It is the game's data, for the person's own tools; the dialog says
/// so.
pub fn export_frame_region(window: &adw::ApplicationWindow, doc: &Rc<Document>) {
    if !doc.graphics().has_recording() {
        return;
    }
    let present = exportable(doc);
    let frame = doc.graphics().frame();
    let names: Vec<&str> = present.iter().map(|(_, n)| *n).collect();
    let choice = gtk::DropDown::from_strings(&names);
    let dialog = adw::AlertDialog::new(
        Some("Export Frame Region"),
        Some(&format!("Frame {frame}: game data, not for sharing")),
    );
    dialog.set_extra_child(Some(&choice));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("export", "Export…");
    dialog.set_response_appearance("export", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("cancel");
    let (window, doc) = (window.clone(), Rc::clone(doc));
    dialog.choose(
        Some(&window.clone()),
        gio::Cancellable::NONE,
        move |response| {
            if response != "export" {
                return;
            }
            let (region, name) = present[choice.selected() as usize];
            let save = gtk::FileDialog::builder()
                .title("Export Frame Region")
                .initial_name(format!("frame{frame}.bin"))
                .build();
            let (window, doc) = (window.clone(), Rc::clone(&doc));
            save.save(Some(&window.clone()), gio::Cancellable::NONE, move |out| {
                let Some(out) = out.ok().and_then(|f| f.path()) else {
                    return;
                };
                let Some(bytes) = doc.graphics().region_bytes(region) else {
                    alert(
                        Some(window.upcast_ref()),
                        "Nothing to export",
                        &format!("The recording has no {name} at this frame."),
                    );
                    return;
                };
                if let Err(e) = write_atomic(&out, &bytes) {
                    alert(
                        Some(window.upcast_ref()),
                        "The region could not be exported",
                        &e.to_string(),
                    );
                }
            });
        },
    );
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

// MARK: Save Mesen Recorder Script

pub const SCRIPT_STEPS: &str = "\
1. In Mesen, open Debug > Script Window and load the script.
2. In the script window's settings, allow access to I/O and OS functions, then run it.
3. Play, then stop the script, and open what it wrote with File > Open Recording…: Romlens packs the stream itself.
Or choose File > Start Live Session first: while it runs, the views follow the game, sound included.
4. A Mesen with an execution log (the MesenCE fork) also gets a .mxlog beside the stream: import it with File > Import > Execution Trace… for the calls, jumps, reads and DMA the game made.
5. To watch the game live, also allow network access in the script settings, and choose File > Start Live Session here. The script connects within two seconds.";

pub fn save_recorder_script(window: &adw::ApplicationWindow) {
    let save = gtk::FileDialog::builder()
        .title("Save Mesen Recorder Script")
        .initial_name("mesen_recorder.lua")
        .build();
    let window = window.clone();
    save.save(Some(&window.clone()), gio::Cancellable::NONE, move |out| {
        let Some(out) = out.ok().and_then(|f| f.path()) else {
            return;
        };
        match std::fs::write(&out, romlens_ffi::recorder_script()) {
            Ok(()) => alert(
                Some(window.upcast_ref()),
                "Recorder script saved",
                SCRIPT_STEPS,
            ),
            Err(e) => alert(
                Some(window.upcast_ref()),
                "The script could not be saved",
                &e.to_string(),
            ),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dumps_are_told_apart_by_size_and_the_two_512_byte_kinds_by_name() {
        let d = Dumps::classify([
            ("vram.bin".to_owned(), vec![0; 0x10000]),
            ("cgram.bin".to_owned(), vec![1; 512]),
            ("oam_dump.bin".to_owned(), vec![2; 512]),
            ("ppu.bin".to_owned(), vec![3; 256]),
            ("junk".to_owned(), vec![0; 7]),
        ]);
        assert_eq!(d.vram.as_ref().map(Vec::len), Some(0x10000));
        assert_eq!(d.cgram, Some(vec![1; 512]));
        assert_eq!(d.oam, Some(vec![2; 512]));
        assert_eq!(d.ppu, Some(vec![3; 256]));
        assert!(!d.none());
        // The 544-byte OAM is recognised by size alone.
        let d = Dumps::classify([("x".to_owned(), vec![0; 544])]);
        assert_eq!(d.oam.as_ref().map(Vec::len), Some(544));
        assert!(Dumps::classify([("x".to_owned(), vec![0; 3])]).none());
        // Sprite is a name for OAM too.
        let d = Dumps::classify([("sprites.bin".to_owned(), vec![0; 512])]);
        assert!(d.oam.is_some() && d.cgram.is_none());
    }

    #[test]
    fn the_script_instructions_name_every_step() {
        assert_eq!(
            SCRIPT_STEPS
                .lines()
                .filter(|l| l.starts_with(|c: char| c.is_ascii_digit()))
                .count(),
            5
        );
        assert!(!SCRIPT_STEPS.contains('\u{2013}') && !SCRIPT_STEPS.contains('\u{2014}'));
    }

    #[test]
    fn the_packed_folder_is_under_the_data_dir() {
        assert!(packed_folder().ends_with("romlens/Recordings"));
    }
}
