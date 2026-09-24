//! Structuring: the graph as `if`, loops and `switch`, with a goto only
//! where it does not reduce.
//!
//! Emission walks the graph from the entry. A natural loop becomes
//! `do … while` when its one back edge is a conditional branch (the usual
//! 65816 shape, `DEX; BPL`), `while` when its header only tests, and
//! `for (;;)` otherwise, with `break` and `continue` for the edges out and
//! back. A conditional branch becomes `if`/`else` joined at its immediate
//! post-dominator. A block reached a second time, which the structure could
//! not place, is a `goto` to its label. Iterative where it matters: the
//! recursion is bounded by nesting depth, not by the routine's length.

use std::collections::BTreeSet;

use crate::decompile::cfg::{BlockId, Cfg, Loop, Term};
use crate::decompile::dataflow::simplify;
use crate::decompile::ir::{Expr, LiftedBlock, UnOp};

#[derive(Debug, Clone)]
pub enum LoopKind {
    /// `while (cond) { body }`
    While(Expr),
    /// `do { body } while (cond);`
    DoWhile(Expr),
    /// `for (;;) { body }`
    Forever,
    /// `for (i = 3; i != 0; i--) { body }` (`loops`).
    For(Box<crate::decompile::loops::Counted>),
}

#[derive(Debug, Clone)]
pub enum Node {
    /// A block's own statements (and its label, if a goto names it).
    Block(BlockId),
    If {
        cond: Expr,
        then: Vec<Node>,
        els: Vec<Node>,
        /// The block whose branch this is.
        at: BlockId,
    },
    Loop {
        kind: LoopKind,
        body: Vec<Node>,
        head: BlockId,
        /// The block the condition came from.
        at: BlockId,
    },
    Switch {
        index: Expr,
        /// Case values sharing a body, and the body.
        cases: Vec<(Vec<u32>, Vec<Node>)>,
        at: BlockId,
    },
    Goto(BlockId, BlockId),
    Break(BlockId),
    Continue(BlockId),
    /// Leave the function: a return, a tail call, or an unfollowed transfer.
    /// The block that ends so, and the block the transfer came from.
    Exit(BlockId, BlockId),
}

/// Recursion limit for nesting; deeper structure falls back to gotos.
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy)]
struct Ctx<'l> {
    /// The innermost loop and its follow.
    lp: Option<(&'l Loop, Option<BlockId>)>,
    /// Inside a `switch` within that loop: `break` would leave the switch.
    in_switch: bool,
    depth: usize,
}

pub struct Structurer<'a> {
    cfg: &'a Cfg,
    blocks: &'a [LiftedBlock],
    visited: BTreeSet<BlockId>,
    pub gotos: BTreeSet<BlockId>,
}

impl<'a> Structurer<'a> {
    pub fn new(cfg: &'a Cfg, blocks: &'a [LiftedBlock]) -> Self {
        Self {
            cfg,
            blocks,
            visited: BTreeSet::new(),
            gotos: BTreeSet::new(),
        }
    }

    pub fn run(mut self) -> (Vec<Node>, BTreeSet<BlockId>) {
        let ctx = Ctx {
            lp: None,
            in_switch: false,
            depth: 0,
        };
        let mut out = Vec::new();
        self.seq(self.cfg.entry, None, ctx, &mut out, self.cfg.entry);
        tidy(&mut out, false);
        normalize(&mut out, self.blocks);
        // A `return;` that ends the function says nothing.
        if let Some(Node::Exit(b, _)) = out.last()
            && matches!(self.cfg.blocks[*b].term, Term::Return)
            && self.blocks[*b].exit.as_ref().is_none_or(|x| x.is_plain())
        {
            out.pop();
        }
        (out, self.gotos)
    }

