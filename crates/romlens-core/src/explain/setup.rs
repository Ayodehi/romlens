//! The hardware registers as they stand when an instruction runs
//! (docs/21), from the code before it on every path, calls included.
//!
//! A walk back through the routine's blocks keeps the latest write to each
//! register byte. Where paths meet it keeps what they agree on; a register
//! written on only some of them is `Varies`. A call counts as the writes
//! its routine makes on every way out, found the same way and memoised;
//! the walk then carries on back past the call.

use std::collections::{BTreeSet, HashMap};

use super::values::{self, Byte, Store, Values};
use crate::analysis::snapshot::AnalysisSnapshot;
use crate::decompile::cfg::{Cfg, Term};
use crate::decompile::function::{self, Callee, Function, Transfer};
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::project::Project;
use crate::rom::image::RomImage;

/// One register byte where the walk ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Set to this, by the store at the offset.
    Known(u8, FileOffset),
    /// Set from this address, unchanged.
    From(SnesAddress, FileOffset),
    /// Set to something worked out along the way.
    Computed(FileOffset),
    /// Set differently, or only on some paths, on the ways here.
    Varies,
}

impl Reach {
    pub fn known(self) -> Option<u8> {
        match self {
            Reach::Known(v, _) => Some(v),
            _ => None,
        }
    }

    pub fn at(self) -> Option<FileOffset> {
        match self {
            Reach::Known(_, o) | Reach::From(_, o) | Reach::Computed(o) => Some(o),
            Reach::Varies => None,
        }
    }
}

/// Why a register not written on the way here was not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Before {
    /// The walk reached the routine's entry: its callers set it, if anyone.
    Caller,
    /// It went as far as it goes (a jump it cannot follow, too far back).
    Unknown,
}

/// The registers reaching an instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registers {
    /// By bank offset (`$2100`); a register missing was not written on the
    /// way here.
    pub bytes: HashMap<u16, Reach>,
    pub before: Before,
    /// The routine the instruction is in.
    pub routine: SnesAddress,
}

impl Registers {
    pub fn get(&self, reg: u16) -> Option<Reach> {
        self.bytes.get(&reg).copied()
    }
}

/// The registers as they stand when the instruction at `off` runs.
pub fn registers_at(rom: &RomImage, snap: &AnalysisSnapshot, off: FileOffset) -> Option<Registers> {
    let entries = function::entries(snap);
    let f = function::containing(rom, snap, &entries, off)?;
    let routine = f.entry;
    let mut w = Walker {
        rom,
        snap,
        entries: &entries,
        routines: HashMap::new(),
        summaries: HashMap::new(),
        busy: BTreeSet::new(),
    };
    let info = w.info_for(f);
    let at = info.f.index_of(off)?;
    let b = info.block[at];
    let mut budget = BUDGET;
    let (bytes, before) = w.walk(&info, b, at, &mut BTreeSet::new(), &mut budget, 0);
    Some(Registers {
        bytes,
        before,
        routine,
    })
}

const BUDGET: usize = 4096;
const DEPTH: usize = 12;
const CALL_DEPTH: usize = 6;

struct Info {
    f: Function,
    cfg: Cfg,
    block: Vec<usize>,
    stores: HashMap<usize, Store>,
}

type Map = HashMap<u16, Reach>;

struct Walker<'a> {
    rom: &'a RomImage,
    snap: &'a AnalysisSnapshot,
    entries: &'a BTreeSet<SnesAddress>,
    routines: HashMap<SnesAddress, Option<std::rc::Rc<Info>>>,
    /// Each routine's writes on every way out.
    summaries: HashMap<SnesAddress, Map>,
    busy: BTreeSet<SnesAddress>,
}

impl<'a> Walker<'a> {
    fn info_for(&mut self, f: Function) -> std::rc::Rc<Info> {
        let cfg = Cfg::build(&f);
        let v: Values = values::values(self.rom, &f, &cfg);
        let mut block = vec![usize::MAX; f.steps.len()];
        for (b, blk) in cfg.blocks.iter().enumerate() {
            for i in blk.steps.clone() {
                block[i] = b;
            }
        }
        let stores = v
            .stores
            .into_iter()
            .filter_map(|s| f.index_of(s.offset).map(|i| (i, s)))
            .collect();
        let entry = f.entry;
        let info = std::rc::Rc::new(Info {
            f,
            cfg,
            block,
            stores,
        });
        self.routines.insert(entry, Some(info.clone()));
        info
    }

    fn routine(&mut self, at: SnesAddress) -> Option<std::rc::Rc<Info>> {
        if let Some(r) = self.routines.get(&at) {
            return r.clone();
        }
        match function::discover(self.rom, self.snap, self.entries, at) {
            Ok(f) => Some(self.info_for(f)),
            Err(_) => {
                self.routines.insert(at, None);
                None
            }
        }
    }

