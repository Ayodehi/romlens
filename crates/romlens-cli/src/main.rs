//! `romlens`: the core's capabilities from the command line, so every shell
//! feature has a scriptable twin (docs/08 rule 7) and CI can pin output on
//! all three operating systems.

mod commands;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use romlens_core::analysis::AnalysisOptions;
use romlens_core::{AddressStyle, MappingMode};

use commands::labels::SourceFilter;

fn long_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        format!(
            "{} (core API {})",
            env!("CARGO_PKG_VERSION"),
            romlens_core::API_VERSION
        )
    })
}

#[derive(Parser)]
#[command(name = "romlens", about = "SNES ROM study tool, command line", version = long_version())]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Identify a ROM: mapping, header fields, vectors, checksum, SHA-256.
    Info {
        rom: PathBuf,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Print hex rows.
    Hex {
        rom: PathBuf,
        /// Start address: `$80:841C`, `80841C` or `0x41C` (default: start of file).
        #[arg(long)]
        from: Option<String>,
        /// Number of 16-byte rows.
        #[arg(long, default_value_t = 16)]
        rows: u32,
        /// Which address column(s) to print.
        #[arg(long, value_enum, default_value_t = AddressArg::Both)]
        address: AddressArg,
    },
    /// Resolve an address expression to a file offset and a CPU address.
    Resolve { rom: PathBuf, expr: String },
    /// Write one of the homebrew test ROMs the test suite assembles.
    Testrom {
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_enum, default_value_t = MappingArg::Lorom)]
        mapping: MappingArg,
        /// Which fixture; the default is the minimal ROM for `--mapping`.
        #[arg(long, value_enum, default_value_t = FixtureArg::Minimal)]
        fixture: FixtureArg,
        /// The 32 KB LoROM holding all 256 opcodes in order (`--fixture
        /// all-opcodes`).
        #[arg(long)]
        all_opcodes: bool,
    },
    /// Disassemble: the analyzed listing, or a raw linear decode with --flags.
    Disasm {
        rom: PathBuf,
        /// Start address (default: the first line).
        #[arg(long)]
        from: Option<String>,
        /// Lines (or instructions with --flags).
        #[arg(long, default_value_t = 32)]
        count: u32,
        /// A .romlens package to apply.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Decode linearly under these flags (e.g. m1x0e0), ignoring the analysis.
        #[arg(long)]
        flags: Option<String>,
        #[arg(long, value_enum, default_value_t = AddressArg::Both)]
        address: AddressArg,
        /// Add the flag state column.
        #[arg(long)]
        verbose: bool,
    },
    /// Run the analyzer and report code/data/unknown coverage.
    Analyze {
        rom: PathBuf,
        #[arg(long)]
        project: Option<PathBuf>,
        /// Print the statistics (the default; kept for scripts).
        #[arg(long)]
        stats: bool,
        #[arg(long)]
        json: bool,
        /// Report phases on stderr.
        #[arg(long)]
        progress: bool,
        /// Also list the analyzer's warnings.
        #[arg(long)]
        warnings: bool,
        /// Leave `JMP`/`JSR (abs,X)` dispatch tables unresolved, to measure
        /// what resolving them is worth.
        #[arg(long)]
        no_tables: bool,
        /// Leave unclassified bytes unscored, likewise.
        #[arg(long)]
        no_heuristics: bool,
    },
    /// Read what another tool knows about a ROM into a project.
    Import {
        #[command(subcommand)]
        what: ImportCommand,
    },
    /// Build a ground-truth file for `romlens accuracy`.
    Truth {
        #[command(subcommand)]
        what: TruthCommand,
    },
    /// Score the classifier against ground truth.
    Accuracy {
        rom: PathBuf,
        #[arg(long)]
        project: Option<PathBuf>,
        /// A ground-truth file: `start<TAB>end<TAB>kind`, offsets hex, end
        /// exclusive, with an optional `sha256=` line.
        #[arg(long)]
        truth: Option<PathBuf>,
        /// Use the truth built into the fixture builder instead of a file.
        #[arg(long)]
        fixture: bool,
        #[arg(long)]
        json: bool,
    },
    /// List the analyzer's scored guesses about unclassified bytes.
    Heuristics {
        rom: PathBuf,
        #[arg(long)]
        project: Option<PathBuf>,
        /// Only this heuristic (`entropy`, `pointers`, `ascii`, `palette`,
        /// `graphics`).
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// The whole-ROM overview the shell's strip draws.
    Map {
        rom: PathBuf,
        #[arg(long)]
        project: Option<PathBuf>,
        /// Columns across the whole image.
        #[arg(long, default_value_t = 256)]
        buckets: u32,
        /// Columns per printed row.
        #[arg(long, default_value_t = 64)]
        width: u32,
        #[arg(long)]
        json: bool,
    },
    /// List the dispatch tables the analyzer resolved.
    Tables {
        rom: PathBuf,
        #[arg(long)]
        project: Option<PathBuf>,
        #[arg(long)]
        json: bool,
        /// List each table's entries, not just one line per table.
        #[arg(long)]
        entries: bool,
        /// Also list the dispatch sites that could not be resolved.
        #[arg(long)]
        unresolved: bool,
    },
    /// List labels.
    Labels {
        rom: PathBuf,
        #[arg(long)]
        project: Option<PathBuf>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, value_enum, default_value_t = SourceArg::All)]
        source: SourceArg,
        #[arg(long)]
        count: Option<u32>,
    },
    /// References to and from an address.
    Xrefs {
        rom: PathBuf,
        expr: String,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Export an assembly listing or a symbol file.
    Export {
        #[command(subcommand)]
        what: ExportCommand,
    },
    /// Everything known about one address.
    Inspect {
        rom: PathBuf,
        expr: String,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Find a byte pattern such as "78 18 ?? 5C".
    Search {
        rom: PathBuf,
        pattern: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value_t = 100)]
        max: u32,
        /// Match the pattern as text rather than as hex bytes.
        #[arg(long)]
        text: bool,
        /// Match text in either case. Implies `--text`.
        #[arg(long)]
        ignore_case: bool,
    },
    /// Create or edit a .romlens package.
    Project {
        /// The package directory.
        path: PathBuf,
        #[command(subcommand)]
        action: ProjectCommand,
    },
    /// The built-in hardware register names.
    Registers { address: Option<String> },
}

