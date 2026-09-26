//! The SPC700's code explained (docs/23, A3): each write to the DSP or to
//! an I/O register decoded, with the DSP register and the value followed
//! through the instructions before it, and the loops a driver waits in
//! named.
//!
//! A write to the DSP is two steps: pick the register with `DSPADDR`
//! (`$F2`), then write `DSPDATA` (`$F3`). Drivers do it as
//! `MOV $F2,#$4C` / `MOV $F3,A`, or in one instruction as `MOVW $F2,YA`
//! (A is the register, Y the value). The pass keeps A, X, Y and the
//! register picked while they are known, within a run of instructions with
//! no branch into it.

use std::collections::{BTreeMap, BTreeSet};

use super::fields::RegisterWrite;
use super::sound::{describe_dsp, describe_spc_io, duration, timer_period_ms};
use crate::spc700::aram::Walk;
use crate::spc700::decode::{Flow, Instruction, Value, decode_at};
use crate::spc700::opcodes::{Arg, Mnemonic};

/// A write the pass explained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpcWrite {
    /// The instruction.
    pub at: u16,
    /// To the DSP (the register is `write.address`), or to an I/O register.
    pub dsp: bool,
    pub write: RegisterWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpcIdiomKind {
    /// A loop reading a timer's count until it ticks.
    TimerWait,
    /// A loop reading a port until the S-CPU writes it.
    PortWait,
}

