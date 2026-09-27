//! `apu song`: Nintendo's N-SPC driver's songs read as notes and commands
//! (docs/23, A14), from a recording's frame or from the ROM's driver run by
//! Romlens. Text only (`12-content-policy.md` rule 11).

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use romlens_core::audio::nspc::{self, ListEntry};
use romlens_core::recording::{MachineStateSource, RomrecSource, StateRegion};

pub struct SongArgs<'a> {
    pub rec: Option<&'a Path>,
    pub rom: Option<&'a Path>,
    pub project: Option<&'a Path>,
    pub with: &'a [String],
    pub ports: &'a [String],
    pub seconds: f64,
    pub frame: Option<u64>,
    pub song: Option<u8>,
    pub limit: usize,
}

pub fn run(a: SongArgs) -> Result<()> {
    let aram = match (a.rec, a.rom) {
        (Some(rec), _) => {
            let src =
                RomrecSource::open(rec).with_context(|| format!("opening {}", rec.display()))?;
            if !src.regions().contains(&StateRegion::Aram) {
                return Err(anyhow!("{} has no sound side", rec.display()));
            }
            let count = src.frame_count().unwrap_or(0);
            let frame = a.frame.unwrap_or(count.saturating_sub(1));
            if frame >= count {
                return Err(anyhow!(
                    "frame {frame} is past the end of the recording ({count} frames)"
                ));
            }
            println!("the recording at frame {frame}");
            src.region_at(frame, StateRegion::Aram)?
        }
        (None, Some(rom)) => {
            let (mut player, driver, more) =
                super::apu_upload::booted_player(rom, a.project, a.with)?;
            let lists = std::iter::once(driver)
                .chain(more)
                .collect::<Vec<_>>()
                .join(" + ");
            for p in a.ports {
                let (port, value) = super::apu_upload::port_value(p)?;
                player.send_port(port, value);
            }
            player.render((a.seconds.clamp(0.0, 600.0) * 32_000.0) as usize);
            println!(
                "the driver from {lists} booted by Romlens, {} sent, run {:.2} s",
                if a.ports.is_empty() {
                    "nothing".to_owned()
                } else {
                    a.ports.join(", ")
                },
                a.seconds
            );
            player.apu.bus.aram.clone()
        }
        (None, None) => return Err(anyhow!("apu song needs --rec or --rom")),
    };
    let Some(d) = nspc::recognise(&aram) else {
        println!(
            "no N-SPC driver: its table of command lengths is not in audio RAM (the other views read any driver)"
        );
        return Ok(());
    };
    println!(
        "{}: its commands' lengths at ${:04X}",
        d.dialect.name(),
        d.lengths_at
    );
    match d.song_table {
        Some(t) => println!(
            "song table at ${t:04X}: {} songs, number n at entry n - 1: {}",
            d.songs.len(),
            d.songs
                .iter()
                .enumerate()
                .map(|(i, s)| format!("{}=${s:04X}", i + 1))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        None => println!("no song table found"),
    }
    let playing = nspc::playing(&aram, &d);
    match &playing {
        Some(p) => println!(
            "playing: song {} (${:04X}), the block at ${:04X} (entry ${:04X})",
            p.song.map_or("?".to_owned(), |s| s.to_string()),
            p.list,
            p.block,
            p.entry
        ),
        None => println!(
            "nothing playing: the track pointers at $30-$3F and the song pointer at $40 point into no song"
        ),
    }
    let number = a.song.or(playing.as_ref().and_then(|p| p.song));
    let Some(number) = number else { return Ok(()) };
    let at = *d
        .songs
        .get((number as usize).wrapping_sub(1))
        .ok_or_else(|| anyhow!("song {number} is not in the table (1 to {})", d.songs.len()))?;
    let list = nspc::song_list(&aram, d.dialect, at)
        .ok_or_else(|| anyhow!("song {number} does not read as a song"))?;
    println!("\nsong {number} at ${at:04X}:");
    let mut blocks = Vec::new();
    for e in &list {
        match *e {
            ListEntry::Block { at, block } => {
                let tracks = nspc::block_tracks(&aram, block);
                let used: Vec<String> = tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| **t != 0)
                    .map(|(v, t)| format!("{v}=${t:04X}"))
                    .collect();
                println!("  ${at:04X}  block ${block:04X}: voices {}", used.join(" "));
                if !blocks.contains(&block) {
                    blocks.push(block);
                }
            }
            ListEntry::Repeat { at, count, to } => {
                println!(
                    "  ${at:04X}  back to ${to:04X}, {count} more time{}",
                    if count == 1 { "" } else { "s" }
                )
            }
            ListEntry::Jump { at, to } => println!("  ${at:04X}  back to ${to:04X} for ever"),
            ListEntry::End { at } => println!("  ${at:04X}  end"),
        }
    }
    // The block playing, else the first.
    let block = playing
        .as_ref()
        .filter(|p| p.list == at)
        .map(|p| p.block)
        .or(blocks.first().copied());
    let Some(block) = block else { return Ok(()) };
    println!("\nthe block at ${block:04X}, voice by voice:");
    for (v, t) in nspc::block_tracks(&aram, block).iter().enumerate() {
        if *t == 0 {
            continue;
        }
        let track = nspc::track(&aram, d.dialect, *t);
        let now = playing
            .as_ref()
            .filter(|p| p.block == block)
            .and_then(|p| p.positions[v]);
        println!(
            "\n  voice {v}, track ${t:04X}, {} events{}",
            track.events.len(),
            now.map_or(String::new(), |n| format!(", reading ${n:04X} now"))
        );
        for e in track.events.iter().take(a.limit) {
            let here = now.is_some_and(|n| (e.at..e.at + e.bytes.len() as u16).contains(&n));
            let bytes: Vec<String> = e.bytes.iter().map(|b| format!("{b:02X}")).collect();
            println!(
                "  {} ${:04X}  {:<12} {}",
                if here { "▶" } else { " " },
                e.at,
                bytes.join(" "),
                e.text()
            );
        }
        if track.events.len() > a.limit {
            println!("    … {} more (--limit)", track.events.len() - a.limit);
        }
    }
    Ok(())
}
