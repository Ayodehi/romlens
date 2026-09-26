//! `apu`: a recording's sound side (docs/23, A5): the voices, the DSP's
//! registers, audio RAM's map, the sample directory, the notes over time and
//! the ports. Text only; nothing here writes sound or samples out
//! (`12-content-policy.md` rule 11).

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use romlens_core::audio::{
    NoteKind, aram_map, directory, note_name, port_messages, sample_tuning, timeline, voices,
};
use romlens_core::explain::sound::{describe_dsp, dsp_layout};
use romlens_core::recording::{MachineStateSource, RomrecSource, SpcState, StateRegion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    Voices,
    Dsp,
    Map,
    Samples,
    Timeline,
    Ports,
}

pub struct ApuArgs<'a> {
    pub what: What,
    pub rec: &'a Path,
    pub frame: Option<u64>,
    /// `A..B`, inclusive.
    pub frames: Option<&'a str>,
    pub limit: usize,
}

fn open(path: &Path) -> Result<RomrecSource> {
    let rec = RomrecSource::open(path).with_context(|| format!("opening {}", path.display()))?;
    if !rec.regions().contains(&StateRegion::Aram) {
        return Err(anyhow!(
            "{} has no sound side: record it again with this Romlens's recorder (romlens rec script)",
            path.display()
        ));
    }
    Ok(rec)
}

fn range(text: Option<&str>, count: u64) -> Result<(u64, u64)> {
    let last = count.saturating_sub(1);
    let Some(t) = text else { return Ok((0, last)) };
    let (a, b) = t
        .split_once("..")
        .ok_or_else(|| anyhow!("--frames is A..B, such as 10..20"))?;
    let a: u64 = if a.is_empty() { 0 } else { a.parse()? };
    let b: u64 = if b.is_empty() { last } else { b.parse()? };
    if a > b || b > last {
        return Err(anyhow!(
            "frames {a} to {b} are not in a recording of {count} frames (0 to {last})"
        ));
    }
    Ok((a, b))
}

pub fn run(a: ApuArgs) -> Result<()> {
    let rec = open(a.rec)?;
    let count = rec.frame_count().unwrap_or(0);
    let frame = a.frame.unwrap_or(count.saturating_sub(1));
    if frame >= count {
        return Err(anyhow!(
            "frame {frame} is past the end of the recording ({count} frames)"
        ));
    }
    match a.what {
        What::Voices => print_voices(&rec, frame),
        What::Dsp => print_dsp(&rec, frame),
        What::Map => print_map(&rec, frame),
        What::Samples => print_samples(&rec, frame),
        What::Timeline => {
            let (from, to) = range(a.frames, count)?;
            print_timeline(&rec, from, to, a.limit)
        }
        What::Ports => {
            let (from, to) = range(a.frames, count)?;
            print_ports(&rec, from, to, a.limit)
        }
    }
}

struct Frame {
    aram: Vec<u8>,
    dsp: Vec<u8>,
    spc: SpcState,
}

fn frame_state(rec: &RomrecSource, frame: u64) -> Result<Frame> {
    let s = rec.state_at(frame)?;
    let get = |r: StateRegion| {
        s.region(r)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| anyhow!("frame {frame} has no {} region", r.name()))
    };
    Ok(Frame {
        aram: get(StateRegion::Aram)?,
        dsp: get(StateRegion::DspRegisters)?,
        spc: SpcState::decode(&get(StateRegion::SpcState)?),
    })
}

fn tuning_words(aram: &[u8], start: u16, loop_at: u16, pitch: u16) -> String {
    match sample_tuning(aram, start, loop_at) {
        Some(hz) => {
            let at = hz * pitch as f64 / 4096.0;
            format!("≈ {} ({at:.0} Hz, estimated from the loop)", note_name(at))
        }
        None => "unknown (no loop, or a loop with no wave in it)".to_owned(),
    }
}

fn print_voices(rec: &RomrecSource, frame: u64) -> Result<()> {
    let f = frame_state(rec, frame)?;
    let v = voices(&f.dsp, Some(&f.aram));
    println!(
        "frame {frame}: {} of 8 voices sounding",
        v.iter().filter(|v| v.sounding()).count()
    );
    for v in &v {
        let flags: Vec<&str> = [
            (v.echo, "echo"),
            (v.noise, "noise"),
            (v.modulated, "pitch modulated"),
            (v.keyed_off, "keyed off"),
            (v.ended, "sample ended"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, t)| *t)
        .collect();
        let state = if v.sounding() {
            format!("envelope {}/127, sample now {}", v.envx, v.outx)
        } else {
            "silent".to_owned()
        };
        if !v.sounding() && v.pitch == 0 && v.volume == (0, 0) {
            println!("\nvoice {}  silent, not set up", v.index);
            continue;
        }
        println!("\nvoice {}  {state}", v.index);
        let sample = match v.sample {
            Some((start, lp)) => format!("sample {} at ${start:04X}, loop ${lp:04X}", v.source),
            None => format!("sample {}", v.source),
        };
        println!("  {sample}");
        println!("  pitch ${:04X}: {}", v.pitch, v.pitch_words());
        if let Some((start, lp)) = v.sample.filter(|_| v.pitch != 0) {
            println!("  note {}", tuning_words(&f.aram, start, lp, v.pitch));
        }
        println!("  volume {} left, {} right", v.volume.0, v.volume.1);
        println!("  {}", v.envelope());
        if !flags.is_empty() {
            println!("  {}", flags.join(", "));
        }
    }
    Ok(())
}

