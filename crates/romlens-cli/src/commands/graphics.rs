//! The graphics views from the command line (checklist 2.20–2.27): `tiles`,
//! `palette`, `oam`, `tilemap` and `decompress`, all on raw ROM bytes.
//!
//! Nothing here writes an image file. `12-content-policy.md` rule 5 keeps
//! sprite-sheet export out of the app, and a CLI that wrote a PNG at an
//! arbitrary offset would be a sprite ripper; `--text`, `--json`, `--ascii`
//! and `--digest` satisfy the CLI-twin rule and make better goldens.
//! `decompress --out` writes bytes, which is the one exception.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Result, anyhow};
use romlens_core::graphics::compress::sm_lz::{self, COMMAND_NAMES};
use romlens_core::graphics::oam::ObjSelect;
use romlens_core::graphics::palette::{PaletteRef, resolve};
use romlens_core::graphics::render::render_tile_sheet;
use romlens_core::graphics::tile::TileFormat;
use romlens_core::graphics::tilemap::ScreenSize;
use romlens_core::viewmodel::graphics::{
    OamSort, format_oam_text, format_palette_text, format_planes, format_tile_grid,
    format_tilemap_text, oam_rows, palette_entries, tile_view, tilemap_cells,
};
use romlens_core::{FileOffset, RomImage};

use crate::commands::session::{load_rom, rom_offset};

/// How a view prints.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Text,
    Json,
    Ascii,
    Digest,
}

fn where_is(rom: &RomImage, off: u32) -> String {
    match rom.snes_address_for(FileOffset(off)) {
        Some(a) => format!("{} = {a}", FileOffset(off)),
        None => FileOffset(off).to_string(),
    }
}

fn slice(rom: &RomImage, off: u32, len: usize) -> &[u8] {
    let bytes = rom.bytes();
    let start = (off as usize).min(bytes.len());
    &bytes[start..(start + len).min(bytes.len())]
}

/// A byte value such as `0x30`, `$30` or `30`, read as hex.
pub fn parse_hex_u8(text: &str) -> Result<u8> {
    let t = text.trim_start_matches('$').trim_start_matches("0x");
    u8::from_str_radix(t, 16)
        .map_err(|_| anyhow!("{text:?} is not a byte; write it as $30 or 0x30"))
}

pub struct TilesArgs<'a> {
    pub rom: &'a Path,
    pub from: &'a str,
    pub bpp: u8,
    pub count: u32,
    pub columns: u32,
    /// BGR15 colours in the ROM at this address; grayscale when absent.
    pub palette: Option<&'a str>,
    pub output: Output,
}

pub fn tiles(a: TilesArgs<'_>) -> Result<()> {
    let rom = load_rom(a.rom)?;
    let format = TileFormat::from_bpp(a.bpp)
        .ok_or_else(|| anyhow!("--bpp is 2, 4 or 8, or 7 for Mode 7"))?;
    let off = rom_offset(&rom, a.from)?;
    let count = a.count.max(1);
    let bytes = slice(&rom, off, format.tile_len() * count as usize);
    let (colours, palette_note) = match a.palette {
        Some(expr) => {
            let p = rom_offset(&rom, expr)?;
            let pal = slice(&rom, p, format.colours() * 2);
            (
                resolve(PaletteRef::Bytes(pal), format.colours(), 0, 0),
                format!("palette at {}", where_is(&rom, p)),
            )
        }
        None => (
            resolve(PaletteRef::Grayscale, format.colours(), 0, 0),
            "grayscale".to_owned(),
        ),
    };
    match a.output {
        Output::Text => {
            println!(
                "{}  {}  {count} tile{}  {} bytes",
                where_is(&rom, off),
                format.name(),
                if count == 1 { "" } else { "s" },
                bytes.len()
            );
            print!("{}", format_tile_grid(bytes, format, count, a.columns));
            if count == 1 && format.planes() > 0 {
                println!();
                print!("{}", format_planes(&tile_view(bytes, format)));
            }
        }
        Output::Json => {
            let len = format.tile_len();
            let tiles: Vec<String> = (0..count as usize)
                .map(|i| {
                    let start = (i * len).min(bytes.len());
                    let v = tile_view(&bytes[start..(start + len).min(bytes.len())], format);
                    let planes: Vec<String> = v.planes.iter().map(|p| format!("{p:?}")).collect();
                    format!(
                        "    {{\"offset\": {}, \"indices\": {:?}, \"planes\": [{}]}}",
                        off as usize + i * len,
                        v.indices,
                        planes.join(", ")
                    )
                })
                .collect();
            println!("{{");
            println!("  \"format\": \"{}\",", format.name());
            println!("  \"tileBytes\": {},", format.tile_len());
            println!("  \"tiles\": [");
            println!("{}", tiles.join(",\n"));
            println!("  ]");
            println!("}}");
        }
        Output::Ascii | Output::Digest => {
            let bm = render_tile_sheet(bytes, format, &colours, count, a.columns);
            if a.output == Output::Ascii {
                print!("{}", bm.to_ascii());
            } else {
                println!(
                    "{}x{} {} sha256={}",
                    bm.width,
                    bm.height,
                    palette_note,
                    bm.digest()
                );
            }
        }
    }
    Ok(())
}

