//! `romlens`: the core's capabilities from the command line, so every shell
//! feature has a scriptable twin (docs/08 rule 7) and CI can pin output on
//! all three operating systems.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use romlens_core::{
    AddressStyle, MappingMode, RomImage, RomInfo, SpanIndex, fixtures, format_rows_text,
    header_spans,
};

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

fn load(path: &Path) -> Result<RomImage> {
    RomImage::load(path).with_context(|| format!("opening {}", path.display()))
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Info { rom, json } => {
            let rom = load(&rom)?;
            let info = rom.info();
            print!(
                "{}",
                if json {
                    info_json(&info)
                } else {
                    info_text(&info)
                }
            );
        }
        Command::Hex {
            rom,
            from,
            rows,
            address,
        } => {
            let rom = load(&rom)?;
            let start_row = match from {
                Some(expr) => rom.resolve(&expr)?.row,
                None => 0,
            };
            let index = SpanIndex::new(&header_spans(&rom));
            print!(
                "{}",
                format_rows_text(&rom, &index, start_row, rows, address.into())
            );
        }
        Command::Resolve { rom, expr } => {
            let rom = load(&rom)?;
            let r = rom.resolve(&expr)?;
            let snes = r
                .snes_address
                .map_or_else(|| "(unreachable)".to_owned(), |a| a.to_string());
            println!("{} = {}  row {}", r.file_offset, snes, r.row);
            let mirrors: Vec<String> = rom
                .mirrors(r.file_offset)
                .iter()
                .map(ToString::to_string)
                .collect();
            println!("mirrors: {}", mirrors.join(", "));
        }
        Command::Testrom { out, mapping } => {
            let mode: MappingMode = mapping.into();
            std::fs::write(&out, fixtures::for_mapping(mode))
                .with_context(|| format!("writing {}", out.display()))?;
            println!("wrote {} {} test ROM", out.display(), mode);
        }
    }
    Ok(())
}

fn info_text(i: &RomInfo) -> String {
    let h = &i.header;
    let mut s = String::new();
    let _ = writeln!(s, "File:            {}", i.source_name);
    let _ = writeln!(
        s,
        "Size:            {} bytes ({} rows){}",
        i.byte_len,
        i.row_count,
        if i.has_copier_header {
            ", 512-byte copier header stripped"
        } else {
            ""
        }
    );
    let _ = writeln!(s, "SHA-256:         {}", i.sha256);
    let _ = writeln!(
        s,
        "Mapping:         {} ({}), header at {}",
        i.mapping,
        if i.fast_rom { "FastROM" } else { "SlowROM" },
        i.header_offset
    );
    let _ = writeln!(s, "Title:           {:?}", h.title);
    let _ = writeln!(s, "Map mode:        ${:02X}", h.map_mode);
    let _ = writeln!(
        s,
        "Cartridge type:  ${:02X} = {}",
        h.cartridge_type,
        h.cartridge_type_name()
    );
    let _ = writeln!(
        s,
        "ROM size:        ${:02X} = {} KB declared",
        h.rom_size_code,
        h.declared_rom_size() / 1024
    );
    let _ = writeln!(
        s,
        "RAM size:        ${:02X} = {} KB",
        h.ram_size_code,
        h.declared_ram_size() / 1024
    );
    let _ = writeln!(
        s,
        "Region:          ${:02X} = {}",
        h.region,
        h.region_name()
    );
    let _ = writeln!(s, "Developer:       ${:02X}", h.developer_id);
    let _ = writeln!(s, "Version:         1.{}", h.version);
    if let Some(e) = &h.extended {
        let _ = writeln!(
            s,
            "Extended header: maker {:?}, game {:?}, chipset subtype ${:02X}",
            e.maker_code, e.game_code, e.chipset_subtype
        );
    }
    let _ = writeln!(
        s,
        "Checksum:        ${:04X}, complement ${:04X} ({}), computed ${:04X} ({})",
        h.checksum,
        h.complement,
        if h.complement_valid() {
            "pair valid"
        } else {
            "pair invalid"
        },
        i.computed_checksum,
        if i.checksum_ok {
            "match, mirrored sum"
        } else {
            "mismatch"
        }
    );
    let _ = writeln!(s, "Vectors:         native            emulation");
    let native = h.native.named(true);
    let emu = h.emulation.named(false);
    for (n, e) in native.iter().zip(emu.iter()) {
        let _ = writeln!(s, "  {:<16}${:04X}   {:<16}${:04X}", n.0, n.1, e.0, e.1);
    }
    s
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn info_json(i: &RomInfo) -> String {
    let h = &i.header;
    let vectors = |v: &romlens_core::Vectors| {
        format!(
            "{{\"cop\":{},\"brk\":{},\"abort\":{},\"nmi\":{},\"reset\":{},\"irq\":{}}}",
            v.cop, v.brk, v.abort, v.nmi, v.reset, v.irq
        )
    };
    let mut s = String::new();
    s.push_str("{\n");
    let _ = writeln!(s, "  \"file\": {},", json_str(&i.source_name));
    let _ = writeln!(s, "  \"size\": {},", i.byte_len);
    let _ = writeln!(s, "  \"rows\": {},", i.row_count);
    let _ = writeln!(s, "  \"copierHeader\": {},", i.has_copier_header);
    let _ = writeln!(s, "  \"sha256\": {},", json_str(&i.sha256));
    let _ = writeln!(s, "  \"mapping\": {},", json_str(i.mapping.name()));
    let _ = writeln!(s, "  \"fastRom\": {},", i.fast_rom);
    let _ = writeln!(s, "  \"headerOffset\": {},", i.header_offset.value());
    let _ = writeln!(s, "  \"title\": {},", json_str(&h.title));
    let _ = writeln!(s, "  \"mapMode\": {},", h.map_mode);
    let _ = writeln!(s, "  \"cartridgeType\": {},", h.cartridge_type);
    let _ = writeln!(s, "  \"romSizeCode\": {},", h.rom_size_code);
    let _ = writeln!(s, "  \"ramSizeCode\": {},", h.ram_size_code);
    let _ = writeln!(s, "  \"region\": {},", h.region);
    let _ = writeln!(s, "  \"developerId\": {},", h.developer_id);
    let _ = writeln!(s, "  \"version\": {},", h.version);
    let _ = writeln!(s, "  \"checksum\": {},", h.checksum);
    let _ = writeln!(s, "  \"complement\": {},", h.complement);
    let _ = writeln!(s, "  \"complementValid\": {},", h.complement_valid());
    let _ = writeln!(s, "  \"computedChecksum\": {},", i.computed_checksum);
    let _ = writeln!(s, "  \"checksumOk\": {},", i.checksum_ok);
    let _ = writeln!(s, "  \"nativeVectors\": {},", vectors(&h.native));
    let _ = writeln!(s, "  \"emulationVectors\": {}", vectors(&h.emulation));
    s.push_str("}\n");
    s
}