    fn loop_of(&self, h: BlockId) -> Option<&'a Loop> {
        self.cfg.loops.iter().find(|l| l.header == h)
    }

    fn in_body(l: &Loop, b: BlockId) -> bool {
        l.body.binary_search(&b).is_ok()
    }

    /// The innermost loop containing `b`.
    fn innermost(&self, b: BlockId) -> Option<&'a Loop> {
        self.cfg
            .loops
            .iter()
            .filter(|l| Self::in_body(l, b))
            .min_by_key(|l| l.body.len())
    }

    fn is_exit(&self, b: BlockId) -> bool {
        matches!(
            self.cfg.blocks[b].term,
            Term::Return | Term::Halt | Term::Tail(_) | Term::Unknown(_)
        )
    }

    /// Control to `t` from `from`: a `continue`, `break`, exit or goto if it
    /// cannot be emitted in place; `None` to go on emitting `t`.
    fn jump(&mut self, t: BlockId, from: BlockId, ctx: Ctx, out: &mut Vec<Node>) -> Option<()> {
        if let Some((l, follow)) = ctx.lp {
            // `continue` means the loop even inside a switch; `break` would
            // leave the switch, so there it is a goto.
            if t == l.header {
                out.push(Node::Continue(from));
                return Some(());
            }
            if Some(t) == follow {
                if ctx.in_switch {
                    self.gotos.insert(t);
                    out.push(Node::Goto(t, from));
                } else {
                    out.push(Node::Break(from));
                }
                return Some(());
            }
        }
        if self.cfg.blocks[t].is_stub() || (self.is_exit(t) && self.blocks[t].lines.is_empty()) {
            out.push(Node::Exit(t, from));
            return Some(());
        }
        if self.visited.contains(&t) {
            self.gotos.insert(t);
            out.push(Node::Goto(t, from));
            return Some(());
        }
        None
    }

    /// Emit from `b` until `stop`.
    fn seq(
        &mut self,
        b: BlockId,
        stop: Option<BlockId>,
        ctx: Ctx,
        out: &mut Vec<Node>,
        from: BlockId,
    ) {
        self.seq_from(b, stop, ctx, out, from, false)
    }

    /// Emit from `b` until `stop`. `body_start`: `b` is the header of the
    /// loop being emitted, so it is neither a `continue` nor a new loop.
    fn seq_from(
        &mut self,
        mut b: BlockId,
        stop: Option<BlockId>,
        ctx: Ctx,
        out: &mut Vec<Node>,
        mut from: BlockId,
        mut body_start: bool,
    ) {
        if ctx.depth > MAX_DEPTH {
            if Some(b) != stop && self.jump(b, from, ctx, out).is_none() {
                self.gotos.insert(b);
                out.push(Node::Goto(b, from));
            }
            return;
        }
        loop {
            if !body_start {
                if Some(b) == stop {
                    return;
                }
                if self.jump(b, from, ctx, out).is_some() {
                    return;
                }
                // A loop header not yet entered starts its loop.
                if let Some(l) = self.loop_of(b) {
                    match self.emit_loop(l, ctx, out) {
                        Some(follow) => {
                            from = b;
                            b = follow;
                            continue;
                        }
                        None => return,
                    }
                }
            }
            body_start = false;
            self.visited.insert(b);
            out.push(Node::Block(b));
            let lb = &self.blocks[b];
            match self.cfg.blocks[b].term.clone() {
                Term::Fall(t) | Term::Goto(t) => {
                    from = b;
                    b = t;
                }
                Term::Branch { taken, fall } => {
                    let cond = lb.cond.clone().unwrap_or(Expr::Const(1));
                    match self.branch(b, cond, taken, fall, stop, ctx, out) {
                        Some(next) => {
                            from = b;
                            b = next;
                        }
                        None => return,
                    }
                }
                Term::Switch { cases, .. } => match self.switch(b, &cases, stop, ctx, out) {
                    Some(next) => {
                        from = b;
                        b = next;
                    }
                    None => return,
                },
                Term::Return | Term::Halt | Term::Tail(_) | Term::Unknown(_) => {
                    out.push(Node::Exit(b, b));
                    return;
                }
            }
        }
    }

    /// The join after a branch at `b`: its immediate post-dominator, when
    /// that is inside the same loop (or both are in none).
    fn join(&self, b: BlockId, stop: Option<BlockId>) -> Option<BlockId> {
        let p = self.cfg.ipdom[b]?;
        let same = match (self.innermost(b), self.innermost(p)) {
            (Some(x), Some(y)) => x.header == y.header,
            (None, None) => true,
            (Some(x), None) => {
                // Leaving the loop: only its own follow can be a join.
                let _ = x;
                false
            }
            (None, Some(_)) => false,
        };
        (same || Some(p) == stop).then_some(p)
    }

    #[allow(clippy::too_many_arguments)]
    fn branch(
        &mut self,
        b: BlockId,
        cond: Expr,
        taken: BlockId,
        fall: BlockId,
        stop: Option<BlockId>,
        ctx: Ctx,
        out: &mut Vec<Node>,
    ) -> Option<BlockId> {
        let join = self.join(b, stop).filter(|&j| j != b);
        let inner = Ctx {
            depth: ctx.depth + 1,
            ..ctx
        };
        let not = |c: &Expr| simplify(Expr::un(UnOp::LNot, c.clone()));
        let arm = |s: &mut Self, t: BlockId| {
            let mut v = Vec::new();
            if Some(t) != join {
                s.seq(t, join.or(stop), inner, &mut v, b);
            }
            v
        };
        let (cond, then, els) = if Some(taken) == join {
            (not(&cond), arm(self, fall), Vec::new())
        } else if Some(fall) == join {
            (cond, arm(self, taken), Vec::new())
        } else {
            let then = arm(self, taken);
            let els = arm(self, fall);
            (cond, then, els)
        };
        // An empty `then` with an `else` reads better the other way round.
        let (cond, then, els) = if then.is_empty() && !els.is_empty() {
            (not(&cond), els, Vec::new())
        } else {
            (cond, then, els)
        };
        out.push(Node::If {
            cond,
            then,
            els,
            at: b,
        });
        match join {
            Some(j) if Some(j) != stop => Some(j),
            _ => None,
        }
    }

    fn switch(
        &mut self,
        b: BlockId,
        cases: &[BlockId],
        stop: Option<BlockId>,
        ctx: Ctx,
        out: &mut Vec<Node>,
    ) -> Option<BlockId> {
        let join = self.join(b, stop);
        let values = self.blocks[b]
            .switch
            .clone()
            .map(|s| s.1)
            .unwrap_or_else(|| (0..cases.len() as u32).collect());
        let index = self.blocks[b]
            .switch
            .clone()
            .map(|s| s.0)
            .unwrap_or(Expr::Const(0));
        let inner = Ctx {
            in_switch: true,
            depth: ctx.depth + 1,
            ..ctx
        };
        // Cases sharing a target share a body.
        let mut groups: Vec<(BlockId, Vec<u32>)> = Vec::new();
        for (&c, v) in cases.iter().zip(values) {
            match groups.iter_mut().find(|g| g.0 == c) {
                Some(g) => g.1.push(v),
                None => groups.push((c, vec![v])),
            }
        }
        let mut arms = Vec::new();
        for (c, vals) in groups {
            let mut body = Vec::new();
            if Some(c) != join {
                self.seq(c, join.or(stop), inner, &mut body, b);
            }
            arms.push((vals, body));
        }
        out.push(Node::Switch {
            index,
            cases: arms,
            at: b,
        });
        match join {
            Some(j) if Some(j) != stop => Some(j),
            _ => None,
        }
    }

    /// Emit the loop headed by `l`; its follow, to go on from.
    fn emit_loop(&mut self, l: &'a Loop, ctx: Ctx, out: &mut Vec<Node>) -> Option<BlockId> {
        let h = l.header;
        let outside = |b: BlockId| !Self::in_body(l, b);
        let inner = |follow: Option<BlockId>| Ctx {
            lp: Some((l, follow)),
            in_switch: false,
            depth: ctx.depth + 1,
        };
        // do … while: one latch, branching back, and the only way out of
        // the body. Then every path through the body reaches the latch, so
        // the body can stop there and the latch's branch be the condition.
        if let [latch] = l.latches.as_slice()
            && let Term::Branch { taken, fall } = self.cfg.blocks[*latch].term
            && (taken == h) != (fall == h)
        {
            let exit = if taken == h { fall } else { taken };
            let only_exit = l
                .body
                .iter()
                .all(|&b| b == *latch || self.cfg.succs(b).iter().all(|&s| !outside(s)));
            if outside(exit) && only_exit {
                let cond = self.blocks[*latch].cond.clone().unwrap_or(Expr::Const(1));
                let cond = if taken == h {
                    cond
                } else {
                    simplify(Expr::un(UnOp::LNot, cond))
                };
                let mut body = Vec::new();
                if *latch == h {
                    self.visited.insert(h);
                    body.push(Node::Block(h));
                } else {
                    self.seq_from(h, Some(*latch), inner(Some(exit)), &mut body, h, true);
                    self.visited.insert(*latch);
                    body.push(Node::Block(*latch));
                }
                out.push(Node::Loop {
                    kind: LoopKind::DoWhile(cond),
                    body,
                    head: h,
                    at: *latch,
                });
                return Some(exit);
            }
        }
        // while: a header that only tests, one edge in and one out.
        if let Term::Branch { taken, fall } = self.cfg.blocks[h].term
            && self.blocks[h].lines.is_empty()
            && outside(taken) != outside(fall)
        {
            let cond = self.blocks[h].cond.clone().unwrap_or(Expr::Const(1));
            let (cond, first, follow) = if outside(fall) {
                (cond, taken, fall)
            } else {
                (simplify(Expr::un(UnOp::LNot, cond)), fall, taken)
            };
            self.visited.insert(h);
            let mut body = Vec::new();
            self.seq(first, None, inner(Some(follow)), &mut body, h);
            tidy(&mut body, true);
            out.push(Node::Loop {
                kind: LoopKind::While(cond),
                body,
                head: h,
                at: h,
            });
            return Some(follow);
        }
        // Otherwise for (;;), leaving by break to the first edge out.
        let follow = l
            .body
            .iter()
            .flat_map(|&b| self.cfg.succs(b))
            .filter(|&s| outside(s) && !self.cfg.blocks[s].is_stub())
            .min_by_key(|&s| {
                self.cfg
                    .rpo
                    .iter()
                    .position(|&r| r == s)
                    .unwrap_or(usize::MAX)
            });
        let mut body = Vec::new();
        self.seq_from(h, None, inner(follow), &mut body, h, true);
        tidy(&mut body, true);
        out.push(Node::Loop {
            kind: LoopKind::Forever,
            body,
            head: h,
            at: h,
        });
        follow
    }
}