impl SpcIdiomKind {
    pub const fn name(self) -> &'static str {
        match self {
            SpcIdiomKind::TimerWait => "timer-wait",
            SpcIdiomKind::PortWait => "port-wait",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpcIdiom {
    pub kind: SpcIdiomKind,
    /// The loop's first and last instructions.
    pub start: u16,
    pub end: u16,
    pub title: String,
    pub summary: String,
    pub why: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpcExplanations {
    pub writes: BTreeMap<u16, SpcWrite>,
    pub idioms: Vec<SpcIdiom>,
}

impl SpcExplanations {
    /// The idiom starting at `at`.
    pub fn idiom_at(&self, at: u16) -> Option<&SpcIdiom> {
        self.idioms.iter().find(|i| i.start == at)
    }
}

const WHY_TIMER: &str = "The SPC700 has no interrupts, so a driver keeps time by reading a timer's count. It sets a divider once, then loops until the count moves, and does one tick of the music: every note, envelope and pitch slide moves on tick by tick.";
const WHY_PORT: &str = "The four ports are the only link between the two CPUs. The driver waits here until the S-CPU writes a new byte, which is how a game asks for a song or a sound effect, and how the upload of the driver itself is paced.";

#[derive(Debug, Clone, Copy, Default)]
struct Regs {
    a: Option<u8>,
    x: Option<u8>,
    y: Option<u8>,
    dspaddr: Option<u8>,
}

/// The direct-page or absolute address an operand names in page 0.
fn io_of(insn: &Instruction, i: usize) -> Option<u16> {
    match insn.info.args[i] {
        Arg::Dp | Arg::Abs | Arg::DpBit(_) => {
            insn.address(i, false).filter(|a| (0xF0..=0xFF).contains(a))
        }
        _ => None,
    }
}

fn source(insn: &Instruction, r: &Regs) -> Option<u8> {
    match (insn.info.args[1], insn.values[1]) {
        (Arg::Imm, Value::Byte(v)) => Some(v),
        (Arg::A, _) => r.a,
        (Arg::X, _) => r.x,
        (Arg::Y, _) => r.y,
        _ => None,
    }
}

/// Which of A, X and Y an instruction changes.
fn clobbers(insn: &Instruction) -> (bool, bool, bool) {
    use Mnemonic::*;
    let m = insn.mnemonic();
    let args = insn.info.args;
    let dest = |r: Arg| args[0] == r && !matches!(m, Cmp | Push);
    let ya = args[0] == Arg::Ya && !matches!(m, Cmpw) || matches!(m, Mul | Div);
    (
        dest(Arg::A) || ya || args[1] == Arg::IndXInc && m == Mov && args[0] == Arg::A,
        dest(Arg::X) || args.contains(&Arg::IndXInc) || m == Div,
        dest(Arg::Y) || ya,
    )
}

fn step(insn: &Instruction, r: &mut Regs) {
    use Mnemonic::*;
    let m = insn.mnemonic();
    let args = insn.info.args;
    let imm = match insn.values[1] {
        Value::Byte(v) if args[1] == Arg::Imm => Some(v),
        _ => None,
    };
    let before = *r;
    let (ca, cx, cy) = clobbers(insn);
    if ca {
        r.a = None;
    }
    if cx {
        r.x = None;
    }
    if cy {
        r.y = None;
    }
    match (m, args[0], args[1]) {
        (Mov, Arg::A, Arg::Imm) => r.a = imm,
        (Mov, Arg::X, Arg::Imm) => r.x = imm,
        (Mov, Arg::Y, Arg::Imm) => r.y = imm,
        (Mov, Arg::A, Arg::X) => r.a = before.x,
        (Mov, Arg::A, Arg::Y) => r.a = before.y,
        (Mov, Arg::X, Arg::A) => r.x = before.a,
        (Mov, Arg::Y, Arg::A) => r.y = before.a,
        (Inc, Arg::A, _) => r.a = before.a.map(|v| v.wrapping_add(1)),
        (Inc, Arg::X, _) => r.x = before.x.map(|v| v.wrapping_add(1)),
        (Inc, Arg::Y, _) => r.y = before.y.map(|v| v.wrapping_add(1)),
        (Dec, Arg::A, _) => r.a = before.a.map(|v| v.wrapping_sub(1)),
        (Dec, Arg::X, _) => r.x = before.x.map(|v| v.wrapping_sub(1)),
        (Dec, Arg::Y, _) => r.y = before.y.map(|v| v.wrapping_sub(1)),
        _ => {}
    }
    if matches!(insn.flow(), Flow::Call(_)) {
        *r = Regs::default();
    }
}

/// Explain the code a walk found in `image`.
pub fn explain_spc(image: &[u8], walk: &Walk) -> SpcExplanations {
    // Where the known values stop: every place control can arrive other
    // than from the instruction before.
    let mut joins: BTreeSet<u16> = walk.routines.clone();
    for &at in &walk.starts {
        match decode_at(image, at).flow() {
            Flow::Branch(t) | Flow::Jump(Some(t)) => {
                joins.insert(t);
            }
            _ => {}
        }
    }
    let mut out = SpcExplanations::default();
    let mut dividers: [Option<u8>; 3] = [None; 3];
    let mut r = Regs::default();
    let mut expect: Option<u16> = None;
    for &at in &walk.starts {
        if joins.contains(&at) || expect != Some(at) {
            r = Regs::default();
        }
        let insn = decode_at(image, at);
        expect = Some(insn.next());
        let m = insn.mnemonic();
        let writes_dest = !matches!(
            m,
            Mnemonic::Cmp
                | Mnemonic::Cmpw
                | Mnemonic::Cbne
                | Mnemonic::Dbnz
                | Mnemonic::Bbs
                | Mnemonic::Bbc
        );
        if m == Mnemonic::Movw && insn.info.args[1] == Arg::Ya && io_of(&insn, 0) == Some(0xF2) {
            // MOVW $F2,YA: A picks the register, Y is written to it.
            r.dspaddr = r.a;
            if let Some(reg) = r.a {
                out.writes.insert(
                    at,
                    SpcWrite {
                        at,
                        dsp: true,
                        write: describe_dsp(reg, r.y),
                    },
                );
            }
        } else if writes_dest && let Some(io) = io_of(&insn, 0) {
            let value = if m == Mnemonic::Mov {
                source(&insn, &r)
            } else {
                None
            };
            match io {
                0xF2 => {
                    r.dspaddr = match m {
                        Mnemonic::Mov => value,
                        Mnemonic::Inc => r.dspaddr.map(|v| v.wrapping_add(1)),
                        Mnemonic::Dec => r.dspaddr.map(|v| v.wrapping_sub(1)),
                        _ => None,
                    };
                    if let Some(w) = describe_spc_io(0xF2, r.dspaddr) {
                        out.writes.insert(
                            at,
                            SpcWrite {
                                at,
                                dsp: false,
                                write: w,
                            },
                        );
                    }
                }
                0xF3 => {
                    let w = match r.dspaddr {
                        Some(reg) => SpcWrite {
                            at,
                            dsp: true,
                            write: describe_dsp(reg, value),
                        },
                        None => SpcWrite {
                            at,
                            dsp: false,
                            write: describe_spc_io(0xF3, value).unwrap(),
                        },
                    };
                    out.writes.insert(at, w);
                }
                0xFD..=0xFF => {}
                _ => {
                    if let (0xFA..=0xFC, Some(v)) = (io, value) {
                        dividers[(io - 0xFA) as usize] = Some(v);
                    }
                    if let Some(w) = describe_spc_io(io, value) {
                        out.writes.insert(
                            at,
                            SpcWrite {
                                at,
                                dsp: false,
                                write: w,
                            },
                        );
                    }
                }
            }
        }
        step(&insn, &mut r);
    }
    out.idioms = waits(image, walk, &dividers);
    out
}

/// Short loops that read a timer or a port until it changes.
fn waits(image: &[u8], walk: &Walk, dividers: &[Option<u8>; 3]) -> Vec<SpcIdiom> {
    let mut out = Vec::new();
    for &at in &walk.starts {
        let insn = decode_at(image, at);
        let target = match insn.flow() {
            Flow::Branch(t) | Flow::Jump(Some(t)) => t,
            _ => continue,
        };
        if target > at || at - target > 8 || !walk.starts.contains(&target) {
            continue;
        }
        // The loop's instructions, and the I/O registers they read.
        let mut reads = BTreeSet::new();
        let mut a = target;
        while a <= at {
            let i = decode_at(image, a);
            let reads_first = matches!(
                i.mnemonic(),
                Mnemonic::Cmp | Mnemonic::Cbne | Mnemonic::Bbs | Mnemonic::Bbc
            );
            for n in 0..2 {
                if (n == 1 || reads_first)
                    && let Some(io) = io_of(&i, n)
                {
                    reads.insert(io);
                }
            }
            a = i.next();
        }
        if let Some(&t) = reads.iter().find(|r| (0xFD..=0xFF).contains(*r)) {
            let n = (t - 0xFD) as u8;
            let every = dividers[n as usize]
                .map(|d| {
                    format!(
                        ", every {} (T{n}DIV = {d})",
                        duration(timer_period_ms(n, d))
                    )
                })
                .unwrap_or_default();
            out.push(SpcIdiom {
                kind: SpcIdiomKind::TimerWait,
                start: target,
                end: at,
                title: format!("Wait for timer {n}"),
                summary: format!("Reads T{n}OUT until timer {n} ticks{every}."),
                why: WHY_TIMER,
            });
        } else if let Some(&p) = reads.iter().find(|r| (0xF4..=0xF7).contains(*r)) {
            let n = p - 0xF4;
            // Looping while the reads differ waits for them to agree;
            // looping while they are equal waits for a new byte.
            let summary = match insn.mnemonic() {
                Mnemonic::Bne | Mnemonic::Cbne => format!(
                    "Reads CPUIO{n} until two reads in a row agree, so a byte read just as the S-CPU writes it is not trusted."
                ),
                Mnemonic::Beq => {
                    format!("Reads CPUIO{n} until the S-CPU writes a new byte to port {n}.")
                }
                _ => format!("Reads CPUIO{n} until the S-CPU writes port {n}."),
            };
            out.push(SpcIdiom {
                kind: SpcIdiomKind::PortWait,
                start: target,
                end: at,
                title: "Wait for the S-CPU".to_owned(),
                summary,
                why: WHY_PORT,
            });
        }
    }
    out
}