pub fn palette(rom: &Path, from: &str, count: u16, json: bool) -> Result<()> {
    let rom = load_rom(rom)?;
    let off = rom_offset(&rom, from)?;
    let count = count.clamp(1, 256);
    let bytes = slice(&rom, off, count as usize * 2);
    let entries = palette_entries(bytes, count);
    if json {
        let rows: Vec<String> = entries
            .iter()
            .map(|e| {
                let c = e.colour;
                let (r, g, b) = c.rgb8();
                format!(
                    "    {{\"index\": {}, \"raw\": {}, \"blue5\": {}, \"green5\": {}, \"red5\": {}, \
\"rgb\": \"#{r:02X}{g:02X}{b:02X}\", \"offset\": {}}}",
                    e.index,
                    c.0,
                    c.blue5(),
                    c.green5(),
                    c.red5(),
                    off + e.byte
                )
            })
            .collect();
        println!("{{\n  \"entries\": [\n{}\n  ]\n}}", rows.join(",\n"));
        return Ok(());
    }
    println!("{}  {count} colours", where_is(&rom, off));
    print!("{}", format_palette_text(&entries));
    Ok(())
}

pub struct OamArgs<'a> {
    pub rom: &'a Path,
    pub from: &'a str,
    pub obsel: u8,
    pub sort: &'a str,
    pub visible: bool,
    pub json: bool,
}

pub fn oam(a: OamArgs<'_>) -> Result<()> {
    let rom = load_rom(a.rom)?;
    let off = rom_offset(&rom, a.from)?;
    let sort =
        OamSort::parse(a.sort).ok_or_else(|| anyhow!("--sort is table, screen or priority"))?;
    let obsel = ObjSelect::from_register(a.obsel);
    let rows = oam_rows(slice(&rom, off, 544), sort);
    if a.json {
        let out: Vec<String> = rows
            .iter()
            .map(|e| {
                let (w, h) = obsel.size_of(e.large);
                format!(
                    "    {{\"index\": {}, \"x\": {}, \"y\": {}, \"tile\": {}, \"palette\": {}, \
\"priority\": {}, \"hflip\": {}, \"vflip\": {}, \"large\": {}, \"width\": {w}, \"height\": {h}, \
\"tileWord\": {}}}",
                    e.index,
                    e.x,
                    e.y,
                    e.tile,
                    e.palette,
                    e.priority,
                    e.hflip,
                    e.vflip,
                    e.large,
                    obsel.tile_word_address(e.tile)
                )
            })
            .collect();
        println!(
            "{{\n  \"obsel\": {},\n  \"sprites\": [\n{}\n  ]\n}}",
            a.obsel,
            out.join(",\n")
        );
        return Ok(());
    }
    let (small, large) = (obsel.size_of(false), obsel.size_of(true));
    println!(
        "{}  OBSEL ${:02X}: {}x{} / {}x{}, tiles at word ${:04X}",
        where_is(&rom, off),
        a.obsel,
        small.0,
        small.1,
        large.0,
        large.1,
        obsel.tile_word_address(0)
    );
    print!("{}", format_oam_text(&rows, obsel, a.visible));
    Ok(())
}

pub fn tilemap(rom: &Path, from: &str, size: &str, json: bool) -> Result<()> {
    let rom = load_rom(rom)?;
    let off = rom_offset(&rom, from)?;
    let size =
        ScreenSize::parse(size).ok_or_else(|| anyhow!("--size is 32x32, 64x32, 32x64 or 64x64"))?;
    let bytes = slice(&rom, off, size.entries() * 2);
    if json {
        let cells: Vec<String> = tilemap_cells(bytes, size)
            .iter()
            .map(|(col, row, i, e)| {
                format!(
                    "    {{\"col\": {col}, \"row\": {row}, \"offset\": {}, \"raw\": {}, \"tile\": {}, \
\"palette\": {}, \"priority\": {}, \"hflip\": {}, \"vflip\": {}}}",
                    off as usize + i * 2,
                    e.raw,
                    e.tile,
                    e.palette,
                    e.priority,
                    e.hflip,
                    e.vflip
                )
            })
            .collect();
        println!(
            "{{\n  \"size\": \"{}\",\n  \"cells\": [\n{}\n  ]\n}}",
            size.name(),
            cells.join(",\n")
        );
        return Ok(());
    }
    println!("{}  {} tilemap", where_is(&rom, off), size.name());
    print!("{}", format_tilemap_text(bytes, size));
    Ok(())
}

pub fn decompress(
    rom: &Path,
    from: &str,
    format: &str,
    stats: bool,
    out: Option<&Path>,
) -> Result<()> {
    if format != "sm" {
        return Err(anyhow!("--format sm is the only compression format so far"));
    }
    let rom = load_rom(rom)?;
    let off = rom_offset(&rom, from)?;
    let d = sm_lz::decompress(&rom.bytes()[off as usize..])
        .map_err(|e| anyhow!("{}: {e}", where_is(&rom, off)))?;
    let mut text = String::new();
    let _ = writeln!(
        text,
        "{}: {} bytes from {} ({:.2}:1), ending at {}",
        where_is(&rom, off),
        d.output.len(),
        d.consumed,
        d.output.len() as f64 / d.consumed.max(1) as f64,
        FileOffset(off + d.consumed as u32)
    );
    if stats {
        for (name, n) in COMMAND_NAMES.iter().zip(d.commands) {
            if n > 0 {
                let _ = writeln!(text, "  {name:<26} {n}");
            }
        }
        let _ = writeln!(text, "  {:<26} {}", "long headers", d.long_headers);
    }
    print!("{text}");
    match out {
        Some(path) => {
            std::fs::write(path, &d.output)
                .map_err(|e| anyhow!("writing {}: {e}", path.display()))?;
            println!("wrote {} bytes to {}", d.output.len(), path.display());
        }
        None => {
            for (i, row) in d.output.chunks(16).take(4).enumerate() {
                let hex: Vec<String> = row.iter().map(|b| format!("{b:02X}")).collect();
                println!("{:06X}  {}", i * 16, hex.join(" "));
            }
            if d.output.len() > 64 {
                println!(
                    "… {} more bytes; --out writes them all",
                    d.output.len() - 64
                );
            }
        }
    }
    Ok(())
}
