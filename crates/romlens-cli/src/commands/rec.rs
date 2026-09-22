//! Recordings from the command line (checklist 2.28–2.36, the part pulled
//! forward into 2B): `testrec`, `rec info`, `rec extract`, `rec import-raw`,
//! and `render`, which draws from one.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use romlens_core::graphics::render::{BgConfig, render_bg_layer, render_sprite};
use romlens_core::recording::format::{COMPRESSION_ZSTD, FLAG_WRAM_KEYFRAME_ONLY, KIND_KEY};
use romlens_core::recording::import::state_from_dumps;
use romlens_core::recording::writer::WriterOptions;
use romlens_core::recording::{
    MachineStateSource, RecordingIdentity, RomrecSource, RomrecWriter, StateRegion, fixtures,
};
use romlens_core::viewmodel::graphics::oam_rows;

use crate::commands::graphics::Output;
use crate::commands::session::load_rom;

fn open(path: &Path, recover: bool) -> Result<RomrecSource> {
    let r = if recover {
        RomrecSource::open_recovering(path)
    } else {
        RomrecSource::open(path)
    };
    r.with_context(|| format!("opening {}", path.display()))
}

fn create(path: &Path) -> Result<std::io::BufWriter<std::fs::File>> {
    Ok(std::io::BufWriter::new(
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?,
    ))
}

pub fn testrec(out: &Path, frames: u32, interval: u16) -> Result<()> {
    let frames = frames.max(1);
    let states = fixtures::frames(frames);
    let options = WriterOptions {
        keyframe_interval: interval,
        mapping: 0,
        ..WriterOptions::default()
    };
    let mut w = RomrecWriter::new(
        create(out)?,
        &fixtures::identity(),
        &StateRegion::ALL,
        options,
        0,
    )?;
    for s in &states {
        w.write_frame(s)?;
    }
    w.finish()?;
    println!(
        "wrote {} synthetic recording, {frames} frames, of the graphics test ROM",
        out.display()
    );
    Ok(())
}

pub fn info(path: &Path, rom: Option<&Path>, recover: bool) -> Result<()> {
    let rec = open(path, recover)?;
    let h = rec.header();
    let keys = rec.index().iter().filter(|e| e.kind == KIND_KEY).count();
    let frames = rec.index().len();
    println!(
        "Format:       {}.{}{}",
        h.version.0,
        h.version.1,
        if rec.recovered() {
            ", index rebuilt by scanning (no footer)"
        } else {
            ""
        }
    );
    println!(
        "Producer:     {} {}",
        h.producer,
        if h.producer_version.is_empty() {
            ""
        } else {
            &h.producer_version
        }
    );
    println!("ROM SHA-256:  {}", rec.identity().sha256_hex());
    if let Some(rom) = rom {
        let rom = load_rom(rom)?;
        match rec.check_rom(rom.sha256()) {
            Ok(()) => println!("              matches {}", rom.info().source_name),
            Err(e) => println!("              {e}"),
        }
    }
    println!(
        "Frames:       {frames} ({keys} keyframe{}, every {} frames)",
        if keys == 1 { "" } else { "s" },
        h.keyframe_interval
    );
    println!(
        "Storage:      {} bytes, {}{}",
        rec.file_len(),
        if h.compression == COMPRESSION_ZSTD {
            "zstd"
        } else {
            "uncompressed"
        },
        if h.flags & FLAG_WRAM_KEYFRAME_ONLY != 0 {
            ", WRAM in keyframes only"
        } else {
            ""
        }
    );
    let delta: u64 = rec
        .index()
        .iter()
        .filter(|e| e.kind != KIND_KEY)
        .map(|e| e.len as u64)
        .sum();
    let key: u64 = rec
        .index()
        .iter()
        .filter(|e| e.kind == KIND_KEY)
        .map(|e| e.len as u64)
        .sum();
    if frames > keys {
        println!(
            "              keyframes {} bytes each on average, deltas {}",
            key / keys.max(1) as u64,
            delta / (frames - keys) as u64
        );
    }
    let regions: Vec<String> = h
        .regions
        .iter()
        .map(|r| format!("{} {}", r.name(), r.size()))
        .collect();
    println!("Regions:      {}", regions.join(", "));
    let l = rec.layers();
    let layers: Vec<&str> = [
        (l.framebuffer, "framebuffer"),
        (l.write_log, "write log"),
        (l.trace, "trace"),
        (l.read_log, "read log"),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .map(|(_, n)| *n)
    .collect();
    println!(
        "Layers:       {}",
        if layers.is_empty() {
            "none".to_owned()
        } else {
            layers.join(", ")
        }
    );
    Ok(())
}

pub fn extract(path: &Path, frame: u64, region: &str, out: Option<&Path>, hex: bool) -> Result<()> {
    let region = StateRegion::parse(region).ok_or_else(|| {
        anyhow!("--region is one of cpu, ppu, io, wram, vram, cgram, oam, timing")
    })?;
    let rec = open(path, false)?;
    let bytes = rec.region_at(frame, region)?;
    if let Some(out) = out {
        std::fs::write(out, &bytes).with_context(|| format!("writing {}", out.display()))?;
        println!(
            "wrote {} bytes of {} at frame {frame} to {}",
            bytes.len(),
            region.name(),
            out.display()
        );
        return Ok(());
    }
    if !hex {
        return Err(anyhow!("give --out FILE, or --hex to print the bytes"));
    }
    for (i, row) in bytes.chunks(16).enumerate() {
        if row.iter().all(|b| *b == 0) && bytes.len() > 256 {
            continue;
        }
        let h: Vec<String> = row.iter().map(|b| format!("{b:02X}")).collect();
        println!("{:05X}  {}", i * 16, h.join(" "));
    }
    Ok(())
}

pub struct ImportRaw<'a> {
    pub rom: &'a Path,
    pub vram: Option<&'a Path>,
    pub cgram: Option<&'a Path>,
    pub oam: Option<&'a Path>,
    pub wram: Option<&'a Path>,
    pub ppu: Option<&'a Path>,
    pub out: &'a Path,
}

