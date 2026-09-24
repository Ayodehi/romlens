//! The sequences every SNES game contains, named where they appear
//! (docs/20): waiting for blanking, DMA and HDMA, the hardware multiplier,
//! clearing memory, the sound CPU's handshake, decimal arithmetic, and a
//! routine with a second way in.
//!
//! Each recogniser works on one routine's blocks and the values
//! [`super::values`] found, and names only what it can see: a DMA whose
//! count is set somewhere else still reads as a DMA, with the count "set
//! elsewhere", never a guess.

use std::collections::{BTreeSet, HashMap};

use super::values::{self, Byte, State, Store, Values};
use crate::cpu65816::{AddressingMode, Instruction, Mnemonic, TargetKind};
use crate::decompile::cfg::{Cfg, Term};
use crate::decompile::function::Function;
use crate::decompile::function::{Callee, Transfer};
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::hardware_register;
use crate::rom::image::RomImage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IdiomKind {
    /// A loop polling HVBJOY, RDNMI or TIMEUP, or a `WAI`.
    Wait,
    Dma,
    Hdma,
    Multiply,
    Divide,
    /// A loop storing one value through an index, or `MVN`/`MVP`.
    ClearMemory,
    BlockMove,
    ApuHandshake,
    Decimal,
    /// A routine that runs on into another routine's entry.
    SharedEntry,
}

impl IdiomKind {
    pub fn as_str(self) -> &'static str {
        match self {
            IdiomKind::Wait => "wait",
            IdiomKind::Dma => "dma",
            IdiomKind::Hdma => "hdma",
            IdiomKind::Multiply => "multiply",
            IdiomKind::Divide => "divide",
            IdiomKind::ClearMemory => "clear-memory",
            IdiomKind::BlockMove => "block-move",
            IdiomKind::ApuHandshake => "apu-handshake",
            IdiomKind::Decimal => "decimal",
            IdiomKind::SharedEntry => "shared-entry",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Idiom {
    pub kind: IdiomKind,
    /// "Wait for vertical blank".
    pub title: String,
    /// What it does here, in its values.
    pub summary: String,
    /// Why SNES games do this.
    pub why: &'static str,
    /// The instructions it spans, ascending.
    pub offsets: Vec<FileOffset>,
    /// Where its note goes, when not the first of `offsets`.
    pub note_at: Option<FileOffset>,
}

impl Idiom {
    /// Where its note goes.
    pub fn first(&self) -> FileOffset {
        self.note_at.unwrap_or(self.offsets[0])
    }
}

/// Names an address for a summary: `Brightness ($7E:0DAE)` or `$7E:0DAE`.
pub type Namer<'a> = &'a dyn Fn(SnesAddress) -> String;

/// Every idiom in one routine.
pub fn find(
    rom: &RomImage,
    f: &Function,
    cfg: &Cfg,
    v: &Values,
    entries: &BTreeSet<SnesAddress>,
    name: Namer,
) -> Vec<Idiom> {
    let r = Routine::new(rom, f, cfg, v);
    let mut out = Vec::new();
    for l in &cfg.loops {
        out.extend(r.wait_loop(&l.body));
        out.extend(r.apu_loop(&l.body));
        out.extend(r.clear_loop(l.header, &l.body, name));
    }
    out.extend(r.wai());
    out.extend(r.dma(name));
    out.extend(r.arithmetic(name));
    out.extend(r.block_moves(name));
    out.extend(r.decimal());
    out.extend(shared_entry(rom, f, entries, name));
    for i in &mut out {
        i.offsets.sort_by_key(|o| o.0);
        i.offsets.dedup();
        i.summary = capitalise(&i.summary);
    }
    out
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

struct Routine<'a> {
    rom: &'a RomImage,
    f: &'a Function,
    cfg: &'a Cfg,
    v: &'a Values,
    /// The block of each step.
    block: Vec<usize>,
    /// Stores by the step they are at.
    store_at: HashMap<usize, &'a Store>,
}

impl<'a> Routine<'a> {
    fn new(rom: &'a RomImage, f: &'a Function, cfg: &'a Cfg, v: &'a Values) -> Self {
        let mut block = vec![usize::MAX; f.steps.len()];
        for (b, blk) in cfg.blocks.iter().enumerate() {
            for i in blk.steps.clone() {
                block[i] = b;
            }
        }
        let store_at = v
            .stores
            .iter()
            .filter_map(|s| f.index_of(s.offset).map(|i| (i, s)))
            .collect();
        Routine {
            rom,
            f,
            cfg,
            v,
            block,
            store_at,
        }
    }

    fn insn(&self, i: usize) -> &'a Instruction {
        &self.f.steps[i].insn
    }

    fn state(&self, i: usize) -> Option<&'a State> {
        self.v.before[i].as_ref()
    }

    /// The hardware register step `i` reads or writes, if the index (if
    /// any) is known.
    fn register(&self, i: usize) -> Option<u16> {
        let s = self.state(i)?;
        values::hardware_target(self.insn(i), s)
            .filter(|(_, indexed)| !indexed)
            .map(|(r, _)| r)
    }

    /// Steps of a loop, in order.
    fn steps_of(&self, body: &[usize]) -> Vec<usize> {
        let mut v: Vec<usize> = body
            .iter()
            .flat_map(|&b| self.cfg.blocks[b].steps.clone())
            .collect();
        v.sort_unstable();
        v
    }

