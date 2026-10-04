//! A `.romlens` package on disk: a directory holding the core's JSON files and
//! a machine-local `local.json` the shell keeps beside them. The macOS shell
//! writes the same layout through `NSFileWrapper`.
//!
//! The core serialises the project into named files; this writes them, each
//! through a temporary file and a rename so a crash never leaves half a file,
//! and never touches a file it does not own, so anything a newer version (or a
//! person) put in the package survives a save.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::workspace::WorkspaceRecord;

/// Machine-local facts: not part of the project, rewritten on every save.
pub const LOCAL_FILE: &str = "local.json";

/// Files the core writes only while they have something in them. A save
/// removes one the core no longer writes, or the last variable, note or C
/// version removed would come back when the project reopens.
pub const OPTIONAL_CORE_FILES: [&str; 3] = ["variables.json", "c_notes.json", "c_versions.json"];

/// What the shell remembers about this machine: where the ROM was, and the
/// window as it was left (docs/29), in the JSON the macOS shell writes. The
/// macOS `bookmark` means nothing here, but is kept as it was, so a project
/// carried between the two still finds its ROM on the Mac.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bookmark: Option<String>,
    #[serde(rename = "lastPath", skip_serializing_if = "Option::is_none")]
    pub last_path: Option<String>,
    /// The tabs, the focus and the panels. One this version cannot read is
    /// left out, rather than costing the rest of the record.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_workspace"
    )]
    pub workspace: Option<WorkspaceRecord>,
}

fn lenient_workspace<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<WorkspaceRecord>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).ok())
}

fn invalid(name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("bad package path {name:?}"),
    )
}

/// `dir/name`, where `name` is `/`-separated and may not climb out of `dir`.
fn path_for(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let mut path = dir.to_path_buf();
    for part in name.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains('\\') {
            return Err(invalid(name));
        }
        path.push(part);
    }
    Ok(path)
}

/// Write `files` into `dir` (created if needed).
pub fn write(dir: &Path, files: &HashMap<String, Vec<u8>>) -> io::Result<()> {
    // Check every name first, so a bad one writes nothing.
    let paths: Vec<_> = files
        .keys()
        .map(|name| path_for(dir, name))
        .collect::<io::Result<_>>()?;
    std::fs::create_dir_all(dir)?;
    for (path, bytes) in paths.iter().zip(files.values()) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let leaf = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = path.with_file_name(format!("{leaf}.tmp"));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)?;
    }
    Ok(())
}

/// Remove the optional core files the core did not write this time.
pub fn remove_stale(dir: &Path, files: &HashMap<String, Vec<u8>>) -> io::Result<()> {
    for name in OPTIONAL_CORE_FILES {
        if !files.contains_key(name) {
            match std::fs::remove_file(dir.join(name)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

pub fn write_local(dir: &Path, record: &LocalRecord) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
    let mut files = HashMap::new();
    files.insert(LOCAL_FILE.to_owned(), bytes);
    write(dir, &files)
}

/// Split a package's files into the core's and the local record.
pub fn split_local(mut files: HashMap<String, Vec<u8>>) -> (HashMap<String, Vec<u8>>, LocalRecord) {
    let local = files
        .remove(LOCAL_FILE)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    (files, local)
}

/// Whether a path is a project package rather than a ROM: a directory, or a
/// name ending `.romlens` (which may not exist yet).
pub fn is_package(path: &Path) -> bool {
    path.is_dir()
        || path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("romlens"))
}

/// `path` with the `.romlens` extension, added if it is not there.
pub fn with_extension(path: &Path) -> PathBuf {
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("romlens"))
    {
        path.to_path_buf()
    } else {
        let mut name = path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        name.push(".romlens");
        path.with_file_name(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("romlens-pkg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn files(pairs: &[(&str, &str)]) -> HashMap<String, Vec<u8>> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn writes_nested_files_and_leaves_no_temporaries() {
        let dir = scratch("nested");
        write(
            &dir,
            &files(&[("project.json", "{}"), ("traces/coverage.cdl", "cdl")]),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(dir.join("traces/coverage.cdl")).unwrap(),
            b"cdl"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_bad_name_writes_nothing() {
        let dir = scratch("bad");
        for bad in ["../evil", "a/../b", "", "/abs", "a\\b", "a//b"] {
            let err = write(&dir, &files(&[("project.json", "{}"), (bad, "x")])).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        }
        assert!(!dir.exists());
    }

    #[test]
    fn files_we_do_not_own_survive_a_save() {
        let dir = scratch("unknown");
        write(&dir, &files(&[("project.json", "old")])).unwrap();
        std::fs::write(dir.join("notes-from-the-future.json"), "keep me").unwrap();
        write(&dir, &files(&[("project.json", "new")])).unwrap();
        assert_eq!(std::fs::read(dir.join("project.json")).unwrap(), b"new");
        assert_eq!(
            std::fs::read(dir.join("notes-from-the-future.json")).unwrap(),
            b"keep me"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn optional_files_the_core_stopped_writing_are_removed() {
        let dir = scratch("stale");
        write(
            &dir,
            &files(&[
                ("project.json", "{}"),
                ("variables.json", "[]"),
                ("c_notes.json", "[]"),
            ]),
        )
        .unwrap();
        // The next save has variables but no notes.
        let next = files(&[("project.json", "{}"), ("variables.json", "[]")]);
        remove_stale(&dir, &next).unwrap();
        assert!(dir.join("variables.json").exists());
        assert!(!dir.join("c_notes.json").exists());
        // Removing what is already gone is fine.
        remove_stale(&dir, &next).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_local_record_round_trips_and_ignores_the_macos_bookmark() {
        let dir = scratch("local");
        let rec = LocalRecord {
            bookmark: None,
            workspace: None,
            last_path: Some("/roms/Game.sfc".into()),
        };
        write_local(&dir, &rec).unwrap();
        let text = std::fs::read_to_string(dir.join(LOCAL_FILE)).unwrap();
        assert!(text.contains("lastPath"));
        let mut all = files(&[("project.json", "{}")]);
        all.insert(
            LOCAL_FILE.into(),
            br#"{"bookmark":"AAAA","lastPath":"/x/y.sfc"}"#.to_vec(),
        );
        let (core, local) = split_local(all);
        assert!(!core.contains_key(LOCAL_FILE));
        assert_eq!(local.last_path.as_deref(), Some("/x/y.sfc"));
        // A missing or garbled record is just empty.
        let (_, none) = split_local(files(&[("project.json", "{}")]));
        assert_eq!(none, LocalRecord::default());
        let (_, bad) = split_local(files(&[(LOCAL_FILE, "{nope")]));
        assert_eq!(bad, LocalRecord::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn packages_are_recognised_by_name_or_by_being_a_directory() {
        assert!(is_package(Path::new("Metroid.romlens")));
        assert!(is_package(Path::new("/x/Metroid.ROMLENS")));
        assert!(!is_package(Path::new("/x/Metroid.sfc")));
        assert_eq!(
            with_extension(Path::new("/x/Metroid")),
            Path::new("/x/Metroid.romlens")
        );
        assert_eq!(
            with_extension(Path::new("/x/Metroid.romlens")),
            Path::new("/x/Metroid.romlens")
        );
        assert_eq!(
            with_extension(Path::new("/x/My.Game")),
            Path::new("/x/My.Game.romlens")
        );
    }
}
