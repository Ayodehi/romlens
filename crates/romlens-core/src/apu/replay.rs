//! Running the SPC700 beside a recording (docs/23, A7): from a frame's
//! snapshot, the S-CPU's port writes fed in on the SPC700's clock as Mesen
//! recorded them, then everything compared with the next frame's snapshot:
//! the registers, audio RAM, the DSP's registers and every write the
//! SPC700 made to its I/O registers.
//!
//! Two things shape the comparison:
//! - The DSP is not emulated yet, so the registers it changes by itself
//!   (each voice's ENVX and OUTX, and ENDX) are left out.
//! - Mesen runs the SPC700 a cycle at a time and stops it at the frame's
//!   end wherever it is, often inside an instruction, and does not say
//!   where. Its registers are then those from before that instruction (or
//!   partly after it), its PC somewhere past the opcode. So a snapshot
//!   partway through our last instruction counts as a match, and a
//!   replay starts from each instruction the snapshot could be inside,
//!   keeping the one that goes on to match.

use super::{Apu, IoWrite, Spc700, Undo};
use crate::recording::apu::{ApuEvent, ApuEventKind};
use crate::recording::{MachineStateSource, RecordingError, SpcState, StateRegion};
use crate::spc700::decode_at;

/// How one frame compares.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrameCheck {
    pub frame: u64,
    /// Registers that differ at the frame's end: `"PC $1234 want $1236"`.
    pub registers: Vec<String>,
    /// Audio RAM addresses that differ.
    pub aram: Vec<u16>,
    /// DSP registers that differ, the DSP's own left out.
    pub dsp: Vec<u8>,
    /// The SPC700's I/O writes, ours and the recording's.
    pub io_writes: (usize, usize),
    /// The first I/O write that differs in register or value, by index.
    pub io_mismatch: Option<usize>,
    /// The largest difference in cycle between our I/O writes and the
    /// recording's, where they agree in register and value.
    pub io_skew: u64,
    /// Where the replay started, when the snapshot was inside an
    /// instruction: that instruction's address.
    pub started_inside: Option<u16>,
    /// Mesen stopped partway through our last instruction.
    pub ended_inside: bool,
}

impl FrameCheck {
    pub fn matches(&self) -> bool {
        self.io_skew == 0
            && self.registers.is_empty()
            && self.aram.is_empty()
            && self.dsp.is_empty()
            && self.io_mismatch.is_none()
            && self.io_writes.0 == self.io_writes.1
    }

    /// Everything the SPC700 did this frame matches: its I/O writes, the
    /// DSP's registers and audio RAM but for a byte or two. What differs is
    /// the registers (or a byte) at the very end, where Mesen stopped
    /// inside an instruction in a way the comparison cannot line up.
    pub fn differs_only_at_the_end(&self) -> bool {
        !self.matches()
            && self.io_skew == 0
            && self.io_mismatch.is_none()
            && self.io_writes.0 == self.io_writes.1
            && self.dsp.is_empty()
            && self.aram.len() <= 2
    }

    fn differences(&self) -> usize {
        self.registers.len()
            + self.aram.len()
            + self.dsp.len()
            + self.io_writes.0.abs_diff(self.io_writes.1)
            + self.io_mismatch.is_some() as usize
            + (self.io_skew > 0) as usize
    }
}

/// Whether the DSP changes a register by itself.
fn dsp_owned(r: u8) -> bool {
    matches!(r & 0xF, 8 | 9) && r < 0x80 || r == 0x7C
}

struct Snapshot {
    aram: Vec<u8>,
    dsp: Vec<u8>,
    spc: SpcState,
}

fn snapshot(src: &dyn MachineStateSource, frame: u64) -> Result<Snapshot, RecordingError> {
    let s = src.state_at(frame)?;
    let get = |r: StateRegion| {
        s.region(r)
            .map(<[u8]>::to_vec)
            .ok_or(RecordingError::MissingRegion(r.name()))
    };
    Ok(Snapshot {
        spc: SpcState::decode(&get(StateRegion::SpcState)?),
        aram: get(StateRegion::Aram)?,
        dsp: get(StateRegion::DspRegisters)?,
    })
}

/// A frame's events that fall after its snapshot: Mesen records each in
/// the frame the S-CPU was in, but the SPC700 had not reached it when the
/// snapshot was taken, so they belong to the next one.
fn after_snapshot(
    src: &dyn MachineStateSource,
    frame: u64,
    cycle: u64,
) -> Result<Vec<ApuEvent>, RecordingError> {
    Ok(src
        .apu_events(frame)?
        .map(|e| e.events)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| carries(e, cycle))
        .collect())
}

