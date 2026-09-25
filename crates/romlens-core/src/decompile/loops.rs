//! Counted loops as `for` (the `full` level, after structuring).
//!
//! A loop is counted when a variable is set to a constant just before it,
//! changes in the loop only by a constant step at the end of its body, and
//! is all the loop's test reads. It prints as `for (x = 3; x != 0; x--)`,
//! with the test said the simplest way that holds: the counter's values
//! are few and known, so the loop is run here, value by value, and the
//! simpler test must agree with the original at every one. Where nothing
//! reads the counter after the loop and its values stay in range, it is an
//! `int` of its own, `for (int i = 0x5F; i >= 0; i--)`, which is how the
//! `DEX; BPL` idiom reads in C.

use std::collections::BTreeSet;

use crate::decompile::cfg::{BlockId, Cfg};
use crate::decompile::ir::{BinOp, CType, Expr, Line, Place, Stmt, UnOp, VarDecl};
use crate::decompile::lift::Lifted;
use crate::decompile::structure::{LoopKind, Node};

/// The most iterations run to check a test.
const MAX_STEPS: usize = 70_000;

/// A counted loop's header parts.
#[derive(Debug, Clone)]
pub struct Counted {
    pub var: u32,
    /// Declared in the `for` (`int i`).
    pub declare: bool,
    pub init: u32,
    pub cond: Expr,
    pub op: BinOp,
    pub step: u32,
    /// The instructions the init and step came from, for the line map.
    pub steps: Vec<usize>,
}

/// Rewrite the counted loops in `nodes`.
pub fn counted(nodes: &mut [Node], cfg: &Cfg, lifted: &mut Lifted) {
    let live = var_liveness(cfg, lifted);
    let mut next_name = 0usize;
    walk(nodes, cfg, lifted, &live, &mut next_name);
}

fn walk(
    nodes: &mut [Node],
    cfg: &Cfg,
    lifted: &mut Lifted,
    live: &[BTreeSet<u32>],
    names: &mut usize,
) {
    for i in 0..nodes.len() {
        // Inner loops first, so their counters take the later names.
        match &mut nodes[i] {
            Node::If { then, els, .. } => {
                walk(then, cfg, lifted, live, names);
                walk(els, cfg, lifted, live, names);
            }
            Node::Loop { body, .. } => walk(body, cfg, lifted, live, names),
            Node::Switch { cases, .. } => {
                for (_, b) in cases {
                    walk(b, cfg, lifted, live, names);
                }
            }
            _ => {}
        }
        if i == 0 {
            continue;
        }
        let Node::Block(before) = nodes[i - 1] else {
            continue;
        };
        if let Some((c, from)) = try_counted(before, &nodes[i], cfg, lifted, live, names)
            && let Node::Loop { kind, body, .. } = &mut nodes[i]
        {
            if c.declare {
                rename_nodes(body, from, c.var);
            }
            *kind = LoopKind::For(Box::new(c));
        }
    }
}

/// The blocks a loop body holds, and whether it has a `continue`.
fn body_blocks(nodes: &[Node], out: &mut Vec<BlockId>, cont: &mut bool) {
    for n in nodes {
        match n {
            Node::Block(b) | Node::Lines(b, ..) => out.push(*b),
            Node::Stmt { blocks, .. } => out.extend(blocks.iter().copied()),
            Node::If {
                then, els, merged, ..
            } => {
                out.extend(merged.iter().copied());
                body_blocks(then, out, cont);
                body_blocks(els, out, cont);
            }
            Node::Loop { body, .. } => body_blocks(body, out, cont),
            Node::Switch { cases, .. } => cases.iter().for_each(|(_, b)| body_blocks(b, out, cont)),
            Node::Continue(_) => *cont = true,
            _ => {}
        }
    }
}

