//! Which value each store to the hardware writes (docs/20).
//!
//! A forward pass over one routine's blocks, like the decompiler's decimal
//! flag: each byte of A, X and Y is a constant, a byte loaded from a known
//! address, or unknown. Blocks meet by keeping what agrees, and the pass
//! repeats in reverse post-order until nothing changes. Calls and anything
//! it does not model make what they change unknown, so what it says is
//! never a guess.

use crate::cpu65816::{AddressingMode, Instruction, Mnemonic, TargetKind, register_for};
use crate::decompile::cfg::Cfg;
use crate::decompile::function::Function;
use crate::memory::address::{FileOffset, SnesAddress};
use crate::memory::map::MemoryClass;
use crate::model::hardware_register;
use crate::rom::image::RomImage;

/// One byte of a register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Byte {
    Known(u8),
    /// Loaded, unchanged, from this address in RAM (or anywhere not ROM).
    From(SnesAddress),
    Unknown,
}

impl Byte {
    fn join(self, other: Byte) -> Byte {
        if self == other { self } else { Byte::Unknown }
    }

    pub fn known(self) -> Option<u8> {
        match self {
            Byte::Known(v) => Some(v),
            _ => None,
        }
    }
}

/// A, X and Y, low byte first, and the bytes pushed within reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub a: [Byte; 2],
    pub x: [Byte; 2],
    pub y: [Byte; 2],
    /// Bytes pushed and not yet pulled, last pushed last. Only what this
    /// routine pushed; a pull past it is unknown.
    stack: Vec<Byte>,
}

const U: Byte = Byte::Unknown;
const STACK_LIMIT: usize = 16;

impl State {
    /// A routine's entry: nothing known.
    pub fn entry() -> State {
        State {
            a: [U; 2],
            x: [U; 2],
            y: [U; 2],
            stack: Vec::new(),
        }
    }

    fn join(&self, other: &State) -> State {
        let j = |a: [Byte; 2], b: [Byte; 2]| [a[0].join(b[0]), a[1].join(b[1])];
        State {
            a: j(self.a, other.a),
            x: j(self.x, other.x),
            y: j(self.y, other.y),
            stack: if self.stack == other.stack {
                self.stack.clone()
            } else {
                Vec::new()
            },
        }
    }

    /// The 8- or 16-bit value of a register, when every byte is known.
    pub fn value(bytes: &[Byte]) -> Option<u32> {
        bytes
            .iter()
            .rev()
            .try_fold(0u32, |acc, b| Some(acc << 8 | u32::from(b.known()?)))
    }
}

/// A store to a hardware register and what it writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    pub offset: FileOffset,
    /// The register's bank offset (`$2100`), after any known index.
    pub register: u16,
    /// 1 or 2.
    pub width: u8,
    /// The bytes written, low first; only `width` of them count.
    pub bytes: [Byte; 2],
    /// Indexed by a register the pass does not know, so `register` is only
    /// the base.
    pub indexed: bool,
}

impl Store {
    pub fn value(&self) -> Option<u32> {
        if self.indexed {
            return None;
        }
        State::value(&self.bytes[..self.width as usize])
    }

    /// Where an unknown value came from: the address it was loaded from, if
    /// every byte came unchanged from consecutive addresses.
    pub fn source(&self) -> Option<SnesAddress> {
        let Byte::From(first) = self.bytes[0] else {
            return None;
        };
        for (i, b) in self.bytes[..self.width as usize].iter().enumerate().skip(1) {
            if *b != Byte::From(plus(first, i as u16)) {
                return None;
            }
        }
        Some(first)
    }
}

/// The pass over one routine.
pub struct Values {
    /// The state before each step of the function, where the pass reached.
    pub before: Vec<Option<State>>,
    pub stores: Vec<Store>,
}

pub fn values(rom: &RomImage, f: &Function, cfg: &Cfg) -> Values {
    let n = cfg.blocks.len();
    let mut at_entry: Vec<Option<State>> = vec![None; n];
    at_entry[cfg.entry] = Some(State::entry());
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 64 {
        changed = false;
        rounds += 1;
        for &b in &cfg.rpo {
            let Some(mut s) = at_entry[b].clone() else {
                continue;
            };
            for i in cfg.blocks[b].steps.clone() {
                s = after(rom, &s, &f.steps[i].insn);
            }
            for succ in cfg.succs(b) {
                let new = match &at_entry[succ] {
                    None => s.clone(),
                    Some(old) => old.join(&s),
                };
                if at_entry[succ].as_ref() != Some(&new) {
                    at_entry[succ] = Some(new);
                    changed = true;
                }
            }
        }
    }

    let mut before = vec![None; f.steps.len()];
    let mut stores = Vec::new();
    for (b, block) in cfg.blocks.iter().enumerate() {
        let Some(mut s) = at_entry[b].clone() else {
            continue;
        };
        for i in block.steps.clone() {
            let insn = &f.steps[i].insn;
            if let Some(st) = store(insn, &s) {
                stores.push(st);
            }
            before[i] = Some(s.clone());
            s = after(rom, &s, insn);
        }
    }
    stores.sort_by_key(|s| s.offset.0);
    Values { before, stores }
}

