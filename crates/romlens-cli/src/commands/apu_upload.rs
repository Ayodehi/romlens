//! `apu upload` and `apu render --rom`: the sound driver, songs and
//! samples a ROM sends the sound CPU, traced from its code (docs/23, A9),
//! and the sound from them. Text only (`12-content-policy.md` rule 11).

use std::path::Path;

use anyhow::{Result, anyhow};
use romlens_core::apu::Apu;
use romlens_core::apu::player::Player;
use romlens_core::audio::upload::{SentUpload, Upload, UploadReport, from_ports, trace};
use romlens_core::audio::{PartKind, aram_map, entries_written, voices};
use romlens_core::recording::{MachineStateSource, RomrecSource, SpcState};
use romlens_core::rom::image::RomImage;

use crate::commands::session;

fn driver(r: &UploadReport) -> Option<&Upload> {
    romlens_core::audio::upload::driver(&r.uploads)
}

/// The machine booted straight into the driver, with `more` uploads laid
/// over it, run for `seconds` so it sets the DSP up.
fn booted(rom: &RomImage, driver: &Upload, more: &[&Upload], seconds: f64) -> Apu {
    Player::from_upload(rom, driver, more, seconds).apu
}

/// What each block's bytes are once in audio RAM, as runs of the ROM.
fn classify(rom: &RomImage, apu: &Apu, d: &Upload, u: &Upload) -> Vec<(u32, u32, String)> {
    let spc = SpcState {
        pc: apu.cpu.pc,
        rom_enabled: apu.bus.io.rom_enabled,
        ..SpcState::default()
    };
    let mut used: Vec<u8> = voices(&apu.bus.dsp, None)
        .iter()
        .map(|v| v.source)
        .collect();
    used.extend(entries_written(
        apu.bus.dsp[0x5D],
        d.blocks
            .iter()
            .chain(&u.blocks)
            .map(|b| (b.aram, b.len as u32)),
    ));
    let parts = aram_map(&apu.bus.aram, &apu.bus.dsp, &spc, &used, &[u.entry], None);
    let kind_at = |a: u16| {
        parts
            .iter()
            .find(|p| p.start <= a && (a as u32) < p.start as u32 + p.len)
            .map(|p| match p.kind {
                PartKind::Sample(_) => "samples".to_owned(),
                PartKind::Code => "driver code".to_owned(),
                PartKind::Directory => "sample directory".to_owned(),
                k => k.name().to_owned(),
            })
            .unwrap_or_default()
    };
    let _ = rom;
    let mut out: Vec<(u32, u32, String)> = Vec::new();
    for b in &u.blocks {
        for i in 0..b.len as u32 {
            let k = kind_at(b.aram.wrapping_add(i as u16));
            let off = b.rom.0 + i;
            match out.last_mut() {
                Some((_, end, last)) if *last == k && *end == off => *end = off + 1,
                _ => out.push((off, off + 1, k)),
            }
        }
    }
    out
}

pub fn upload(rom_path: &Path, project: Option<&Path>, rec: Option<&Path>) -> Result<()> {
    let s = session::open(rom_path, project, false)?;
    let r = trace(&s.rom, &s.snap);
    if r.routines.is_empty() {
        println!(
            "no upload routine found: nothing in the analysed code waits for $BBAA or kicks with $CC on APUIO0 and reads a block list through a long pointer"
        );
    }
    for u in &r.routines {
        println!(
            "upload routine {}: reads its block list through [${:02X}], {} instructions show it",
            u.entry,
            u.pointer,
            u.evidence.len()
        );
    }
    let drv = driver(&r);
    for (n, u) in r.uploads.iter().enumerate() {
        let what = if drv.is_some_and(|d| d.list == u.list) {
            "the driver"
        } else {
            "more for a running driver"
        };
        println!(
            "\nupload {n}: the list at {} (set at {} in {}), {} blocks, {} bytes, then ${:04X}: {what}",
            u.list,
            s.rom
                .snes_address_for(u.set_at)
                .map(|a| a.to_string())
                .unwrap_or_else(|| format!("0x{:06X}", u.set_at.0)),
            u.set_in,
            u.blocks.len(),
            u.bytes(),
            u.entry
        );
        for b in &u.blocks {
            println!(
                "  ROM 0x{:06X}-0x{:06X} ({})  →  audio RAM ${:04X}-${:04X}  {:>6} bytes",
                b.rom.0,
                b.rom.0 + b.len as u32 - 1,
                b.from,
                b.aram,
                b.aram as u32 + b.len as u32 - 1,
                b.len
            );
        }
    }
    if let Some(d) = drv {
        println!(
            "\nwhat the bytes are, once the driver has run a second (booted straight into it by Romlens's SPC700):"
        );
        let apu = booted(&s.rom, d, &[], 1.0);
        for u in &r.uploads {
            let apu = if u.list == d.list {
                apu.clone()
            } else {
                booted(&s.rom, d, &[u], 1.0)
            };
            println!("  upload at {}:", u.list);
            for (from, to, k) in classify(&s.rom, &apu, d, u) {
                println!(
                    "    ROM 0x{from:06X}-0x{:06X}  {:>6} bytes  {k}",
                    to - 1,
                    to - from
                );
            }
        }
    }
    println!(
        "\nsound commands: {} constants the code sends the driver",
        r.commands.len()
    );
    for p in 0..4u8 {
        for via in [None, Some(())] {
            let mut values: Vec<u32> = r
                .commands
                .iter()
                .filter(|c| c.port == p && c.via.is_some() == via.is_some())
                .map(|c| c.value)
                .collect();
            if values.is_empty() {
                continue;
            }
            values.sort_unstable();
            values.dedup();
            let how = match via {
                None => "stored to the port".to_owned(),
                Some(()) => {
                    let mut at: Vec<String> = r
                        .commands
                        .iter()
                        .filter(|c| c.port == p)
                        .filter_map(|c| c.via.map(|a| a.to_string()))
                        .collect();
                    at.sort();
                    at.dedup();
                    format!("set in {}, which is copied to the port", at.join(", "))
                }
            };
            let shown: Vec<String> = values
                .iter()
                .take(32)
                .map(|v| format!("${v:02X}"))
                .collect();
            println!(
                "  port {p} ($214{p}), {how}: {} value{}: {}{}",
                values.len(),
                if values.len() == 1 { "" } else { "s" },
                shown.join(" "),
                if values.len() > 32 { " …" } else { "" }
            );
        }
    }
    if let Some(rec) = rec {
        compare_recording(&s.rom, &r, rec)?;
    }
    Ok(())
}

