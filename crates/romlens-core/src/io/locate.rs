//! Find the ROM a project belongs to: hinted paths whose payload hash
//! matches first, then any `.sfc`/`.smc` beside the package.

use std::path::{Path, PathBuf};

use crate::model::project::RomIdentity;
use crate::rom::image::RomImage;

fn matches(identity: &RomIdentity, path: &Path) -> bool {
    RomImage::load(path).is_ok_and(|rom| identity.matches(&rom))
}

/// `hints` are remembered paths, most recent first.
pub fn locate_rom(
    identity: &RomIdentity,
    package_dir: &Path,
    hints: &[PathBuf],
) -> Option<PathBuf> {
    for hint in hints {
        if hint.is_file() && matches(identity, hint) {
            return Some(hint.clone());
        }
    }
    let beside = package_dir.parent()?;
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(beside)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("sfc") || e.eq_ignore_ascii_case("smc"))
        })
        .collect();
    candidates.sort();
    candidates.into_iter().find(|p| matches(identity, p))
}