    /// The loop's exit branch: its step, and whether taking it stays in the
    /// loop.
    fn exit_branch(&self, body: &[usize]) -> Option<(usize, bool)> {
        let mut found = None;
        for &b in body {
            if let Term::Branch { taken, fall } = self.cfg.blocks[b].term {
                let (t_in, f_in) = (body.contains(&taken), body.contains(&fall));
                if t_in != f_in {
                    if found.is_some() {
                        return None;
                    }
                    found = Some((self.cfg.blocks[b].steps.end - 1, t_in));
                }
            }
        }
        found
    }

    /// The stores reaching step `at`, latest first: back through its
    /// block, then up through its predecessors as far as a call. Where
    /// paths meet, a register keeps the value they agree on.
    fn written_before(&self, at: usize) -> Written {
        let mut visited = BTreeSet::new();
        let mut budget = 256usize;
        self.walk_back(self.block[at], at, &mut visited, &mut budget, 0)
    }

    fn walk_back(
        &self,
        b: usize,
        end: usize,
        visited: &mut BTreeSet<usize>,
        budget: &mut usize,
        depth: usize,
    ) -> Written {
        let mut w = Written {
            bytes: HashMap::new(),
            varies: BTreeSet::new(),
            stop: Stop::Earlier,
        };
        let start = self.cfg.blocks[b].steps.start;
        for i in (start..end).rev() {
            if *budget == 0 {
                return w;
            }
            *budget -= 1;
            if let Some(s) = self.store_at.get(&i)
                && !s.indexed
            {
                for k in 0..s.width {
                    let reg = s.register.wrapping_add(u16::from(k));
                    w.bytes
                        .entry(reg)
                        .or_insert((s.bytes[k as usize], s.offset));
                }
            }
            // A call may have written anything.
            if let Transfer::Call { callee, .. } = &self.f.steps[i].transfer {
                w.stop = Stop::Call(match callee {
                    Callee::Direct(a) => Some(*a),
                    _ => None,
                });
                return w;
            }
        }
        visited.insert(b);
        let preds: Vec<usize> = self.cfg.blocks[b]
            .preds
            .iter()
            .copied()
            .filter(|p| !visited.contains(p))
            .collect();
        if preds.is_empty() {
            if b == self.cfg.entry {
                w.stop = Stop::Entry;
            }
            return w;
        }
        if depth > 8 {
            return w;
        }
        let from: Vec<Written> = preds
            .iter()
            .map(|&p| {
                let mut seen = visited.clone();
                self.walk_back(
                    p,
                    self.cfg.blocks[p].steps.end,
                    &mut seen,
                    budget,
                    depth + 1,
                )
            })
            .collect();
        // What every way in agrees on.
        let own: BTreeSet<u16> = w.bytes.keys().copied().collect();
        let first = &from[0];
        for (&reg, &(byte, off)) in &first.bytes {
            if w.bytes.contains_key(&reg) {
                continue;
            }
            let all: Option<Vec<Byte>> = from
                .iter()
                .map(|x| x.bytes.get(&reg).map(|(b, _)| *b))
                .collect();
            if let Some(all) = all {
                if all.iter().all(|x| *x == byte) {
                    w.bytes.insert(reg, (byte, off));
                } else {
                    w.bytes.insert(reg, (Byte::Unknown, off));
                    w.varies.insert(reg);
                }
            }
        }
        // A register set on this block's own way down was not in doubt.
        for x in &from {
            for r in &x.varies {
                if !own.contains(r) && w.bytes.get(r).is_some_and(|(b, _)| *b == Byte::Unknown) {
                    w.varies.insert(*r);
                }
            }
        }
        w.stop = if from.iter().all(|x| x.stop == first.stop) {
            first.stop
        } else {
            Stop::Earlier
        };
        w
    }

    // ---- waiting ----

    fn wait_loop(&self, body: &[usize]) -> Option<Idiom> {
        let steps = self.steps_of(body);
        if steps.len() > 8 {
            return None;
        }
        let (br, taken_stays) = self.exit_branch(body)?;
        let (reg, bit, loops_while_set) = self.bit_test(br)?;
        let loops_while_set = loops_while_set == taken_stays;
        let until_set = !loops_while_set;
        let (title, why) = match (reg, bit, until_set) {
            (0x4212, 7, true) => ("Wait for vertical blank", WHY_VBLANK),
            (0x4212, 7, false) => ("Wait for vertical blank to end", WHY_VBLANK_END),
            (0x4212, 6, true) => ("Wait for horizontal blank", WHY_HBLANK),
            (0x4212, 6, false) => ("Wait for horizontal blank to end", WHY_HBLANK),
            (0x4212, 0, false) => ("Wait for the joypad auto-read to finish", WHY_JOYPAD),
            (0x4212, 0, true) => ("Wait for the joypad auto-read to start", WHY_JOYPAD),
            (0x4210, 7, true) => ("Wait for vertical blank (the NMI flag)", WHY_RDNMI),
            (0x4211, 7, true) => ("Wait for the timer IRQ", WHY_TIMEUP),
            _ => return None,
        };
        let name = hardware_register(reg).map_or("?", |r| r.name);
        Some(Idiom {
            kind: IdiomKind::Wait,
            title: title.to_owned(),
            summary: format!(
                "Reads {name} until bit {bit} is {}.",
                if until_set { "set" } else { "clear" }
            ),
            why,
            offsets: steps.iter().map(|&i| self.insn(i).file_offset).collect(),
            note_at: None,
        })
    }