fn try_counted(
    before: BlockId,
    node: &Node,
    cfg: &Cfg,
    lifted: &mut Lifted,
    live: &[BTreeSet<u32>],
    names: &mut usize,
) -> Option<(Counted, u32)> {
    let Node::Loop {
        kind, body, head, ..
    } = node
    else {
        return None;
    };
    let (cond, first_tested) = match kind {
        LoopKind::DoWhile(c) => (c.clone(), false),
        LoopKind::While(c) => (c.clone(), true),
        _ => return None,
    };
    // The init: the block before ends `v = c`.
    let Some(Line {
        stmt:
            Stmt::Assign {
                dst: Place::Var(v),
                value: Expr::Const(init),
            },
        step: init_step,
        ..
    }) = lifted.blocks[before].lines.last().cloned()
    else {
        return None;
    };
    let ty = lifted.vars.get(v as usize)?.ty;
    let bits = match ty {
        CType::U8 => 8,
        CType::U16 => 16,
        _ => return None,
    };
    // The step: the body's last block ends `v = v ± k`, and nothing else
    // in the body writes v.
    let mut blocks = Vec::new();
    let mut cont = false;
    body_blocks(body, &mut blocks, &mut cont);
    if cont {
        return None;
    }
    let last = *blocks.last()?;
    let Line {
        stmt:
            Stmt::Assign {
                dst: Place::Var(sv),
                value: Expr::Bin(op @ (BinOp::Add | BinOp::Sub), a, k),
            },
        step: step_step,
        ..
    } = lifted.blocks[last].lines.last()?.clone()
    else {
        return None;
    };
    let (Expr::Var(av), Some(k)) = (&*a, k.as_const()) else {
        return None;
    };
    if sv != v || *av != v || k == 0 {
        return None;
    }
    let n_lines = lifted.blocks[last].lines.len();
    for &b in &blocks {
        for (j, l) in lifted.blocks[b].lines.iter().enumerate() {
            if b == last && j + 1 == n_lines {
                continue;
            }
            if writes_var(&l.stmt, v) {
                return None;
            }
        }
    }
    // The test reads only the counter.
    let mut only = true;
    cond.walk(&mut |e| match e {
        Expr::Var(w) if *w != v => only = false,
        Expr::Const(_)
        | Expr::Var(_)
        | Expr::Bin(..)
        | Expr::Un(..)
        | Expr::Cast(..)
        | Expr::Signed(..) => {}
        _ => only = false,
    });
    if !only {
        return None;
    }
    // Run it: the values the body sees, and the one it leaves with.
    let mask = (1i64 << bits) - 1;
    let step = |x: i64| {
        if op == BinOp::Add {
            x + k as i64
        } else {
            x - k as i64
        }
    };
    let mut body_values: Vec<(i64, i64)> = Vec::new();
    let (mut wrapped, mut plain) = (init as i64 & mask, init as i64);
    if first_tested && !holds(&cond, v, wrapped)? {
        return None;
    }
    loop {
        body_values.push((wrapped, plain));
        if body_values.len() > MAX_STEPS {
            return None;
        }
        wrapped = step(wrapped) & mask;
        plain = step(plain);
        if !holds(&cond, v, wrapped)? {
            break;
        }
    }
    let exit = (wrapped, plain);

    // An `int` of its own where nothing reads the counter after the loop
    // and the body sees the same values.
    let exits: BTreeSet<BlockId> = blocks
        .iter()
        .flat_map(|&b| cfg.succs(b))
        .filter(|s| !blocks.contains(s) && *s != *head)
        .collect();
    let dead_after = exits.iter().all(|s| !live[*s].contains(&v));
    let same = body_values.iter().all(|(w, p)| w == p);
    let declare = dead_after && same;
    let simple = simpler(&cond, v);
    // The test the `for` prints: the simpler one where it agrees at every
    // value (and holds at the first, where the original did not test it).
    let agrees = |c: &Expr, plain_values: bool| -> Option<bool> {
        let pick = |(w, p): (i64, i64)| if plain_values { p } else { w };
        for &pair in &body_values {
            if !holds(c, v, pick(pair))? {
                return Some(false);
            }
        }
        Some(!holds(c, v, pick(exit))?)
    };
    let (cond, declare) = if declare && agrees(&simple, true)? {
        (simple, true)
    } else if agrees(&simple, false)? {
        (simple, false)
    } else if agrees(&cond, false)? {
        (cond, false)
    } else {
        return None;
    };

    // The init and the step move into the header.
    lifted.blocks[before].lines.pop();
    lifted.blocks[last].lines.pop();
    let var = if declare {
        let name = ["i", "j", "k"].get(*names).map(|s| s.to_string());
        *names += 1;
        let name = name.unwrap_or_else(|| format!("i{}", *names));
        let slot = lifted.vars[v as usize].slot;
        lifted.vars.push(VarDecl {
            name,
            ty: CType::Int,
            slot,
            param: false,
            unused: true,
        });
        let i = lifted.vars.len() as u32 - 1;
        for &b in &blocks {
            rename_var(&mut lifted.blocks[b], v, i);
        }
        i
    } else {
        v
    };
    let cond = if declare {
        rename_expr(cond, v, var)
    } else {
        cond
    };
    Some((
        Counted {
            var,
            declare,
            init,
            cond,
            op,
            step: k,
            steps: vec![init_step, step_step],
        },
        v,
    ))
}

