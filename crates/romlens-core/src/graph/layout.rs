//! A layered layout for a directed graph whose boxes the shell has measured
//! (docs/19): boxes in rows top to bottom along the edges, edges as
//! polylines that bend around boxes rather than through them.
//!
//! 1. Edges marked back (and any others a depth-first walk finds closing a
//!    cycle) are turned around, and each box's row is its longest path
//!    from a box nothing enters.
//! 2. An edge spanning rows gets an invisible point in each row it
//!    crosses. A back edge also gets one in its own two rows, kept just to
//!    the right of the box at each end, so it climbs in a column of its own.
//! 3. Each row's order starts depth-first and moves boxes to the median of
//!    their neighbours, sweeping down and up; the order with the fewest
//!    crossings wins.
//! 4. Each row's positions are the closest to the median of their
//!    neighbours that keeps the order and the gaps: a weighted isotonic
//!    regression, solved exactly by pooling adjacent violators.
//! 5. Edges leave the bottom of a box and enter the top, at ports spread
//!    across its width in the order their other ends lie.
//!
//! Deterministic: the same input always gives the same output.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeSize {
    pub width: f64,
    pub height: f64,
}

/// An edge to lay out. `back` marks one that goes back up (a loop's way
/// back to its header); it is drawn climbing, not falling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeSpec {
    pub from: usize,
    pub to: usize,
    pub back: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutOptions {
    /// Between two boxes in a row.
    pub node_gap: f64,
    /// Between rows, where the edges run.
    pub layer_gap: f64,
    /// Between an edge passing a row and whatever is beside it.
    pub edge_gap: f64,
    /// Ordering sweeps; each is one pass down or up.
    pub sweeps: usize,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        LayoutOptions {
            node_gap: 24.0,
            layer_gap: 40.0,
            edge_gap: 12.0,
            sweeps: 16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Each box's top-left corner.
    pub nodes: Vec<Point>,
    /// Each box's row, from 0 at the top.
    pub rows: Vec<u32>,
    /// One per input edge, in order: from the source's bottom to the
    /// target's top.
    pub edges: Vec<LayoutEdge>,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutEdge {
    pub points: Vec<Point>,
    /// Drawn climbing: a back edge, or one turned to break a cycle.
    pub climbs: bool,
}

/// Past this many boxes and edge points the order is not improved: the
/// first depth-first order is kept.
const ORDER_LIMIT: usize = 6000;

struct VNode {
    layer: usize,
    width: f64,
    real: bool,
    /// A back edge's point kept just right of this box.
    anchor: Option<usize>,
    up: Vec<usize>,
    down: Vec<usize>,
}

pub fn layout(sizes: &[NodeSize], edges: &[EdgeSpec], opts: &LayoutOptions) -> Layout {
    let n = sizes.len();
    if n == 0 {
        return Layout {
            nodes: Vec::new(),
            rows: Vec::new(),
            edges: edges
                .iter()
                .map(|e| LayoutEdge {
                    points: Vec::new(),
                    climbs: e.back,
                })
                .collect(),
            width: 0.0,
            height: 0.0,
        };
    }
    let valid = |e: &EdgeSpec| e.from < n && e.to < n;

    // 1. Orientation and rows.
    let mut climbs: Vec<bool> = edges.iter().map(|e| e.back && e.from != e.to).collect();
    let (preorder, topo) = orient(n, edges, &mut climbs);
    let ends = |i: usize| {
        let e = edges[i];
        if climbs[i] {
            (e.to, e.from)
        } else {
            (e.from, e.to)
        }
    };
    let mut layer = vec![0usize; n];
    {
        let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, e) in edges.iter().enumerate() {
            if valid(e) && e.from != e.to {
                out[ends(i).0].push(i);
            }
        }
        for &u in &topo {
            for &i in &out[u] {
                let v = ends(i).1;
                layer[v] = layer[v].max(layer[u] + 1);
            }
        }
    }
    let rows = layer.iter().max().copied().unwrap_or(0) + 1;

    // 2. Boxes and the points edges pass through.
    let mut vn: Vec<VNode> = (0..n)
        .map(|i| VNode {
            layer: layer[i],
            width: sizes[i].width.max(0.0),
            real: true,
            anchor: None,
            up: Vec::new(),
            down: Vec::new(),
        })
        .collect();
    let link = |vn: &mut Vec<VNode>, a: usize, b: usize| {
        vn[a].down.push(b);
        vn[b].up.push(a);
    };
    let dummy = |vn: &mut Vec<VNode>, layer: usize, anchor: Option<usize>| {
        vn.push(VNode {
            layer,
            width: 0.0,
            real: false,
            anchor,
            up: Vec::new(),
            down: Vec::new(),
        });
        vn.len() - 1
    };
    // Per edge, its points from top to bottom.
    let mut chains: Vec<Vec<usize>> = vec![Vec::new(); edges.len()];
    for (i, e) in edges.iter().enumerate() {
        if !valid(e) {
            continue;
        }
        if e.from == e.to {
            chains[i].push(dummy(&mut vn, layer[e.from], Some(e.from)));
            continue;
        }
        let (a, b) = ends(i);
        if climbs[i] {
            let mut prev = None;
            for l in layer[a]..=layer[b] {
                let anchor = if l == layer[a] {
                    Some(a)
                } else if l == layer[b] {
                    Some(b)
                } else {
                    None
                };
                let d = dummy(&mut vn, l, anchor);
                if let Some(p) = prev {
                    link(&mut vn, p, d);
                }
                prev = Some(d);
                chains[i].push(d);
            }
        } else {
            let mut prev = a;
            for l in layer[a] + 1..layer[b] {
                let d = dummy(&mut vn, l, None);
                link(&mut vn, prev, d);
                prev = d;
                chains[i].push(d);
            }
            link(&mut vn, prev, b);
        }
    }

    // 3. Order within rows.
    let mut start = vec![0.0f64; vn.len()];
    for (i, &p) in preorder.iter().enumerate() {
        start[p] = i as f64;
    }
    for (i, e) in edges.iter().enumerate() {
        if !valid(e) {
            continue;
        }
        let (a, b) = ends(i);
        let k = chains[i].len() as f64 + 1.0;
        for (j, &d) in chains[i].iter().enumerate() {
            start[d] = start[a] + (start[b] - start[a]) * (j as f64 + 1.0) / k;
        }
    }
    let mut order: Vec<Vec<usize>> = vec![Vec::new(); rows];
    for (v, node) in vn.iter().enumerate() {
        order[node.layer].push(v);
    }
    for row in &mut order {
        row.sort_by(|&a, &b| start[a].total_cmp(&start[b]).then(a.cmp(&b)));
        fix_anchors(row, &vn);
    }
    if vn.len() <= ORDER_LIMIT {
        let mut best = order.clone();
        let mut best_cross = crossings(&order, &vn);
        for sweep in 0..opts.sweeps {
            if best_cross == 0 {
                break;
            }
            let down = sweep % 2 == 0;
            let rows_in: Vec<usize> = if down {
                (1..rows).collect()
            } else {
                (0..rows.saturating_sub(1)).rev().collect()
            };
            for l in rows_in {
                let pos = positions(&order, vn.len());
                let row = &mut order[l];
                let key: Vec<f64> = row
                    .iter()
                    .map(|&v| {
                        let nb = if down { &vn[v].up } else { &vn[v].down };
                        median(nb.iter().map(|&u| pos[u] as f64)).unwrap_or(pos[v] as f64)
                    })
                    .collect();
                let mut idx: Vec<usize> = (0..row.len()).collect();
                idx.sort_by(|&a, &b| key[a].total_cmp(&key[b]).then(a.cmp(&b)));
                *row = idx.iter().map(|&i| row[i]).collect();
                fix_anchors(row, &vn);
            }
            let c = crossings(&order, &vn);
            if c < best_cross {
                best_cross = c;
                best = order.clone();
            }
        }
        order = best;
    }

    // 4. Positions within rows (centres).
    let gap = |a: usize, b: usize| {
        let g = if vn[a].real && vn[b].real {
            opts.node_gap
        } else {
            opts.edge_gap
        };
        vn[a].width / 2.0 + vn[b].width / 2.0 + g
    };
    let mut x = vec![0.0f64; vn.len()];
    for row in &order {
        let mut at = 0.0;
        for (j, &v) in row.iter().enumerate() {
            if j > 0 {
                at += gap(row[j - 1], v);
            }
            x[v] = at;
        }
    }
    let passes = if vn.len() <= ORDER_LIMIT { 8 } else { 2 };
    for pass in 0..passes {
        let down = pass % 2 == 0;
        let rows_in: Vec<usize> = if down {
            (1..rows).collect()
        } else {
            (0..rows.saturating_sub(1)).rev().collect()
        };
        for l in rows_in {
            let row = &order[l];
            let mut want = Vec::with_capacity(row.len());
            let mut weight = Vec::with_capacity(row.len());
            for &v in row {
                let nb = if down { &vn[v].up } else { &vn[v].down };
                let w = match vn[v].anchor {
                    Some(a) => {
                        want.push(x[a]);
                        0.05
                    }
                    None => match median(nb.iter().map(|&u| x[u])) {
                        Some(m) => {
                            want.push(m);
                            if vn[v].real { 1.0 } else { 2.0 }
                        }
                        None => {
                            want.push(x[v]);
                            0.25
                        }
                    },
                };
                weight.push(w);
            }
            let seps: Vec<f64> = (1..row.len()).map(|j| gap(row[j - 1], row[j])).collect();
            let placed = isotonic(&want, &weight, &seps);
            for (j, &v) in row.iter().enumerate() {
                x[v] = placed[j];
            }
        }
    }

    // Rows top to bottom.
    let mut band_top = vec![0.0f64; rows];
    let mut band_h = vec![0.0f64; rows];
    for v in 0..n {
        band_h[layer[v]] = band_h[layer[v]].max(sizes[v].height.max(0.0));
    }
    for l in 1..rows {
        band_top[l] = band_top[l - 1] + band_h[l - 1] + opts.layer_gap;
    }
    let bt = |l: usize| band_top[l];
    let bb = |l: usize| band_top[l] + band_h[l];
    let half = opts.layer_gap / 2.0;
    let left = |v: usize| x[v] - sizes[v].width / 2.0;

    // 5. Ports: out of the bottom, into the top, in the order the other
    // ends lie.
    let next_hop = |i: usize| -> f64 {
        let e = edges[i];
        if e.from == e.to || climbs[i] {
            // The point beside the source (the chain's last).
            x[*chains[i].last().unwrap()]
        } else {
            chains[i].first().map(|&d| x[d]).unwrap_or(x[e.to])
        }
    };
    let prev_hop = |i: usize| -> f64 {
        let e = edges[i];
        if e.from == e.to || climbs[i] {
            x[chains[i][0]]
        } else {
            chains[i].last().map(|&d| x[d]).unwrap_or(x[e.from])
        }
    };
    let mut out_port = vec![0.0f64; edges.len()];
    let mut in_port = vec![0.0f64; edges.len()];
    let mut outs: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut ins: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, e) in edges.iter().enumerate() {
        if valid(e) {
            outs[e.from].push(i);
            ins[e.to].push(i);
        }
    }
    for v in 0..n {
        let w = sizes[v].width;
        let mut o = outs[v].clone();
        o.sort_by(|&a, &b| next_hop(a).total_cmp(&next_hop(b)).then(a.cmp(&b)));
        for (k, &i) in o.iter().enumerate() {
            out_port[i] = left(v) + w * (k as f64 + 1.0) / (o.len() as f64 + 1.0);
        }
        let mut p = ins[v].clone();
        p.sort_by(|&a, &b| prev_hop(a).total_cmp(&prev_hop(b)).then(a.cmp(&b)));
        for (k, &i) in p.iter().enumerate() {
            in_port[i] = left(v) + w * (k as f64 + 1.0) / (p.len() as f64 + 1.0);
        }
    }

    let pt = |x: f64, y: f64| Point { x, y };
    let mut paths: Vec<LayoutEdge> = Vec::with_capacity(edges.len());
    for (i, e) in edges.iter().enumerate() {
        if !valid(e) {
            paths.push(LayoutEdge {
                points: Vec::new(),
                climbs: climbs[i],
            });
            continue;
        }
        let (u, v) = (e.from, e.to);
        let bottom_u = bt(layer[u]) + sizes[u].height;
        let top_v = bt(layer[v]);
        let mut p = vec![pt(out_port[i], bottom_u)];
        if u == v || climbs[i] {
            // Down out of the source, along under its row, up the column
            // beside it and through each row, along over the target's row,
            // and down into the target.
            let lu = layer[u];
            let lv = layer[v];
            p.push(pt(out_port[i], bb(lu) + half));
            for &d in chains[i].iter().rev() {
                let l = vn[d].layer;
                let lower = if l == lu { bb(l) + half } else { bb(l) };
                p.push(pt(x[d], lower));
                let upper = if l == lv { bt(l) - half } else { bt(l) };
                p.push(pt(x[d], upper));
            }
            p.push(pt(in_port[i], bt(lv) - half));
        } else {
            if bottom_u < bb(layer[u]) {
                p.push(pt(out_port[i], bb(layer[u])));
            }
            for &d in &chains[i] {
                let l = vn[d].layer;
                p.push(pt(x[d], bt(l)));
                p.push(pt(x[d], bb(l)));
            }
        }
        p.push(pt(in_port[i], top_v));
        paths.push(LayoutEdge {
            points: simplify(p),
            climbs: climbs[i] || u == v,
        });
    }

    // Everything from 0.
    let mut min_x = f64::INFINITY;
    let mut min_y: f64 = 0.0;
    for v in 0..n {
        min_x = min_x.min(left(v));
    }
    for e in &paths {
        for q in &e.points {
            min_x = min_x.min(q.x);
            min_y = min_y.min(q.y);
        }
    }
    let mut nodes: Vec<Point> = (0..n)
        .map(|v| pt(left(v) - min_x, bt(layer[v]) - min_y))
        .collect();
    for e in &mut paths {
        for q in &mut e.points {
            q.x -= min_x;
            q.y -= min_y;
        }
    }
    let mut width: f64 = 0.0;
    let mut height: f64 = 0.0;
    for (v, q) in nodes.iter_mut().enumerate() {
        // No negative zero in the output.
        q.x += 0.0;
        q.y += 0.0;
        width = width.max(q.x + sizes[v].width);
        height = height.max(q.y + sizes[v].height);
    }
    for e in &paths {
        for q in &e.points {
            width = width.max(q.x);
            height = height.max(q.y);
        }
    }
    Layout {
        nodes,
        rows: layer.iter().map(|&l| l as u32).collect(),
        edges: paths,
        width,
        height,
    }
}

