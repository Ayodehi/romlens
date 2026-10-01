//! Running the SPC700 beside a recording (docs/23, A7): from a frame's
//! snapshot, the S-CPU's port writes fed in on the SPC700's clock as Mesen
//! recorded them, then everything compared with the next frame's snapshot:
//! the registers, audio RAM, the DSP's registers and every write the
//! SPC700 made to its I/O registers.
//!
//! Two things shape the comparison:
//! - The registers the DSP changes by itself (each voice's ENVX and OUTX,
//!   and ENDX) are compared apart, since the DSP's inside is not in a
//!   snapshot and a replay starts it from rest.
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

/// The most SPC700 cycles a replay runs from one snapshot to the next: a
/// frame is about 17,000 (20,500 on a PAL machine), so this is several
/// frames' worth. A larger gap is a damaged recording, not a slow frame,
/// and running it out would take hours.
pub const MAX_FRAME_CYCLES: u64 = 1 << 17;

/// Refuse a run from `cycle` to `target` longer than `frames` frames can
/// be. `frame` is the frame it ends at, for the message.
fn within(cycle: u64, target: u64, frames: u64, frame: u64) -> Result<(), RecordingError> {
    if target.saturating_sub(cycle) > MAX_FRAME_CYCLES.saturating_mul(frames.max(1)) {
        return Err(RecordingError::Corrupt(format!(
            "frame {frame}'s SPC700 clock jumps from {cycle} to {target} cycles"
        )));
    }
    Ok(())
}

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
    /// The registers the DSP sets itself (each voice's ENVX and OUTX, and
    /// ENDX) that differ. Its inside is not in a snapshot, so a replay
    /// starts it from rest: these match only once every voice has been
    /// keyed on since.
    /// Each as (register, ours, the recording's).
    pub dsp_own: Vec<(u8, u8, u8)>,
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
    /// Audio RAM bytes in the echo buffer that differ: the DSP writes
    /// them itself, and where it is in the buffer is not in a snapshot, so
    /// they are compared apart, as ENVX and OUTX are.
    pub echo_bytes: usize,
    /// The SPC700 was in the boot ROM at either end of the frame: not
    /// compared, since Romlens's boot program is its own and the
    /// instructions at `$FFC0–$FFFF` are not Nintendo's.
    pub boot: bool,
}