    /// What the branch at step `br` tests: a register, a bit, and whether
    /// the branch is taken while the bit is set.
    fn bit_test(&self, br: usize) -> Option<(u16, u8, bool)> {
        use Mnemonic::*;
        let m = self.insn(br).mnemonic;
        let start = self.cfg.blocks[self.block[br]].steps.start;
        let mut mask: Option<u32> = None;
        for i in (start..br).rev() {
            let insn = self.insn(i);
            match insn.mnemonic {
                AND if insn.mode.is_immediate() && mask.is_none() => {
                    mask = Some(insn.operand.value());
                }
                LDA | LDX | LDY | BIT => {
                    let reg = self.register(i)?;
                    let wide = match insn.mnemonic {
                        LDA | BIT => !insn.flags_before.m,
                        _ => !insn.flags_before.x,
                    };
                    let immediate_mask = if insn.mnemonic == BIT {
                        self.state(i)
                            .and_then(|s| State::value(&s.a[..if wide { 2 } else { 1 }]))
                    } else {
                        None
                    };
                    let (bit, set) = match m {
                        BPL | BMI => (if wide { 15 } else { 7 }, m == BMI),
                        BVC | BVS if insn.mnemonic == BIT => (if wide { 14 } else { 6 }, m == BVS),
                        BEQ | BNE => {
                            let k = mask.or(immediate_mask)?;
                            if k.count_ones() != 1 {
                                return None;
                            }
                            (k.trailing_zeros(), m == BNE)
                        }
                        _ => return None,
                    };
                    // N after an AND reads the masked value's top bit.
                    if matches!(m, BPL | BMI) && mask.is_some_and(|k| k & (1 << bit) == 0) {
                        return None;
                    }
                    let (reg, bit) = if bit >= 8 {
                        (reg.wrapping_add(1), bit - 8)
                    } else {
                        (reg, bit)
                    };
                    return Some((reg, bit as u8, set));
                }
                NOP => {}
                _ => return None,
            }
        }
        None
    }

    fn wai(&self) -> Vec<Idiom> {
        (0..self.f.steps.len())
            .filter(|&i| self.insn(i).mnemonic == Mnemonic::WAI)
            .map(|i| Idiom {
                kind: IdiomKind::Wait,
                title: "Wait for an interrupt".to_owned(),
                summary: "WAI stops the CPU until the next NMI or IRQ.".to_owned(),
                why: WHY_WAI,
                offsets: vec![self.insn(i).file_offset],
                note_at: None,
            })
            .collect()
    }

    // ---- the sound CPU ----

    fn apu_loop(&self, body: &[usize]) -> Option<Idiom> {
        use Mnemonic::*;
        let steps = self.steps_of(body);
        if steps.len() > 8 {
            return None;
        }
        let (br, taken_stays) = self.exit_branch(body)?;
        let bm = self.insn(br).mnemonic;
        if !matches!(bm, BEQ | BNE) {
            return None;
        }
        let start = self.cfg.blocks[self.block[br]].steps.start;
        // The comparison that sets Z, and the port it reads.
        let mut port = None;
        let mut against: Option<u32> = None;
        let mut wide = false;
        for i in (start..br).rev() {
            let insn = self.insn(i);
            match insn.mnemonic {
                CMP | CPX | CPY if insn.mode.is_immediate() => {
                    against = Some(insn.operand.value());
                    wide = insn.len == 3;
                }
                CMP | CPX | CPY => {
                    let r = self.register(i)?;
                    port = Some(r);
                    let s = self.state(i)?;
                    let reg = match insn.mnemonic {
                        CMP => s.a,
                        CPX => s.x,
                        _ => s.y,
                    };
                    wide = match insn.mnemonic {
                        CMP => !insn.flags_before.m,
                        _ => !insn.flags_before.x,
                    };
                    against = State::value(&reg[..if wide { 2 } else { 1 }]);
                    break;
                }
                LDA | LDX | LDY => {
                    port = Some(self.register(i)?);
                    break;
                }
                NOP => {}
                _ => return None,
            }
        }
        let port = port.filter(|p| (0x2140..=0x2143).contains(p))?;
        let name = hardware_register(port).map_or("?", |r| r.name);
        // The loop runs while the branch stays in it: BNE staying means
        // "until equal".
        let until_equal = (bm == BNE) == taken_stays;
        let what = match against {
            Some(v) if wide => format!("${v:04X}"),
            Some(v) => format!("${v:02X}"),
            None => "the value the code expects".to_owned(),
        };
        let ports = if wide {
            let next = hardware_register(port + 1).map_or("?", |r| r.name);
            format!("{name} and {next}")
        } else {
            name.to_owned()
        };
        let summary = if until_equal {
            format!("Waits until the sound CPU puts {what} in {ports}.")
        } else {
            format!("Waits until {ports} no longer reads {what}.")
        };
        Some(Idiom {
            kind: IdiomKind::ApuHandshake,
            title: "Wait for the sound CPU".to_owned(),
            summary,
            why: if against == Some(0xBBAA) || against == Some(0xAA) {
                WHY_APU_BOOT
            } else {
                WHY_APU
            },
            offsets: steps.iter().map(|&i| self.insn(i).file_offset).collect(),
            note_at: None,
        })
    }

    // ---- clearing memory ----

