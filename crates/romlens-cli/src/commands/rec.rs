//! Recordings from the command line (checklist 2.28–2.36, the part pulled
//! forward into 2B): `testrec`, `rec info`, `rec extract`, `rec import-raw`,
//! `rec pack` and `rec script` for recording from Mesen, and `render`, which
//! draws from one.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use romlens_core::graphics::mode7;
use romlens_core::graphics::render::{BgConfig, render_bg_layer, render_sprite};
use romlens_core::recording::format::{COMPRESSION_ZSTD, FLAG_WRAM_KEYFRAME_ONLY, KIND_KEY};
use romlens_core::recording::import::state_from_dumps;
use romlens_core::recording::mesen::{PackOptions, RECORDER_SCRIPT, pack as pack_stream};
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

/// A recording being written: to `<out>.part`, moved over `out` only by
/// [`commit`](Staged::commit), so a command that fails part-way — a stream
/// from the wrong ROM, a damaged one — never destroys a good file already at
/// `out`. Dropped uncommitted, it removes the partial file.
struct Staged {
    part: std::path::PathBuf,
    out: std::path::PathBuf,
    done: bool,
}

impl Staged {
    fn create(out: &Path) -> Result<(Staged, std::io::BufWriter<std::fs::File>)> {
        let mut part = out.as_os_str().to_owned();
        part.push(".part");
        let part = std::path::PathBuf::from(part);
        let file =
            std::fs::File::create(&part).with_context(|| format!("creating {}", out.display()))?;
        Ok((
            Staged {
                part,
                out: out.to_owned(),
                done: false,
            },
            std::io::BufWriter::new(file),
        ))
    }

    fn commit(mut self) -> Result<()> {
        std::fs::rename(&self.part, &self.out)
            .with_context(|| format!("writing {}", self.out.display()))?;
        self.done = true;
        Ok(())
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.done {
            let _ = std::fs::remove_file(&self.part);
        }
    }
}

pub fn testrec(out: &Path, frames: u32, interval: u16) -> Result<()> {
    let frames = frames.max(1);
    let states = fixtures::frames(frames);
    let options = WriterOptions {
        keyframe_interval: interval,
        mapping: 0,
        ..WriterOptions::default()
    };
    let (staged, file) = Staged::create(out)?;
    let mut w = RomrecWriter::new(file, &fixtures::identity(), &StateRegion::ALL, options, 0)?;
    for s in &states {
        w.write_frame(s)?;
    }
    w.finish()?;
    staged.commit()?;
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
    let (staged, file) = Staged::create(a.out)?;
    let mut w = RomrecWriter::new(file, &identity, &regions, WriterOptions::default(), 0)?;
    w.write_frame(&state)?;
    w.finish()?;
    staged.commit()?;
    let names: Vec<&str> = regions.iter().map(|r| r.name()).collect();
    println!(
        "wrote {}: one frame with {}",
        a.out.display(),
        names.join(", ")
    );
    Ok(())
}

fn region_arg(name: &str) -> Result<StateRegion> {
    StateRegion::parse(name).ok_or_else(|| {
        anyhow!("no region {name:?}: use cpu, ppu, io, wram, vram, cgram, oam or timing")
    })
}

fn number(text: &str) -> Result<u32> {
    let t = text.trim();
    let parsed = match t.strip_prefix("0x").or_else(|| t.strip_prefix('$')) {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => t.parse(),
    };
    parsed.map_err(|_| anyhow!("{text:?} is not a number (0x for hex)"))
}

pub fn index(rec: &Path, rebuild: bool) -> Result<()> {
    use romlens_core::recording::change_index::{load_or_build, sidecar_path};
    let src = open(rec, false)?;
    let (index, built) = load_or_build(rec, &src, rebuild)?;
    let (entries, bytes) = index.stats();
    println!(
        "{} {}: {} frames, {entries} block entries, {bytes} bytes",
        if built { "built" } else { "loaded" },
        sidecar_path(rec).display(),
        src.index().len()
    );
    for r in StateRegion::ALL {
        if src.regions().contains(&r) && !index.covers(r) {
            println!(
                "{} is not indexed: the recording keeps it in keyframes only",
                r.name()
            );
        }
    }
    Ok(())
}

pub fn when(
    rec: &Path,
    region: &str,
    offset: &str,
    len: u32,
    after: u64,
    backward: bool,
) -> Result<()> {
    use romlens_core::recording::change_index::load_or_build;
    let region = region_arg(region)?;
    let offset = number(offset)?;
    let len = len.max(1);
    if offset as u64 + len as u64 > region.size() as u64 {
        return Err(anyhow!(
            "{} is {} bytes; {offset:#x}+{len} runs past it",
            region.name(),
            region.size()
        ));
    }
    let src = open(rec, false)?;
    let (index, _) = load_or_build(rec, &src, false)?;
    if !index.covers(region) {
        return Err(anyhow!(
            "{} is not indexed: the recording keeps it in keyframes only",
            region.name()
        ));
    }
    let what = format!("{} {offset:#06x}+{len}", region.name());
    match index.when(&src, region, offset, len, after, backward)? {
        Some(0) if backward => println!("{what}: unchanged since the recording began (frame 0)"),
        Some(f) if backward => println!("{what}: last changed at frame {f}"),
        Some(f) => println!("{what}: next changes at frame {f}"),
        None if backward => println!("{what}: not in the recording"),
        None => println!("{what}: does not change after frame {after}"),
    }
    Ok(())
}

