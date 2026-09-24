//! Records and enums that mirror core types through `From` impls, so the
//! core never depends on `uniffi`.

use romlens_core::analysis::{self, snapshot};
use romlens_core::cpu65816;
use romlens_core::model;
use romlens_core::{FileOffset, MemoryClass as CoreMemoryClass, SnesAddress};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FlagState {
    pub m: bool,
    pub x: bool,
    pub e: bool,
    pub dbr: Option<u8>,
    pub dp: Option<u16>,
}

impl From<cpu65816::FlagState> for FlagState {
    fn from(f: cpu65816::FlagState) -> Self {
        Self {
            m: f.m,
            x: f.x,
            e: f.e,
            dbr: f.dbr,
            dp: f.dp,
        }
    }
}

impl From<FlagState> for cpu65816::FlagState {
    fn from(f: FlagState) -> Self {
        Self {
            m: f.m,
            x: f.x,
            e: f.e,
            dbr: f.dbr,
            dp: f.dp,
            c: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AddressStyle {
    Both,
    Snes,
    File,
}

impl From<AddressStyle> for romlens_core::AddressStyle {
    fn from(a: AddressStyle) -> Self {
        match a {
            AddressStyle::Both => romlens_core::AddressStyle::Both,
            AddressStyle::Snes => romlens_core::AddressStyle::Snes,
            AddressStyle::File => romlens_core::AddressStyle::File,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TargetKind {
    Code,
    Data,
    Pointer,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HardwareRegisterInfo {
    pub address: u16,
    pub name: String,
    pub access: Access,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Access {
    Read,
    Write,
    ReadWrite,
}

impl From<&model::HardwareRegister> for HardwareRegisterInfo {
    fn from(r: &model::HardwareRegister) -> Self {
        Self {
            address: r.address,
            name: r.name.to_owned(),
            access: match r.access {
                model::Access::Read => Access::Read,
                model::Access::Write => Access::Write,
                model::Access::ReadWrite => Access::ReadWrite,
            },
            description: r.description.to_owned(),
        }
    }
}

/// One decoded instruction with everything the inspector shows.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InstructionInfo {
    pub file_offset: u32,
    pub snes_address: u32,
    pub len: u8,
    pub bytes: Vec<u8>,
    pub opcode: u8,
    pub mnemonic: String,
    pub description: String,
    pub mode: String,
    /// `LDA #$01`, labels applied.
    pub text: String,
    pub operand_text: String,
    pub target: Option<u32>,
    pub target_file_offset: Option<u32>,
    pub target_kind: Option<TargetKind>,
    pub target_certain: bool,
    pub flags_before: FlagState,
    pub flags_after: FlagState,
    pub assumptions: Vec<String>,
    pub hardware_register: Option<HardwareRegisterInfo>,
}

pub fn instruction_info(
    rom: &romlens_core::RomImage,
    insn: &cpu65816::Instruction,
    symbols: &dyn cpu65816::SymbolLookup,
) -> InstructionInfo {
    let f = cpu65816::format_instruction(insn, symbols);
    InstructionInfo {
        file_offset: insn.file_offset.0,
        snes_address: insn.address.as_u24(),
        len: insn.len,
        bytes: insn.bytes().to_vec(),
        opcode: insn.opcode,
        mnemonic: insn.mnemonic.as_str().to_owned(),
        description: insn.mnemonic.describe().to_owned(),
        mode: insn.mode.describe().to_owned(),
        text: f.text,
        operand_text: f.operand_text,
        target: insn.target.map(|t| t.address.as_u24()),
        target_file_offset: insn
            .target
            .and_then(|t| rom.file_offset_for(t.address))
            .map(FileOffset::value),
        target_kind: insn.target.map(|t| match t.kind {
            cpu65816::TargetKind::Code => TargetKind::Code,
            cpu65816::TargetKind::Data => TargetKind::Data,
            cpu65816::TargetKind::Pointer => TargetKind::Pointer,
        }),
        target_certain: insn.target.is_some_and(|t| t.certain),
        flags_before: insn.flags_before.into(),
        flags_after: insn.flags_after.into(),
        assumptions: cpu65816::assumption_names(insn.assumptions)
            .into_iter()
            .map(str::to_owned)
            .collect(),
        hardware_register: f.register.map(HardwareRegisterInfo::from),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RegionKind {
    Unknown,
    Code,
    Data,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DataKind {
    Byte,
    Word,
    Long,
    Pointer,
    Table,
    String,
    Graphics,
    Tilemap,
    Palette,
    Compressed,
    Struct,
}

/// Which bank a 16-bit pointer's target is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BankRule {
    /// The bank the table itself is in.
    SameBank,
    /// One bank for every entry.
    Fixed { bank: u8 },
    /// The entry carries its own bank.
    FromEntry,
}

impl From<model::BankRule> for BankRule {
    fn from(b: model::BankRule) -> Self {
        match b {
            model::BankRule::SameBank => BankRule::SameBank,
            model::BankRule::Fixed(bank) => BankRule::Fixed { bank },
            model::BankRule::FromEntry => BankRule::FromEntry,
        }
    }
}

impl From<BankRule> for model::BankRule {
    fn from(b: BankRule) -> Self {
        match b {
            BankRule::SameBank => model::BankRule::SameBank,
            BankRule::Fixed { bank } => model::BankRule::Fixed(bank),
            BankRule::FromEntry => model::BankRule::FromEntry,
        }
    }
}

/// What one element of a table is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TableElem {
    Raw,
    Pointer,
    Code,
}

impl From<model::TableElem> for TableElem {
    fn from(e: model::TableElem) -> Self {
        match e {
            model::TableElem::Raw => TableElem::Raw,
            model::TableElem::Pointer(_) => TableElem::Pointer,
            model::TableElem::Code(_) => TableElem::Code,
        }
    }
}

impl From<model::DataKind> for DataKind {
    fn from(d: model::DataKind) -> Self {
        match d {
            model::DataKind::Byte => DataKind::Byte,
            model::DataKind::Word => DataKind::Word,
            model::DataKind::Long => DataKind::Long,
            model::DataKind::Pointer { .. } => DataKind::Pointer,
            model::DataKind::Table { .. } => DataKind::Table,
            model::DataKind::String => DataKind::String,
            model::DataKind::Graphics { .. } => DataKind::Graphics,
            model::DataKind::Tilemap => DataKind::Tilemap,
            model::DataKind::Palette => DataKind::Palette,
            model::DataKind::Compressed => DataKind::Compressed,
            model::DataKind::Struct => DataKind::Struct,
        }
    }
}

pub fn core_data_kind(
    d: DataKind,
    stride: Option<u8>,
    bpp: Option<u8>,
    elem: Option<TableElem>,
    bank: Option<BankRule>,
) -> model::DataKind {
    let bank = bank.map_or(model::BankRule::SameBank, Into::into);
    match d {
        DataKind::Byte => model::DataKind::Byte,
        DataKind::Word => model::DataKind::Word,
        DataKind::Long => model::DataKind::Long,
        DataKind::Pointer => model::DataKind::Pointer { bank },
        DataKind::Table => model::DataKind::Table {
            stride: stride.unwrap_or(2),
            elem: match elem.unwrap_or(TableElem::Raw) {
                TableElem::Raw => model::TableElem::Raw,
                TableElem::Pointer => model::TableElem::Pointer(bank),
                TableElem::Code => model::TableElem::Code(bank),
            },
        },
        DataKind::String => model::DataKind::String,
        DataKind::Graphics => model::DataKind::Graphics {
            bpp: bpp.unwrap_or(4),
        },
        DataKind::Tilemap => model::DataKind::Tilemap,
        DataKind::Palette => model::DataKind::Palette,
        DataKind::Compressed => model::DataKind::Compressed,
        DataKind::Struct => model::DataKind::Struct,
    }
}

/// What an import did, for the sheet that reports it.
///
/// Every count is here because nothing may be dropped silently: a shell that
/// shows only the successes is the bug `io::import::symbols` is written to
/// prevent.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ImportResult {
    pub source: String,
    pub format: String,
    pub labels_added: u32,
    pub labels_replaced: u32,
    pub comments_added: u32,
    /// Addresses left alone because the user had named them.
    pub kept_user: u32,
    /// `original -> rewritten`, for the names that had to change.
    pub rewritten: Vec<String>,
    /// Lines that were not understood.
    pub skipped: Vec<String>,
    /// The file's leading comment block, kept with the project.
    pub notice: String,
    /// For an execution log: what it holds, in words. Empty otherwise.
    pub detail: String,
    /// For a trace: bytes seen to execute and to be read.
    pub executed_bytes: u64,
    pub read_bytes: u64,
    /// Whether the trace recorded M/X widths.
    pub has_widths: bool,
}

/// Everything ⌘F needs to ask for, in one record.
///
/// A record rather than seven parameters: a call site with seven positional
/// arguments is unreadable in every generated language, and a search gains
/// options over time.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SearchQuery {
    /// Hex byte pairs with `??` wildcards, or text when `text` is set.
    pub pattern: String,
    /// Match the pattern's own bytes rather than parsing it as hex.
    pub text: bool,
    /// Match ASCII letters in either case. Implies `text`.
    pub ignore_case: bool,
    pub start: u32,
    pub len: u32,
    /// At most this many hits.
    pub max: u32,
    /// Bytes either side of each hit.
    pub context: u32,
}

/// One search result, with enough context to judge it without a second call.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SearchHit {
    pub file_offset: u32,
    pub snes_address: Option<String>,
    /// Bytes matched.
    pub len: u32,
    /// The match and some bytes either side.
    pub context: Vec<u8>,
    /// Where the match starts inside `context`.
    pub match_start: u32,
    /// What the analyzer calls these bytes, so a hit inside code reads
    /// differently from one inside a table.
    pub region_kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum EvidenceKind {
    VectorReach,
    Heuristic,
    User,
    Imported,
    Trace,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct EvidenceInfo {
    pub kind: EvidenceKind,
    pub detail: String,
    pub score: f32,
}

impl From<&model::Evidence> for EvidenceInfo {
    fn from(e: &model::Evidence) -> Self {
        match e {
            model::Evidence::VectorReach { depth } => EvidenceInfo {
                kind: EvidenceKind::VectorReach,
                detail: format!("reached from a vector through {depth} calls"),
                score: 1.0,
            },
            model::Evidence::Heuristic { name, score } => EvidenceInfo {
                kind: EvidenceKind::Heuristic,
                detail: name.clone(),
                score: *score,
            },
            model::Evidence::User => EvidenceInfo {
                kind: EvidenceKind::User,
                detail: "marked by the user".into(),
                score: 1.0,
            },
            model::Evidence::Imported(s) => EvidenceInfo {
                kind: EvidenceKind::Imported,
                detail: s.clone(),
                score: 1.0,
            },
            model::Evidence::Trace { file, hits } => EvidenceInfo {
                kind: EvidenceKind::Trace,
                detail: format!("{file} ({hits} hits)"),
                score: 1.0,
            },
            model::Evidence::Observed(what) => EvidenceInfo {
                kind: EvidenceKind::Trace,
                detail: what.clone(),
                score: 1.0,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RegionInfo {
    pub start: u32,
    pub len: u32,
    pub kind: RegionKind,
    pub data_kind: Option<DataKind>,
    pub bpp: Option<u8>,
    pub stride: Option<u8>,
    /// For `Table`: what one element is.
    pub elem: Option<TableElem>,
    /// For `Table` and `Pointer`: which bank an entry's target is in.
    pub bank: Option<BankRule>,
    pub confidence: f32,
    pub evidence: Vec<EvidenceInfo>,
    /// The batch encoding of the kind (0 unknown, 1 code, 2.. data kinds).
    pub kind_code: u8,
    pub name: String,
}

impl From<&model::Region> for RegionInfo {
    fn from(r: &model::Region) -> Self {
        let (kind, data_kind, bpp, stride, elem, bank) = match r.kind {
            model::RegionKind::Unknown => (RegionKind::Unknown, None, None, None, None, None),
            model::RegionKind::Code => (RegionKind::Code, None, None, None, None, None),
            model::RegionKind::Data(d) => (
                RegionKind::Data,
                Some(d.into()),
                match d {
                    model::DataKind::Graphics { bpp } => Some(bpp),
                    _ => None,
                },
                match d {
                    model::DataKind::Table { stride, .. } => Some(stride),
                    _ => None,
                },
                match d {
                    model::DataKind::Table { elem, .. } => Some(elem.into()),
                    _ => None,
                },
                match d {
                    model::DataKind::Pointer { bank } => Some(bank.into()),
                    model::DataKind::Table { elem, .. } => elem.bank().map(Into::into),
                    _ => None,
                },
            ),
        };
        Self {
            start: r.start.0,
            len: r.len,
            kind,
            data_kind,
            bpp,
            stride,
            elem,
            bank,
            confidence: r.confidence,
            evidence: r.evidence.iter().map(EvidenceInfo::from).collect(),
            kind_code: r.kind.code(),
            name: r.kind.name().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LabelSource {
    Auto,
    User,
    Imported,
    Builtin,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LabelInfo {
    pub address: u32,
    pub name: String,
    pub source: LabelSource,
    /// The import's name for imported labels, else empty.
    pub origin: String,
    pub file_offset: Option<u32>,
}

pub fn label_info(rom: &romlens_core::RomImage, l: &model::Label) -> LabelInfo {
    LabelInfo {
        address: l.address.as_u24(),
        name: l.name.clone(),
        source: match &l.source {
            model::LabelSource::Auto => LabelSource::Auto,
            model::LabelSource::User => LabelSource::User,
            model::LabelSource::Imported(_) => LabelSource::Imported,
            model::LabelSource::Builtin => LabelSource::Builtin,
        },
        origin: match &l.source {
            model::LabelSource::Imported(s) => s.clone(),
            _ => String::new(),
        },
        file_offset: rom.file_offset_for(l.address).map(FileOffset::value),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum XRefKind {
    Call,
    Jump,
    Branch,
    Read,
    Write,
    ReadWrite,
    Pointer,
    JumpTable,
    Vector,
}

impl From<model::XRefKind> for XRefKind {
    fn from(k: model::XRefKind) -> Self {
        match k {
            model::XRefKind::Call => XRefKind::Call,
            model::XRefKind::Jump => XRefKind::Jump,
            model::XRefKind::Branch => XRefKind::Branch,
            model::XRefKind::Read => XRefKind::Read,
            model::XRefKind::Write => XRefKind::Write,
            model::XRefKind::ReadWrite => XRefKind::ReadWrite,
            model::XRefKind::Pointer => XRefKind::Pointer,
            model::XRefKind::JumpTable => XRefKind::JumpTable,
            model::XRefKind::Vector => XRefKind::Vector,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct XRefInfo {
    pub from_offset: u32,
    pub from_address: Option<u32>,
    pub to_address: u32,
    pub to_offset: Option<u32>,
    pub kind: XRefKind,
    pub kind_name: String,
    pub certain: bool,
    /// An emulator saw it happen.
    pub observed: bool,
}

pub fn xref_info(rom: &romlens_core::RomImage, x: &model::XRef) -> XRefInfo {
    XRefInfo {
        from_offset: x.from.0,
        from_address: rom.snes_address_for(x.from).map(SnesAddress::as_u24),
        to_address: x.to.as_u24(),
        to_offset: x.to_offset.map(FileOffset::value),
        kind: x.kind.into(),
        kind_name: x.kind.name().to_owned(),
        certain: x.certain,
        observed: x.observed,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CommentKind {
    Line,
    Block,
}

impl From<CommentKind> for model::CommentKind {
    fn from(k: CommentKind) -> Self {
        match k {
            CommentKind::Line => model::CommentKind::Line,
            CommentKind::Block => model::CommentKind::Block,
        }
    }
}

impl From<model::CommentKind> for CommentKind {
    fn from(k: model::CommentKind) -> Self {
        match k {
            model::CommentKind::Line => CommentKind::Line,
            model::CommentKind::Block => CommentKind::Block,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CommentInfo {
    pub address: u32,
    pub kind: CommentKind,
    pub text: String,
}

impl From<&model::Comment> for CommentInfo {
    fn from(c: &model::Comment) -> Self {
        Self {
            address: c.address.as_u24(),
            kind: c.kind.into(),
            text: c.text.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum WarningKind {
    ComputedJump,
    JumpTable,
    FlagConflict,
    UnknownCarryXce,
    SuspiciousEntry,
    SuspiciousFallthrough,
    BankWrap,
    WalkedIntoUserData,
}

/// How a warning reads. A shell sorts and styles by this rather than by kind,
/// so a new kind does not need a shell change to be shown sensibly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Severity {
    Info,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WarningInfo {
    pub file_offset: u32,
    pub kind: WarningKind,
    pub kind_name: String,
    pub severity: Severity,
    pub text: String,
}

impl From<&snapshot::Warning> for WarningInfo {
    fn from(w: &snapshot::Warning) -> Self {
        Self {
            file_offset: w.offset.0,
            kind: match w.kind {
                snapshot::WarningKind::ComputedJump => WarningKind::ComputedJump,
                snapshot::WarningKind::JumpTable => WarningKind::JumpTable,
                snapshot::WarningKind::FlagConflict => WarningKind::FlagConflict,
                snapshot::WarningKind::UnknownCarryXce => WarningKind::UnknownCarryXce,
                snapshot::WarningKind::SuspiciousEntry => WarningKind::SuspiciousEntry,
                snapshot::WarningKind::SuspiciousFallthrough => WarningKind::SuspiciousFallthrough,
                snapshot::WarningKind::BankWrap => WarningKind::BankWrap,
                snapshot::WarningKind::WalkedIntoUserData => WarningKind::WalkedIntoUserData,
            },
            kind_name: w.kind.name().to_owned(),
            severity: match w.kind.severity() {
                snapshot::Severity::Info => Severity::Info,
                snapshot::Severity::Warning => Severity::Warning,
            },
            text: w.text.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Record)]
pub struct AnalysisStats {
    pub code_bytes: u64,
    pub data_bytes: u64,
    pub unknown_bytes: u64,
    pub instructions: u64,
    pub blocks: u64,
    pub regions: u64,
    pub labels: u64,
    pub xrefs: u64,
    pub conflicts: u64,
    pub warnings: u64,
    pub elapsed_ms: u64,
}

impl From<snapshot::AnalysisStats> for AnalysisStats {
    fn from(s: snapshot::AnalysisStats) -> Self {
        Self {
            code_bytes: s.code_bytes,
            data_bytes: s.data_bytes,
            unknown_bytes: s.unknown_bytes,
            instructions: s.instructions,
            blocks: s.blocks,
            regions: s.regions,
            labels: s.labels,
            xrefs: s.xrefs,
            conflicts: s.conflicts,
            warnings: s.warnings,
            elapsed_ms: s.elapsed_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RomIdentityInfo {
    pub sha256: String,
    pub size: u32,
    pub mapping: crate::Mapping,
    pub fast_rom: bool,
    pub title: String,
}

impl From<&model::RomIdentity> for RomIdentityInfo {
    fn from(i: &model::RomIdentity) -> Self {
        Self {
            sha256: i.sha256.clone(),
            size: i.size,
            mapping: i.mapping.into(),
            fast_rom: i.fast_rom,
            title: i.title.clone(),
        }
    }
}

impl From<RomIdentityInfo> for model::RomIdentity {
    fn from(i: RomIdentityInfo) -> Self {
        Self {
            sha256: i.sha256,
            size: i.size,
            mapping: i.mapping.into(),
            fast_rom: i.fast_rom,
            title: i.title,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MemoryClass {
    Rom,
    Wram,
    LowRam,
    Hardware,
    Sram,
    OpenBus,
}

impl From<CoreMemoryClass> for MemoryClass {
    fn from(m: CoreMemoryClass) -> Self {
        match m {
            CoreMemoryClass::Rom => MemoryClass::Rom,
            CoreMemoryClass::Wram => MemoryClass::Wram,
            CoreMemoryClass::LowRam => MemoryClass::LowRam,
            CoreMemoryClass::Hardware => MemoryClass::Hardware,
            CoreMemoryClass::Sram => MemoryClass::Sram,
            CoreMemoryClass::OpenBus => MemoryClass::OpenBus,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ResolvedAny {
    pub snes_address: u32,
    pub file_offset: Option<u32>,
    pub memory_class: MemoryClass,
    pub register: Option<HardwareRegisterInfo>,
    pub mirrors: Vec<u32>,
}

impl From<romlens_core::ResolvedAny> for ResolvedAny {
    fn from(r: romlens_core::ResolvedAny) -> Self {
        Self {
            snes_address: r.snes_address.as_u24(),
            file_offset: r.file_offset.map(FileOffset::value),
            memory_class: r.memory_class.into(),
            register: r.register.map(HardwareRegisterInfo::from),
            mirrors: r.mirrors.iter().map(|a| a.as_u24()).collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ByteRange {
    pub start: u32,
    pub len: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum OverrideKind {
    Code,
    Data,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Record)]
pub struct FlagOverride {
    pub m: Option<bool>,
    pub x: Option<bool>,
    pub e: Option<bool>,
    pub dbr: Option<u8>,
    pub dp: Option<u16>,
}

impl From<FlagOverride> for model::FlagOverride {
    fn from(f: FlagOverride) -> Self {
        Self {
            m: f.m,
            x: f.x,
            e: f.e,
            dbr: f.dbr,
            dp: f.dp,
        }
    }
}

impl From<model::FlagOverride> for FlagOverride {
    fn from(f: model::FlagOverride) -> Self {
        Self {
            m: f.m,
            x: f.x,
            e: f.e,
            dbr: f.dbr,
            dp: f.dp,
        }
    }
}

/// Preview options for a marked range; `None` is the view's default.
/// Addresses are 24-bit SNES addresses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Record)]
pub struct RegionParamsInfo {
    pub palette: Option<u32>,
    pub columns: Option<u16>,
    pub screen_size: Option<crate::graphics::ScreenSize>,
    pub tiles: Option<u32>,
}

impl From<RegionParamsInfo> for model::RegionParams {
    fn from(p: RegionParamsInfo) -> Self {
        model::RegionParams {
            palette: p.palette.map(SnesAddress::from_u24),
            columns: p.columns,
            screen_size: p.screen_size.map(Into::into),
            tiles: p.tiles.map(SnesAddress::from_u24),
        }
    }
}

impl From<model::RegionParams> for RegionParamsInfo {
    fn from(p: model::RegionParams) -> Self {
        RegionParamsInfo {
            palette: p.palette.map(|a| a.as_u24()),
            columns: p.columns,
            screen_size: p.screen_size.map(Into::into),
            tiles: p.tiles.map(|a| a.as_u24()),
        }
    }
}

/// An edit. Addresses are 24-bit SNES addresses; ranges are file offsets.
/// The width of one element of a variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VarWidth {
    Byte,
    Word,
    Long,
}

impl From<VarWidth> for model::VarWidth {
    fn from(w: VarWidth) -> Self {
        match w {
            VarWidth::Byte => model::VarWidth::Byte,
            VarWidth::Word => model::VarWidth::Word,
            VarWidth::Long => model::VarWidth::Long,
        }
    }
}

impl From<model::VarWidth> for VarWidth {
    fn from(w: model::VarWidth) -> Self {
        match w {
            model::VarWidth::Byte => VarWidth::Byte,
            model::VarWidth::Word => VarWidth::Word,
            model::VarWidth::Long => VarWidth::Long,
        }
    }
}

/// What a variable holds: `count` elements of `width`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct VarTypeInfo {
    pub width: VarWidth,
    pub count: u16,
}

impl From<VarTypeInfo> for model::VarType {
    fn from(t: VarTypeInfo) -> Self {
        model::VarType {
            width: t.width.into(),
            count: t.count,
        }
    }
}

/// A variable: a named address and its type.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VariableInfo {
    /// Canonical: WRAM's own address for a low-RAM mirror.
    pub address: u32,
    /// Empty when the label was removed but the type kept.
    pub name: String,
    pub width: VarWidth,
    pub count: u16,
    /// Bytes spanned.
    pub len: u32,
    /// `word`, `byte[16]`.
    pub description: String,
    /// Where it is, in words: WRAM, SRAM, a register, ROM.
    pub memory: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Command {
    SetLabel {
        address: u32,
        name: Option<String>,
    },
    SetComment {
        address: u32,
        kind: CommentKind,
        text: Option<String>,
    },
    MarkRegion {
        start: u32,
        len: u32,
        kind: OverrideKind,
        data_kind: Option<DataKind>,
        stride: Option<u8>,
        bpp: Option<u8>,
        /// For `Table`: what one element is. `Raw` when omitted.
        elem: Option<TableElem>,
        /// For `Table` and `Pointer`: which bank an entry's target is in.
        /// `SameBank` when omitted.
        bank: Option<BankRule>,
    },
    ClearRegionOverride {
        start: u32,
        len: u32,
    },
    /// How the marked range starting at `start` previews.
    SetRegionParams {
        start: u32,
        params: RegionParamsInfo,
    },
    SetFlagOverride {
        offset: u32,
        flags: Option<FlagOverride>,
    },
    /// The type of the variable at `address`; `None` removes it.
    SetVariable {
        address: u32,
        ty: Option<VarTypeInfo>,
    },
}

impl From<Command> for model::Command {
    fn from(c: Command) -> Self {
        match c {
            Command::SetLabel { address, name } => model::Command::SetLabel {
                address: SnesAddress::from_u24(address),
                name,
            },
            Command::SetComment {
                address,
                kind,
                text,
            } => model::Command::SetComment {
                address: SnesAddress::from_u24(address),
                kind: kind.into(),
                text,
            },
            Command::MarkRegion {
                start,
                len,
                kind,
                data_kind,
                stride,
                bpp,
                elem,
                bank,
            } => model::Command::MarkRegion {
                start: FileOffset(start),
                len,
                kind: match kind {
                    OverrideKind::Code => model::OverrideKind::Code,
                    OverrideKind::Unknown => model::OverrideKind::Unknown,
                    OverrideKind::Data => model::OverrideKind::Data(core_data_kind(
                        data_kind.unwrap_or(DataKind::Byte),
                        stride,
                        bpp,
                        elem,
                        bank,
                    )),
                },
            },
            Command::ClearRegionOverride { start, len } => model::Command::ClearRegionOverride {
                start: FileOffset(start),
                len,
            },
            Command::SetRegionParams { start, params } => model::Command::SetRegionParams {
                start: FileOffset(start),
                params: params.into(),
            },
            Command::SetFlagOverride { offset, flags } => model::Command::SetFlagOverride {
                offset: FileOffset(offset),
                flags: flags.map(Into::into),
            },
            Command::SetVariable { address, ty } => model::Command::SetVariable {
                address: SnesAddress::from_u24(address),
                ty: ty.map(Into::into),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AnalysisPhase {
    Descent,
    Tables,
    Sweep,
    Heuristics,
    Labels,
    Lines,
}

impl From<analysis::AnalysisPhase> for AnalysisPhase {
    fn from(p: analysis::AnalysisPhase) -> Self {
        match p {
            analysis::AnalysisPhase::Descent => AnalysisPhase::Descent,
            analysis::AnalysisPhase::Tables => AnalysisPhase::Tables,
            analysis::AnalysisPhase::Heuristics => AnalysisPhase::Heuristics,
            analysis::AnalysisPhase::Sweep => AnalysisPhase::Sweep,
            analysis::AnalysisPhase::Labels => AnalysisPhase::Labels,
            analysis::AnalysisPhase::Lines => AnalysisPhase::Lines,
        }
    }
}

/// What a `WorkbenchListener` receives. Events may arrive on any thread.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum WorkbenchEvent {
    SnapshotChanged {
        analysis_generation: u64,
    },
    ViewChanged {
        view_generation: u64,
    },
    ProjectChanged {
        dirty: bool,
    },
    AnalysisProgress {
        phase: AnalysisPhase,
        done: u64,
        total: u64,
    },
}

/// How far the decompiler goes (`docs/18-decompiler.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DecompileLevel {
    Lift,
    Clean,
    Full,
}

impl From<DecompileLevel> for romlens_core::decompile::Level {
    fn from(l: DecompileLevel) -> Self {
        use romlens_core::decompile::Level;
        match l {
            DecompileLevel::Lift => Level::Lift,
            DecompileLevel::Clean => Level::Clean,
            DecompileLevel::Full => Level::Full,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CTokenKind {
    Keyword,
    Type,
    Number,
    Comment,
    Function,
    Variable,
    Register,
    Label,
    Helper,
    Local,
    GotoLabel,
}

impl From<romlens_core::decompile::CTokenKind> for CTokenKind {
    fn from(k: romlens_core::decompile::CTokenKind) -> Self {
        use romlens_core::decompile::CTokenKind as K;
        match k {
            K::Keyword => CTokenKind::Keyword,
            K::Type => CTokenKind::Type,
            K::Number => CTokenKind::Number,
            K::Comment => CTokenKind::Comment,
            K::Function => CTokenKind::Function,
            K::Variable => CTokenKind::Variable,
            K::Register => CTokenKind::Register,
            K::Label => CTokenKind::Label,
            K::Helper => CTokenKind::Helper,
            K::Local => CTokenKind::Local,
            K::GotoLabel => CTokenKind::GotoLabel,
        }
    }
}

/// A token of the C, in UTF-16 units so a shell's string APIs can use it
/// directly.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CTokenInfo {
    pub start: u32,
    pub len: u32,
    pub kind: CTokenKind,
    /// The routine, variable, register or label it names.
    pub address: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DecompiledInfo {
    pub name: String,
    pub entry: u32,
    pub text: String,
    pub tokens: Vec<CTokenInfo>,
    /// For each line of `text`, the file offsets of the instructions it came
    /// from.
    pub lines: Vec<Vec<u32>>,
    pub warnings: Vec<String>,
    pub instructions: u32,
    pub statements: u32,
    pub gotos: u32,
    pub asm_comments: u32,
}

impl From<romlens_core::decompile::Decompiled> for DecompiledInfo {
    fn from(d: romlens_core::decompile::Decompiled) -> Self {
        // Byte offsets to UTF-16 offsets.
        let mut utf16_at = Vec::with_capacity(d.text.len() + 1);
        let mut n = 0u32;
        for c in d.text.chars() {
            for _ in 0..c.len_utf8() {
                utf16_at.push(n);
            }
            n += c.len_utf16() as u32;
        }
        utf16_at.push(n);
        let tokens = d
            .tokens
            .iter()
            .map(|t| {
                let start = utf16_at[t.start as usize];
                let end = utf16_at[(t.start + t.len) as usize];
                CTokenInfo {
                    start,
                    len: end - start,
                    kind: t.kind.into(),
                    address: t.address.map(|a| a.as_u24()),
                }
            })
            .collect();
        DecompiledInfo {
            name: d.name,
            entry: d.entry.as_u24(),
            tokens,
            lines: d
                .lines
                .iter()
                .map(|l| l.iter().map(|o| o.0).collect())
                .collect(),
            warnings: d.warnings,
            instructions: d.stats.instructions,
            statements: d.stats.statements,
            gotos: d.stats.gotos,
            asm_comments: d.stats.asm_comments,
            text: d.text,
        }
    }
}