fn print_dsp(rec: &RomrecSource, frame: u64) -> Result<()> {
    let f = frame_state(rec, frame)?;
    println!("frame {frame}: the DSP's registers");
    let global = |r: u8| r & 0x0F >= 0x0C;
    for (title, regs) in [
        (
            "global",
            (0..0x80u8).filter(|r| global(*r)).collect::<Vec<_>>(),
        ),
        ("voices", (0..0x80u8).filter(|r| !global(*r)).collect()),
    ] {
        println!("\n{title}:");
        for r in regs {
            let voice = (r & 0xF0) as usize;
            if title == "voices" && f.dsp[voice..voice + 10].iter().all(|b| *b == 0) {
                if r & 0xF == 0 {
                    println!("  ${r:02X}  V{}: all zero", r >> 4);
                }
                continue;
            }
            let l = dsp_layout(r);
            if l.data && l.about.starts_with("Not used") {
                continue;
            }
            println!(
                "  ${r:02X}  {}",
                describe_dsp(r, Some(f.dsp[r as usize])).parts[0].short()
            );
        }
    }
    Ok(())
}

fn print_map(rec: &RomrecSource, frame: u64) -> Result<()> {
    let f = frame_state(rec, frame)?;
    let used: Vec<u8> = voices(&f.dsp, None).iter().map(|v| v.source).collect();
    println!("frame {frame}: audio RAM, the SPC700 at ${:04X}", f.spc.pc);
    for p in aram_map(&f.aram, &f.dsp, &f.spc, &used, &[]) {
        let end = p.start as u32 + p.len - 1;
        println!(
            "  ${:04X}-${end:04X}  {:>6} bytes  {}",
            p.start, p.len, p.label
        );
    }
    Ok(())
}

fn print_samples(rec: &RomrecSource, frame: u64) -> Result<()> {
    let f = frame_state(rec, frame)?;
    let dir = f.dsp[0x5D];
    let used: Vec<u8> = voices(&f.dsp, None).iter().map(|v| v.source).collect();
    let d = directory(&f.aram, dir, &used);
    println!(
        "frame {frame}: the sample directory at ${:04X} (DIR = ${dir:02X}), {} sample{}",
        (dir as u16) << 8,
        d.len(),
        if d.len() == 1 { "" } else { "s" }
    );
    for e in &d {
        let lp = if e.loops {
            format!("loops at ${:04X}", e.loop_at)
        } else {
            "stops at its end".to_owned()
        };
        println!(
            "  {:>3}  ${:04X}  {} blocks, {} bytes, {} samples; {lp}; at $1000 {}",
            e.index,
            e.start,
            e.blocks,
            e.blocks * 9,
            e.blocks * 16,
            tuning_words(&f.aram, e.start, e.loop_at, 0x1000)
        );
    }
    Ok(())
}

fn print_timeline(rec: &RomrecSource, from: u64, to: u64, limit: usize) -> Result<()> {
    let t = timeline(rec, from, to)?;
    println!("frames {from} to {to}: {} note events", t.len());
    // Each frame's audio RAM, fetched once, for the notes' tunings.
    let mut arams: HashMap<u64, Vec<u8>> = HashMap::new();
    for e in t.iter().take(limit) {
        let aram = match arams.get(&e.frame) {
            Some(a) => a,
            None => {
                let a = rec.region_at(e.frame, StateRegion::Aram)?;
                arams.entry(e.frame).or_insert(a)
            }
        };
        let dsp = rec.region_at(e.frame, StateRegion::DspRegisters)?;
        let entry = (dsp[0x5D] as usize) << 8 | (e.source as usize * 4);
        let w = |i: usize| {
            u16::from_le_bytes([aram[(entry + i) & 0xFFFF], aram[(entry + i + 1) & 0xFFFF]])
        };
        let what = match e.kind {
            NoteKind::Off => String::new(),
            _ => format!(
                "  pitch ${:04X}  sample {}  {}",
                e.pitch,
                e.source,
                tuning_words(aram, w(0), w(2), e.pitch)
            ),
        };
        println!(
            "  frame {:>5}  cycle {:>10}  voice {}  {:<5}{what}",
            e.frame,
            e.spc_cycle,
            e.voice,
            e.kind.name()
        );
    }
    if t.len() > limit {
        println!("  … {} more (--limit)", t.len() - limit);
    }
    Ok(())
}

fn print_ports(rec: &RomrecSource, from: u64, to: u64, limit: usize) -> Result<()> {
    let mut all = Vec::new();
    for frame in from..=to {
        if let Some(e) = rec.apu_events(frame)? {
            all.extend(port_messages(&e));
        }
    }
    let commands = all.iter().filter(|m| m.from_cpu).count();
    println!(
        "frames {from} to {to}: {} port writes, {commands} from the S-CPU and {} from the SPC700",
        all.len(),
        all.len() - commands
    );
    for m in all.iter().take(limit) {
        let who = if m.from_cpu {
            "S-CPU  → port"
        } else {
            "SPC700 → port"
        };
        println!(
            "  frame {:>5}  cycle {:>10}  {who} {}  ${:02X}",
            m.frame, m.spc_cycle, m.port, m.value
        );
    }
    if all.len() > limit {
        println!("  … {} more (--limit)", all.len() - limit);
    }
    Ok(())
}