pub fn changes(rec: &Path, from: u64, to: u64, region: &str) -> Result<()> {
    let region = region_arg(region)?;
    let src = open(rec, false)?;
    let runs = src.changes(from, to, region)?;
    let bytes: u64 = runs.iter().map(|r| r.len as u64).sum();
    println!(
        "{} between frames {from} and {to}: {} run{}, {bytes} bytes",
        region.name(),
        runs.len(),
        if runs.len() == 1 { "" } else { "s" }
    );
    for r in &runs {
        println!("  {:#07x}+{:#x}", r.offset, r.len);
    }
    Ok(())
}

pub fn validate(
    rec: &Path,
    rom: Option<&Path>,
    sample: u32,
    strict: bool,
    recover: bool,
) -> Result<()> {
    use romlens_core::recording::validate::{Severity, ValidateOptions, validate as check};
    let rom_sha256 = match rom {
        Some(p) => Some(*load_rom(p)?.sha256()),
        None => None,
    };
    let file = std::fs::File::open(rec).with_context(|| format!("opening {}", rec.display()))?;
    let report = check(
        Box::new(std::io::BufReader::new(file)),
        ValidateOptions {
            rom_sha256,
            sample,
            recover,
        },
    );
    for d in &report.diagnostics {
        let level = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let at = d.frame.map(|f| format!(" (frame {f})")).unwrap_or_default();
        println!("{} {level}{at}: {}", d.code, d.message);
    }
    let count = |n: usize, what: &str| format!("{n} {what}{}", if n == 1 { "" } else { "s" });
    println!(
        "{}: {} frames, {} sampled; {}, {}",
        rec.display(),
        report.frames,
        report.sampled,
        count(report.errors(), "error"),
        count(report.warnings(), "warning")
    );
    if report.errors() > 0 || (strict && report.warnings() > 0) {
        return Err(anyhow!("the recording is not valid"));
    }
    Ok(())
}

pub fn pack(stream: &Path, rom: &Path, out: &Path, options: PackOptions) -> Result<()> {
    let rom = load_rom(rom)?;
    let input = std::io::BufReader::new(
        std::fs::File::open(stream).with_context(|| format!("opening {}", stream.display()))?,
    );
    let (staged, file) = Staged::create(out)?;
    let report = pack_stream(input, &rom, file, options)
        .with_context(|| format!("packing {}", stream.display()))?;
    staged.commit()?;
    println!(
        "wrote {}: {} frames from {}, {} DMA transfers",
        out.display(),
        report.frames,
        report.producer,
        report.dma_events
    );
    if report.truncated {
        println!(
            "the stream ends mid-recording (the emulator closed first); every whole frame was kept"
        );
    }
    if report.state_loads > 0 {
        println!(
            "{} savestate loads during the recording: frames either side are not continuous",
            report.state_loads
        );
    }
    let missing = &report.missing_fields;
    if !missing.is_empty() {
        let names = if missing.len() <= 5 {
            missing.join(", ")
        } else {
            format!(
                "{} fields, among them {}",
                missing.len(),
                missing[..3].join(", ")
            )
        };
        println!("this Mesen does not export {names}; their bits are zero");
    }
    if report.disagreements > 0 {
        println!(
            "warning: {} times a rebuilt PPU register disagreed with the byte the game wrote; please report this",
            report.disagreements
        );
        for (port, frame, rebuilt, written) in &report.first_disagreements {
            println!(
                "  ${port:04X} at frame {frame}: rebuilt {rebuilt:02X}, written {written:02X}"
            );
        }
    }
    if !report.unknown_registers.is_empty() {
        let regs: Vec<String> = report
            .unknown_registers
            .iter()
            .map(|r| format!("${r:04X}"))
            .collect();
        println!(
            "Mesen does not export {}, so they read as zero until the game writes them",
            regs.join(", ")
        );
    }
    Ok(())
}

