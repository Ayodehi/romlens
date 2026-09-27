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
use romlens_core::model::spc_log::{SpcAccess, SpcLog};
use romlens_core::recording::{MachineStateSource, RomrecSource, SpcState, StateRegion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    Voices,
    Dsp,
    Map,
    Samples,
    Timeline,
    Ports,
    Replay,
}

pub struct ApuArgs<'a> {
    pub what: What,
    pub rec: &'a Path,
    pub frame: Option<u64>,
    /// `A..B`, inclusive.
    pub frames: Option<&'a str>,
    pub limit: usize,
    /// The SPC700's execution log; `<recording>.spc.mxlog` when there is one.
    pub log: Option<&'a Path>,
    /// `replay`: run on from the first frame.
    pub free: bool,
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

/// The SPC700's log given, or the one beside the recording.
fn spc_log(rec: &Path, given: Option<&Path>) -> Result<Option<SpcLog>> {
    let beside = rec.with_extension("spc.mxlog");
    let path = match given {
        Some(p) => p.to_path_buf(),
        None if beside.exists() => beside,
        None => return Ok(None),
    };
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let log = romlens_core::io::import::spc_log::read(&bytes, None)
        .with_context(|| format!("reading {}", path.display()))?;
    println!(
        "with the SPC700's execution log {}",
        path.file_name().unwrap_or_default().to_string_lossy()
    );
    Ok(Some(log))
}

