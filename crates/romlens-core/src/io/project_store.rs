//! The `.romlens` package: five human-readable JSON files. Serde derives live
//! on private camelCase DTOs here, never on the model, so the file format can
//! evolve independently. Unknown files and unknown fields are ignored;
//! `version > PROJECT_VERSION` is refused.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::API_VERSION;
use crate::error::ProjectError;
use crate::memory::address::{FileOffset, SnesAddress};
use crate::memory::map::MappingMode;
use crate::memory::parse::{AddressExpr, parse_address_expr};
use crate::model::comment::{Comment, CommentKind};
use crate::model::label::{Label, LabelSource};
use crate::model::project::{FlagOverride, Project, RomIdentity, Settings};
use crate::model::region::{DataKind, OverrideKind, RegionOverride};
use crate::rom::image::RomImage;
use crate::viewmodel::hex_rows::AddressStyle;

pub const PROJECT_FORMAT: &str = "romlens-project";
/// 2 since Phase 2: a package may now hold subdirectories (`traces/`,
/// `imports/`) and `project.json` gains arrays describing them. Every added
/// field is `#[serde(default)]` and unknown files are ignored, so a v1 package
/// still opens — `v1_package_still_opens` in `tests/project_store.rs` pins that
/// against a literal v1 package rather than one this code wrote.
pub const PROJECT_VERSION: u32 = 2;
pub const PROJECT_FILES: [&str; 5] = [
    "project.json",
    "labels.json",
    "comments.json",
    "regions.json",
    "flags.json",
];

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectDto {
    format: String,
    version: u32,
    #[serde(default)]
    created_by: String,
    rom: RomDto,
    #[serde(default)]
    settings: SettingsDto,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RomDto {
    sha256: String,
    size: u32,
    mapping: String,
    fast_rom: bool,
    title: String,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SettingsDto {
    #[serde(default)]
    address_style: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LabelDto {
    address: String,
    name: String,
    source: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentDto {
    address: String,
    kind: String,
    text: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegionDto {
    start: String,
    length: u32,
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stride: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bpp: Option<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlagDto {
    address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    m: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    x: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    e: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dbr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dp: Option<String>,
}

fn json(file: &str, e: impl std::fmt::Display) -> ProjectError {
    ProjectError::Json {
        file: file.to_owned(),
        msg: e.to_string(),
    }
}

fn to_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    let mut v = serde_json::to_vec_pretty(value).expect("DTOs serialise");
    v.push(b'\n');
    v
}

fn parse_snes(file: &str, text: &str) -> Result<SnesAddress, ProjectError> {
    match parse_address_expr(text) {
        Ok(AddressExpr::Snes(a)) => Ok(a),
        _ => Err(json(file, format!("bad address {text:?}"))),
    }
}

fn parse_hex(file: &str, text: &str, digits: usize) -> Result<u32, ProjectError> {
    let t = text.trim().trim_start_matches('$');
    if t.is_empty() || t.len() > digits {
        return Err(json(file, format!("bad value {text:?}")));
    }
    u32::from_str_radix(t, 16).map_err(|_| json(file, format!("bad value {text:?}")))
}

/// Serialise a project into its five files.
pub fn to_files(rom: &RomImage, project: &Project) -> BTreeMap<String, Vec<u8>> {
    let addr = |off: FileOffset| {
        rom.snes_address_for(off)
            .map(|a| a.to_string())
            .unwrap_or_else(|| format!("0x{:06X}", off.0))
    };
    let mut files = BTreeMap::new();
    files.insert(
        "project.json".to_owned(),
        to_bytes(&ProjectDto {
            format: PROJECT_FORMAT.to_owned(),
            version: PROJECT_VERSION,
            created_by: format!("romlens {API_VERSION}"),
            rom: RomDto {
                sha256: project.rom.sha256.clone(),
                size: project.rom.size,
                mapping: project.rom.mapping.name().to_owned(),
                fast_rom: project.rom.fast_rom,
                title: project.rom.title.clone(),
            },
            settings: SettingsDto {
                address_style: Some(
                    match project.settings.address_style {
                        AddressStyle::Both => "both",
                        AddressStyle::Snes => "snes",
                        AddressStyle::File => "file",
                    }
                    .to_owned(),
                ),
            },
        }),
    );
    let labels: Vec<LabelDto> = project
        .labels
        .values()
        .map(|l| LabelDto {
            address: l.address.to_string(),
            name: l.name.clone(),
            source: match &l.source {
                LabelSource::Imported(s) => format!("imported:{s}"),
                LabelSource::User => "user".to_owned(),
                LabelSource::Auto => "auto".to_owned(),
                LabelSource::Builtin => "builtin".to_owned(),
            },
        })
        .collect();
    files.insert("labels.json".to_owned(), to_bytes(&labels));
    let comments: Vec<CommentDto> = project
        .comments
        .values()
        .map(|c| CommentDto {
            address: c.address.to_string(),
            kind: c.kind.as_str().to_owned(),
            text: c.text.clone(),
        })
        .collect();
    files.insert("comments.json".to_owned(), to_bytes(&comments));
    let regions: Vec<RegionDto> = project
        .region_overrides
        .iter()
        .map(|r| {
            let (kind, data_kind, stride, bpp) = match r.kind {
                OverrideKind::Code => ("code", None, None, None),
                OverrideKind::Unknown => ("unknown", None, None, None),
                OverrideKind::Data(d) => (
                    "data",
                    Some(d.name().to_owned()),
                    match d {
                        DataKind::Table { stride } => Some(stride),
                        _ => None,
                    },
                    match d {
                        DataKind::Graphics { bpp } => Some(bpp),
                        _ => None,
                    },
                ),
            };
            RegionDto {
                start: addr(r.start),
                length: r.len,
                kind: kind.to_owned(),
                data_kind,
                stride,
                bpp,
            }
        })
        .collect();
    files.insert("regions.json".to_owned(), to_bytes(&regions));
    let flags: Vec<FlagDto> = project
        .flag_overrides
        .iter()
        .map(|(off, f)| FlagDto {
            address: addr(*off),
            m: f.m,
            x: f.x,
            e: f.e,
            dbr: f.dbr.map(|b| format!("${b:02X}")),
            dp: f.dp.map(|d| format!("${d:04X}")),
        })
        .collect();
    files.insert("flags.json".to_owned(), to_bytes(&flags));
    files
}

fn read_list<T: for<'de> Deserialize<'de>>(
    files: &BTreeMap<String, Vec<u8>>,
    name: &str,
) -> Result<Vec<T>, ProjectError> {
    match files.get(name) {
        Some(bytes) => serde_json::from_slice(bytes).map_err(|e| json(name, e)),
        None => Ok(Vec::new()),
    }
}

/// Parse the identity out of `project.json` alone (the shell needs it before
/// it has a ROM).
pub fn read_identity(files: &BTreeMap<String, Vec<u8>>) -> Result<RomIdentity, ProjectError> {
    let bytes = files
        .get("project.json")
        .ok_or_else(|| ProjectError::MissingFile("project.json".into()))?;
    let dto: ProjectDto = serde_json::from_slice(bytes).map_err(|e| json("project.json", e))?;
    if dto.format != PROJECT_FORMAT {
        return Err(ProjectError::BadFormat(format!("format {:?}", dto.format)));
    }
    if dto.version > PROJECT_VERSION {
        return Err(ProjectError::NewerVersion(dto.version));
    }
    let mapping = MappingMode::all()
        .into_iter()
        .find(|m| m.name().eq_ignore_ascii_case(&dto.rom.mapping))
        .ok_or_else(|| {
            json(
                "project.json",
                format!("unknown mapping {:?}", dto.rom.mapping),
            )
        })?;
    Ok(RomIdentity {
        sha256: dto.rom.sha256,
        size: dto.rom.size,
        mapping,
        fast_rom: dto.rom.fast_rom,
        title: dto.rom.title,
    })
}

/// Rebuild a project from its files. `rom` canonicalises addresses.
pub fn from_files(
    rom: &RomImage,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<Project, ProjectError> {
    let identity = read_identity(files)?;
    let dto: ProjectDto =
        serde_json::from_slice(&files["project.json"]).map_err(|e| json("project.json", e))?;
    let mut project = Project::new(rom);
    project.rom = identity;
    project.settings = Settings {
        address_style: match dto.settings.address_style.as_deref() {
            Some("snes") => AddressStyle::Snes,
            Some("file") => AddressStyle::File,
            _ => AddressStyle::Both,
        },
    };
    for l in read_list::<LabelDto>(files, "labels.json")? {
        let address = Project::canonical(rom, parse_snes("labels.json", &l.address)?);
        let source = match l.source.as_str() {
            "user" => LabelSource::User,
            s if s.starts_with("imported:") => {
                LabelSource::Imported(s["imported:".len()..].to_owned())
            }
            "imported" => LabelSource::Imported(String::new()),
            _ => LabelSource::User,
        };
        project.labels.insert(
            address,
            Label {
                address,
                name: l.name,
                source,
            },
        );
    }
    for c in read_list::<CommentDto>(files, "comments.json")? {
        let address = Project::canonical(rom, parse_snes("comments.json", &c.address)?);
        let kind = match c.kind.as_str() {
            "block" => CommentKind::Block,
            _ => CommentKind::Line,
        };
        project.comments.insert(
            (address, kind),
            Comment {
                address,
                kind,
                text: c.text,
            },
        );
    }
    let offset_of = |file: &str, text: &str| -> Result<FileOffset, ProjectError> {
        match parse_address_expr(text) {
            Ok(AddressExpr::File(off)) => Ok(off),
            Ok(AddressExpr::Snes(a)) => rom
                .file_offset_for(a)
                .ok_or_else(|| json(file, format!("{text} is not in ROM"))),
            Err(_) => Err(json(file, format!("bad address {text:?}"))),
        }
    };
    let mut regions = Vec::new();
    for r in read_list::<RegionDto>(files, "regions.json")? {
        let start = offset_of("regions.json", &r.start)?;
        let kind = match r.kind.as_str() {
            "code" => OverrideKind::Code,
            "unknown" => OverrideKind::Unknown,
            "data" => OverrideKind::Data(
                DataKind::parse(r.data_kind.as_deref().unwrap_or("byte"), r.stride, r.bpp)
                    .ok_or_else(|| {
                        json(
                            "regions.json",
                            format!("unknown data kind {:?}", r.data_kind),
                        )
                    })?,
            ),
            other => {
                return Err(json(
                    "regions.json",
                    format!("unknown region kind {other:?}"),
                ));
            }
        };
        if r.length == 0 || start.0 as u64 + r.length as u64 > rom.len() as u64 {
            return Err(ProjectError::BadRange(format!("{}+{}", r.start, r.length)));
        }
        regions.push(RegionOverride {
            start,
            len: r.length,
            kind,
        });
    }
    regions.sort_by_key(|r| r.start);
    project.region_overrides = regions;
    for f in read_list::<FlagDto>(files, "flags.json")? {
        let offset = offset_of("flags.json", &f.address)?;
        let flags = FlagOverride {
            m: f.m,
            x: f.x,
            e: f.e,
            dbr: f
                .dbr
                .as_deref()
                .map(|t| parse_hex("flags.json", t, 2))
                .transpose()?
                .map(|v| v as u8),
            dp: f
                .dp
                .as_deref()
                .map(|t| parse_hex("flags.json", t, 4))
                .transpose()?
                .map(|v| v as u16),
        };
        if !flags.is_empty() {
            project.flag_overrides.insert(offset, flags);
        }
    }
    Ok(project)
}

/// A package name is always `/`-separated and always relative, on every
/// platform, so the same key round-trips through a project written on macOS
/// and read on Windows. Nothing may climb out of the package directory.
fn package_path(dir: &Path, name: &str) -> Result<std::path::PathBuf, ProjectError> {
    let mut path = dir.to_path_buf();
    let mut any = false;
    for part in name.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains('\\') {
            return Err(ProjectError::MissingFile(name.to_owned()));
        }
        path.push(part);
        any = true;
    }
    if !any {
        return Err(ProjectError::MissingFile(name.to_owned()));
    }
    Ok(path)
}

fn package_leaf(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// Write the files into `dir` (created if needed), each through a temporary
/// file and a rename so a crash never leaves a half-written file.
///
/// A name may carry one or more `/` segments: traces and imports live in
/// subdirectories of the package (`16-phase2-plan.md`, 2A.3), so the parent is
/// created before the temporary file is written rather than assuming `dir`
/// itself is where every file lands.
pub fn write_package(dir: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<(), ProjectError> {
    std::fs::create_dir_all(dir)?;
    for (name, bytes) in files {
        let path = package_path(dir, name)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_file_name(format!("{}.tmp", package_leaf(name)));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
    }
    Ok(())
}

/// Read every regular file in a package directory, including subdirectories.
/// Keys are `/`-separated paths relative to `dir`, so `traces/play.cdl` reads
/// back under that name on Windows too.
pub fn read_package(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, ProjectError> {
    let mut files = BTreeMap::new();
    read_into(dir, "", &mut files)?;
    if !files.contains_key("project.json") {
        return Err(ProjectError::MissingFile("project.json".into()));
    }
    Ok(files)
}

fn read_into(
    dir: &Path,
    prefix: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), ProjectError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let key = format!("{prefix}{name}");
        let kind = entry.file_type()?;
        if kind.is_dir() {
            read_into(&entry.path(), &format!("{key}/"), files)?;
        } else if kind.is_file() && !name.ends_with(".tmp") {
            files.insert(key, std::fs::read(entry.path())?);
        }
    }
    Ok(())
}