/// The uploads a recording saw sent, and where their bytes are in the ROM.
fn compare_recording(rom: &RomImage, r: &UploadReport, path: &Path) -> Result<()> {
    let rec = RomrecSource::open(path)?;
    let mut events = Vec::new();
    for f in 0..rec.frame_count().unwrap_or(0) {
        if let Some(e) = rec.apu_events(f)? {
            events.extend(e.events);
        }
    }
    let sent: Vec<SentUpload> = from_ports(&events);
    println!(
        "\nthe recording: {} uploads sent through the ports",
        sent.len()
    );
    for (n, u) in sent.iter().enumerate() {
        let entry = u
            .entry
            .map(|e| format!("${e:04X}"))
            .unwrap_or_else(|| "(unfinished)".to_owned());
        let total: usize = u.blocks.iter().map(|b| b.1.len()).sum();
        println!(
            "  sent {n}: {} blocks, {total} bytes, then {entry}",
            u.blocks.len()
        );
        for (aram, bytes) in &u.blocks {
            let traced = r.uploads.iter().find_map(|t| {
                t.blocks
                    .iter()
                    .find(|b| {
                        b.aram == *aram
                            && b.len as usize == bytes.len()
                            && rom.bytes()[b.rom.as_usize()..b.rom.as_usize() + bytes.len()]
                                == bytes[..]
                    })
                    .map(|b| (t.list, b.rom))
            });
            let origin = match traced {
                Some((list, off)) => format!("the traced list at {list}, ROM 0x{:06X}", off.0),
                None => match find(rom.bytes(), bytes) {
                    Some(off) => format!("not traced; the bytes are at ROM 0x{off:06X}"),
                    None => {
                        "not traced, and not in the ROM as they are (built or unpacked)".to_owned()
                    }
                },
            };
            println!(
                "    audio RAM ${aram:04X}, {:>6} bytes: {origin}",
                bytes.len()
            );
        }
    }
    Ok(())
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// `--port 2=$05`.
pub fn port_value(p: &str) -> Result<(u8, u8)> {
    let (port, value) = p
        .split_once('=')
        .ok_or_else(|| anyhow!("--port is PORT=VALUE, such as 2=$05"))?;
    let port: u8 = port.trim().parse()?;
    let v = value.trim().trim_start_matches('$');
    let value = u8::from_str_radix(v, 16).map_err(|_| anyhow!("{value}: a byte in hex"))?;
    if port > 3 {
        return Err(anyhow!("ports are 0 to 3"));
    }
    Ok((port, value))
}

/// The driver booted from the ROM with the uploads at `with` laid over
/// it, run a quarter second so it starts: the player, the driver's list and
/// the others'.
pub fn booted_player(
    rom_path: &Path,
    project: Option<&Path>,
    with: &[String],
) -> Result<(Player, String, Vec<String>)> {
    let s = session::open(rom_path, project, false)?;
    let r = trace(&s.rom, &s.snap);
    let d = driver(&r).ok_or_else(|| {
        anyhow!("no upload of a driver traced in this ROM (see romlens apu upload)")
    })?;
    let mut more = Vec::new();
    for w in with {
        let at = s.rom.resolve(w).map_err(|e| anyhow!("{w}: {e}"))?;
        let u = r
            .uploads
            .iter()
            .find(|u| s.rom.file_offset_for(u.list) == Some(at.file_offset))
            .ok_or_else(|| anyhow!("{w} is not a traced upload (see romlens apu upload)"))?;
        more.push(u);
    }
    let player = Player::from_upload(&s.rom, d, &more, 0.25);
    Ok((
        player,
        d.list.to_string(),
        more.iter().map(|u| u.list.to_string()).collect(),
    ))
}

/// `apu render --rom`: the driver booted from the ROM, with more uploads
/// laid over it, a command sent, and the sound described.
pub fn render_rom(
    rom_path: &Path,
    project: Option<&Path>,
    with: &[String],
    ports: &[String],
    seconds: f64,
) -> Result<()> {
    use romlens_core::apu::render::SAMPLE_RATE;
    // The driver starts before the command.
    let (mut player, driver_list, more) = booted_player(rom_path, project, with)?;
    for p in ports {
        let (port, value) = port_value(p)?;
        player.send_port(port, value);
    }
    let n = (seconds * SAMPLE_RATE as f64) as usize;
    let out = player.render(n);
    println!(
        "the driver from {} booted by Romlens{}, {} sent, {:.2} s: {} samples",
        driver_list,
        if more.is_empty() {
            String::new()
        } else {
            format!(" with {}", more.join(", "))
        },
        if ports.is_empty() {
            "nothing".to_owned()
        } else {
            ports.join(", ")
        },
        out.len() as f64 / SAMPLE_RATE as f64,
        out.len()
    );
    crate::commands::apu::describe(&out);
    Ok(())
}