/// The tests the structure copied out of the blocks, renamed too.
fn rename_nodes(nodes: &mut [Node], from: u32, to: u32) {
    for n in nodes {
        match n {
            Node::If {
                cond, then, els, ..
            } => {
                *cond = rename_expr(cond.clone(), from, to);
                rename_nodes(then, from, to);
                rename_nodes(els, from, to);
            }
            Node::Loop { kind, body, .. } => {
                match kind {
                    LoopKind::While(c) | LoopKind::DoWhile(c) => {
                        *c = rename_expr(c.clone(), from, to)
                    }
                    LoopKind::For(c) => c.cond = rename_expr(c.cond.clone(), from, to),
                    LoopKind::Forever => {}
                }
                rename_nodes(body, from, to);
            }
            Node::Stmt {
                stmt: Stmt::Assign { value, .. },
                ..
            } => *value = rename_expr(value.clone(), from, to),
            Node::Switch { index, cases, .. } => {
                *index = rename_expr(index.clone(), from, to);
                for (_, b) in cases {
                    rename_nodes(b, from, to);
                }
            }
            _ => {}
        }
    }
}

fn writes_var(s: &Stmt, v: u32) -> bool {
    match s {
        Stmt::Assign { dst, .. } => *dst == Place::Var(v),
        Stmt::Invoke { ret, outs, .. } => {
            ret.as_ref() == Some(&Place::Var(v)) || outs.contains(&Place::Var(v))
        }
        _ => false,
    }
}

/// The test with the counter's casts taken off: `(s8)x >= 0` is `x >= 0`,
/// `(s16)(x - 0x0200) < 0` is `x < 0x0200`.
fn simpler(e: &Expr, v: u32) -> Expr {
    let strip = |x: &Expr| -> Expr {
        match x {
            Expr::Signed(_, inner) | Expr::Cast(_, inner) => (**inner).clone(),
            x => x.clone(),
        }
    };
    match e {
        Expr::Bin(op, a, b) if is_cmp(*op) => {
            let a = strip(a);
            let b = strip(b);
            // (x - K) op 0  →  x op K
            if let (Expr::Bin(BinOp::Sub, x, k), Expr::Const(0)) = (&a, &b)
                && matches!(**x, Expr::Var(w) if w == v)
                && k.is_const()
            {
                return Expr::Bin(*op, x.clone(), k.clone());
            }
            Expr::Bin(*op, Box::new(a), Box::new(b))
        }
        Expr::Un(UnOp::LNot, x) => Expr::un(UnOp::LNot, simpler(x, v)),
        e => e.clone(),
    }
}

fn is_cmp(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
    )
}

/// `e` as C computes it, with the counter `v` at `x`: operands promote to
/// `int`, a cast truncates, a signed cast extends the sign. `None` for
/// anything but a counter, constants and arithmetic.
fn holds(e: &Expr, v: u32, x: i64) -> Option<bool> {
    Some(value(e, v, x)? != 0)
}

fn value(e: &Expr, v: u32, x: i64) -> Option<i64> {
    Some(match e {
        Expr::Const(c) => *c as i64,
        Expr::Var(w) if *w == v => x,
        Expr::Cast(w, inner) => value(inner, v, x)? & w.mask() as i64,
        Expr::Signed(w, inner) => {
            let bits = w.bits() as i64;
            let m = value(inner, v, x)? & ((1i64 << bits) - 1);
            if m >> (bits - 1) & 1 == 1 {
                m - (1i64 << bits)
            } else {
                m
            }
        }
        Expr::Un(UnOp::LNot, inner) => (value(inner, v, x)? == 0) as i64,
        Expr::Un(UnOp::Not, inner) => !value(inner, v, x)?,
        Expr::Bin(op, a, b) => {
            let (a, b) = (value(a, v, x)?, value(b, v, x)?);
            match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::And => a & b,
                BinOp::Or => a | b,
                BinOp::Xor => a ^ b,
                BinOp::Shl => a.checked_shl(b as u32)?,
                BinOp::Shr => a.checked_shr(b as u32)?,
                BinOp::Eq => (a == b) as i64,
                BinOp::Ne => (a != b) as i64,
                BinOp::Lt => (a < b) as i64,
                BinOp::Le => (a <= b) as i64,
                BinOp::Gt => (a > b) as i64,
                BinOp::Ge => (a >= b) as i64,
                BinOp::LAnd => (a != 0 && b != 0) as i64,
                BinOp::LOr => (a != 0 || b != 0) as i64,
            }
        }
        _ => return None,
    })
}