/// The hardware register an instruction reads or writes, with whether an
/// unknown index is involved. Covers the direct page too, for code that
/// points D at `$2100`.
pub fn hardware_target(insn: &Instruction, s: &State) -> Option<(u16, bool)> {
    use AddressingMode::*;
    let t = insn.target.filter(|t| t.kind == TargetKind::Data)?;
    let direct = matches!(insn.mode, Direct | DirectX | DirectY)
        && insn.flags_before.dp.is_some()
        && hardware_register(t.address.offset()).is_some()
        && crate::model::is_system_bank(t.address.bank());
    if register_for(insn).is_none() && !direct {
        return None;
    }
    let base = t.address.offset();
    let index = match insn.mode {
        AbsoluteX | AbsoluteLongX | DirectX => Some(s.x),
        AbsoluteY | DirectY => Some(s.y),
        _ => None,
    };
    // The decoder's target is the base; an index moves it.
    match index {
        None => Some((base, false)),
        Some(ix) => match State::value(&ix[..if insn.flags_before.x { 1 } else { 2 }]) {
            Some(v) => {
                let a = base.wrapping_add(v as u16);
                hardware_register(a).map(|_| (a, false))
            }
            None => Some((base, true)),
        },
    }
}

fn store(insn: &Instruction, s: &State) -> Option<Store> {
    use Mnemonic::*;
    let (bytes, wide) = match insn.mnemonic {
        STA => (s.a, !insn.flags_before.m),
        STX => (s.x, !insn.flags_before.x),
        STY => (s.y, !insn.flags_before.x),
        STZ => ([Byte::Known(0); 2], !insn.flags_before.m),
        _ => return None,
    };
    let (register, indexed) = hardware_target(insn, s)?;
    Some(Store {
        offset: insn.file_offset,
        register,
        width: if wide { 2 } else { 1 },
        bytes,
        indexed,
    })
}

/// The bytes a load reads: constants from ROM, the address otherwise.
fn load(rom: &RomImage, insn: &Instruction, wide: bool) -> [Byte; 2] {
    use AddressingMode::*;
    let n = if wide { 2 } else { 1 };
    let mut out = [U; 2];
    if insn.mode.is_immediate() {
        let v = insn.operand.value();
        out[0] = Byte::Known(v as u8);
        if wide {
            out[1] = Byte::Known((v >> 8) as u8);
        }
        return out;
    }
    let direct = matches!(insn.mode, Absolute | AbsoluteLong | Direct);
    let Some(t) = insn
        .target
        .filter(|t| direct && t.kind == TargetKind::Data)
    else {
        return out;
    };
    if !t.certain {
        // An absolute address below $2000 is low RAM in every bank a data
        // bank register normally holds, so where it came from is known even
        // when the bank is not; its value never is.
        if insn.mode == Absolute && u32::from(t.address.offset()) + n as u32 <= 0x2000 {
            for (i, slot) in out.iter_mut().enumerate().take(n) {
                *slot = Byte::From(SnesAddress::new(0x7E, t.address.offset() + i as u16));
            }
        }
        return out;
    }
    for (i, slot) in out.iter_mut().enumerate().take(n) {
        let a = plus(t.address, i as u16);
        *slot = match rom.map().classify(a) {
            MemoryClass::Rom => rom
                .file_offset_for(a)
                .and_then(|o| rom.bytes().get(o.0 as usize))
                .map_or(U, |b| Byte::Known(*b)),
            // Reading a register is not a value that stays put.
            MemoryClass::Hardware => U,
            _ => Byte::From(a),
        };
    }
    out
}