/// Turn edges so the graph has no cycle: the ones marked, then any a
/// depth-first walk (from each box in turn) finds going to a box still on
/// its stack. Returns the walk's preorder and a topological order of the
/// result.
fn orient(n: usize, edges: &[EdgeSpec], climbs: &mut [bool]) -> (Vec<usize>, Vec<usize>) {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, e) in edges.iter().enumerate() {
        if e.from < n && e.to < n && e.from != e.to {
            let a = if climbs[i] { e.to } else { e.from };
            out[a].push(i);
        }
    }
    let far = |i: usize, climbs: &[bool]| {
        let e = edges[i];
        if climbs[i] { e.from } else { e.to }
    };
    let mut state = vec![0u8; n];
    let mut pre = Vec::with_capacity(n);
    let mut post = Vec::with_capacity(n);
    for root in 0..n {
        if state[root] != 0 {
            continue;
        }
        state[root] = 1;
        pre.push(root);
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some(&mut (u, ref mut next)) = stack.last_mut() {
            if let Some(&i) = out[u].get(*next) {
                *next += 1;
                let v = far(i, climbs);
                match state[v] {
                    0 => {
                        state[v] = 1;
                        pre.push(v);
                        stack.push((v, 0));
                    }
                    // Closes a cycle: turn it. Every edge left then runs
                    // from later to earlier in the post-order.
                    1 => climbs[i] = !climbs[i],
                    _ => {}
                }
            } else {
                state[u] = 2;
                post.push(u);
                stack.pop();
            }
        }
    }
    post.reverse();
    (pre, post)
}

