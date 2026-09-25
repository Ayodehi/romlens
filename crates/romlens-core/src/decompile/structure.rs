//! Structuring: the graph as `if`, loops and `switch`, with a goto only
//! where it does not reduce.
//!
//! Emission walks the graph from the entry. A natural loop becomes
//! `do … while` when its one back edge is a conditional branch (the usual
//! 65816 shape, `DEX; BPL`), `while` when its header only tests, and
//! `for (;;)` otherwise, with `break` and `continue` for the edges out and
//! back. A conditional branch becomes `if`/`else` joined at its immediate
//! post-dominator, and a branch whose other way is only another test (the
//! `BEQ`, `BEQ` of two checks for one outcome) becomes one `if` with `&&`
//! or `||`. A block reached a second time, which the structure could
//! not place, is a `goto` to its label. Iterative where it matters: the
//! recursion is bounded by nesting depth, not by the routine's length.

use std::collections::BTreeSet;

use crate::decompile::cfg::{BlockId, Cfg, Loop, Term};
use crate::decompile::dataflow::simplify;
use crate::decompile::ir::{BinOp, Expr, LiftedBlock, Place, Stmt, UnOp};

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
    /// Some of a block's statements, `from..to`: the rest are in the test
    /// after it or after the `if` both ways share. With its label, if a
    /// goto names it, when the last is `true`.
    Lines(BlockId, usize, usize, bool),
    /// A statement the structure made from an `if`: `c = a != 0;` for one
    /// whose arms set `c` to 1 and 0. The instructions it stands for and
    /// the blocks it came from.
    Stmt {
        stmt: Stmt,
        steps: Vec<usize>,
        blocks: Vec<BlockId>,
    },
    If {
        cond: Expr,
        then: Vec<Node>,
        els: Vec<Node>,
        /// The block whose branch this is.
        at: BlockId,
        /// Blocks whose tests the condition takes in, and the instructions
        /// they and a stepped variable came from.
        merged: Vec<BlockId>,
        steps: Vec<usize>,
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
        normalize(&mut out, self.cfg, self.blocks, &self.gotos);
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
        let (cond, taken, fall, merged, steps) =
            self.combine(b, cond, taken, fall, join, stop, ctx, out);
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
            merged,
            steps,
        });
        match join {
            Some(j) if Some(j) != stop => Some(j),
            _ => None,
        }
    }

    /// The branch at `b` with the tests after it that only choose between
    /// the same two ways taken in: `if (c1) goto T; if (c2) goto T;` is
    /// `if (c1 || c2) goto T`, and `if (!c1) goto F; if (c2) goto T; goto
    /// F;` is `if (c1 && c2) goto T`. Each step keeps the meaning exactly,
    /// and `&&` and `||` test the second only when the first did not
    /// decide, as the branches did. Returns the test, where it goes each
    /// way, the blocks taken in and their instructions.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn combine(
        &mut self,
        b: BlockId,
        mut cond: Expr,
        mut taken: BlockId,
        mut fall: BlockId,
        join: Option<BlockId>,
        stop: Option<BlockId>,
        ctx: Ctx,
        out: &mut [Node],
    ) -> (Expr, BlockId, BlockId, Vec<BlockId>, Vec<usize>) {
        let mut merged: Vec<BlockId> = Vec::new();
        let mut steps = Vec::new();
        // The block's own last statement, `y++`, can be the first test's
        // `++y` once there is a combined test to read it in.
        let head = self.blocks[b]
            .lines
            .last()
            .and_then(|l| step_into(&l.stmt, &cond).map(|c| (c, l.step)));
        let first = cond.clone();
        loop {
            let mut grew = false;
            for side in [false, true] {
                let y = if side { taken } else { fall };
                if Some(y) == join || Some(y) == stop {
                    continue;
                }
                let Some((cy, yt, yf, ysteps)) = self.test_only(y, b, &merged, ctx) else {
                    continue;
                };
                let (op, cy, t, f) = if !side && yt == taken {
                    (BinOp::LOr, cy, taken, yf)
                } else if !side && yf == taken {
                    (BinOp::LOr, simplify(Expr::un(UnOp::LNot, cy)), taken, yt)
                } else if side && yf == fall {
                    (BinOp::LAnd, cy, yt, fall)
                } else if side && yt == fall {
                    (BinOp::LAnd, simplify(Expr::un(UnOp::LNot, cy)), yf, fall)
                } else {
                    continue;
                };
                cond = Expr::bin(op, cond, cy);
                (taken, fall) = (t, f);
                merged.push(y);
                steps.extend(ysteps);
                self.visited.insert(y);
                grew = true;
            }
            if !grew {
                break;
            }
        }
        if !merged.is_empty()
            && let Some((c, at)) = head
            && let Some(Node::Block(last)) = out.last()
            && *last == b
        {
            cond = replace_first(cond, &first, c);
            let n = self.blocks[b].lines.len() - 1;
            *out.last_mut().unwrap() = Node::Lines(b, 0, n, true);
            steps.push(at);
        }
        (cond, taken, fall, merged, steps)
    }

    /// A block that only tests, reached only from `b` or a block already
    /// taken into its test, in the same loop: its test and ways, and its
    /// instructions. It may also step a variable its test reads, which
    /// becomes `++v` in the test.
    fn test_only(
        &self,
        y: BlockId,
        b: BlockId,
        merged: &[BlockId],
        ctx: Ctx,
    ) -> Option<(Expr, BlockId, BlockId, Vec<usize>)> {
        let Term::Branch { taken, fall } = self.cfg.blocks[y].term else {
            return None;
        };
        let cb = &self.cfg.blocks[y];
        if self.visited.contains(&y) || self.loop_of(y).is_some() || cb.is_stub() {
            return None;
        }
        if let Some((l, follow)) = ctx.lp
            && (y == l.header || Some(y) == follow)
        {
            return None;
        }
        match cb.preds.as_slice() {
            [p] if *p == b || merged.contains(p) => {}
            _ => return None,
        }
        if self.innermost(y).map(|l| l.header) != self.innermost(b).map(|l| l.header) {
            return None;
        }
        let lb = &self.blocks[y];
        let cond = lb.cond.clone()?;
        let cond = match lb.lines.as_slice() {
            [] => cond,
            [line] => step_into(&line.stmt, &cond)?,
            _ => return None,
        };
        let mut steps: Vec<usize> = lb.lines.iter().flat_map(|l| l.steps()).collect();
        steps.extend(lb.term_step);
        steps.extend(lb.term_merged.iter().copied());
        Some((cond, taken, fall, steps))
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
/// one with nothing in either arm, and a pure condition, is dropped; arms
/// that end the routine the same way share that ending after the `if`; and
/// arms that only set one variable to 1 and 0 are that variable set to the
/// test.
fn normalize(nodes: &mut Vec<Node>, cfg: &Cfg, blocks: &[LiftedBlock], gotos: &BTreeSet<BlockId>) {
    let mut i = 0;
    while i < nodes.len() {
        match &mut nodes[i] {
            Node::If {
                cond, then, els, ..
            } => {
                normalize(then, cfg, blocks, gotos);
                normalize(els, cfg, blocks, gotos);
                if then.is_empty() && !els.is_empty() {
                    *cond = simplify(Expr::un(UnOp::LNot, cond.clone()));
                    std::mem::swap(then, els);
                }
                if then.is_empty() && els.is_empty() && !cond.has_effects() {
                    nodes.remove(i);
                    continue;
                }
                if let Some(tail) = share_tail(then, els, cfg, blocks) {
                    nodes.splice(i + 1..i + 1, tail);
                }
                if let Some(set) = as_assignment(&nodes[i], blocks, gotos) {
                    nodes[i] = set;
                    i += 1;
                    continue;
                }
                let Node::If { then, els, .. } = &mut nodes[i] else {
                    unreachable!()
                };
                // `if (c) { …; return; } else { rest }` is `if (c) { …;
                // return; } rest`: nothing after the `then` reaches it.
                if leaves(then) && !els.is_empty() {
                    let rest = std::mem::take(els);
                    nodes.splice(i + 1..i + 1, rest);
                }
            }
            Node::Loop { body, .. } => normalize(body, cfg, blocks, gotos),
            Node::Switch { cases, .. } => cases
                .iter_mut()
                .for_each(|(_, b)| normalize(b, cfg, blocks, gotos)),
            _ => {}
        }
        i += 1;
    }
}

/// Where in an arm a block's statements are: the node's index, and which
/// of them it prints.
type Span = Option<(usize, usize, usize)>;

/// An arm that ends the routine: the block that leaves, and its
/// statements.
fn ending(arm: &[Node], blocks: &[LiftedBlock]) -> Option<(BlockId, Span)> {
    let Some(Node::Exit(x, _)) = arm.last() else {
        return None;
    };
    let at = arm.len().checked_sub(2);
    let lines = match at.map(|k| (k, &arm[k])) {
        Some((k, Node::Block(b))) if b == x => Some((k, 0, blocks[*b].lines.len())),
        Some((k, Node::Lines(b, from, to, true))) if b == x => Some((k, *from, *to)),
        _ => None,
    };
    Some((*x, lines))
}

/// Both arms leave the routine the same way (the same kind of return, with
/// the same results): their last statements in common and the way out, to
/// go after the `if`, with the arms cut to what differs.
fn share_tail(
    then: &mut Vec<Node>,
    els: &mut Vec<Node>,
    cfg: &Cfg,
    blocks: &[LiftedBlock],
) -> Option<Vec<Node>> {
    let (a, la) = ending(then, blocks)?;
    let (b, lb) = ending(els, blocks)?;
    if a == b || cfg.blocks[a].term != cfg.blocks[b].term || blocks[a].exit != blocks[b].exit {
        return None;
    }
    let range = |l: Span, x: BlockId| match l {
        Some((_, f, t)) => &blocks[x].lines[f..t],
        None => &[],
    };
    let (ra, rb) = (range(la, a), range(lb, b));
    let common = ra
        .iter()
        .rev()
        .zip(rb.iter().rev())
        .take_while(|(p, q)| p.stmt == q.stmt)
        .count();
    let Some(Node::Exit(_, from)) = then.pop() else {
        unreachable!()
    };
    els.pop();
    let cut = |arm: &mut Vec<Node>, l: Span, x: BlockId| {
        if let Some((k, f, t)) = l {
            arm[k] = Node::Lines(x, f, t - common, true);
        }
    };
    cut(then, la, a);
    cut(els, lb, b);
    let mut tail = Vec::new();
    if let Some((_, _, t)) = la
        && common > 0
    {
        tail.push(Node::Lines(a, t - common, t, false));
    }
    tail.push(Node::Exit(a, from));
    Some(tail)
}

/// `if (c) v = 1; else v = 0;` as `v = c;` (or `v = !c;` the other way).
fn as_assignment(node: &Node, blocks: &[LiftedBlock], gotos: &BTreeSet<BlockId>) -> Option<Node> {
    let Node::If {
        cond,
        then,
        els,
        at,
        merged,
        steps,
    } = node
    else {
        return None;
    };
    let only = |arm: &[Node]| -> Option<(BlockId, &crate::decompile::ir::Line)> {
        let (b, lines) = match arm {
            [Node::Block(b)] => (*b, &blocks[*b].lines[..]),
            [Node::Lines(b, f, t, _)] => (*b, &blocks[*b].lines[*f..*t]),
            _ => return None,
        };
        match lines {
            [l] => Some((b, l)),
            _ => None,
        }
    };
    let (ba, la) = only(then)?;
    let (bb, lb) = only(els)?;
    // A goto into either arm needs its label.
    if gotos.contains(&ba) || gotos.contains(&bb) {
        return None;
    }
    let (
        Stmt::Assign {
            dst,
            value: Expr::Const(x),
        },
        Stmt::Assign {
            dst: dst2,
            value: Expr::Const(y),
        },
    ) = (&la.stmt, &lb.stmt)
    else {
        return None;
    };
    if dst != dst2 || matches!(dst, Place::Mem { .. }) || !cond.is_boolean() {
        return None;
    }
    let value = match (x, y) {
        (1, 0) => cond.clone(),
        (0, 1) => simplify(Expr::un(UnOp::LNot, cond.clone())),
        _ => return None,
    };
    let lb_at = &blocks[*at];
    let mut all: Vec<usize> = lb_at.term_step.into_iter().collect();
    all.extend(lb_at.term_merged.iter().copied());
    all.extend(steps.iter().copied());
    all.extend(la.steps());
    all.extend(lb.steps());
    let mut from = vec![ba, bb];
    from.extend(merged.iter().copied());
    Some(Node::Stmt {
        stmt: Stmt::Assign {
            dst: dst.clone(),
            value,
        },
        steps: all,
        blocks: from,
    })
}

/// `v = v ± 1` then a test that reads `v` once, directly: the test with
/// `++v` (or `--v`) in its place.
fn step_into(stmt: &Stmt, cond: &Expr) -> Option<Expr> {
    let Stmt::Assign {
        dst: Place::Var(v),
        value: Expr::Bin(op @ (BinOp::Add | BinOp::Sub), a, one),
    } = stmt
    else {
        return None;
    };
    if **a != Expr::Var(*v) || **one != Expr::Const(1) {
        return None;
    }
    let mut reads = 0;
    cond.walk(&mut |e| {
        if matches!(e, Expr::Var(w) | Expr::Step(_, w) if w == v) {
            reads += 1;
        }
    });
    if reads != 1 {
        return None;
    }
    let var = Expr::Var(*v);
    let step = Expr::Step(*op, *v);
    match cond {
        Expr::Bin(cmp, l, r) if **l == var => Some(Expr::Bin(*cmp, Box::new(step), r.clone())),
        Expr::Bin(cmp, l, r) if **r == var => Some(Expr::Bin(*cmp, l.clone(), Box::new(step))),
        Expr::Un(UnOp::LNot, x) if **x == var => Some(Expr::Un(UnOp::LNot, Box::new(step))),
        e if *e == var => Some(step),
        _ => None,
    }
}

/// `e` with the leftmost operand of its `&&`/`||` chain that is `old`
/// replaced by `new`.
fn replace_first(e: Expr, old: &Expr, new: Expr) -> Expr {
    if e == *old {
        return new;
    }
    match e {
        Expr::Bin(op @ (BinOp::LAnd | BinOp::LOr), a, b) => {
            Expr::Bin(op, Box::new(replace_first(*a, old, new)), b)
        }
        e => e,
    }
}

/// Whether control never runs past the end of `nodes`.
fn leaves(nodes: &[Node]) -> bool {
    match nodes.last() {
        Some(Node::Exit(..) | Node::Goto(..) | Node::Break(_) | Node::Continue(_)) => true,
        Some(Node::If { then, els, .. }) => leaves(then) && leaves(els),
        _ => false,
    }
}