impl FrameCheck {
    pub fn matches(&self) -> bool {
        !self.boot
            && self.io_skew == 0
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
        !self.boot
            && !self.matches()
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
    inside: Option<crate::recording::DspInside>,
}

/// In the boot ROM: its page mapped and the SPC700 in it.
fn in_boot(s: &Snapshot) -> bool {
    s.spc.rom_enabled && s.spc.pc >= crate::spc700::aram::IPL_START
}

fn boot_check(frame: u64) -> FrameCheck {
    FrameCheck {
        frame,
        boot: true,
        ..FrameCheck::default()
    }
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
        inside: s
            .region(StateRegion::DspInside)
            .map(crate::recording::DspInside::decode),
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
    within(apu.bus.cycle, target, 1, frame)?;
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
    let echo = |d: &[u8]| {
        let start = (d[0x6D] as usize) << 8;
        let edl = d[0x7D] as usize & 0xF;
        start..start + if edl == 0 { 4 } else { edl * 2048 }
    };
    let (echo_a, echo_b) = (echo(&want.dsp), echo(&apu.bus.dsp));
    let in_echo = |a: usize| {
        [&echo_a, &echo_b]
            .iter()
            .any(|r| r.contains(&a) || r.contains(&(a + 0x10000)))
    };
    let differ: Vec<usize> = (0..0x10000usize)
        .filter(|&a| {
            let now = apu.bus.aram[a];
            now != want.aram[a] && pre(a, now) != want.aram[a]
        })
        .collect();
    c.echo_bytes = differ.iter().filter(|a| in_echo(**a)).count();
    c.aram = differ
        .into_iter()
        .filter(|a| !in_echo(*a))
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
    c.dsp_own = (0..0x80u8)
        .filter(|&r| dsp_owned(r) && apu.bus.dsp[r as usize] != want.dsp[r as usize])
        .map(|r| (r, apu.bus.dsp[r as usize], want.dsp[r as usize]))
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
    if let Some(inside) = &s.inside {
        apu.bus.resume_dsp(inside);
        apu.bus.dsp_hold = back;
    }
    apu.cpu.pc = pc;
    apu.bus.cycle = apu.bus.cycle.saturating_sub(back as u64);
    for (t, period) in apu.bus.io.timers.iter_mut().zip(super::TIMER_PERIODS) {
        if t.phase >= back {
            t.phase -= back;
        } else {
            // It ticked in those cycles; the count before it is not known
            // exactly, so only the phase goes back.
            // A recorded phase can be out of range; widen so it cannot
            // overflow.
            t.phase = (u16::from(t.phase) + u16::from(period))
                .saturating_sub(u16::from(back))
                .min(0xFF) as u8;
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
        if in_boot(&prev) || in_boot(&want) {
            out.push(boot_check(frame));
            continue;
        }
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
    let mut out = Vec::new();
    // Started (again) from `seed`'s snapshot: at `from`, and after any
    // stretch in the boot ROM, where Romlens's boot program cannot follow
    // Nintendo's instruction by instruction.
    let mut apu: Option<Apu> = None;
    let mut carried = Vec::new();
    for frame in from + 1..=to {
        let prev = snapshot(src, frame - 1)?;
        let want = snapshot(src, frame)?;
        if in_boot(&want) || (apu.is_none() && in_boot(&prev)) {
            out.push(boot_check(frame));
            apu = None;
            continue;
        }
        let a = match apu.as_mut() {
            Some(a) => a,
            None => {
                carried = after_snapshot(src, frame - 1, prev.spc.cycle)?;
                apu.insert(best_of(src, frame, &prev, &want, &carried)?)
            }
        };
        out.push(run_frame(a, src, frame, &want, &mut carried)?);
    }
    Ok(out)
}

/// The machine at `prev`'s snapshot, started from whichever instruction it
/// could be inside that runs frame `frame` closest to the recording.
fn best_of(
    src: &dyn MachineStateSource,
    frame: u64,
    prev: &Snapshot,
    want: &Snapshot,
    carried: &[ApuEvent],
) -> Result<Apu, RecordingError> {
    let mut best: Option<(usize, Apu)> = None;
    for at in start_candidates(prev) {
        let a = start(prev, at);
        let mut probe = a.clone();
        let d = run_frame(&mut probe, src, frame, want, &mut carried.to_vec())?.differences();
        if best.as_ref().is_none_or(|(b, _)| d < *b) {
            best = Some((d, a));
        }
    }
    Ok(best
        .map(|b| b.1)
        .unwrap_or_else(|| start(prev, (prev.spc.pc, 0))))
}

/// The machine at `frame`'s snapshot, ready to run on: Mesen stops the
/// SPC700 inside an instruction at a frame's end, so it starts from the
/// instruction that runs the next frame as the recording did, when there
/// is a next frame, and from the snapshot's program counter when not.
pub fn start_at(src: &dyn MachineStateSource, frame: u64) -> Result<Apu, RecordingError> {
    let prev = snapshot(src, frame)?;
    let last = src.frame_count().unwrap_or(0);
    if frame + 1 >= last || in_boot(&prev) {
        return Ok(start(&prev, (prev.spc.pc, 0)));
    }
    let want = snapshot(src, frame + 1)?;
    // A next frame whose clock is out of reach says nothing about where
    // to start: start from the snapshot's program counter.
    if within(prev.spc.cycle, want.spc.cycle, 1, frame + 1).is_err() {
        return Ok(start(&prev, (prev.spc.pc, 0)));
    }
    let carried = after_snapshot(src, frame, prev.spc.cycle)?;
    best_of(src, frame + 1, &prev, &want, &carried)
}

/// Frame `frame`'s I/O writes by the SPC700, each with the address of the
/// instruction that made it: the frame run again from the snapshot before
/// it, from each instruction the snapshot could be inside, keeping the run
/// whose writes agree with the recording's the longest (Mesen stopping
/// inside an instruction can leave the last write or two on either side
/// of a frame's end). `None` for frame 0, which has no snapshot before
/// it.
pub fn frame_writers(
    src: &dyn MachineStateSource,
    frame: u64,
) -> Result<Option<Vec<(IoWrite, u16)>>, RecordingError> {
    if frame == 0 {
        return Ok(None);
    }
    let prev = snapshot(src, frame - 1)?;
    let want = snapshot(src, frame)?;
    let target = want.spc.cycle;
    within(prev.spc.cycle, target, 1, frame)?;
    let mut events = after_snapshot(src, frame - 1, prev.spc.cycle)?;
    events.extend(src.apu_events(frame)?.map(|e| e.events).unwrap_or_default());
    let events: Vec<ApuEvent> = events.into_iter().filter(|e| !carries(e, target)).collect();
    let theirs: Vec<(u8, u8)> = events
        .iter()
        .filter(|e| e.kind == ApuEventKind::SpcIo)
        .map(|e| (e.address, e.value))
        .collect();
    let mut best: Option<(usize, Vec<(IoWrite, u16)>)> = None;
    for candidate in start_candidates(&prev) {
        let mut apu = start(&prev, candidate);
        for e in events.iter().filter(|e| e.kind == ApuEventKind::CpuPort) {
            apu.queue_port(e.spc_cycle, e.address, e.value);
        }
        apu.bus.io_writes = Some(Vec::new());
        let mut out = Vec::new();
        while apu.bus.cycle < target {
            let pc = apu.cpu.pc;
            apu.cpu.step(&mut apu.bus);
            let log = apu.bus.io_writes.as_mut().unwrap();
            out.extend(log.drain(..).map(|w| (w, pc)));
        }
        let agree = out
            .iter()
            .zip(&theirs)
            .take_while(|((w, _), t)| (w.register, w.value) == **t)
            .count();
        if agree == theirs.len() && agree == out.len() {
            return Ok(Some(out));
        }
        if best.as_ref().is_none_or(|(n, _)| agree > *n) {
            best = Some((agree, out));
        }
    }
    Ok(best.map(|(_, w)| w))
}

/// How well the notes Romlens's machine plays agree with the recording's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoteAgreement {
    /// Key-ons the recording has in the frames.
    pub recorded: usize,
    /// Key-ons Romlens's machine made, run from the first frame's snapshot
    /// with the recording's port writes.
    pub ours: usize,
    /// Of the recorded, those ours matched: the same voice, pitch and
    /// sample within a frame's time.
    pub matched: usize,
    /// The first recorded key-on ours did not match, by frame.
    pub first_miss: Option<u64>,
}

/// Run Romlens's machine from `from`'s snapshot, following the recording's
/// port writes, to the end of `to`, and match its key-ons to the
/// recording's in order.
pub fn note_agreement(
    src: &dyn MachineStateSource,
    from: u64,
    to: u64,
) -> Result<NoteAgreement, RecordingError> {
    use super::player::{CYCLES_PER_FRAME, Player};
    use crate::audio::{NoteKind, timeline};
    let recorded: Vec<_> = timeline(src, from + 1, to)?
        .into_iter()
        .filter(|n| n.kind == NoteKind::On)
        .collect();
    let end = snapshot(src, to)?.spc.cycle;
    within(
        snapshot(src, from)?.spc.cycle,
        end,
        to.saturating_sub(from),
        to,
    )?;
    let mut p = Player::from_recording(src, from, true)?;
    p.log_notes();
    while p.apu.bus.cycle < end {
        p.render(1024);
    }
    let ours: Vec<_> = p
        .notes()
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| n.kind == NoteKind::On && n.spc_cycle <= end)
        .collect();
    let window = CYCLES_PER_FRAME as u64;
    let mut used = vec![false; ours.len()];
    let mut out = NoteAgreement {
        recorded: recorded.len(),
        ours: ours.len(),
        ..NoteAgreement::default()
    };
    for r in &recorded {
        let hit = ours.iter().enumerate().position(|(i, o)| {
            !used[i]
                && o.voice == r.voice
                && o.pitch == r.pitch
                && o.source == r.source
                && o.spc_cycle.abs_diff(r.spc_cycle) <= window
        });
        match hit {
            Some(i) => {
                used[i] = true;
                out.matched += 1;
            }
            None => {
                out.first_miss.get_or_insert(r.frame);
            }
        }
    }
    Ok(out)
}
