//! Opening a ROM with an optional project and running the analyzer: the
//! CLI's stand-in for the FFI `Workbench`.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use romlens_core::analysis::{AnalysisControl, AnalysisSnapshot, analyze};
use romlens_core::io::{
    from_files, locate_rom, read_identity, read_package, to_files, write_package,
};
use romlens_core::model::Project;
use romlens_core::{AddressExpr, LineIndex, RomImage, parse_address_expr};

pub struct Session {
    pub rom: RomImage,
    pub project: Project,
    pub snap: AnalysisSnapshot,
    pub idx: LineIndex,
}

pub fn load_rom(path: &Path) -> Result<RomImage> {
    RomImage::load(path).with_context(|| format!("opening {}", path.display()))
}

/// Read a package for `rom`, checking the identity.
pub fn load_project(rom: &RomImage, dir: &Path) -> Result<Project> {
    let files = read_package(dir).with_context(|| format!("reading {}", dir.display()))?;
    let identity = read_identity(&files)?;
    if !identity.matches(rom) {
        return Err(anyhow!(
            "project {} belongs to a different ROM (expected SHA-256 {}, found {})",
            dir.display(),
            identity.sha256,
            rom.sha256_hex()
        ));
    }
    Ok(from_files(rom, &files)?)
}

pub fn save_project(rom: &RomImage, dir: &Path, project: &Project) -> Result<()> {
    write_package(dir, &to_files(rom, project))
        .with_context(|| format!("writing {}", dir.display()))
}

/// The ROM a package belongs to: `--rom` if given, else located beside it.
pub fn rom_for_project(dir: &Path, rom: Option<&Path>) -> Result<RomImage> {
    if let Some(p) = rom {
        return load_rom(p);
    }
    let files = read_package(dir).with_context(|| format!("reading {}", dir.display()))?;
    let identity = read_identity(&files)?;
    let found = locate_rom(&identity, dir, &[]).ok_or_else(|| {
        anyhow!(
            "no ROM with SHA-256 {} beside {}; pass --rom",
            identity.sha256,
            dir.display()
        )
    })?;
    load_rom(&found)
}

pub fn open(rom: &Path, project: Option<&Path>, progress: bool) -> Result<Session> {
    let rom = load_rom(rom)?;
    let project = match project {
        Some(dir) => load_project(&rom, dir)?,
        None => Project::new(&rom),
    };
    open_with(rom, project, progress)
}

pub fn open_with(rom: RomImage, project: Project, progress: bool) -> Result<Session> {
    let control = if progress {
        AnalysisControl::with_progress(|p| {
            eprintln!("{}: {}/{}", p.phase.name(), p.done, p.total);
        })
    } else {
        AnalysisControl::silent()
    };
    let snap = analyze(&rom, &project, &control).map_err(|_| anyhow!("analysis cancelled"))?;
    if progress {
        eprintln!("analysis: {} ms", snap.stats.elapsed_ms);
    }
    let idx = LineIndex::build(&rom, &snap, &project);
    Ok(Session {
        rom,
        project,
        snap,
        idx,
    })
}

/// A ROM offset from an address expression.
pub fn rom_offset(rom: &RomImage, expr: &str) -> Result<u32> {
    Ok(rom.resolve(expr)?.file_offset.0)
}

/// A parsed expression that may be RAM or hardware.
pub fn any_address(expr: &str) -> Result<AddressExpr> {
    Ok(parse_address_expr(expr)?)
}