    fn clear_loop(&self, header: usize, body: &[usize], name: Namer) -> Option<Idiom> {
        use AddressingMode::*;
        use Mnemonic::*;
        let steps = self.steps_of(body);
        if steps.len() > 10 {
            return None;
        }
        let (br, _) = self.exit_branch(body)?;
        // One indexed store of a known value to memory that is not a
        // register, and nothing else stored.
        let mut store = None;
        for &i in &steps {
            let insn = self.insn(i);
            if matches!(insn.mnemonic, STA | STZ | STX | STY) {
                if store.is_some() {
                    return None;
                }
                store = Some(i);
            }
            if insn.mnemonic.is_call() {
                return None;
            }
        }
        let si = store?;
        let insn = self.insn(si);
        let index_x = match insn.mode {
            AbsoluteX | AbsoluteLongX | DirectX => true,
            AbsoluteY | DirectY => false,
            _ => return None,
        };
        if self.register(si).is_some() || values::hardware_target(insn, self.state(si)?).is_some() {
            return None;
        }
        let base = insn.target.filter(|t| t.kind == TargetKind::Data)?.address;
        let wide = !insn.flags_before.m;
        let s = self.state(si)?;
        let value = match insn.mnemonic {
            STZ => Some(0),
            STA => State::value(&s.a[..if wide { 2 } else { 1 }]),
            _ => None,
        }?;
        // The index moves by a fixed step each time round.
        let mut step: i32 = 0;
        for &i in &steps {
            match (self.insn(i).mnemonic, index_x) {
                (INX, true) | (INY, false) => step += 1,
                (DEX, true) | (DEY, false) => step -= 1,
                (TAX | TXA | LDX | PLX, true) | (TAY | TYA | LDY | PLY, false) => return None,
                _ => {}
            }
        }
        if step == 0 {
            return None;
        }
        let width = if wide { 2 } else { 1 };
        // The index on the way in, from outside the loop.
        let init = self.index_on_entry(header, body, index_x);
        let bm = self.insn(br).mnemonic;
        let compare = (steps[0]..br)
            .rev()
            .find(|&i| matches!(self.insn(i).mnemonic, CPX | CPY))
            .map(|i| self.insn(i))
            .filter(|c| c.mode.is_immediate())
            .map(|c| c.operand.value() as i32);
        let store_first = steps.iter().position(|&i| i == si).is_some_and(|p| {
            !steps[..p]
                .iter()
                .any(|&i| matches!(self.insn(i).mnemonic, INX | INY | DEX | DEY))
        });
        let range = init.and_then(|s0| {
            let s0 = s0 as i32;
            let (lo, hi) = match (step < 0, bm, compare) {
                (true, BPL, None) => (s0.rem_euclid(-step), s0),
                (true, BNE, None) if store_first => (-step, s0),
                (true, BNE, None) => (0, s0 + step),
                (false, BNE | BCC | BMI, Some(end)) if store_first => (s0, end - step),
                _ => return None,
            };
            (lo <= hi).then_some((lo, hi))
        });
        let what = if value == 0 {
            "Clear memory".to_owned()
        } else {
            "Fill memory".to_owned()
        };
        let reg = if index_x { "X" } else { "Y" };
        let unit = if width == 2 { "a word" } else { "a byte" };
        let (verb, with) = if value == 0 {
            ("Clears", String::new())
        } else {
            (
                "Fills",
                format!(" with ${value:0w$X}", w = width as usize * 2),
            )
        };
        let summary = match range {
            Some((lo, hi)) => {
                let from = plus(base, lo as u16);
                let to = crate::model::project::Project::canonical(
                    self.rom,
                    plus(base, (hi + width - 1) as u16),
                );
                format!(
                    "{verb} {} to {to}{with}: {} bytes, {unit} at a time, indexed by {reg}.",
                    name(from),
                    hi - lo + width,
                )
            }
            None => format!(
                "{verb} memory from {} on{with}, {unit} at a time, indexed by {reg}.",
                name(base)
            ),
        };
        Some(Idiom {
            kind: IdiomKind::ClearMemory,
            title: what,
            summary,
            why: WHY_CLEAR,
            offsets: steps.iter().map(|&i| self.insn(i).file_offset).collect(),
            note_at: None,
        })
    }

    /// X (or Y) when control first reaches the loop, if every way in agrees.
    fn index_on_entry(&self, header: usize, body: &[usize], x: bool) -> Option<u32> {
        let mut out: Option<Option<u32>> = None;
        for &p in &self.cfg.blocks[header].preds {
            if body.contains(&p) {
                continue;
            }
            let last = self.cfg.blocks[p].steps.clone().last()?;
            let s = values::after(self.rom, self.state(last)?, self.insn(last));
            let insn = self.insn(self.cfg.blocks[header].steps.start);
            let n = if insn.flags_before.x { 1 } else { 2 };
            let v = State::value(&(if x { s.x } else { s.y })[..n]);
            match out {
                None => out = Some(v),
                Some(o) if o == v => {}
                Some(_) => return None,
            }
        }
        out.flatten()
    }

