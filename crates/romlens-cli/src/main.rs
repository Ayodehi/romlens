//! `romlens`: the core's capabilities from the command line, so every shell
//! feature has a scriptable twin (docs/08 rule 7) and CI can pin output on
//! all three operating systems.

mod commands;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use romlens_core::analysis::AnalysisOptions;
use romlens_core::recording::mesen::{PackOptions, WramMode};
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
    /// A routine's control-flow graph, or its callers and callees
    /// (docs/19).
    Graph {
        rom: PathBuf,
        /// The routine's entry.
        expr: String,
        #[arg(long)]
        project: Option<PathBuf>,
        /// Callers and callees instead of blocks.
        #[arg(long)]
        calls: bool,
        /// Graphviz DOT (`dot -Tsvg`).
        #[arg(long, conflicts_with = "json")]
        dot: bool,
        /// JSON, with a layout in character cells.
        #[arg(long)]
        json: bool,
    },
    /// Pseudo-C for the routine entered at an address (docs/18).
    Decompile {
        /// The ROM (not needed with --header alone).
        rom: Option<PathBuf>,
        /// The routine's entry.
        expr: Option<String>,
        #[arg(long)]
        project: Option<PathBuf>,
        /// lift, clean or full.
        #[arg(long, default_value = "full")]
        level: String,
        #[arg(long)]
        json: bool,
        /// Write snes.h, which every result includes.
        #[arg(long)]
        header: Option<PathBuf>,
        /// Every routine, with counts.
        #[arg(long)]
        all: bool,
        /// With --all: check each result with the C compiler.
        #[arg(long)]
        check: bool,
        /// Every memory access as MEM8(...), without the project's names.
        #[arg(long)]
        no_names: bool,
        /// The direct page where the analysis does not know it (hex).
        #[arg(long)]
        assume_dp: Option<String>,
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
    /// Decode bytes as 8×8 tiles: the index grid, the planes, or a picture.
    Tiles {
        rom: PathBuf,
        #[arg(long)]
        from: String,
        /// 2, 4 or 8, or 7 for Mode 7's one byte per pixel.
        #[arg(long, default_value_t = 4)]
        bpp: u8,
        #[arg(long, default_value_t = 1)]
        count: u32,
        /// Tiles per row.
        #[arg(long, default_value_t = 16)]
        columns: u32,
        /// Colours from BGR15 entries at this address (for --ascii and
        /// --digest); grayscale when absent.
        #[arg(long)]
        palette: Option<String>,
        /// The index grid, and the planes for a single tile (the default).
        #[arg(long, conflicts_with_all = ["json", "ascii", "digest"])]
        text: bool,
        #[arg(long, conflicts_with_all = ["ascii", "digest"])]
        json: bool,
        /// The coloured sheet as characters, darkest to brightest.
        #[arg(long, conflicts_with = "digest")]
        ascii: bool,
        /// The SHA-256 of the coloured sheet: what a golden pins.
        #[arg(long)]
        digest: bool,
    },
    /// Decode BGR15 colours.
    Palette {
        rom: PathBuf,
        #[arg(long)]
        from: String,
        #[arg(long, default_value_t = 16)]
        count: u16,
        #[arg(long)]
        json: bool,
    },
    /// Decode a 544-byte sprite table.
    Oam {
        rom: PathBuf,
        #[arg(long)]
        from: String,
        /// The OBSEL value that sizes the sprites, e.g. $60.
        #[arg(long, default_value = "0")]
        obsel: String,
        /// table, screen or priority.
        #[arg(long, default_value = "table")]
        sort: String,
        /// Leave out sprites parked off screen.
        #[arg(long)]
        visible: bool,
        #[arg(long)]
        json: bool,
    },
    /// Decode a BG tilemap.
    Tilemap {
        rom: PathBuf,
        #[arg(long)]
        from: String,
        /// 32x32, 64x32, 32x64 or 64x64.
        #[arg(long, default_value = "32x32")]
        size: String,
        #[arg(long)]
        json: bool,
    },
    /// Write the synthetic recording: the graphics test ROM's machine,
    /// animated, so every recording command works with no emulator.
    Testrec {
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 60)]
        frames: u32,
        /// Frames between keyframes.
        #[arg(long, default_value_t = 60)]
        keyframe_interval: u16,
    },
    /// Read a `.romrec` recording.
    Rec {
        #[command(subcommand)]
        what: RecCommand,
    },
    /// Draw from a recording with the bounded reference renderer: one BG
    /// layer, or one sprite.
    Render {
        #[command(subcommand)]
        what: RenderCommand,
    },
    /// Decompress a block.
    Decompress {
        rom: PathBuf,
        #[arg(long)]
        from: String,
        /// `sm`: Super Metroid's format.
        #[arg(long, default_value = "sm")]
        format: String,
        /// Count the chunks by command.
        #[arg(long)]
        stats: bool,
        /// Write the decompressed bytes to this file.
        #[arg(long)]
        out: Option<PathBuf>,
    },
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
enum RecCommand {
    /// What a recording holds.
    Info {
        rec: PathBuf,
        /// Check it against this ROM.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// Rebuild the index by scanning, for a file with no footer.
        #[arg(long)]
        recover: bool,
    },
    /// Every problem with a recording, each with a stable code.
    Validate {
        rec: PathBuf,
        /// Check it was made from this ROM.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// Rebuild this many frames and check their changes against the truth.
        #[arg(long, default_value_t = 16)]
        sample: u32,
        /// Fail on warnings too.
        #[arg(long)]
        strict: bool,
        /// Treat a missing footer as a recording in progress.
        #[arg(long)]
        recover: bool,
    },
    /// Build (or refresh) the index beside a recording that answers `when`.
    Index {
        rec: PathBuf,
        /// Build it even if the saved one matches.
        #[arg(long)]
        rebuild: bool,
    },
    /// The next frame that changes a byte range, or with --backward the last.
    When {
        rec: PathBuf,
        /// cpu, ppu, io, vram, cgram, oam, timing, or wram when every frame
        /// carries it.
        #[arg(long)]
        region: String,
        /// A byte offset in the region; 0x for hex.
        #[arg(long)]
        offset: String,
        #[arg(long, default_value_t = 1)]
        len: u32,
        /// Search after this frame (backward: at or before it).
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long)]
        backward: bool,
    },
    /// The byte ranges of a region that may differ between two frames.
    Changes {
        rec: PathBuf,
        #[arg(long)]
        from: u64,
        #[arg(long)]
        to: u64,
        #[arg(long)]
        region: String,
    },
    /// One region at one frame.
    Extract {
        rec: PathBuf,
        #[arg(long)]
        frame: u64,
        /// cpu, ppu, io, wram, vram, cgram, oam or timing.
        #[arg(long)]
        region: String,
        #[arg(long)]
        out: Option<PathBuf>,
        /// Print the bytes, leaving out all-zero rows of a large region.
        #[arg(long)]
        hex: bool,
    },
    /// A one-frame recording from loose memory dumps.
    ImportRaw {
        /// The ROM the dumps were taken while running.
        #[arg(long)]
        rom: PathBuf,
        #[arg(long)]
        vram: Option<PathBuf>,
        #[arg(long)]
        cgram: Option<PathBuf>,
        /// 544 bytes, or the 512-byte low table alone.
        #[arg(long)]
        oam: Option<PathBuf>,
        #[arg(long)]
        wram: Option<PathBuf>,
        /// A 256-byte PPU register block in the docs/13 layout.
        #[arg(long)]
        ppu: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// A recording from the stream the Mesen recorder script wrote.
    Pack {
        stream: PathBuf,
        /// The ROM that was running; the stream is checked against it.
        #[arg(long)]
        rom: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// How much WRAM to keep.
        #[arg(long, value_enum, default_value = "keyframe")]
        wram: WramArg,
        #[arg(long, default_value_t = 60)]
        keyframe_interval: u16,
        /// Store payloads uncompressed.
        #[arg(long)]
        no_compress: bool,
    },
    /// A new recording of frames FROM to TO of one, numbered from 0.
    Convert {
        rec: PathBuf,
        #[arg(long)]
        from: u64,
        /// The last frame kept, inclusive.
        #[arg(long)]
        to: u64,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 60)]
        keyframe_interval: u16,
    },
    /// Write the Mesen recorder script.
    Script {
        #[arg(long)]
        out: PathBuf,
    },
    /// Listen for the recorder script's live stream and report what arrives.
    Live {
        #[arg(long)]
        rom: PathBuf,
        /// 0 for any free port, printed on start.
        #[arg(long, default_value_t = romlens_core::recording::live::DEFAULT_PORT)]
        port: u16,
        /// Stop after this many frames.
        #[arg(long)]
        frames: Option<u64>,
        /// Stop when the first connection ends.
        #[arg(long)]
        once: bool,
        /// On exit, write the last frame's vram.bin, cgram.bin and oam.bin
        /// to this directory.
        #[arg(long)]
        dump: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum WramArg {
    Full,
    Keyframe,
    Off,
}

impl From<WramArg> for WramMode {
    fn from(a: WramArg) -> Self {
        match a {
            WramArg::Full => WramMode::Full,
            WramArg::Keyframe => WramMode::Keyframe,
            WramArg::Off => WramMode::Off,
        }
    }
}

#[derive(Subcommand)]
enum RenderCommand {
    /// One background layer's whole map.
    Bg {
        #[arg(long)]
        rec: PathBuf,
        #[arg(long, default_value_t = 0)]
        frame: u64,
        /// 1 to 4.
        #[arg(long)]
        bg: u8,
        #[arg(long)]
        ascii: bool,
        /// The SHA-256 of the image (the default).
        #[arg(long)]
        digest: bool,
    },
    /// One sprite at its own size.
    Sprite {
        #[arg(long)]
        rec: PathBuf,
        #[arg(long, default_value_t = 0)]
        frame: u64,
        /// 0 to 127.
        #[arg(long)]
        index: u8,
        #[arg(long)]
        ascii: bool,
        #[arg(long)]
        digest: bool,
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
    /// How the marked range starting here previews; options not given go
    /// back to the defaults.
    Preview {
        expr: String,
        /// A palette in ROM (BGR15) to draw graphics in.
        #[arg(long)]
        palette: Option<String>,
        /// Tiles across.
        #[arg(long)]
        columns: Option<u16>,
        /// For a tilemap: 32x32, 64x32, 32x64 or 64x64.
        #[arg(long)]
        size: Option<String>,
        /// For a tilemap: the 4 bpp tiles in ROM to draw it with.
        #[arg(long)]
        tiles: Option<String>,
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
    /// The recordings the project refers to (never copies).
    Recordings {
        #[command(subcommand)]
        what: Option<RecordingsCommand>,
        #[arg(long, global = true)]
        rom: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum RecordingsCommand {
    /// Refer to a recording of this ROM.
    Add { rec: PathBuf },
    /// Drop a reference; the file is left alone.
    Remove { rec: PathBuf },
    /// What is referred to, and whether each file is still there and unchanged.
    List,
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
    /// 64 KB LoROM with tiles, a palette, OAM, a tilemap and a compressed block.
    Graphics,
    /// 32 KB LoROM calling small routines for the decompiler.
    Routines,
}

impl From<FixtureArg> for commands::rom::Fixture {
    fn from(f: FixtureArg) -> Self {
        match f {
            FixtureArg::Minimal => commands::rom::Fixture::Minimal,
            FixtureArg::AllOpcodes => commands::rom::Fixture::AllOpcodes,
            FixtureArg::Dispatch => commands::rom::Fixture::Dispatch,
            FixtureArg::MixedData => commands::rom::Fixture::MixedData,
            FixtureArg::Graphics => commands::rom::Fixture::Graphics,
            FixtureArg::Routines => commands::rom::Fixture::Routines,
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

/// The stack every command runs on: what macOS and Linux give a main thread.
/// Windows gives 1 MB, and an unoptimized build of `run`, one `match` whose
/// arms' locals all share its frame, has outgrown that.
const STACK: usize = 8 << 20;

fn main() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(run)
        .expect("the command thread starts")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

fn run() -> Result<()> {
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
        Command::Graph {
            rom,
            expr,
            project,
            calls,
            dot,
            json,
        } => commands::graph::run(commands::graph::GraphArgs {
            rom: &rom,
            expr: &expr,
            project: project.as_deref(),
            calls,
            dot,
            json,
        }),
        Command::Decompile {
            rom,
            expr,
            project,
            level,
            json,
            header,
            all,
            check,
            no_names,
            assume_dp,
        } => commands::decompile::run(commands::decompile::DecompileArgs {
            rom: rom.as_deref(),
            expr: expr.as_deref(),
            project: project.as_deref(),
            level: &level,
            json,
            header: header.as_deref(),
            all,
            check,
            no_names,
            assume_dp: assume_dp.as_deref(),
        }),
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
            ProjectCommand::Preview {
                expr,
                palette,
                columns,
                size,
                tiles,
                rom,
            } => commands::project::preview(commands::project::PreviewArgs {
                dir: &path,
                rom: rom.as_deref(),
                expr: &expr,
                palette: palette.as_deref(),
                columns,
                size: size.as_deref(),
                tiles: tiles.as_deref(),
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
            ProjectCommand::Recordings { what, rom } => {
                let (add, remove) = match what {
                    Some(RecordingsCommand::Add { rec }) => (Some(rec), None),
                    Some(RecordingsCommand::Remove { rec }) => (None, Some(rec)),
                    Some(RecordingsCommand::List) | None => (None, None),
                };
                commands::project::recordings(
                    &path,
                    rom.as_deref(),
                    add.as_deref(),
                    remove.as_deref(),
                )
            }
        },
        Command::Registers { address } => commands::registers::run(address.as_deref()),
        Command::Tiles {
            rom,
            from,
            bpp,
            count,
            columns,
            palette,
            text: _,
            json,
            ascii,
            digest,
        } => commands::graphics::tiles(commands::graphics::TilesArgs {
            rom: &rom,
            from: &from,
            bpp,
            count,
            columns,
            palette: palette.as_deref(),
            output: if json {
                commands::graphics::Output::Json
            } else if ascii {
                commands::graphics::Output::Ascii
            } else if digest {
                commands::graphics::Output::Digest
            } else {
                commands::graphics::Output::Text
            },
        }),
        Command::Palette {
            rom,
            from,
            count,
            json,
        } => commands::graphics::palette(&rom, &from, count, json),
        Command::Oam {
            rom,
            from,
            obsel,
            sort,
            visible,
            json,
        } => commands::graphics::oam(commands::graphics::OamArgs {
            rom: &rom,
            from: &from,
            obsel: commands::graphics::parse_hex_u8(&obsel)?,
            sort: &sort,
            visible,
            json,
        }),
        Command::Tilemap {
            rom,
            from,
            size,
            json,
        } => commands::graphics::tilemap(&rom, &from, &size, json),
        Command::Testrec {
            out,
            frames,
            keyframe_interval,
        } => commands::rec::testrec(&out, frames, keyframe_interval),
        Command::Rec { what } => match what {
            RecCommand::Info { rec, rom, recover } => {
                commands::rec::info(&rec, rom.as_deref(), recover)
            }
            RecCommand::Validate {
                rec,
                rom,
                sample,
                strict,
                recover,
            } => commands::rec::validate(&rec, rom.as_deref(), sample, strict, recover),
            RecCommand::Index { rec, rebuild } => commands::rec::index(&rec, rebuild),
            RecCommand::When {
                rec,
                region,
                offset,
                len,
                after,
                backward,
            } => commands::rec::when(&rec, &region, &offset, len, after, backward),
            RecCommand::Changes {
                rec,
                from,
                to,
                region,
            } => commands::rec::changes(&rec, from, to, &region),
            RecCommand::Extract {
                rec,
                frame,
                region,
                out,
                hex,
            } => commands::rec::extract(&rec, frame, &region, out.as_deref(), hex),
            RecCommand::ImportRaw {
                rom,
                vram,
                cgram,
                oam,
                wram,
                ppu,
                out,
            } => commands::rec::import_raw(commands::rec::ImportRaw {
                rom: &rom,
                vram: vram.as_deref(),
                cgram: cgram.as_deref(),
                oam: oam.as_deref(),
                wram: wram.as_deref(),
                ppu: ppu.as_deref(),
                out: &out,
            }),
            RecCommand::Pack {
                stream,
                rom,
                out,
                wram,
                keyframe_interval,
                no_compress,
            } => commands::rec::pack(
                &stream,
                &rom,
                &out,
                PackOptions {
                    wram: wram.into(),
                    keyframe_interval,
                    compress: !no_compress,
                },
            ),
            RecCommand::Script { out } => commands::rec::script(&out),
            RecCommand::Live {
                rom,
                port,
                frames,
                once,
                dump,
            } => commands::rec::live(&rom, port, frames, once, dump.as_deref()),
            RecCommand::Convert {
                rec,
                from,
                to,
                out,
                keyframe_interval,
            } => commands::rec::convert(&rec, from, to, &out, keyframe_interval),
        },
        Command::Render { what } => {
            let (rec, frame, bg, sprite, ascii) = match what {
                RenderCommand::Bg {
                    rec,
                    frame,
                    bg,
                    ascii,
                    digest: _,
                } => (rec, frame, Some(bg), None, ascii),
                RenderCommand::Sprite {
                    rec,
                    frame,
                    index,
                    ascii,
                    digest: _,
                } => (rec, frame, None, Some(index), ascii),
            };
            commands::rec::render(commands::rec::RenderArgs {
                rec: &rec,
                frame,
                bg,
                sprite,
                output: if ascii {
                    commands::graphics::Output::Ascii
                } else {
                    commands::graphics::Output::Digest
                },
            })
        }
        Command::Decompress {
            rom,
            from,
            format,
            stats,
            out,
        } => commands::graphics::decompress(&rom, &from, &format, stats, out.as_deref()),
    }
}