/// A back edge's points at its ends sit just right of their boxes.
fn fix_anchors(row: &mut Vec<usize>, vn: &[VNode]) {
    if !row.iter().any(|&v| vn[v].anchor.is_some()) {
        return;
    }
    let free: Vec<usize> = row
        .iter()
        .copied()
        .filter(|&v| vn[v].anchor.is_none())
        .collect();
    let mut anchored: Vec<usize> = row
        .iter()
        .copied()
        .filter(|&v| vn[v].anchor.is_some())
        .collect();
    anchored.sort_unstable();
    let mut out = Vec::with_capacity(row.len());
    for v in free {
        out.push(v);
        out.extend(
            anchored
                .iter()
                .copied()
                .filter(|&d| vn[d].anchor == Some(v)),
        );
    }
    // An anchor not in this row (never, but keep every node).
    for &d in &anchored {
        if !out.contains(&d) {
            out.push(d);
        }
    }
    *row = out;
}

fn positions(order: &[Vec<usize>], len: usize) -> Vec<usize> {
    let mut pos = vec![0usize; len];
    for row in order {
        for (i, &v) in row.iter().enumerate() {
            pos[v] = i;
        }
    }
    pos
}

fn median(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut v: Vec<f64> = values.collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    Some(if v.len() % 2 == 1 {
        v[m]
    } else {
        (v[m - 1] + v[m]) / 2.0
    })
}