    /// What a call to `at` leaves in the registers.
    fn summary(&mut self, at: SnesAddress, calls: usize) -> Map {
        let at = Project::canonical(self.rom, at);
        if let Some(s) = self.summaries.get(&at) {
            return s.clone();
        }
        if calls >= CALL_DEPTH || self.busy.contains(&at) {
            return Map::new();
        }
        let Some(info) = self.routine(at) else {
            return Map::new();
        };
        self.busy.insert(at);
        let mut exits: Vec<Map> = Vec::new();
        for (b, blk) in info.cfg.blocks.iter().enumerate() {
            let end = match &blk.term {
                Term::Return => blk.steps.end,
                Term::Tail(target) => {
                    // Its writes, then the routine it continues in.
                    let mut budget = BUDGET;
                    let (mine, _) = self.walk_calls(
                        &info,
                        b,
                        blk.steps.end,
                        &mut BTreeSet::new(),
                        &mut budget,
                        0,
                        calls + 1,
                    );
                    let mut theirs = self.summary(*target, calls + 1);
                    for (r, v) in mine {
                        theirs.entry(r).or_insert(v);
                    }
                    exits.push(theirs);
                    continue;
                }
                _ => continue,
            };
            if blk.steps.is_empty() {
                continue;
            }
            let mut budget = BUDGET;
            let (m, _) = self.walk_calls(
                &info,
                b,
                end,
                &mut BTreeSet::new(),
                &mut budget,
                0,
                calls + 1,
            );
            exits.push(m);
        }
        self.busy.remove(&at);
        let joined = join_all(&exits);
        self.summaries.insert(at, joined.clone());
        joined
    }

    fn walk(
        &mut self,
        info: &Info,
        b: usize,
        end: usize,
        visited: &mut BTreeSet<usize>,
        budget: &mut usize,
        depth: usize,
    ) -> (Map, Before) {
        self.walk_calls(info, b, end, visited, budget, depth, 0)
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_calls(
        &mut self,
        info: &Info,
        b: usize,
        end: usize,
        visited: &mut BTreeSet<usize>,
        budget: &mut usize,
        depth: usize,
        calls: usize,
    ) -> (Map, Before) {
        let mut m = Map::new();
        let start = info.cfg.blocks[b].steps.start;
        for i in (start..end).rev() {
            if *budget == 0 {
                return (m, Before::Unknown);
            }
            *budget -= 1;
            if let Some(s) = info.stores.get(&i) {
                for k in 0..s.width {
                    let reg = s.register.wrapping_add(u16::from(k));
                    let reach = if s.indexed {
                        Reach::Computed(s.offset)
                    } else {
                        match s.bytes[k as usize] {
                            Byte::Known(v) => Reach::Known(v, s.offset),
                            Byte::From(a) => Reach::From(a, s.offset),
                            Byte::Unknown => Reach::Computed(s.offset),
                        }
                    };
                    // An indexed store's register is only its base: it
                    // says nothing certain about that register.
                    if !s.indexed {
                        m.entry(reg).or_insert(reach);
                    }
                }
            }
            match &info.f.steps[i].transfer {
                Transfer::Call {
                    callee: Callee::Direct(c),
                    ..
                } => {
                    for (r, v) in self.summary(*c, calls) {
                        m.entry(r).or_insert(v);
                    }
                }
                Transfer::Call { .. } => {
                    // Through a pointer: it could have written anything
                    // already set before it, so stop here.
                    return (m, Before::Unknown);
                }
                _ => {}
            }
        }
        visited.insert(b);
        let preds: Vec<usize> = info.cfg.blocks[b]
            .preds
            .iter()
            .copied()
            .filter(|p| !visited.contains(p))
            .collect();
        if preds.is_empty() {
            let before = if b == info.cfg.entry {
                Before::Caller
            } else {
                Before::Unknown
            };
            return (m, before);
        }
        if depth > DEPTH {
            return (m, Before::Unknown);
        }
        let mut from = Vec::new();
        let mut befores = Vec::new();
        for p in preds {
            let mut seen = visited.clone();
            let end = info.cfg.blocks[p].steps.end;
            let (pm, pb) = self.walk_calls(info, p, end, &mut seen, budget, depth + 1, calls);
            from.push(pm);
            befores.push(pb);
        }
        for (r, v) in join_all(&from) {
            m.entry(r).or_insert(v);
        }
        let before = if befores.iter().all(|x| *x == Before::Caller) {
            Before::Caller
        } else {
            Before::Unknown
        };
        (m, before)
    }
}

/// What every way agrees on; a register set differently, or on some ways
/// and not others, varies.
fn join_all(maps: &[Map]) -> Map {
    let Some(first) = maps.first() else {
        return Map::new();
    };
    let mut regs: BTreeSet<u16> = BTreeSet::new();
    for m in maps {
        regs.extend(m.keys());
    }
    let mut out = Map::new();
    for r in regs {
        let vals: Vec<Option<Reach>> = maps.iter().map(|m| m.get(&r).copied()).collect();
        let v = match first.get(&r) {
            Some(v) if vals.iter().all(|x| *x == Some(*v)) => *v,
            // The same value from different stores still agrees.
            Some(Reach::Known(k, o))
                if vals
                    .iter()
                    .all(|x| matches!(x, Some(Reach::Known(k2, _)) if k2 == k)) =>
            {
                Reach::Known(*k, *o)
            }
            _ => Reach::Varies,
        };
        out.insert(r, v);
    }
    out
}