pub fn import_raw(a: ImportRaw<'_>) -> Result<()> {
    let rom = load_rom(a.rom)?;
    let mut dumps = Vec::new();
    for (region, file) in [
        (StateRegion::Vram, a.vram),
        (StateRegion::Cgram, a.cgram),
        (StateRegion::Oam, a.oam),
        (StateRegion::Wram, a.wram),
        (StateRegion::PpuState, a.ppu),
    ] {
        if let Some(f) = file {
            let bytes = std::fs::read(f).with_context(|| format!("reading {}", f.display()))?;
            dumps.push((region, bytes));
        }
    }
    let state = state_from_dumps(&dumps)?;
    let regions: Vec<StateRegion> = state.regions.keys().copied().collect();
    let identity = RecordingIdentity {
        rom_sha256: *rom.sha256(),
        producer: "romlens import-raw".to_owned(),
        producer_version: romlens_core::API_VERSION.to_owned(),
    };
    let mut w = RomrecWriter::new(
        create(a.out)?,
        &identity,
        &regions,
        WriterOptions::default(),
        0,
    )?;
    w.write_frame(&state)?;
    w.finish()?;
    let names: Vec<&str> = regions.iter().map(|r| r.name()).collect();
    println!(
        "wrote {}: one frame with {}",
        a.out.display(),
        names.join(", ")
    );
    Ok(())
}

pub struct RenderArgs<'a> {
    pub rec: &'a Path,
    pub frame: u64,
    pub bg: Option<u8>,
    pub sprite: Option<u8>,
    pub output: Output,
}

pub fn render(a: RenderArgs<'_>) -> Result<()> {
    let rec = open(a.rec, false)?;
    let state = rec.state_at(a.frame)?;
    let vram = state
        .vram()
        .ok_or_else(|| anyhow!("the recording has no VRAM"))?;
    let cgram = state
        .cgram()
        .ok_or_else(|| anyhow!("the recording has no CGRAM"))?;
    let ppu = state.ppu().ok_or_else(|| {
        anyhow!("the recording has no PPU registers, so there is no mode to draw")
    })?;
    let (bm, what) = match (a.bg, a.sprite) {
        (Some(bg), _) => {
            let cfg = BgConfig::from_ppu(&ppu, bg).ok_or_else(|| match ppu.bg_mode() {
                7 => anyhow!("Mode 7 is not drawn by the Phase 2 renderer"),
                mode => anyhow!("mode {mode} has no BG{bg}"),
            })?;
            let what = format!(
                "BG{bg}, mode {}, {} {}, map at word ${:04X}, tiles at ${:04X}{}",
                ppu.bg_mode(),
                cfg.format.name(),
                cfg.size.name(),
                cfg.map_word,
                cfg.char_word,
                if cfg.tile16 { ", 16×16 cells" } else { "" }
            );
            (render_bg_layer(vram, cgram, &cfg), what)
        }
        (None, Some(i)) => {
            let oam = state
                .oam()
                .ok_or_else(|| anyhow!("the recording has no OAM"))?;
            let e = oam_rows(oam, romlens_core::viewmodel::graphics::OamSort::Table)
                .into_iter()
                .find(|e| e.index == i)
                .ok_or_else(|| anyhow!("there are 128 sprites, 0 to 127"))?;
            let obsel = ppu.obj_select();
            let what = format!(
                "sprite {i} at ({}, {}), tile ${:03X}, palette {}",
                e.x, e.y, e.tile, e.palette
            );
            (render_sprite(vram, cgram, obsel, &e), what)
        }
        (None, None) => return Err(anyhow!("give --bg 1-4 or --sprite N")),
    };
    println!("frame {}: {what}", a.frame);
    match a.output {
        Output::Ascii => print!("{}", bm.to_ascii()),
        _ => println!("{}x{} sha256={}", bm.width, bm.height, bm.digest()),
    }
    Ok(())
}