/// Whether an event belongs after a snapshot at `cycle`. The S-CPU's write
/// on the snapshot's own cycle may not have landed in it yet (Mesen calls
/// the recorder before the write takes effect, and the frame can end
/// between the two), and writing a port again with the same byte changes
/// nothing, so it goes after.
fn carries(e: &ApuEvent, cycle: u64) -> bool {
    match e.kind {
        ApuEventKind::CpuPort => e.spc_cycle >= cycle,
        ApuEventKind::SpcIo => e.spc_cycle > cycle,
    }
}

/// Run to frame `frame`'s snapshot, the S-CPU's port writes landing on
/// their cycles, and compare. `carried` holds the events from before that
/// fall in this frame, and gets those of this frame that fall in the next.
fn run_frame(
    apu: &mut Apu,
    src: &dyn MachineStateSource,
    frame: u64,
    want: &Snapshot,
    carried: &mut Vec<ApuEvent>,
) -> Result<FrameCheck, RecordingError> {
    let target = want.spc.cycle;
    let mut events = std::mem::take(carried);
    events.extend(src.apu_events(frame)?.map(|e| e.events).unwrap_or_default());
    // Writes our last instruction made past the frame before's end belong
    // to this one, as Mesen's do.
    apu.bus.io_writes.get_or_insert_with(Vec::new);
    let mut theirs: Vec<IoWrite> = Vec::new();
    for e in events {
        if carries(&e, target) {
            carried.push(e);
            continue;
        }
        match e.kind {
            ApuEventKind::CpuPort => apu.queue_port(e.spc_cycle, e.address, e.value),
            ApuEventKind::SpcIo => theirs.push(IoWrite {
                cycle: e.spc_cycle,
                register: e.address,
                value: e.value,
            }),
        }
    }
    // The last instruction, kept undoable: Mesen may be partway through it.
    let mut before = apu.cpu;
    apu.bus.undo = Some(Vec::new());
    while apu.bus.cycle < target {
        before = apu.cpu;
        apu.bus.undo.as_mut().unwrap().clear();
        apu.cpu.step(&mut apu.bus);
    }
    let undo = apu.bus.undo.take().unwrap_or_default();
    let (ours, later): (Vec<IoWrite>, Vec<IoWrite>) = apu
        .bus
        .io_writes
        .take()
        .unwrap_or_default()
        .into_iter()
        .partition(|w| w.cycle <= target);
    apu.bus.io_writes = Some(later);

    let mut c = FrameCheck {
        frame,
        io_writes: (ours.len(), theirs.len()),
        ..FrameCheck::default()
    };
    let lens = (
        decode_at(&apu.bus.aram, before.pc).len() as u16,
        decode_at(&apu.bus.aram, apu.cpu.pc).len() as u16,
    );
    compare_registers(&mut c, &before, &apu.cpu, &want.spc, lens);
    // A byte differs unless it matches either side of the last instruction.
    let pre = |a: usize, now: u8| {
        undo.iter()
            .rev()
            .find_map(|u| match *u {
                Undo::Aram(at, old) if at as usize == a => Some(old),
                _ => None,
            })
            .unwrap_or(now)
    };
    c.aram = (0..0x10000usize)
        .filter(|&a| {
            let now = apu.bus.aram[a];
            now != want.aram[a] && pre(a, now) != want.aram[a]
        })
        .map(|a| a as u16)
        .collect();
    let pre_dsp = |r: u8, now: u8| {
        undo.iter()
            .rev()
            .find_map(|u| match *u {
                Undo::Dsp(at, old) if at == r => Some(old),
                _ => None,
            })
            .unwrap_or(now)
    };
    c.dsp = (0..0x80u8)
        .filter(|&r| {
            let now = apu.bus.dsp[r as usize];
            let w = want.dsp[r as usize];
            !dsp_owned(r) && now != w && pre_dsp(r, now) != w
        })
        .collect();
    for (i, (o, t)) in ours.iter().zip(&theirs).enumerate() {
        if (o.register, o.value) != (t.register, t.value) {
            c.io_mismatch = Some(i);
            break;
        }
        c.io_skew = c.io_skew.max(o.cycle.abs_diff(t.cycle));
    }
    Ok(c)
}