#[derive(Subcommand)]
enum ImportCommand {
    /// A Mesen2 CDL or a bsnes-plus usage map.
    Trace {
        /// The `.romlens` package to import into.
        project: PathBuf,
        /// The ROM, when it is not beside the package.
        #[arg(long)]
        rom: Option<PathBuf>,
        file: PathBuf,
        /// `cdl` or `usage`; detected from the file when omitted.
        #[arg(long)]
        format: Option<String>,
    },
    /// A WLA-DX / bsnes-plus `.sym`, a no$sns `.sym` or a VICE `.lbl`.
    Symbols {
        /// The `.romlens` package to import into.
        project: PathBuf,
        /// The ROM, when it is not beside the package.
        #[arg(long)]
        rom: Option<PathBuf>,
        file: PathBuf,
        /// `wla`, `nocash` or `lbl`; detected from the file when omitted.
        #[arg(long)]
        format: Option<String>,
        /// The name the labels are attributed to; the file name by default.
        #[arg(long)]
        source: Option<String>,
    },
}

#[derive(Subcommand)]
enum TruthCommand {
    /// From a trace: executed bytes are code, bytes only ever read are data,
    /// and bytes the session never touched stay unlabelled.
    FromCdl {
        rom: PathBuf,
        file: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Subcommand)]
enum ExportCommand {
    /// asar-syntax listing with explicit operand widths.
    Asm {
        rom: PathBuf,
        /// Output file, or - for stdout.
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        project: Option<PathBuf>,
        /// start..end address expressions (end exclusive).
        #[arg(long)]
        range: Option<String>,
    },
    /// bsnes-plus style symbol file (labels and comments only).
    Sym {
        rom: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        project: Option<PathBuf>,
        /// Also write the analyzer's automatic labels.
        #[arg(long)]
        include_auto: bool,
    },
}