/// Cut a recording down to frames `from`..=`to`: the state at each is
/// rebuilt and written anew, so the first becomes a keyframe. The write log
/// and other layers are not carried over. A source that kept WRAM in
/// keyframes only has none at most frames, and the cut's keyframes fall on
/// arbitrary ones, so its WRAM is left out rather than written stale.
pub fn convert(rec: &Path, from: u64, to: u64, out: &Path, interval: u16) -> Result<()> {
    use romlens_core::recording::format::FLAG_WRAM_KEYFRAME_ONLY;
    let src = open(rec, false)?;
    let count = src.index().len() as u64;
    if from > to || to >= count {
        return Err(anyhow!(
            "frames {from} to {to} are not in a recording of {count} frames (0 to {})",
            count.saturating_sub(1)
        ));
    }
    let header = src.header();
    let sparse_wram = header.flags & FLAG_WRAM_KEYFRAME_ONLY != 0;
    let options = WriterOptions {
        keyframe_interval: interval,
        mapping: header.mapping,
        compress: header.compression != 0,
        ..WriterOptions::default()
    };
    let regions: Vec<StateRegion> = src
        .regions()
        .into_iter()
        .filter(|r| !(sparse_wram && *r == StateRegion::Wram))
        .collect();
    let (staged, file) = Staged::create(out)?;
    let mut w = RomrecWriter::new(file, src.identity(), &regions, options, header.created)?;
    for f in from..=to {
        let mut state = src.state_at(f)?;
        state.regions.retain(|r, _| regions.contains(r));
        w.write_frame(&state)?;
    }
    w.finish()?;
    staged.commit()?;
    println!(
        "wrote {}: frames {from} to {to} of {}, {} frames",
        out.display(),
        rec.display(),
        to - from + 1
    );
    if sparse_wram {
        println!("WRAM is left out: the recording kept it in keyframes only");
    }
    Ok(())
}

pub fn script(out: &Path) -> Result<()> {
    std::fs::write(out, RECORDER_SCRIPT).with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {}", out.display());
    println!("1. In Mesen, open Debug > Script Window and load the script.");
    println!(
        "2. In the script window's settings, allow access to I/O and OS functions, then run it."
    );
    println!(
        "3. Play, then stop the script and run: romlens rec pack <stream> --rom <rom> --out <recording>.romrec"
    );
    println!(
        "   The stream goes to Mesen's script data folder unless ROMLENS_REC_OUT names a file."
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
        (Some(1), _) if ppu.bg_mode() == 7 => (
            mode7::render_plane(vram, cgram),
            "BG1, mode 7, the 128x128 plane untransformed (M7A-M7D not applied)".to_owned(),
        ),
        (Some(bg), _) => {
            let cfg = BgConfig::from_ppu(&ppu, bg)
                .ok_or_else(|| anyhow!("mode {} has no BG{bg}", ppu.bg_mode()))?;
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

/// `rec live`: listen for the recorder's live stream, printing each status
/// change and a line per second of frames.
pub fn live(
    rom: &Path,
    port: u16,
    frames: Option<u64>,
    once: bool,
    dump: Option<&Path>,
) -> Result<()> {
    use romlens_core::recording::MachineStateSource;
    use romlens_core::recording::live::{DEFAULT_CAPACITY, LiveEvents, LiveServer, LiveStatus};
    use std::sync::Mutex;
    use std::sync::mpsc::{Sender, channel};

    enum Event {
        Frame(u64),
        Status(LiveStatus),
    }
    struct Events(Mutex<Sender<Event>>);
    impl LiveEvents for Events {
        fn frame(&self, n: u64) {
            let _ = self.0.lock().unwrap().send(Event::Frame(n));
        }
        fn status(&self, s: LiveStatus) {
            let _ = self.0.lock().unwrap().send(Event::Status(s));
        }
    }

    let rom = crate::commands::session::load_rom(rom)?;
    let (tx, rx) = channel();
    let mut server = LiveServer::start(
        &rom,
        port,
        DEFAULT_CAPACITY,
        std::sync::Arc::new(Events(Mutex::new(tx))),
    )
    .with_context(|| format!("listening on port {port}"))?;
    let source = server.source();
    let mut received = 0u64;
    let mut last_report = std::time::Instant::now();
    let mut connected = false;
    while let Ok(event) = rx.recv() {
        match event {
            Event::Frame(n) => {
                received += 1;
                if last_report.elapsed().as_secs() >= 1 {
                    last_report = std::time::Instant::now();
                    let pc = source
                        .state_at(n)
                        .ok()
                        .and_then(|s| {
                            s.region(StateRegion::CpuRegisters)
                                .map(romlens_core::recording::CpuRegisters::decode)
                        })
                        .map(|c| format!("${:02X}:{:04X}", c.pb, c.pc))
                        .unwrap_or_default();
                    println!("frame {n}: {received} received, PC {pc}");
                }
                if frames.is_some_and(|f| received >= f) {
                    break;
                }
            }
            Event::Status(s) => {
                println!("{s:?}");
                match s {
                    LiveStatus::Connected { .. } => connected = true,
                    LiveStatus::Disconnected { .. } | LiveStatus::Refused { .. }
                        if once && connected =>
                    {
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    server.stop();
    println!("{received} frames received");
    if let (Some(dir), Some(last)) = (dump, source.latest()) {
        std::fs::create_dir_all(dir)?;
        for (region, name) in [
            (StateRegion::Vram, "vram.bin"),
            (StateRegion::Cgram, "cgram.bin"),
            (StateRegion::Oam, "oam.bin"),
        ] {
            std::fs::write(dir.join(name), source.region_at(last, region)?)?;
        }
        println!("frame {last} written to {}", dir.display());
    }
    Ok(())
}