/// The registers against the snapshot: each may match its value after
/// our last instruction or, if Mesen stopped inside it, before. Mesen's
/// clock and ours can also put its stop a cycle into the next instruction:
/// the registers as after ours, the PC past the next opcode. `lens` are
/// the two instructions' lengths.
fn compare_registers(
    c: &mut FrameCheck,
    before: &Spc700,
    after: &Spc700,
    want: &SpcState,
    lens: (u16, u16),
) {
    let inside = |pc: u16| {
        (1..=lens.0).contains(&pc.wrapping_sub(before.pc))
            || (1..=lens.1).contains(&pc.wrapping_sub(after.pc))
    };
    let mut mid = false;
    for (name, pre, post, w) in [
        ("A", before.a, after.a, want.a),
        ("X", before.x, after.x, want.x),
        ("Y", before.y, after.y, want.y),
        ("SP", before.sp, after.sp, want.sp),
        ("PSW", before.psw, after.psw, want.psw),
    ] {
        if post != w {
            if pre == w {
                mid = true;
            } else {
                c.registers
                    .push(format!("{name} ${post:02X} want ${w:02X}"));
            }
        }
    }
    if after.pc != want.pc {
        if inside(want.pc) {
            mid = true;
        } else {
            c.registers
                .push(format!("PC ${:04X} want ${:04X}", after.pc, want.pc));
        }
    }
    c.ended_inside = mid;
}

/// Where a replay can start from a snapshot: at its PC, or at one of the
/// one to three bytes before it that start an instruction long enough to
/// have fetched up to it, with the clock wound back by each number of
/// cycles that instruction could already have run.
fn start_candidates(s: &Snapshot) -> Vec<(u16, u8)> {
    let pc = s.spc.pc;
    let mut out = vec![(pc, 0)];
    for back in 1..=3u16 {
        let at = pc.wrapping_sub(back);
        let insn = decode_at(&s.aram, at);
        if insn.len() as u16 >= back {
            // A branch taken runs two cycles more.
            let most = insn.info.cycles + 2;
            out.extend((back as u8..most).map(|k| (at, k)));
        }
    }
    out
}

/// The machine at a snapshot, the SPC700 at `pc` and its clock `back`
/// cycles earlier, the timers' phases with it.
fn start(s: &Snapshot, (pc, back): (u16, u8)) -> Apu {
    let mut apu = Apu::from_snapshot(&s.aram, &s.dsp, &s.spc);
    apu.cpu.pc = pc;
    apu.bus.cycle -= back as u64;
    for (t, period) in apu.bus.io.timers.iter_mut().zip(super::TIMER_PERIODS) {
        if t.phase >= back {
            t.phase -= back;
        } else {
            // It ticked in those cycles; the count before it is not known
            // exactly, so only the phase goes back.
            t.phase = t.phase + period - back;
        }
    }
    apu
}

/// Each frame of `from..=to` on its own: from the snapshot before it,
/// trying each instruction the snapshot could be inside of.
pub fn check_frames(
    src: &dyn MachineStateSource,
    from: u64,
    to: u64,
) -> Result<Vec<FrameCheck>, RecordingError> {
    let mut out = Vec::new();
    for frame in from.max(1)..=to {
        let (prev, want) = (snapshot(src, frame - 1)?, snapshot(src, frame)?);
        let carried = after_snapshot(src, frame - 1, prev.spc.cycle)?;
        let mut best: Option<FrameCheck> = None;
        for at in start_candidates(&prev) {
            let mut apu = start(&prev, at);
            let mut c = run_frame(&mut apu, src, frame, &want, &mut carried.clone())?;
            if at.0 != prev.spc.pc {
                c.started_inside = Some(at.0);
            }
            let better = best
                .as_ref()
                .is_none_or(|b| c.differences() < b.differences());
            if better {
                let done = c.matches();
                best = Some(c);
                if done {
                    break;
                }
            }
        }
        out.push(best.unwrap());
    }
    Ok(out)
}

/// From the snapshot at `from`, running on without resetting to the
/// recording, each frame's check until `to`. The start is the candidate
/// that matches the first frame best.
pub fn run_free(
    src: &dyn MachineStateSource,
    from: u64,
    to: u64,
) -> Result<Vec<FrameCheck>, RecordingError> {
    let first = snapshot(src, from)?;
    let mut carried = after_snapshot(src, from, first.spc.cycle)?;
    let mut apu: Option<Apu> = None;
    let mut best = usize::MAX;
    if from < to {
        let want = snapshot(src, from + 1)?;
        for at in start_candidates(&first) {
            let a = start(&first, at);
            let mut probe = a.clone();
            let d =
                run_frame(&mut probe, src, from + 1, &want, &mut carried.clone())?.differences();
            if d < best {
                best = d;
                apu = Some(a);
            }
        }
    }
    let mut apu = apu.unwrap_or_else(|| start(&first, (first.spc.pc, 0)));
    let mut out = Vec::new();
    for frame in from + 1..=to {
        let want = snapshot(src, frame)?;
        let c = run_frame(&mut apu, src, frame, &want, &mut carried)?;
        // The DSP's own registers come from the recording, as it plays.
        for r in (0..0x80u8).filter(|&r| dsp_owned(r)) {
            apu.bus.dsp[r as usize] = want.dsp[r as usize];
        }
        out.push(c);
    }
    Ok(out)
}