#[derive(Subcommand)]
enum ProjectCommand {
    /// Create an empty package for a ROM.
    Init {
        #[arg(long)]
        rom: PathBuf,
    },
    /// Set (or remove with `-`) the label at an address.
    Label {
        expr: String,
        name: String,
        #[arg(long)]
        rom: Option<PathBuf>,
    },
    /// Set (or remove with `-`) a comment.
    Comment {
        expr: String,
        text: String,
        #[arg(long, conflicts_with = "block")]
        line: bool,
        #[arg(long)]
        block: bool,
        #[arg(long)]
        rom: Option<PathBuf>,
    },
    /// Mark a range as code, a data kind or unknown.
    Mark {
        expr: String,
        len: u32,
        kind: String,
        #[arg(long)]
        rom: Option<PathBuf>,
        /// For `table`: bytes per entry.
        #[arg(long)]
        stride: Option<u8>,
        /// For `graphics`: bitplanes.
        #[arg(long)]
        bpp: Option<u8>,
        /// For `table`: what one entry is — `raw`, `pointer` or `code`.
        #[arg(long)]
        elem: Option<String>,
        /// For `table` and `pointer`: which bank an entry's target is in —
        /// `same` (the default), `entry`, or a bank such as `$C0`.
        #[arg(long)]
        bank: Option<String>,
    },
    /// Remove marks from a range.
    Clear {
        expr: String,
        len: u32,
        #[arg(long)]
        rom: Option<PathBuf>,
    },
    /// Pin M/X/E, the data bank or the direct page at an address.
    Flags {
        expr: String,
        #[arg(long)]
        m: Option<u8>,
        #[arg(long)]
        x: Option<u8>,
        #[arg(long)]
        e: Option<u8>,
        #[arg(long)]
        dbr: Option<String>,
        #[arg(long)]
        dp: Option<String>,
        #[arg(long)]
        remove: bool,
        #[arg(long)]
        rom: Option<PathBuf>,
    },
    /// List what the package holds.
    History {
        #[arg(long)]
        rom: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum AddressArg {
    Both,
    Snes,
    File,
}

impl From<AddressArg> for AddressStyle {
    fn from(a: AddressArg) -> Self {
        match a {
            AddressArg::Both => AddressStyle::Both,
            AddressArg::Snes => AddressStyle::Snes,
            AddressArg::File => AddressStyle::File,
        }
    }
}

/// Which homebrew fixture `testrom` writes.
#[derive(Clone, Copy, ValueEnum)]
enum FixtureArg {
    /// The minimal ROM for `--mapping`.
    Minimal,
    /// 32 KB LoROM with the 256 opcodes in order.
    AllOpcodes,
    /// 32 KB LoROM whose boot dispatches through two jump tables.
    Dispatch,
    /// 64 KB LoROM with one block per data heuristic.
    MixedData,
}

impl From<FixtureArg> for commands::rom::Fixture {
    fn from(f: FixtureArg) -> Self {
        match f {
            FixtureArg::Minimal => commands::rom::Fixture::Minimal,
            FixtureArg::AllOpcodes => commands::rom::Fixture::AllOpcodes,
            FixtureArg::Dispatch => commands::rom::Fixture::Dispatch,
            FixtureArg::MixedData => commands::rom::Fixture::MixedData,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum MappingArg {
    Lorom,
    Hirom,
    Exhirom,
}

impl From<MappingArg> for MappingMode {
    fn from(m: MappingArg) -> Self {
        match m {
            MappingArg::Lorom => MappingMode::LoRom,
            MappingArg::Hirom => MappingMode::HiRom,
            MappingArg::Exhirom => MappingMode::ExHiRom,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum SourceArg {
    Auto,
    User,
    Imported,
    All,
}

impl From<SourceArg> for SourceFilter {
    fn from(s: SourceArg) -> Self {
        match s {
            SourceArg::Auto => SourceFilter::Auto,
            SourceArg::User => SourceFilter::User,
            SourceArg::Imported => SourceFilter::Imported,
            SourceArg::All => SourceFilter::All,
        }
    }
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Info { rom, json } => commands::rom::info(&rom, json),
        Command::Hex {
            rom,
            from,
            rows,
            address,
        } => commands::rom::hex(&rom, from.as_deref(), rows, address.into()),
        Command::Resolve { rom, expr } => commands::rom::resolve(&rom, &expr),
        Command::Testrom {
            out,
            mapping,
            fixture,
            all_opcodes,
        } => commands::rom::testrom(
            &out,
            mapping.into(),
            if all_opcodes {
                commands::rom::Fixture::AllOpcodes
            } else {
                fixture.into()
            },
        ),
        Command::Disasm {
            rom,
            from,
            count,
            project,
            flags,
            address,
            verbose,
        } => commands::disasm::run(commands::disasm::DisasmArgs {
            rom: &rom,
            from: from.as_deref(),
            count,
            project: project.as_deref(),
            flags: flags.as_deref(),
            style: address.into(),
            verbose,
        }),
        Command::Analyze {
            rom,
            project,
            stats: _,
            json,
            progress,
            warnings,
            no_tables,
            no_heuristics,
        } => commands::analyze::run(
            &rom,
            project.as_deref(),
            json,
            progress,
            warnings,
            AnalysisOptions {
                jump_tables: !no_tables,
                heuristics: !no_heuristics,
            },
        ),
        Command::Import { what } => match what {
            ImportCommand::Trace {
                project,
                rom,
                file,
                format,
            } => commands::import::trace(&project, rom.as_deref(), &file, format.as_deref()),
            ImportCommand::Symbols {
                project,
                rom,
                file,
                format,
                source,
            } => commands::import::symbols(
                &project,
                rom.as_deref(),
                &file,
                format.as_deref(),
                source.as_deref(),
            ),
        },
        Command::Truth { what } => match what {
            TruthCommand::FromCdl { rom, file, out } => {
                commands::truth::from_cdl(&rom, &file, &out)
            }
        },
        Command::Accuracy {
            rom,
            project,
            truth,
            fixture,
            json,
        } => commands::accuracy::run(&rom, project.as_deref(), truth.as_ref(), fixture, json),
        Command::Heuristics {
            rom,
            project,
            kind,
            from,
            to,
            json,
        } => commands::heuristics::run(
            &rom,
            project.as_deref(),
            kind.as_deref(),
            from.as_deref(),
            to.as_deref(),
            json,
        ),
        Command::Map {
            rom,
            project,
            buckets,
            width,
            json,
        } => commands::map::run(&rom, project.as_deref(), buckets, width, json),
        Command::Tables {
            rom,
            project,
            json,
            entries,
            unresolved,
        } => commands::tables::run(&rom, project.as_deref(), json, entries, unresolved),
        Command::Labels {
            rom,
            project,
            from,
            to,
            source,
            count,
        } => commands::labels::run(commands::labels::LabelsArgs {
            rom: &rom,
            project: project.as_deref(),
            from: from.as_deref(),
            to: to.as_deref(),
            source: source.into(),
            count,
        }),
        Command::Xrefs { rom, expr, project } => {
            commands::xrefs::run(&rom, &expr, project.as_deref())
        }
        Command::Export { what } => match what {
            ExportCommand::Asm {
                rom,
                out,
                project,
                range,
            } => commands::export::asm(&rom, &out, project.as_deref(), range.as_deref()),
            ExportCommand::Sym {
                rom,
                out,
                project,
                include_auto,
            } => commands::export::sym(&rom, &out, project.as_deref(), include_auto),
        },
        Command::Inspect { rom, expr, project } => {
            commands::inspect::run(&rom, &expr, project.as_deref())
        }
        Command::Search {
            rom,
            pattern,
            from,
            to,
            max,
            text,
            ignore_case,
        } => commands::search::run(commands::search::SearchArgs {
            rom: &rom,
            pattern: &pattern,
            from: from.as_deref(),
            to: to.as_deref(),
            max,
            text,
            ignore_case,
        }),
        Command::Project { path, action } => match action {
            ProjectCommand::Init { rom } => commands::project::init(&path, &rom),
            ProjectCommand::Label { expr, name, rom } => {
                commands::project::label(&path, rom.as_deref(), &expr, &name)
            }
            ProjectCommand::Comment {
                expr,
                text,
                line: _,
                block,
                rom,
            } => commands::project::comment(&path, rom.as_deref(), &expr, block, &text),
            ProjectCommand::Mark {
                expr,
                len,
                kind,
                rom,
                stride,
                bpp,
                elem,
                bank,
            } => commands::project::mark(commands::project::MarkArgs {
                dir: &path,
                rom: rom.as_deref(),
                expr: &expr,
                len,
                kind: &kind,
                stride,
                bpp,
                elem: elem.as_deref(),
                bank: bank.as_deref(),
            }),
            ProjectCommand::Clear { expr, len, rom } => {
                commands::project::clear(&path, rom.as_deref(), &expr, len)
            }
            ProjectCommand::Flags {
                expr,
                m,
                x,
                e,
                dbr,
                dp,
                remove,
                rom,
            } => commands::project::flags(
                &path,
                rom.as_deref(),
                &expr,
                commands::project::FlagArgs {
                    m,
                    x,
                    e,
                    dbr: dbr.as_deref(),
                    dp: dp.as_deref(),
                    remove,
                },
            ),
            ProjectCommand::History { rom } => commands::project::history(&path, rom.as_deref()),
        },
        Command::Registers { address } => commands::registers::run(address.as_deref()),
    }
}
