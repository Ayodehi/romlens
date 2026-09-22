//! `romlens`: the core's capabilities from the command line, so every shell
//! feature has a scriptable twin (docs/08 rule 7) and CI can pin output on
//! all three operating systems.

mod commands;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
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
        /// The 32 KB LoROM holding all 256 opcodes in order.
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
            all_opcodes,
        } => commands::rom::testrom(&out, mapping.into(), all_opcodes),
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
        } => commands::analyze::run(&rom, project.as_deref(), json, progress, warnings),
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
        } => commands::search::run(&rom, &pattern, from.as_deref(), to.as_deref(), max),
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
            } => commands::project::mark(&path, rom.as_deref(), &expr, len, &kind),
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
