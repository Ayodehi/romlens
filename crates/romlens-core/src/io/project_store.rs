//! The `.romlens` package: five human-readable JSON files. Serde derives live
//! on private camelCase DTOs here, never on the model, so the file format can
//! evolve independently. Unknown files and unknown fields are ignored;
//! `version > PROJECT_VERSION` is refused.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::API_VERSION;
use crate::error::ProjectError;
use crate::graphics::tilemap::ScreenSize;
use crate::memory::address::{FileOffset, SnesAddress};
use crate::memory::map::MappingMode;
use crate::memory::parse::{AddressExpr, parse_address_expr};
use crate::model::comment::{Comment, CommentKind};
use crate::model::label::{Label, LabelSource};
use crate::model::project::{
    FlagOverride, ImportRecord, Project, RecordingRef, RomIdentity, Settings, TraceRecord,
};
use crate::model::region::{
    BankRule, DataKind, OverrideKind, RegionOverride, RegionParams, TableElem,
};
use crate::model::source_map::{LineKind, SourceFile, SourceLine, SourceMap};
use crate::model::variable::{VarType, VarWidth};
use crate::rom::image::RomImage;
use crate::viewmodel::hex_rows::AddressStyle;

pub const PROJECT_FORMAT: &str = "romlens-project";
/// 2 since Phase 2: a package may now hold subdirectories (`traces/`,
/// `imports/`) and `project.json` gains arrays describing them. Every added
/// field is `#[serde(default)]` and unknown files are ignored, so a v1 package
/// still opens — `v1_package_still_opens` in `tests/project_store.rs` pins that
/// against a literal v1 package rather than one this code wrote.
pub const PROJECT_VERSION: u32 = 2;
/// Variable types. Optional: a package without it has none, and older
/// Romlens builds ignore it.
pub const VARIABLES_FILE: &str = "variables.json";