    fn block_moves(&self, name: Namer) -> Vec<Idiom> {
        let mut out = Vec::new();
        for i in 0..self.f.steps.len() {
            let insn = self.insn(i);
            if !matches!(insn.mnemonic, Mnemonic::MVN | Mnemonic::MVP) {
                continue;
            }
            let crate::cpu65816::Operand::Move { src, dst } = insn.operand else {
                continue;
            };
            let s = self.state(i);
            let word = |b: Option<[Byte; 2]>| b.and_then(|b| State::value(&b));
            let count = word(s.map(|s| s.a)).map(|a| a + 1);
            let x = word(s.map(|s| s.x));
            let y = word(s.map(|s| s.y));
            let at = |bank: u8, off: Option<u32>| match off {
                Some(o) => name(SnesAddress::new(bank, o as u16)),
                None => format!("bank ${bank:02X}"),
            };
            let n = count.map_or("A + 1".to_owned(), |c| format!("${c:04X}"));
            out.push(Idiom {
                kind: IdiomKind::BlockMove,
                title: "Block move".to_owned(),
                summary: format!(
                    "Copies {n} bytes from {} to {} ({}), a byte every 7 cycles.",
                    at(src, x),
                    at(dst, y),
                    if insn.mnemonic == Mnemonic::MVN {
                        "MVN: addresses count up"
                    } else {
                        "MVP: addresses count down"
                    }
                ),
                why: WHY_BLOCK_MOVE,
                offsets: vec![insn.file_offset],
                note_at: None,
            });
        }
        out
    }

    // ---- DMA and HDMA ----

    fn dma(&self, name: Namer) -> Vec<Idiom> {
        let mut out = Vec::new();
        for (&i, s) in &self.store_at {
            let hdma = match s.register {
                0x420B => false,
                0x420C => true,
                _ => continue,
            };
            let Some(mask) = s.value() else { continue };
            if mask == 0 {
                continue;
            }
            let w = self.written_before(i);
            let mut offsets = vec![s.offset];
            let mut parts = Vec::new();
            for c in (0..8u16).filter(|c| mask & (1 << c) != 0) {
                let base = 0x4300 + c * 0x10;
                let o = &mut offsets;
                let dmap = w.val(&[base], o).known();
                let bbad = w.val(&[base + 1], o);
                let a1t = w.val(&[base + 2, base + 3], o);
                let a1b = w.val(&[base + 4], o);
                let text = if hdma {
                    hdma_text(c, dmap, bbad, a1t, a1b, &w, name)
                } else {
                    let das = w.val(&[base + 5, base + 6], o);
                    let dest = destination(bbad, &w, o, name);
                    dma_text(c, dmap, dest, a1t, a1b, das, &w, name)
                };
                parts.push(text);
            }
            let (kind, title, why) = if hdma {
                (IdiomKind::Hdma, "HDMA setup", WHY_HDMA)
            } else {
                (IdiomKind::Dma, "DMA transfer", WHY_DMA)
            };
            out.push(Idiom {
                kind,
                title: title.to_owned(),
                summary: parts.join(" "),
                why,
                offsets,
                note_at: None,
            });
        }
        out.sort_by_key(|i| i.offsets[0].0);
        out
    }

    // ---- the multiplier and divider ----

    fn arithmetic(&self, name: Namer) -> Vec<Idiom> {
        use Mnemonic::*;
        let mut out = Vec::new();
        for (&i, s) in &self.store_at {
            let (kind, results): (IdiomKind, &[u16]) = match s.register {
                0x4203 => (IdiomKind::Multiply, &[0x4216, 0x4217]),
                0x4206 => (IdiomKind::Divide, &[0x4214, 0x4215, 0x4216, 0x4217]),
                0x211C if s.width == 1 => (IdiomKind::Multiply, &[0x2134, 0x2135, 0x2136]),
                _ => continue,
            };
            // The result read a few instructions on, in the same block.
            let end = self.cfg.blocks[self.block[i]].steps.end;
            let Some(read) = (i + 1..end.min(i + 16)).find(|&k| {
                matches!(self.insn(k).mnemonic, LDA | LDX | LDY)
                    && self.register(k).is_some_and(|r| results.contains(&r))
            }) else {
                continue;
            };
            let w = self.written_before(i);
            let mut offsets = vec![s.offset, self.insn(read).file_offset];
            // A known number, or where it comes from.
            let say = |v: Val, w: &Written| match v {
                Val::Known(k) => k.to_string(),
                v => w.say(v, "value", name),
            };
            let started = match s.value() {
                Some(v) => Val::Known(v),
                None => s.source().map_or(Val::Computed, Val::From),
            };
            let (title, summary, why) = match s.register {
                0x4203 => {
                    let a = w.val(&[0x4202], &mut offsets);
                    let product = a
                        .known()
                        .zip(started.known())
                        .map(|(a, b)| format!(" = {}", a * b));
                    (
                        "Hardware multiply",
                        format!(
                            "{} × {}{}, unsigned 8 × 8 bits; the 16-bit product is read from RDMPY.",
                            say(a, &w),
                            say(started, &w),
                            product.unwrap_or_default()
                        ),
                        WHY_MULTIPLY,
                    )
                }
                0x4206 => {
                    let a = w.val(&[0x4204, 0x4205], &mut offsets);
                    (
                        "Hardware divide",
                        format!(
                            "{} ÷ {}, 16 by 8 bits; the quotient is in RDDIV and the remainder in RDMPY.",
                            say(a, &w),
                            say(started, &w)
                        ),
                        WHY_DIVIDE,
                    )
                }
                _ => (
                    "Signed multiply (the mode 7 hardware)",
                    format!(
                        "M7A × {}, signed 16 × 8 bits; the 24-bit product is read from MPYL/M/H.",
                        say(started, &w)
                    ),
                    WHY_M7_MULTIPLY,
                ),
            };
            out.push(Idiom {
                kind,
                title: title.to_owned(),
                summary,
                why,
                offsets,
                note_at: None,
            });
        }
        out
    }