/// Drop what says nothing: a `continue` at the end of a loop body.
fn tidy(nodes: &mut Vec<Node>, loop_body: bool) {
    if loop_body {
        strip_trailing_continue(nodes);
    }
}

fn strip_trailing_continue(nodes: &mut Vec<Node>) {
    match nodes.last_mut() {
        Some(Node::Continue(_)) => {
            nodes.pop();
        }
        Some(Node::If { then, els, .. }) => {
            strip_trailing_continue(then);
            strip_trailing_continue(els);
        }
        _ => {}
    }
}

/// After tidying: an `if` whose `then` came out empty tests the other way;
/// one with nothing in either arm, and a pure condition, is dropped.
fn normalize(nodes: &mut Vec<Node>, blocks: &[LiftedBlock]) {
    let _ = blocks;
    let mut i = 0;
    while i < nodes.len() {
        match &mut nodes[i] {
            Node::If {
                cond, then, els, ..
            } => {
                normalize(then, blocks);
                normalize(els, blocks);
                if then.is_empty() && !els.is_empty() {
                    *cond = simplify(Expr::un(UnOp::LNot, cond.clone()));
                    std::mem::swap(then, els);
                }
                if then.is_empty() && els.is_empty() && !cond.has_effects() {
                    nodes.remove(i);
                    continue;
                }
            }
            Node::Loop { body, .. } => normalize(body, blocks),
            Node::Switch { cases, .. } => cases.iter_mut().for_each(|(_, b)| normalize(b, blocks)),
            _ => {}
        }
        i += 1;
    }
}
