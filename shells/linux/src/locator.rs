//! Finds the ROM a project belongs to. Remembered paths by hash (this
//! machine's `roms.json`), then the path the project last used, then any
//! `.sfc` or `.smc` beside the package; the core checks each candidate's hash.
//! Asking the person is the caller's last resort. The macOS twin is
//! `DefaultRomLocator`, which does the same with security-scoped bookmarks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use romlens_ffi::RomIdentityInfo;

use crate::config::config_dir;
use crate::package::LocalRecord;

pub fn hints_path() -> PathBuf {
    config_dir().join("roms.json")
}

fn load(path: &Path) -> HashMap<String, String> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Remember where a ROM lives, keyed by its payload hash.
pub fn remember(sha256: &str, rom: &Path) {
    remember_in(&hints_path(), sha256, rom);
}

pub fn remember_in(path: &Path, sha256: &str, rom: &Path) {
    let mut all = load(path);
    all.insert(sha256.to_owned(), rom.to_string_lossy().into_owned());
    // Failing to remember is not worth an error.
    let _ = (|| -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(&all).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(tmp, path)
    })();
}

/// Candidate paths, most trusted first.
pub fn hints(sha256: &str, local: &LocalRecord, hints_file: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(p) = load(hints_file).get(sha256) {
        out.push(p.clone());
    }
    if let Some(p) = &local.last_path {
        out.push(p.clone());
    }
    out
}

pub fn locate(identity: &RomIdentityInfo, package: &Path, local: &LocalRecord) -> Option<PathBuf> {
    locate_with(identity, package, local, &hints_path())
}

pub fn locate_with(
    identity: &RomIdentityInfo,
    package: &Path,
    local: &LocalRecord,
    hints_file: &Path,
) -> Option<PathBuf> {
    romlens_ffi::workbench::locate_rom(
        identity.clone(),
        package.to_string_lossy().into_owned(),
        hints(&identity.sha256, local, hints_file),
    )
    .map(PathBuf::from)
}

/// Whether the file at `path` is the ROM `identity` describes.
pub fn matches(identity: &RomIdentityInfo, path: &Path) -> bool {
    romlens_ffi::Rom::open(path.to_string_lossy().into_owned())
        .is_ok_and(|rom| rom.info().sha256 == identity.sha256)
}

#[cfg(test)]
mod tests {
    use super::*;
    use romlens_ffi::{Mapping, Rom, make_test_rom};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("romlens-loc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn identity(bytes: &[u8]) -> RomIdentityInfo {
        let info = Rom::from_bytes(bytes.to_vec(), "x.sfc".into())
            .unwrap()
            .info();
        RomIdentityInfo {
            sha256: info.sha256,
            size: info.byte_len,
            mapping: info.mapping,
            fast_rom: info.fast_rom,
            title: info.title,
        }
    }

    #[test]
    fn a_rom_beside_the_package_is_found_by_hash() {
        let dir = scratch("beside");
        let rom = make_test_rom(Mapping::LoRom);
        std::fs::write(dir.join("other.sfc"), make_test_rom(Mapping::HiRom)).unwrap();
        std::fs::write(dir.join("game.sfc"), &rom).unwrap();
        let package = dir.join("game.romlens");
        let found = locate_with(
            &identity(&rom),
            &package,
            &LocalRecord::default(),
            &dir.join("none.json"),
        );
        assert_eq!(found, Some(dir.join("game.sfc")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn remembered_paths_win_and_must_match_the_hash() {
        let dir = scratch("hints");
        let rom = make_test_rom(Mapping::LoRom);
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("kept.sfc"), &rom).unwrap();
        let id = identity(&rom);
        let hints_file = dir.join("roms.json");
        remember_in(&hints_file, &id.sha256, &elsewhere.join("kept.sfc"));
        let package = dir.join("proj").join("game.romlens");
        assert_eq!(
            locate_with(&id, &package, &LocalRecord::default(), &hints_file),
            Some(elsewhere.join("kept.sfc"))
        );
        // A hint to a different ROM is not trusted.
        std::fs::write(elsewhere.join("kept.sfc"), make_test_rom(Mapping::HiRom)).unwrap();
        assert_eq!(
            locate_with(&id, &package, &LocalRecord::default(), &hints_file),
            None
        );
        // The project's own last path is the next hint.
        std::fs::write(dir.join("last.sfc"), &rom).unwrap();
        let local = LocalRecord {
            last_path: Some(dir.join("last.sfc").to_string_lossy().into_owned()),
        };
        assert_eq!(
            locate_with(&id, &package, &local, &hints_file),
            Some(dir.join("last.sfc"))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_found_is_none_and_matches_checks_the_hash() {
        let dir = scratch("none");
        let rom = make_test_rom(Mapping::LoRom);
        let id = identity(&rom);
        assert_eq!(
            locate_with(
                &id,
                &dir.join("g.romlens"),
                &LocalRecord::default(),
                &dir.join("n.json")
            ),
            None
        );
        std::fs::write(dir.join("a.sfc"), &rom).unwrap();
        std::fs::write(dir.join("b.sfc"), make_test_rom(Mapping::HiRom)).unwrap();
        assert!(matches(&id, &dir.join("a.sfc")));
        assert!(!matches(&id, &dir.join("b.sfc")));
        assert!(!matches(&id, &dir.join("missing.sfc")));
        let _ = std::fs::remove_dir_all(dir);
    }
}