/// The state after `insn`.
pub fn after(rom: &RomImage, s: &State, insn: &Instruction) -> State {
    use Mnemonic::*;
    let m8 = insn.flags_before.m;
    let x8 = insn.flags_before.x;
    let mut o = s.clone();
    let acc = insn.mode == AddressingMode::Accumulator;
    // Set A to `v`, keeping the high byte (B) when A is 8-bit.
    let set_a = |o: &mut State, v: [Byte; 2]| {
        o.a[0] = v[0];
        if !m8 {
            o.a[1] = v[1];
        }
    };
    let set_index = |v: [Byte; 2]| -> [Byte; 2] { if x8 { [v[0], Byte::Known(0)] } else { v } };
    let arith = |bytes: [Byte; 2], wide: bool, f: &dyn Fn(u32) -> u32| -> [Byte; 2] {
        let n = if wide { 2 } else { 1 };
        match State::value(&bytes[..n]) {
            Some(v) => {
                let r = f(v) & if wide { 0xFFFF } else { 0xFF };
                [Byte::Known(r as u8), Byte::Known((r >> 8) as u8)]
            }
            None => [U; 2],
        }
    };
    match insn.mnemonic {
        LDA => set_a(&mut o, load(rom, insn, !m8)),
        LDX => o.x = set_index(load(rom, insn, !x8)),
        LDY => o.y = set_index(load(rom, insn, !x8)),
        TAX => o.x = set_index(s.a),
        TAY => o.y = set_index(s.a),
        TXY => o.y = set_index(s.x),
        TYX => o.x = set_index(s.y),
        TXA => set_a(&mut o, s.x),
        TYA => set_a(&mut o, s.y),
        // These always move 16 bits.
        TDC => {
            let d = insn.flags_before.dp;
            o.a = d.map_or([U; 2], |d| {
                [Byte::Known(d as u8), Byte::Known((d >> 8) as u8)]
            });
        }
        TSC => o.a = [U; 2],
        TSX => o.x = set_index([U; 2]),
        XBA => o.a = [s.a[1], s.a[0]],
        INX => o.x = set_index(arith(s.x, !x8, &|v| v.wrapping_add(1))),
        DEX => o.x = set_index(arith(s.x, !x8, &|v| v.wrapping_sub(1))),
        INY => o.y = set_index(arith(s.y, !x8, &|v| v.wrapping_add(1))),
        DEY => o.y = set_index(arith(s.y, !x8, &|v| v.wrapping_sub(1))),
        INC if acc => set_a(&mut o, arith(s.a, !m8, &|v| v.wrapping_add(1))),
        DEC if acc => set_a(&mut o, arith(s.a, !m8, &|v| v.wrapping_sub(1))),
        ASL if acc => set_a(&mut o, arith(s.a, !m8, &|v| v << 1)),
        LSR if acc => set_a(&mut o, arith(s.a, !m8, &|v| v >> 1)),
        ROL | ROR if acc => set_a(&mut o, [U; 2]),
        AND | ORA | EOR if insn.mode.is_immediate() => {
            let k = insn.operand.value();
            let f: &dyn Fn(u32) -> u32 = match insn.mnemonic {
                AND => &move |v| v & k,
                ORA => &move |v| v | k,
                _ => &move |v| v ^ k,
            };
            set_a(&mut o, arith(s.a, !m8, f));
        }
        ADC | SBC | AND | ORA | EOR => set_a(&mut o, [U; 2]),
        PHA => push(&mut o, &s.a[..if m8 { 1 } else { 2 }]),
        PHX => push(&mut o, &s.x[..if x8 { 1 } else { 2 }]),
        PHY => push(&mut o, &s.y[..if x8 { 1 } else { 2 }]),
        PHB => push(&mut o, &[insn.flags_before.dbr.map_or(U, Byte::Known)]),
        PHK => push(&mut o, &[Byte::Known(insn.address.bank())]),
        PHD => {
            let d = insn.flags_before.dp;
            push(
                &mut o,
                &d.map_or([U; 2], |d| {
                    [Byte::Known(d as u8), Byte::Known((d >> 8) as u8)]
                }),
            );
        }
        PHP => push(&mut o, &[U]),
        PEA => {
            let v = insn.operand.value();
            push(&mut o, &[Byte::Known(v as u8), Byte::Known((v >> 8) as u8)]);
        }
        PEI | PER => push(&mut o, &[U; 2]),
        PLA => {
            let v = pull(&mut o, if m8 { 1 } else { 2 });
            set_a(&mut o, v);
        }
        PLX => o.x = set_index(pull(&mut o, if x8 { 1 } else { 2 })),
        PLY => o.y = set_index(pull(&mut o, if x8 { 1 } else { 2 })),
        PLB | PLP => {
            pull(&mut o, 1);
        }
        PLD => {
            pull(&mut o, 2);
        }
        MVN | MVP => {
            o.a = [Byte::Known(0xFF); 2];
            o.x = [U; 2];
            o.y = [U; 2];
        }
        JSR | JSL | COP | BRK => {
            o.a = [U; 2];
            o.x = [U; 2];
            o.y = [U; 2];
        }
        TCS => o.stack.clear(),
        _ => {}
    }
    // An 8-bit X or Y has a high byte of zero, whatever set it.
    if insn.flags_after.x {
        o.x[1] = Byte::Known(0);
        o.y[1] = Byte::Known(0);
    }
    o
}

/// Push bytes, high byte first as the CPU does, so the low byte is on top.
fn push(o: &mut State, bytes: &[Byte]) {
    for b in bytes.iter().rev() {
        o.stack.push(*b);
    }
    if o.stack.len() > STACK_LIMIT {
        let extra = o.stack.len() - STACK_LIMIT;
        o.stack.drain(..extra);
    }
}

fn pull(o: &mut State, n: usize) -> [Byte; 2] {
    let mut out = [U; 2];
    for slot in out.iter_mut().take(n) {
        *slot = o.stack.pop().unwrap_or(U);
    }
    out
}

/// `a` moved on `n` bytes within its bank.
fn plus(a: SnesAddress, n: u16) -> SnesAddress {
    SnesAddress::new(a.bank(), a.offset().wrapping_add(n))
}