/// Edge crossings between each pair of neighbouring rows.
fn crossings(order: &[Vec<usize>], vn: &[VNode]) -> usize {
    let pos = positions(order, vn.len());
    let mut total = 0;
    for l in 0..order.len().saturating_sub(1) {
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for &u in &order[l] {
            for &v in &vn[u].down {
                pairs.push((pos[u], pos[v]));
            }
        }
        pairs.sort_unstable();
        // Inversions among the lower ends, counted with a Fenwick tree.
        let size = order[l + 1].len() + 1;
        let mut tree = vec![0usize; size + 1];
        for (seen, &(_, b)) in pairs.iter().enumerate() {
            // How many earlier pairs end at or left of `b`.
            let mut i = b + 1;
            let mut le = 0;
            while i > 0 {
                le += tree[i];
                i &= i - 1;
            }
            total += seen - le;
            let mut i = b + 1;
            while i <= size {
                tree[i] += 1;
                i += i & i.wrapping_neg();
            }
        }
    }
    total
}

/// The positions closest (by weighted squares) to `want` that keep their
/// order with at least `seps[j - 1]` between `j - 1` and `j`.
fn isotonic(want: &[f64], weight: &[f64], seps: &[f64]) -> Vec<f64> {
    let k = want.len();
    let mut off = vec![0.0f64; k];
    for j in 1..k {
        off[j] = off[j - 1] + seps[j - 1];
    }
    // Pools of (total weight, weighted sum, count).
    let mut pools: Vec<(f64, f64, usize)> = Vec::with_capacity(k);
    for j in 0..k {
        let w = weight[j].max(1e-9);
        pools.push((w, w * (want[j] - off[j]), 1));
        while pools.len() > 1 {
            let (w1, s1, _) = pools[pools.len() - 2];
            let (w2, s2, _) = pools[pools.len() - 1];
            if s1 / w1 <= s2 / w2 {
                break;
            }
            let (_, _, c2) = pools.pop().unwrap();
            let last = pools.last_mut().unwrap();
            last.0 += w2;
            last.1 += s2;
            last.2 += c2;
        }
    }
    let mut out = Vec::with_capacity(k);
    for (w, s, c) in pools {
        for _ in 0..c {
            let j = out.len();
            out.push(s / w + off[j]);
        }
    }
    out
}