    // ---- decimal mode ----

    fn decimal(&self) -> Vec<Idiom> {
        use Mnemonic::*;
        let mut out = Vec::new();
        let n = self.f.steps.len();
        for i in 0..n {
            let sets = match self.insn(i).mnemonic {
                SED => true,
                SEP => self.insn(i).operand.value() & 0x08 != 0,
                _ => false,
            };
            if !sets {
                continue;
            }
            // On through consecutive instructions to the CLD.
            let mut ops = Vec::new();
            let mut offsets = vec![self.insn(i).file_offset];
            let mut end = None;
            for k in i + 1..n.min(i + 40) {
                let prev = self.insn(k - 1);
                let insn = self.insn(k);
                if insn.file_offset.0 != prev.file_offset.0 + u32::from(prev.len) {
                    break;
                }
                offsets.push(insn.file_offset);
                match insn.mnemonic {
                    ADC | SBC => ops.push(k),
                    CLD => {
                        end = Some(k);
                        break;
                    }
                    REP if insn.operand.value() & 0x08 != 0 => {
                        end = Some(k);
                        break;
                    }
                    _ if insn.mnemonic.is_block_end() || insn.mnemonic.is_call() => break,
                    _ => {}
                }
            }
            if end.is_none() || ops.is_empty() {
                continue;
            }
            let adds = ops
                .iter()
                .filter(|&&k| self.insn(k).mnemonic == ADC)
                .count();
            let subs = ops.len() - adds;
            let what = match (adds, subs) {
                (_, 0) => "adds",
                (0, _) => "subtracts",
                _ => "adds and subtracts",
            };
            out.push(Idiom {
                kind: IdiomKind::Decimal,
                title: "Decimal arithmetic".to_owned(),
                summary: format!(
                    "With the D flag set, ADC and SBC work on decimal digits: this {what} two digits a byte, so $09 + $01 is $10."
                ),
                why: WHY_DECIMAL,
                offsets,
                note_at: None,
            });
        }
        out
    }
}

fn shared_entry(
    rom: &RomImage,
    f: &Function,
    entries: &BTreeSet<SnesAddress>,
    name: Namer,
) -> Vec<Idiom> {
    let mut out = Vec::new();
    for (i, st) in f.steps.iter().enumerate().skip(1) {
        let a = st.insn.address;
        let canon = crate::model::project::Project::canonical(rom, a);
        if canon == f.entry || !entries.contains(&canon) {
            continue;
        }
        // Only when this routine's own code runs on into it.
        let Some(prev) = i.checked_sub(1).map(|p| &f.steps[p].insn) else {
            continue;
        };
        if prev.file_offset.0 + u32::from(prev.len) != st.insn.file_offset.0
            || prev.mnemonic.is_block_end()
        {
            continue;
        }
        out.push(Idiom {
            kind: IdiomKind::SharedEntry,
            title: "A second way in".to_owned(),
            summary: format!(
                "{} runs on into {}, which other code also calls: entering at {} runs its first lines, then everything {} does.",
                name(f.entry),
                name(canon),
                name(f.entry),
                name(canon)
            ),
            why: WHY_SHARED,
            offsets: vec![st.insn.file_offset, f.entry_offset],
            note_at: Some(st.insn.file_offset),
        });
    }
    out
}

/// Why the search back for a register's value stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// At a call, to this routine when it is known.
    Call(Option<SnesAddress>),
    /// At the routine's entry: the caller set it.
    Entry,
    /// Where paths meet, or too far back.
    Earlier,
}

/// The register writes reaching an instruction.
struct Written {
    bytes: HashMap<u16, (Byte, FileOffset)>,
    /// Registers the ways in set differently.
    varies: BTreeSet<u16>,
    stop: Stop,
}

/// A register's value, from the writes reaching an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Val {
    Known(u32),
    /// Loaded, unchanged, from here.
    From(SnesAddress),
    /// Written with something worked out along the way.
    Computed,
    /// Set differently on the paths that lead here.
    Varies,
    /// Not written on the way.
    Unset,
}

impl Val {
    fn known(self) -> Option<u32> {
        match self {
            Val::Known(v) => Some(v),
            _ => None,
        }
    }
}

impl Written {
    /// The value of a register (or a pair, low first), noting the stores
    /// it came from.
    fn val(&self, regs: &[u16], offsets: &mut Vec<FileOffset>) -> Val {
        let bytes: Vec<Option<Byte>> = regs
            .iter()
            .map(|r| {
                self.bytes.get(r).map(|(b, off)| {
                    offsets.push(*off);
                    *b
                })
            })
            .collect();
        if bytes.iter().all(Option::is_none) {
            return Val::Unset;
        }
        if regs.iter().any(|r| self.varies.contains(r)) {
            return Val::Varies;
        }
        let known: Option<u32> = bytes.iter().rev().try_fold(0u32, |acc, b| {
            Some(acc << 8 | u32::from(b.and_then(Byte::known)?))
        });
        if let Some(v) = known {
            return Val::Known(v);
        }
        if let Some(Some(Byte::From(a))) = bytes.first() {
            let consecutive = bytes
                .iter()
                .enumerate()
                .all(|(k, b)| *b == Some(Byte::From(plus(*a, k as u16))));
            if consecutive {
                return Val::From(*a);
            }
        }
        Val::Computed
    }