/// Where the merged coverage lives inside a package.
pub const COVERAGE_FILE: &str = "traces/coverage.cdl";
/// Where the merged execution log lives, in the format the fork writes.
pub const EXEC_LOG_FILE: &str = "traces/execution.mxlog";
/// Source lines from imported debug information (`io::import::dbg`).
pub const SOURCES_FILE: &str = "imports/sources.json";

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
    /// Imported traces. `default` so a v1 package still opens.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    traces: Vec<TraceDto>,
    /// Imported symbol files, with their licence notices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    imports: Vec<ImportDto>,
    /// Recordings, by reference only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    recordings: Vec<RecordingDto>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordingDto {
    path: String,
    #[serde(default)]
    frames: u64,
    #[serde(default)]
    producer: String,
    #[serde(default)]
    fingerprint: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceMapDto {
    source: String,
    #[serde(default)]
    dir: String,
    files: Vec<SourceFileDto>,
    lines: Vec<SourceLineDto>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceFileDto {
    name: String,
    #[serde(default)]
    size: u32,
    #[serde(default)]
    mtime: u32,
}

/// `ranges` are `[file offset, length]` pairs: a package of a large program
/// holds tens of thousands of lines, so they are numbers, not addresses.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceLineDto {
    file: u32,
    line: u32,
    kind: String,
    ranges: Vec<[u32; 2]>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportDto {
    source: String,
    format: String,
    #[serde(default)]
    labels: u64,
    #[serde(default)]
    comments: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    notice: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TraceDto {
    source: String,
    format: String,
    #[serde(default)]
    executed_bytes: u64,
    #[serde(default)]
    read_bytes: u64,
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

/// One variable's type; its name is the label at the same address.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VariableDto {
    address: String,
    #[serde(rename = "type")]
    width: String,
    count: u16,
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
    /// For `table`: `raw`, `pointer` or `code`. `default` so a v1 package,
    /// which had neither field, still opens as `raw` in the same bank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    elem: Option<String>,
    /// For `table` and `pointer`: `same`, `entry` or a bank such as `$C0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bank: Option<String>,
    /// How the range previews (track 2B). Absent in every package written
    /// before it, which reads as "the view's defaults".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview: Option<PreviewDto>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PreviewDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    palette: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    columns: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    screen_size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tiles: Option<String>,
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
            traces: project
                .traces
                .iter()
                .map(|t| TraceDto {
                    source: t.source.clone(),
                    format: t.format.clone(),
                    executed_bytes: t.executed_bytes,
                    read_bytes: t.read_bytes,
                })
                .collect(),
            imports: project
                .imports
                .iter()
                .map(|i| ImportDto {
                    source: i.source.clone(),
                    format: i.format.clone(),
                    labels: i.labels,
                    comments: i.comments,
                    notice: i.notice.clone(),
                })
                .collect(),
            recordings: project
                .recordings
                .iter()
                .map(|r| RecordingDto {
                    path: r.path.clone(),
                    frames: r.frames,
                    producer: r.producer.clone(),
                    fingerprint: r.fingerprint.clone(),
                })
                .collect(),
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
    // Only when there are any: a project with none keeps the five files it
    // has always had.
    if !project.variables.is_empty() {
        let variables: Vec<VariableDto> = project
            .variables
            .iter()
            .map(|(a, t)| VariableDto {
                address: a.to_string(),
                width: t.width.name().to_owned(),
                count: t.count,
            })
            .collect();
        files.insert(VARIABLES_FILE.to_owned(), to_bytes(&variables));
    }
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
            let (kind, data_kind, stride, bpp, elem, bank) = match r.kind {
                OverrideKind::Code => ("code", None, None, None, None, None),
                OverrideKind::Unknown => ("unknown", None, None, None, None, None),
                OverrideKind::Data(d) => (
                    "data",
                    Some(d.name().to_owned()),
                    match d {
                        DataKind::Table { stride, .. } => Some(stride),
                        _ => None,
                    },
                    match d {
                        DataKind::Graphics { bpp } => Some(bpp),
                        _ => None,
                    },
                    // Both parameters are written only when they are not
                    // the default, so a plain `dw` table's JSON is exactly
                    // what v1 wrote.
                    match d {
                        DataKind::Table { elem, .. } => Some(elem),
                        _ => None,
                    }
                    .filter(|e| *e != TableElem::Raw)
                    .map(|e| e.name().to_owned()),
                    // Written only when it is not the default, so a plain
                    // `dw` table's JSON stays as short as it was in v1.
                    match d {
                        DataKind::Pointer { bank } => Some(bank),
                        DataKind::Table { elem, .. } => elem.bank(),
                        _ => None,
                    }
                    .filter(|b| *b != BankRule::SameBank)
                    .map(BankRule::name),
                ),
            };
            let p = r.params;
            RegionDto {
                start: addr(r.start),
                length: r.len,
                kind: kind.to_owned(),
                data_kind,
                stride,
                bpp,
                elem,
                bank,
                preview: (!p.is_default()).then(|| PreviewDto {
                    palette: p.palette.map(|a| a.to_string()),
                    columns: p.columns,
                    screen_size: p.screen_size.map(|s| s.name().to_owned()),
                    tiles: p.tiles.map(|a| a.to_string()),
                }),
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
    if !project.source_maps.is_empty() {
        let maps: Vec<SourceMapDto> = project
            .source_maps
            .iter()
            .map(|m| SourceMapDto {
                source: m.source.clone(),
                dir: m.dir.clone(),
                files: m
                    .files
                    .iter()
                    .map(|f| SourceFileDto {
                        name: f.name.clone(),
                        size: f.size,
                        mtime: f.mtime,
                    })
                    .collect(),
                lines: m
                    .lines
                    .iter()
                    .map(|l| SourceLineDto {
                        file: l.file,
                        line: l.line,
                        kind: l.kind.name().to_owned(),
                        ranges: l.ranges.iter().map(|(o, n)| [o.0, *n]).collect(),
                    })
                    .collect(),
            })
            .collect();
        files.insert(SOURCES_FILE.to_owned(), to_bytes(&maps));
    }
    // Every imported trace, merged, folded to one byte per ROM byte. A
    // bsnes-plus usage map is 16.8 MB of mostly nothing and a package must not
    // carry that; `project.json`'s `traces` array says what went in.
    if let Some(coverage) = &project.coverage
        && !project.traces.is_empty()
    {
        files.insert(
            COVERAGE_FILE.to_owned(),
            crate::io::import::to_stored(coverage),
        );
        if let Some(log) = &project.exec_log {
            files.insert(
                EXEC_LOG_FILE.to_owned(),
                crate::io::import::exec_log::write(log),
            );
        }
    }
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
    for v in read_list::<VariableDto>(files, VARIABLES_FILE)? {
        let address = Project::canonical(rom, parse_snes(VARIABLES_FILE, &v.address)?);
        let width = VarWidth::parse(&v.width).ok_or_else(|| {
            ProjectError::InvalidVariable(format!(
                "{VARIABLES_FILE}: {:?} is not byte, word or long",
                v.width
            ))
        })?;
        let ty = VarType {
            width,
            count: v.count,
        };
        ty.validate()?;
        project.variables.insert(address, ty);
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
            "data" => {
                let bank = match r.bank.as_deref() {
                    Some(text) => Some(BankRule::parse(text).ok_or_else(|| {
                        json("regions.json", format!("unknown bank rule {text:?}"))
                    })?),
                    None => None,
                };
                OverrideKind::Data(
                    DataKind::parse_with(
                        r.data_kind.as_deref().unwrap_or("byte"),
                        r.stride,
                        r.bpp,
                        r.elem.as_deref(),
                        bank,
                    )
                    .ok_or_else(|| {
                        json(
                            "regions.json",
                            format!(
                                "unknown data kind {:?} or element {:?}",
                                r.data_kind, r.elem
                            ),
                        )
                    })?,
                )
            }
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
        let preview = r.preview.unwrap_or_default();
        let params = RegionParams {
            palette: preview
                .palette
                .as_deref()
                .map(|t| parse_snes("regions.json", t))
                .transpose()?,
            columns: preview.columns,
            screen_size: preview
                .screen_size
                .as_deref()
                .map(|t| {
                    ScreenSize::parse(t)
                        .ok_or_else(|| json("regions.json", format!("unknown screen size {t:?}")))
                })
                .transpose()?,
            tiles: preview
                .tiles
                .as_deref()
                .map(|t| parse_snes("regions.json", t))
                .transpose()?,
        };
        regions.push(RegionOverride {
            start,
            len: r.length,
            kind,
            params,
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
    project.imports = dto
        .imports
        .iter()
        .map(|i| ImportRecord {
            source: i.source.clone(),
            format: i.format.clone(),
            labels: i.labels,
            comments: i.comments,
            notice: i.notice.clone(),
        })
        .collect();
    project.recordings = dto
        .recordings
        .iter()
        .map(|r| RecordingRef {
            path: r.path.clone(),
            frames: r.frames,
            producer: r.producer.clone(),
            fingerprint: r.fingerprint.clone(),
        })
        .collect();
    let maps: Vec<SourceMapDto> = read_list(files, SOURCES_FILE)?;
    project.source_maps = maps
        .into_iter()
        .map(|m| {
            let lines = m
                .lines
                .into_iter()
                .map(|l| {
                    Ok(SourceLine {
                        file: l.file,
                        line: l.line,
                        kind: LineKind::parse(&l.kind).ok_or_else(|| {
                            ProjectError::BadFormat(format!(
                                "{SOURCES_FILE}: unknown line kind {:?}",
                                l.kind
                            ))
                        })?,
                        ranges: l
                            .ranges
                            .into_iter()
                            .map(|[o, n]| (FileOffset(o), n))
                            .collect(),
                    })
                })
                .collect::<Result<Vec<_>, ProjectError>>()?;
            Ok(SourceMap {
                source: m.source,
                dir: m.dir,
                files: m
                    .files
                    .into_iter()
                    .map(|f| SourceFile {
                        name: f.name,
                        size: f.size,
                        mtime: f.mtime,
                    })
                    .collect(),
                lines,
            })
        })
        .collect::<Result<Vec<_>, ProjectError>>()?;
    project.traces = dto
        .traces
        .iter()
        .map(|t| TraceRecord {
            source: t.source.clone(),
            format: t.format.clone(),
            executed_bytes: t.executed_bytes,
            read_bytes: t.read_bytes,
        })
        .collect();
    // A missing payload drops the records rather than refusing the package: a
    // project copied without its `traces/` directory should still open, with
    // every annotation intact and the coverage simply gone.
    if let Some(bytes) = files.get(COVERAGE_FILE)
        && !project.traces.is_empty()
    {
        project.coverage = Some(std::sync::Arc::new(crate::io::import::from_stored(
            bytes,
            rom.len() as u32,
        )?));
        if let Some(bytes) = files.get(EXEC_LOG_FILE) {
            project.exec_log = Some(std::sync::Arc::new(crate::io::import::exec_log::read(
                bytes,
                rom.bytes(),
            )?));
        }
    } else {
        project.traces.clear();
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