/// Drop repeated points and middle points of straight runs.
fn simplify(p: Vec<Point>) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(p.len());
    for q in p {
        if out
            .last()
            .is_some_and(|l| (l.x - q.x).abs() < 1e-9 && (l.y - q.y).abs() < 1e-9)
        {
            continue;
        }
        if out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            let cross = (b.x - a.x) * (q.y - a.y) - (b.y - a.y) * (q.x - a.x);
            let same_way = (b.x - a.x) * (q.x - b.x) + (b.y - a.y) * (q.y - b.y) >= 0.0;
            if cross.abs() < 1e-9 && same_way {
                out.pop();
            }
        }
        out.push(q);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(w: f64, h: f64) -> NodeSize {
        NodeSize {
            width: w,
            height: h,
        }
    }

    fn e(from: usize, to: usize) -> EdgeSpec {
        EdgeSpec {
            from,
            to,
            back: false,
        }
    }

    fn overlaps(l: &Layout, sizes: &[NodeSize]) -> bool {
        for a in 0..sizes.len() {
            for b in a + 1..sizes.len() {
                let (p, q) = (l.nodes[a], l.nodes[b]);
                let x = p.x < q.x + sizes[b].width && q.x < p.x + sizes[a].width;
                let y = p.y < q.y + sizes[b].height && q.y < p.y + sizes[a].height;
                if x && y {
                    return true;
                }
            }
        }
        false
    }

    #[test]
    fn a_diamond_is_three_rows_with_the_join_centred() {
        let sizes = vec![size(100.0, 40.0); 4];
        let edges = vec![e(0, 1), e(0, 2), e(1, 3), e(2, 3)];
        let l = layout(&sizes, &edges, &LayoutOptions::default());
        assert_eq!(l.rows, vec![0, 1, 1, 2]);
        assert!(!overlaps(&l, &sizes));
        let centre = |i: usize| l.nodes[i].x + 50.0;
        assert!((centre(0) - centre(3)).abs() < 1e-6);
        assert!((centre(0) - (centre(1) + centre(2)) / 2.0).abs() < 1e-6);
        // Every edge starts on its source's bottom and ends on its target's top.
        for (i, s) in edges.iter().enumerate() {
            let p = &l.edges[i].points;
            let a = l.nodes[s.from];
            let b = l.nodes[s.to];
            assert_eq!(p[0].y, a.y + 40.0);
            assert!(p[0].x > a.x && p[0].x < a.x + 100.0);
            assert_eq!(p.last().unwrap().y, b.y);
        }
    }

    #[test]
    fn a_loop_climbs_beside_its_boxes() {
        // 0 → 1 → 2 → 1 (back), 2 → 3.
        let sizes = vec![size(80.0, 30.0); 4];
        let edges = vec![
            e(0, 1),
            e(1, 2),
            EdgeSpec {
                from: 2,
                to: 1,
                back: true,
            },
            e(2, 3),
        ];
        let l = layout(&sizes, &edges, &LayoutOptions::default());
        assert_eq!(l.rows, vec![0, 1, 2, 3]);
        let back = &l.edges[2];
        assert!(back.climbs);
        // It leaves 2's bottom, and its highest point is above 1's top.
        assert_eq!(back.points[0].y, l.nodes[2].y + 30.0);
        let top = back
            .points
            .iter()
            .map(|p| p.y)
            .fold(f64::INFINITY, f64::min);
        assert!(top < l.nodes[1].y);
        // Its column is right of both boxes.
        let column = back.points.iter().map(|p| p.x).fold(0.0, f64::max);
        assert!(column > l.nodes[1].x + 80.0 && column > l.nodes[2].x + 80.0);
        assert!(!overlaps(&l, &sizes));
    }

    #[test]
    fn a_cycle_nobody_marked_is_broken() {
        let sizes = vec![size(10.0, 10.0); 3];
        let edges = vec![e(0, 1), e(1, 2), e(2, 0)];
        let l = layout(&sizes, &edges, &LayoutOptions::default());
        assert_eq!(l.rows, vec![0, 1, 2]);
        assert!(l.edges[2].climbs);
    }

    #[test]
    fn a_self_loop_goes_round_its_box() {
        let sizes = vec![size(50.0, 20.0)];
        let edges = vec![e(0, 0)];
        let l = layout(&sizes, &edges, &LayoutOptions::default());
        let p = &l.edges[0].points;
        assert!(p.len() >= 4);
        assert_eq!(p[0].y, l.nodes[0].y + 20.0);
        assert_eq!(p.last().unwrap().y, l.nodes[0].y);
    }

    #[test]
    fn long_edges_bend_round_boxes() {
        // 0 → 1 → 2 → 3 and 0 → 3: the long edge passes rows 1 and 2
        // beside their boxes, never through them.
        let sizes = vec![size(60.0, 20.0); 4];
        let edges = vec![e(0, 1), e(1, 2), e(2, 3), e(0, 3)];
        let l = layout(&sizes, &edges, &LayoutOptions::default());
        for &(b, row) in &[(1usize, 1u32), (2, 2)] {
            assert_eq!(l.rows[b], row);
            let n = l.nodes[b];
            for w in l.edges[3].points.windows(2) {
                let (p, q) = (w[0], w[1]);
                let vertical = (p.x - q.x).abs() < 1e-9;
                if vertical && p.y.max(q.y) > n.y && p.y.min(q.y) < n.y + 20.0 {
                    assert!(p.x < n.x || p.x > n.x + 60.0, "through box {b}");
                }
            }
        }
    }

    #[test]
    fn crossings_are_undone() {
        // Two chains drawn crossed at first: 0→3, 1→2 with 2 and 3 in the
        // opposite order to 0 and 1.
        let sizes = vec![size(20.0, 10.0); 4];
        let edges = vec![e(0, 3), e(1, 2), e(0, 2)];
        let l = layout(&sizes, &edges, &LayoutOptions::default());
        let (a, b) = (l.nodes[0].x < l.nodes[1].x, l.nodes[3].x < l.nodes[2].x);
        // 0 feeds both, 1 only 2: 2 should sit under 1's side.
        assert!(!overlaps(&l, &sizes));
        assert_eq!(a, b, "the order that crosses less");
    }

    #[test]
    fn the_same_input_gives_the_same_layout() {
        let sizes: Vec<NodeSize> = (0..30)
            .map(|i| size(20.0 + (i % 7) as f64 * 9.0, 12.0 + (i % 3) as f64 * 10.0))
            .collect();
        let mut edges = Vec::new();
        for i in 0..29 {
            edges.push(e(i, i + 1));
            if i % 4 == 0 && i + 5 < 30 {
                edges.push(e(i, i + 5));
            }
            if i % 6 == 5 {
                edges.push(EdgeSpec {
                    from: i,
                    to: i - 4,
                    back: true,
                });
            }
        }
        let a = layout(&sizes, &edges, &LayoutOptions::default());
        let b = layout(&sizes, &edges, &LayoutOptions::default());
        assert_eq!(a, b);
        assert!(!overlaps(&a, &sizes));
        assert!(a.nodes.iter().all(|p| p.x >= 0.0 && p.y >= 0.0));
    }

    #[test]
    fn isotonic_keeps_order_and_gaps() {
        let x = isotonic(&[10.0, 0.0, 5.0], &[1.0, 1.0, 1.0], &[4.0, 4.0]);
        assert!(x[1] - x[0] >= 4.0 - 1e-9 && x[2] - x[1] >= 4.0 - 1e-9);
        // Unconstrained wants are met exactly.
        let y = isotonic(&[0.0, 10.0, 20.0], &[1.0, 1.0, 1.0], &[4.0, 4.0]);
        assert_eq!(y, vec![0.0, 10.0, 20.0]);
    }
}