pub fn run(a: ApuArgs) -> Result<()> {
    let rec = open(a.rec)?;
    let log = match a.what {
        What::Map | What::Samples => spc_log(a.rec, a.log)?,
        _ => None,
    };
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
        What::Map => print_map(&rec, frame, log.as_ref()),
        What::Samples => print_samples(&rec, frame, log.as_ref()),
        What::Timeline => {
            let (from, to) = range(a.frames, count)?;
            print_timeline(&rec, from, to, a.limit)
        }
        What::Ports => {
            let (from, to) = range(a.frames, count)?;
            print_ports(&rec, from, to, a.limit)
        }
        What::Replay => {
            let (from, to) = range(a.frames, count)?;
            print_replay(&rec, from, to, a.free, a.limit)
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

fn print_map(rec: &RomrecSource, frame: u64, log: Option<&SpcLog>) -> Result<()> {
    let f = frame_state(rec, frame)?;
    let used: Vec<u8> = voices(&f.dsp, None).iter().map(|v| v.source).collect();
    println!("frame {frame}: audio RAM, the SPC700 at ${:04X}", f.spc.pc);
    for p in aram_map(&f.aram, &f.dsp, &f.spc, &used, &[], log) {
        let end = p.start as u32 + p.len - 1;
        println!(
            "  ${:04X}-${end:04X}  {:>6} bytes  {}",
            p.start, p.len, p.label
        );
    }
    Ok(())
}

fn print_samples(rec: &RomrecSource, frame: u64, log: Option<&SpcLog>) -> Result<()> {
    let played = log.map(|l| l.touched(SpcAccess::DspRead));
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
        let heard = match &played {
            Some(p) if p.contains(&e.start) => "played; ",
            Some(_) => "not played while logged; ",
            None => "",
        };
        println!(
            "  {:>3}  ${:04X}  {} blocks, {} bytes, {} samples; {lp}; {heard}at $1000 {}",
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

fn print_replay(rec: &RomrecSource, from: u64, to: u64, free: bool, limit: usize) -> Result<()> {
    use romlens_core::apu::replay::{check_frames, run_free};
    let checks = if free {
        run_free(rec, from, to)?
    } else {
        check_frames(rec, from, to)?
    };
    let boot = checks.iter().filter(|c| c.boot).count();
    let good = checks.iter().filter(|c| c.matches()).count();
    let at_end = checks
        .iter()
        .filter(|c| c.differs_only_at_the_end())
        .count();
    let skew = checks.iter().map(|c| c.io_skew).max().unwrap_or(0);
    let writes: usize = checks.iter().map(|c| c.io_writes.1).sum();
    println!(
        "frames {} to {to}, {}: {} frames",
        if free { from + 1 } else { from.max(1) },
        if free {
            format!("run on from frame {from}'s snapshot")
        } else {
            "each from the snapshot before it".to_owned()
        },
        checks.len(),
    );
    println!("  {good} match");
    println!(
        "  {at_end} match but for the registers or a byte at the frame's end, where Mesen stopped inside an instruction"
    );
    if boot > 0 {
        let last = checks
            .iter()
            .filter(|c| c.boot)
            .map(|c| c.frame)
            .max()
            .unwrap_or(0);
        println!(
            "  {boot} in the boot ROM (the last at frame {last}), not compared: Romlens's boot program is its own, so its instructions are not Nintendo's"
        );
    }
    println!("  {} differ", checks.len() - good - at_end - boot);
    println!("  {writes} I/O writes by Mesen; where ours agree, at most {skew} cycles apart");
    let n = checks.len() - boot;
    let of = |pick: &dyn Fn(u8) -> bool| {
        checks
            .iter()
            .filter(|c| !c.boot)
            .map(|c| c.dsp_own.iter().filter(|r| pick(r.0)).count())
            .sum::<usize>()
    };
    let (envx, outx, endx) = (
        of(&|r| r != 0x7C && r & 0xF == 8),
        of(&|r| r & 0xF == 9),
        of(&|r| r == 0x7C),
    );
    println!(
        "  the DSP, started from rest: ENVX {} of {} match, OUTX {} of {}, ENDX {} of {n}",
        8 * n - envx,
        8 * n,
        8 * n - outx,
        8 * n,
        n - endx
    );
    let echo: usize = checks.iter().map(|c| c.echo_bytes).sum();
    if echo > 0 {
        println!(
            "  {} frames with echo buffer bytes that differ ({echo} bytes in all), compared apart: the DSP writes them, from a place in the buffer a snapshot does not hold",
            checks.iter().filter(|c| c.echo_bytes > 0).count()
        );
    }
    if free {
        let a = romlens_core::apu::replay::note_agreement(rec, from, to)?;
        println!(
            "  notes: {} of the recording's {} key-ons played by Romlens too (same voice, pitch and sample within a frame), of {} it made{}",
            a.matched,
            a.recorded,
            a.ours,
            a.first_miss.map_or(String::new(), |f| format!(
                "; the first missed at frame {f}"
            ))
        );
    }
    let differ: Vec<_> = checks
        .iter()
        .filter(|c| !c.boot && !c.matches() && !c.differs_only_at_the_end())
        .collect();
    for c in differ.iter().take(limit) {
        let mut parts = c.registers.clone();
        if !c.aram.is_empty() {
            let first: Vec<String> = c.aram.iter().take(4).map(|a| format!("${a:04X}")).collect();
            parts.push(format!(
                "{} audio RAM bytes ({}…)",
                c.aram.len(),
                first.join(" ")
            ));
        }
        if !c.dsp.is_empty() {
            let regs: Vec<String> = c.dsp.iter().map(|r| format!("${r:02X}")).collect();
            parts.push(format!("DSP {}", regs.join(" ")));
        }
        if c.io_writes.0 != c.io_writes.1 {
            parts.push(format!(
                "{} I/O writes, Mesen {}",
                c.io_writes.0, c.io_writes.1
            ));
        }
        if let Some(i) = c.io_mismatch {
            parts.push(format!("I/O write {i} differs"));
        }
        if c.io_skew > 0 {
            parts.push(format!(
                "I/O writes up to {} cycles from Mesen's",
                c.io_skew
            ));
        }
        println!("  frame {:>5}  {}", c.frame, parts.join("; "));
    }
    if differ.len() > limit {
        println!("  … {} more (--limit)", differ.len() - limit);
    }
    Ok(())
}

/// `apu render`: the sound from a frame, described but never written out.
pub fn render(path: &Path, frame: u64, seconds: f64, follow: bool) -> Result<()> {
    use romlens_core::apu::render::{SAMPLE_RATE, render};
    let rec = open(path)?;
    let count = rec.frame_count().unwrap_or(0);
    if frame >= count {
        return Err(anyhow!(
            "frame {frame} is past the end of the recording ({count} frames)"
        ));
    }
    if !(0.0..=600.0).contains(&seconds) {
        return Err(anyhow!("--seconds is 0 to 600"));
    }
    let n = (seconds * SAMPLE_RATE as f64) as usize;
    let out = render(&rec, frame, n, follow)?;
    println!(
        "from frame {frame}, {:.2} s: {} samples at 32 kHz, {}",
        out.len() as f64 / SAMPLE_RATE as f64,
        out.len(),
        if follow {
            "the S-CPU's port writes following the recording"
        } else {
            "the driver on its own"
        }
    );
    describe(&out);
    Ok(())
}

/// The digest, the levels and each voice of rendered samples.
pub fn describe(out: &[romlens_core::dsp::Frame]) {
    use romlens_core::apu::render::digest;
    println!("  digest {}", digest(out));
    let peak = |v: &mut dyn Iterator<Item = i16>| v.map(|x| (x as i32).abs()).max().unwrap_or(0);
    let rms = |v: &mut dyn Iterator<Item = i16>| {
        let (s, k) = v.fold((0f64, 0usize), |(s, k), x| (s + (x as f64).powi(2), k + 1));
        if k == 0 { 0.0 } else { (s / k as f64).sqrt() }
    };
    println!(
        "  left peak {}, RMS {:.0}; right peak {}, RMS {:.0} (of 32767)",
        peak(&mut out.iter().map(|f| f.left)),
        rms(&mut out.iter().map(|f| f.left)),
        peak(&mut out.iter().map(|f| f.right)),
        rms(&mut out.iter().map(|f| f.right)),
    );
    for v in 0..8 {
        let p = peak(&mut out.iter().map(|f| f.voices[v]));
        let sounding = out.iter().filter(|f| f.voices[v] != 0).count();
        if p == 0 {
            println!("  voice {v}  silent");
        } else {
            println!(
                "  voice {v}  peak {p}, sounding {:.0}% of the time",
                100.0 * sounding as f64 / out.len().max(1) as f64
            );
        }
    }
}
