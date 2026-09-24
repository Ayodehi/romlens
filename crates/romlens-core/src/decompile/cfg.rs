//! A function's basic blocks, and the graph facts structuring needs:
//! dominators, post-dominators and natural loops.
//!
//! Every edge ends at a block. A transfer this function cannot follow (a tail
//! call, a jump through a pointer, a call that never returns) ends at a stub:
//! a block with no instructions whose terminator says what happened, so the
//! later stages never meet a dangling edge.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::decompile::function::{Dest, Function, Transfer};
use crate::memory::address::{FileOffset, SnesAddress};

pub type BlockId = usize;

/// How a block ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// Runs on into the next block (which starts at a branch target).
    Fall(BlockId),
    Goto(BlockId),
    /// The last instruction is a conditional branch.
    Branch {
        taken: BlockId,
        fall: BlockId,
    },
    /// `JMP (abs,X)` through a table, one block per entry.
    Switch {
        table: SnesAddress,
        cases: Vec<BlockId>,
    },
    Return,
    Halt,
    /// A stub: control continues in another routine.
    Tail(SnesAddress),
    /// A stub: control goes somewhere this cannot follow.
    Unknown(String),
}

impl Term {
    pub fn succs(&self) -> Vec<BlockId> {
        match self {
            Term::Fall(b) | Term::Goto(b) => vec![*b],
            Term::Branch { taken, fall } => vec![*taken, *fall],
            Term::Switch { cases, .. } => {
                let mut v = cases.clone();
                v.sort_unstable();
                v.dedup();
                v
            }
            Term::Return | Term::Halt | Term::Tail(_) | Term::Unknown(_) => vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct Block {
    /// Indices into `Function::steps`; empty for a stub.
    pub steps: Range<usize>,
    pub term: Term,
    pub preds: Vec<BlockId>,
}

impl Block {
    pub fn is_stub(&self) -> bool {
        self.steps.is_empty()
    }
}

/// A natural loop: its header and every block in it, header included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loop {
    pub header: BlockId,
    /// Sorted.
    pub body: Vec<BlockId>,
    /// The blocks with a back edge to the header.
    pub latches: Vec<BlockId>,
}

#[derive(Debug, Clone)]
pub struct Cfg {
    pub blocks: Vec<Block>,
    pub entry: BlockId,
    /// Reverse post-order from the entry; unreachable blocks are absent.
    pub rpo: Vec<BlockId>,
    /// Immediate dominator; `None` for the entry and unreachable blocks.
    pub idom: Vec<Option<BlockId>>,
    /// Immediate post-dominator; `None` where the only post-dominator is the
    /// virtual exit, or no exit is reachable (an endless loop).
    pub ipdom: Vec<Option<BlockId>>,
    /// Outermost first (by header order in `rpo`).
    pub loops: Vec<Loop>,
    /// Some cycle has more than one way in, so structuring will need a goto.
    pub irreducible: bool,
}

impl Cfg {
    pub fn build(f: &Function) -> Cfg {
        let n = f.steps.len();
        let offset_of = |i: usize| f.steps[i].insn.file_offset;
        let index: BTreeMap<u32, usize> = (0..n).map(|i| (offset_of(i).0, i)).collect();

        // Leaders: the entry, every jump or branch target, and whatever
        // follows an instruction that does not simply run on.
        let mut leader = vec![false; n];
        if let Some(&e) = index.get(&f.entry_offset.0) {
            leader[e] = true;
        }
        for (i, s) in f.steps.iter().enumerate() {
            let runs_on = continues(&s.transfer, i, f);
            if !runs_on && i + 1 < n {
                leader[i + 1] = true;
            }
            if let Transfer::Branch { .. } | Transfer::Jump(_) | Transfer::Switch { .. } =
                &s.transfer
            {
                for d in s.transfer.dests() {
                    if let Dest::Local(o) = d
                        && let Some(&t) = index.get(&o.0)
                    {
                        leader[t] = true;
                    }
                }
            }
        }
        if n > 0 {
            leader[0] = true;
        }

        let mut blocks: Vec<Block> = Vec::new();
        let mut block_of = vec![0usize; n];
        let mut i = 0;
        while i < n {
            let start = i;
            i += 1;
            while i < n && !leader[i] {
                i += 1;
            }
            for b in &mut block_of[start..i] {
                *b = blocks.len();
            }
            blocks.push(Block {
                steps: start..i,
                term: Term::Halt,
                preds: Vec::new(),
            });
        }

        let mut tails: BTreeMap<SnesAddress, BlockId> = BTreeMap::new();
        let mut stubs: Vec<Block> = Vec::new();
        let real = blocks.len();
        let mut target = |d: &Dest, stubs: &mut Vec<Block>| -> BlockId {
            let stub = |term: Term, stubs: &mut Vec<Block>| {
                stubs.push(Block {
                    steps: 0..0,
                    term,
                    preds: Vec::new(),
                });
                real + stubs.len() - 1
            };
            match d {
                Dest::Local(o) => match index.get(&o.0) {
                    Some(&t) => block_of[t],
                    None => stub(Term::Unknown(format!("{o} was not decoded")), stubs),
                },
                Dest::Tail(a) => *tails
                    .entry(*a)
                    .or_insert_with(|| stub(Term::Tail(*a), stubs)),
                Dest::Unknown(why) => stub(Term::Unknown(why.clone()), stubs),
            }
        };
        for block in &mut blocks {
            let last = block.steps.end - 1;
            let t = &f.steps[last].transfer;
            let term = match t {
                Transfer::Next(d) | Transfer::Call { next: d, .. } => {
                    Term::Fall(target(d, &mut stubs))
                }
                Transfer::Branch { taken, next } => Term::Branch {
                    taken: target(taken, &mut stubs),
                    fall: target(next, &mut stubs),
                },
                Transfer::Jump(d) => match d {
                    Dest::Local(_) => Term::Goto(target(d, &mut stubs)),
                    // A stub already says where control went.
                    _ => Term::Fall(target(d, &mut stubs)),
                },
                Transfer::Switch { table, cases } => Term::Switch {
                    table: *table,
                    cases: cases.iter().map(|d| target(d, &mut stubs)).collect(),
                },
                Transfer::Return => Term::Return,
                Transfer::Halt => Term::Halt,
            };
            block.term = term;
        }
        blocks.extend(stubs);

        let entry = index
            .get(&f.entry_offset.0)
            .map(|&i| block_of[i])
            .unwrap_or(0);
        let mut cfg = Cfg {
            blocks,
            entry,
            rpo: Vec::new(),
            idom: Vec::new(),
            ipdom: Vec::new(),
            loops: Vec::new(),
            irreducible: false,
        };
        cfg.analyze();
        cfg
    }

    pub fn succs(&self, b: BlockId) -> Vec<BlockId> {
        self.blocks[b].term.succs()
    }

    /// The block holding the instruction at `off`.
    pub fn block_of(&self, f: &Function, off: FileOffset) -> Option<BlockId> {
        let i = f.index_of(off)?;
        self.blocks.iter().position(|b| b.steps.contains(&i))
    }

    /// `a` dominates `b`.
    pub fn dominates(&self, a: BlockId, b: BlockId) -> bool {
        let mut x = Some(b);
        while let Some(y) = x {
            if y == a {
                return true;
            }
            x = self.idom[y];
        }
        false
    }

    fn analyze(&mut self) {
        let n = self.blocks.len();
        for b in 0..n {
            for s in self.succs(b) {
                self.blocks[s].preds.push(b);
            }
        }
        for b in &mut self.blocks {
            b.preds.sort_unstable();
            b.preds.dedup();
        }
        if n == 0 {
            return;
        }
        let succs: Vec<Vec<BlockId>> = (0..n).map(|b| self.succs(b)).collect();
        let preds: Vec<Vec<BlockId>> = self.blocks.iter().map(|b| b.preds.clone()).collect();

        let (rpo, retreating) = rpo_from(self.entry, &succs);
        self.rpo = rpo;
        self.idom = dominators(self.entry, &self.rpo, &preds, n);

        // Post-dominators: dominators of the reversed graph, from a virtual
        // exit that every block without successors leads to.
        let exit = n;
        let mut rsuccs: Vec<Vec<BlockId>> = preds.clone();
        rsuccs.push((0..n).filter(|&b| succs[b].is_empty()).collect());
        let mut rpreds: Vec<Vec<BlockId>> = succs.clone();
        for &b in &rsuccs[exit] {
            rpreds[b].push(exit);
        }
        rpreds.push(Vec::new());
        let (rrpo, _) = rpo_from(exit, &rsuccs);
        let pdom = dominators(exit, &rrpo, &rpreds, n + 1);
        self.ipdom = pdom[..n].iter().map(|d| d.filter(|&d| d != exit)).collect();

        // Natural loops, one per header.
        let mut by_header: BTreeMap<BlockId, Loop> = BTreeMap::new();
        for &(u, v) in &retreating {
            if !self.dominates(v, u) {
                self.irreducible = true;
                continue;
            }
            let l = by_header.entry(v).or_insert_with(|| Loop {
                header: v,
                body: vec![v],
                latches: Vec::new(),
            });
            l.latches.push(u);
            let mut stack = vec![u];
            while let Some(x) = stack.pop() {
                if l.body.contains(&x) {
                    continue;
                }
                l.body.push(x);
                stack.extend(preds[x].iter().copied());
            }
        }
        let order: BTreeMap<BlockId, usize> =
            self.rpo.iter().enumerate().map(|(i, &b)| (b, i)).collect();
        let mut loops: Vec<Loop> = by_header.into_values().collect();
        for l in &mut loops {
            l.body.sort_unstable();
            l.latches.sort_unstable();
            l.latches.dedup();
        }
        loops.sort_by_key(|l| order.get(&l.header).copied().unwrap_or(usize::MAX));
        self.loops = loops;
    }
}

/// Whether the instruction at `i` runs on into `i + 1`.
fn continues(t: &Transfer, i: usize, f: &Function) -> bool {
    let next = match t {
        Transfer::Next(Dest::Local(o))
        | Transfer::Call {
            next: Dest::Local(o),
            ..
        } => *o,
        _ => return false,
    };
    f.steps
        .get(i + 1)
        .is_some_and(|s| s.insn.file_offset == next)
}

/// Reverse post-order from `root`, and the retreating edges (to a node still
/// on the DFS stack). Iterative, so a long routine does not need a deep stack.
fn rpo_from(root: BlockId, succs: &[Vec<BlockId>]) -> (Vec<BlockId>, Vec<(BlockId, BlockId)>) {
    let n = succs.len();
    let mut state = vec![0u8; n]; // 0 new, 1 on stack, 2 done
    let mut post = Vec::with_capacity(n);
    let mut retreating = Vec::new();
    let mut stack: Vec<(BlockId, usize)> = vec![(root, 0)];
    state[root] = 1;
    while let Some(&mut (b, ref mut next)) = stack.last_mut() {
        if let Some(&s) = succs[b].get(*next) {
            *next += 1;
            match state[s] {
                0 => {
                    state[s] = 1;
                    stack.push((s, 0));
                }
                1 => retreating.push((b, s)),
                _ => {}
            }
        } else {
            state[b] = 2;
            post.push(b);
            stack.pop();
        }
    }
    post.reverse();
    (post, retreating)
}

/// Cooper, Harvey and Kennedy's iterative dominator algorithm.
fn dominators(
    root: BlockId,
    rpo: &[BlockId],
    preds: &[Vec<BlockId>],
    n: usize,
) -> Vec<Option<BlockId>> {
    let mut order = vec![usize::MAX; n];
    for (i, &b) in rpo.iter().enumerate() {
        order[b] = i;
    }
    let mut idom: Vec<Option<BlockId>> = vec![None; n];
    idom[root] = Some(root);
    let intersect = |idom: &[Option<BlockId>], mut a: BlockId, mut b: BlockId| {
        while a != b {
            while order[a] > order[b] {
                a = idom[a].unwrap();
            }
            while order[b] > order[a] {
                b = idom[b].unwrap();
            }
        }
        a
    };
    let mut changed = true;
    while changed {
        changed = false;
        for &b in rpo.iter().skip(1) {
            let mut new: Option<BlockId> = None;
            for &p in &preds[b] {
                if idom[p].is_none() || order[p] == usize::MAX {
                    continue;
                }
                new = Some(match new {
                    None => p,
                    Some(x) => intersect(&idom, p, x),
                });
            }
            if new.is_some() && idom[b] != new {
                idom[b] = new;
                changed = true;
            }
        }
    }
    idom[root] = None;
    idom
}