fn rename_expr(e: Expr, from: u32, to: u32) -> Expr {
    match e {
        Expr::Var(v) if v == from => Expr::Var(to),
        Expr::Step(op, v) if v == from => Expr::Step(op, to),
        Expr::Mem { addr, width } => Expr::mem(rename_expr(*addr, from, to), width),
        Expr::Un(op, x) => Expr::Un(op, Box::new(rename_expr(*x, from, to))),
        Expr::Bin(op, a, b) => Expr::Bin(
            op,
            Box::new(rename_expr(*a, from, to)),
            Box::new(rename_expr(*b, from, to)),
        ),
        Expr::Cast(w, x) => Expr::Cast(w, Box::new(rename_expr(*x, from, to))),
        Expr::Signed(w, x) => Expr::Signed(w, Box::new(rename_expr(*x, from, to))),
        Expr::Call(n, args) => Expr::Call(
            n,
            args.into_iter().map(|a| rename_expr(a, from, to)).collect(),
        ),
        e => e,
    }
}

/// Reads of `from` in a block's statements and tests, as `to`.
fn rename_var(lb: &mut crate::decompile::ir::LiftedBlock, from: u32, to: u32) {
    let r = |e: &mut Expr| *e = rename_expr(e.clone(), from, to);
    for l in &mut lb.lines {
        match &mut l.stmt {
            Stmt::Assign { dst, value } => {
                if let Place::Mem { addr, .. } = dst {
                    r(addr);
                }
                r(value);
            }
            Stmt::Invoke { args, .. } => args.iter_mut().for_each(r),
            Stmt::Effect(_, args) => args.iter_mut().for_each(r),
            Stmt::Eval(e) => r(e),
            Stmt::Call(crate::decompile::ir::CallTarget::Table { index, .. }) => r(index),
            _ => {}
        }
    }
    if let Some(c) = &mut lb.cond {
        r(c);
    }
    if let Some((s, _)) = &mut lb.switch {
        r(s);
    }
    if let Some(x) = &mut lb.exit {
        for (_, e) in &mut x.outs {
            r(e);
        }
        if let Some(e) = &mut x.ret {
            r(e);
        }
    }
}

/// Which variables are live at each block's entry.
fn var_liveness(cfg: &Cfg, lifted: &Lifted) -> Vec<BTreeSet<u32>> {
    let n = cfg.blocks.len();
    let vars_in = |e: &Expr, out: &mut BTreeSet<u32>| {
        e.walk(&mut |x| {
            if let Expr::Var(v) = x {
                out.insert(*v);
            }
        })
    };
    // Each block's statements, backwards: (reads, writes) per statement.
    let effects = |b: BlockId| -> Vec<(BTreeSet<u32>, BTreeSet<u32>)> {
        let lb = &lifted.blocks[b];
        let mut out = Vec::new();
        let stmts = lb
            .lines
            .iter()
            .map(|l| &l.stmt)
            .chain(lb.exit.iter().flat_map(|x| x.before.iter().chain(&x.after)));
        for s in stmts {
            let (mut r, mut w) = (BTreeSet::new(), BTreeSet::new());
            match s {
                Stmt::Assign { dst, value } => {
                    vars_in(value, &mut r);
                    match dst {
                        Place::Var(v) => {
                            w.insert(*v);
                        }
                        Place::Mem { addr, .. } => vars_in(addr, &mut r),
                        _ => {}
                    }
                }
                Stmt::Invoke {
                    args, ret, outs, ..
                } => {
                    args.iter().for_each(|a| vars_in(a, &mut r));
                    for p in ret.iter().chain(outs) {
                        if let Place::Var(v) = p {
                            w.insert(*v);
                        }
                    }
                }
                Stmt::Effect(_, args) => args.iter().for_each(|a| vars_in(a, &mut r)),
                Stmt::Eval(e) => vars_in(e, &mut r),
                Stmt::Call(crate::decompile::ir::CallTarget::Table { index, .. }) => {
                    vars_in(index, &mut r)
                }
                _ => {}
            }
            out.push((r, w));
        }
        let mut end = BTreeSet::new();
        if let Some(c) = &lb.cond {
            vars_in(c, &mut end);
        }
        if let Some((s, _)) = &lb.switch {
            vars_in(s, &mut end);
        }
        if let Some(x) = &lb.exit {
            x.outs.iter().for_each(|(_, e)| vars_in(e, &mut end));
            if let Some(e) = &x.ret {
                vars_in(e, &mut end);
            }
        }
        out.push((end, BTreeSet::new()));
        out
    };
    let per: Vec<Vec<(BTreeSet<u32>, BTreeSet<u32>)>> = (0..n).map(effects).collect();
    let mut live_in: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); n];
    let mut order = cfg.rpo.clone();
    order.reverse();
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &order {
            let mut live: BTreeSet<u32> = BTreeSet::new();
            for s in cfg.succs(b) {
                live.extend(live_in[s].iter().copied());
            }
            // The tests and returned values come last.
            for (r, w) in per[b].iter().rev() {
                for v in w {
                    live.remove(v);
                }
                live.extend(r.iter().copied());
            }
            if live != live_in[b] {
                live_in[b] = live;
                changed = true;
            }
        }
    }
    live_in
}