    /// How to say a value that is not a constant: "the address in
    /// $7E:0330", "an address set by SUB_8091A9".
    fn say(&self, v: Val, noun: &str, name: Namer) -> String {
        match v {
            Val::From(_) => format!("the {noun} {}", self.how(v, name)),
            _ => format!("{} {noun} {}", article(noun), self.how(v, name)),
        }
    }

    /// Where a value that is not a constant comes from: "in $7E:0330",
    /// "worked out here", "set by SUB_8091A9".
    fn how(&self, v: Val, name: Namer) -> String {
        match v {
            Val::Known(k) => format!("${k:X}"),
            Val::From(a) => format!("in {}", name(a)),
            Val::Computed => "worked out here".to_owned(),
            Val::Varies => "set differently on each path here".to_owned(),
            Val::Unset => match self.stop {
                Stop::Call(Some(c)) => format!("set by {}", name(c)),
                Stop::Call(None) => "set by the call before".to_owned(),
                Stop::Entry => "set by the caller".to_owned(),
                Stop::Earlier => "set earlier".to_owned(),
            },
        }
    }
}

fn article(noun: &str) -> &'static str {
    if noun.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

/// Where a DMA writes, from its B-bus register and the address set for
/// that memory.
fn destination(bbad: Val, w: &Written, offsets: &mut Vec<FileOffset>, name: Namer) -> String {
    let mut at =
        |regs: &[u16], known: &dyn Fn(u32) -> String, what: &str| match w.val(regs, offsets) {
            Val::Known(v) => known(v),
            v => format!("{what} at {}", w.say(v, "address", name)),
        };
    match bbad {
        Val::Known(0x18 | 0x19) => at(
            &[0x2116, 0x2117],
            &|a| format!("VRAM word ${a:04X}"),
            "VRAM",
        ),
        Val::Known(0x22) => at(
            &[0x2121],
            &|c| format!("the palette from colour {c}"),
            "the palette",
        ),
        Val::Known(0x04) => at(
            &[0x2102, 0x2103],
            &|a| {
                if a & 0x1FF == 0 {
                    "sprite memory (OAM) from the start".to_owned()
                } else {
                    format!("sprite memory (OAM) from word ${:03X}", a & 0x1FF)
                }
            },
            "sprite memory (OAM)",
        ),
        Val::Known(0x80) => {
            let low = w.val(&[0x2181, 0x2182], offsets);
            let bank = w.val(&[0x2183], offsets);
            match (low, bank) {
                (Val::Known(a), Val::Known(b)) => format!("WRAM ${:02X}:{a:04X}", 0x7E + (b & 1)),
                (v, _) => format!("WRAM at {}", w.say(v, "address", name)),
            }
        }
        Val::Known(b) => {
            let reg = 0x2100 | b as u16;
            match hardware_register(reg) {
                Some(r) => format!("${reg:04X} {}", r.name),
                None => format!("${reg:04X}"),
            }
        }
        v => w.say(v, "register", name),
    }
}

#[allow(clippy::too_many_arguments)]
fn dma_text(
    c: u16,
    dmap: Option<u32>,
    dest: String,
    a1t: Val,
    a1b: Val,
    das: Val,
    w: &Written,
    name: Namer,
) -> String {
    let src = match (a1t, a1b) {
        (Val::Known(a), Val::Known(b)) => name(SnesAddress::new(b as u8, a as u16)),
        (Val::Known(a), b) => format!("${a:04X} in {}", w.say(b, "bank", name)),
        (a, _) => w.say(a, "address", name),
    };
    let (size, count) = match das {
        Val::Known(0) => ("$10000 bytes".to_owned(), String::new()),
        Val::Known(n) => (format!("${n:04X} bytes"), String::new()),
        v => (
            "bytes".to_owned(),
            format!("; the byte count is {}", w.how(v, name)),
        ),
    };
    let fixed = dmap.is_some_and(|d| d & 0x08 != 0);
    let reverse = dmap.is_some_and(|d| d & 0x80 != 0);
    if reverse {
        format!("Channel {c} copies {size} from {dest} to {src}{count}.")
    } else if fixed {
        format!("Channel {c} fills {dest} with {size} copied from the one byte at {src}{count}.")
    } else {
        format!("Channel {c} copies {size} from {src} to {dest}{count}.")
    }
}

fn hdma_text(
    c: u16,
    dmap: Option<u32>,
    bbad: Val,
    a1t: Val,
    a1b: Val,
    w: &Written,
    name: Namer,
) -> String {
    let target = match bbad {
        Val::Known(b) => {
            let reg = 0x2100 | b as u16;
            hardware_register(reg).map_or(format!("${reg:04X}"), |r| r.name.to_owned())
        }
        v => w.say(v, "register", name),
    };
    let table = match (a1t, a1b) {
        (Val::Known(a), Val::Known(b)) => {
            format!("the table at {}", name(SnesAddress::new(b as u8, a as u16)))
        }
        (Val::Known(a), b) => format!("the table at ${a:04X} in {}", w.say(b, "bank", name)),
        (a, _) => format!("the table at {}", w.say(a, "address", name)),
    };
    let indirect = if dmap.is_some_and(|d| d & 0x40 != 0) {
        " (indirect: the table points at the data)"
    } else {
        ""
    };
    format!("Channel {c} writes {target} on every line from {table}{indirect}.")
}

fn plus(a: SnesAddress, n: u16) -> SnesAddress {
    SnesAddress::new(a.bank(), a.offset().wrapping_add(n))
}

// ---- why games do it ----

const WHY_VBLANK: &str = "The PPU draws the picture line by line, and while it does, VRAM, the palette and sprite memory cannot be changed safely. Vertical blank, the gap of about 37 lines between frames (NTSC), is the time to update them, so code that is about to change graphics first waits for it.";
const WHY_VBLANK_END: &str = "Waiting for vertical blank to end makes sure the next wait catches the start of a new blanking period, not the tail of the current one, so the code runs once per frame.";
const WHY_HBLANK: &str = "Horizontal blank is the short pause at the end of each line. A change made there takes effect from the next line down, which is how a game changes a setting partway down the screen without HDMA.";
const WHY_JOYPAD: &str = "With NMITIMEN bit 0 set, the hardware reads the controllers at the start of vertical blank, which takes about three lines. JOY1L and the rest are only valid once it has finished.";
const WHY_RDNMI: &str = "RDNMI's bit 7 is set at the start of every vertical blank and cleared when read. Polling it waits for a new frame even with the NMI interrupt turned off, which is common during start-up.";
const WHY_TIMEUP: &str = "TIMEUP's bit 7 is set when the H/V timer fires. Polling it instead of taking the interrupt lets code wait for a chosen line.";
const WHY_WAI: &str = "WAI halts the CPU until an interrupt. Most games end their main loop with it, or with a loop on a flag the NMI handler sets, so the game logic runs exactly once per frame.";
const WHY_DMA: &str = "The CPU copying a byte at a time is slow. DMA moves data between memory and the PPU at about 2.7 MB a second while the CPU waits, which is the only practical way to fill VRAM, the palette or sprite memory in the short vertical blank.";
const WHY_HDMA: &str = "HDMA writes a register at the start of each line from a table in memory, with no CPU time. Games use it for gradients, wavy water, split-screen scrolling and window shapes.";
const WHY_MULTIPLY: &str = "The 65816 has no multiply instruction. The SNES adds an 8 × 8 multiplier: write the two numbers, wait 8 cycles, read the product. Anything done in the meantime is free.";
const WHY_DIVIDE: &str = "The 65816 has no divide instruction either. The SNES divider takes a 16-bit number and an 8-bit divisor, and has the quotient and remainder ready 16 cycles after the divisor is written.";
const WHY_M7_MULTIPLY: &str = "The mode 7 matrix hardware doubles as a signed 16 × 8 multiplier with the result ready at once. It is only usable while mode 7 is not being drawn.";
const WHY_CLEAR: &str = "Memory holds leftovers at power-on, so games clear their variables before using them. A small loop like this is the simplest way; larger areas are often filled by DMA instead.";
const WHY_BLOCK_MOVE: &str = "MVN and MVP copy a block of memory in one instruction, with A holding the count minus one and X and Y the source and destination. They are simpler than a loop, but slower than DMA.";
const WHY_APU: &str = "The sound CPU (an SPC700 with its own 64 KB of RAM) runs on its own, and the four APUIO ports are the only link. A game writes a command and waits for the sound driver to echo it, so both sides stay in step.";
const WHY_APU_BOOT: &str = "At power-on the sound CPU's boot ROM puts $AA and $BB in ports 0 and 1 to say it is ready. A game waits for them before uploading its sound driver a byte at a time through the same ports.";
const WHY_DECIMAL: &str = "In decimal mode each byte holds two decimal digits, one per nibble. Games keep scores, timers and lives this way so each digit can be drawn straight from its nibble, without dividing by ten.";
const WHY_SHARED: &str = "In assembly a routine can have more than one entry point: code that needs one extra step first starts a few instructions earlier and runs on into the shared part. It saves the bytes of a call or a copy.";

#[cfg(test)]
mod tests {
    use super::*;

    fn written(stop: Stop) -> Written {
        Written {
            bytes: HashMap::new(),
            varies: BTreeSet::new(),
            stop,
        }
    }

    #[test]
    fn dma_text_says_what_is_known() {
        let name: Namer = &|a| format!("{a}");
        let w = written(Stop::Earlier);
        assert_eq!(
            dma_text(
                0,
                Some(0x01),
                "VRAM word $6000".into(),
                Val::Known(0x9000),
                Val::Known(0),
                Val::Known(0x800),
                &w,
                name
            ),
            "Channel 0 copies $0800 bytes from $00:9000 to VRAM word $6000."
        );
        let w = written(Stop::Call(Some(SnesAddress::new(0x80, 0x91A9))));
        assert_eq!(
            dma_text(
                1,
                None,
                w.say(Val::Unset, "register", name),
                Val::Unset,
                Val::Unset,
                Val::Unset,
                &w,
                name
            ),
            "Channel 1 copies bytes from an address set by $80:91A9 to a register set by $80:91A9; the byte count is set by $80:91A9."
        );
        let w = written(Stop::Entry);
        assert_eq!(
            dma_text(
                2,
                Some(0x09),
                "WRAM $7E:0000".into(),
                Val::From(SnesAddress::new(0x7E, 0x10)),
                Val::Known(0x7E),
                Val::Computed,
                &w,
                name
            ),
            "Channel 2 fills WRAM $7E:0000 with bytes copied from the one byte at the address in $7E:0010; the byte count is worked out here."
        );
    }
}
