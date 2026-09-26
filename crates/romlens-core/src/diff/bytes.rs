//! Two images' bytes aligned (docs/22, D1).
//!
//! Anchors first: every 32-byte window of the first image that occurs
//! there once is hashed, the second image is scanned with a rolling hash,
//! and each hit is grown in both directions as far as the bytes agree. The
//! anchors that keep their order in both images are the alignment; the
//! rest are blocks that moved. Between two anchors the bytes are compared
//! in place when both gaps are the same length (a patch), and otherwise
//! reported as changed, inserted or deleted after trimming what they share
//! at either end. So an insertion shifts what follows it without marking
//! it changed.

use std::collections::HashMap;
use std::ops::Range;

/// A window this long anchors the alignment.
pub const WINDOW: usize = 32;
/// A moved block shorter than this is too likely to be chance.
pub const MIN_MOVE: u32 = 64;
/// Equal bytes shorter than this between two changes are part of one
/// change, so a patch reads as a patch rather than a spray of bytes.
const MIN_SAME: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    Same,
    /// The bytes differ; `a` and `b` are the two versions.
    Changed,
    /// Only in the second image (`a` is empty, where it would be).
    Inserted,
    /// Only in the first image (`b` is empty, where it was).
    Deleted,
}

/// A stretch of the alignment, in order through both images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteRun {
    pub kind: RunKind,
    pub a: Range<u32>,
    pub b: Range<u32>,
}

/// A block found in both images, out of order with the alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Move {
    pub a: u32,
    pub b: u32,
    pub len: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ByteDiff {
    /// Covers both images, in order.
    pub runs: Vec<ByteRun>,
    pub moves: Vec<Move>,
}

impl ByteDiff {
    /// Bytes of each kind, counted on the side that has them.
    pub fn count(&self, kind: RunKind) -> u64 {
        self.runs
            .iter()
            .filter(|r| r.kind == kind)
            .map(|r| match kind {
                RunKind::Deleted => r.a.len() as u64,
                _ => r.b.len() as u64,
            })
            .sum()
    }

    /// Where the first image's byte `a` is in the second: in a run the
    /// two share, or at the same place in a changed run of equal length,
    /// or in a moved block.
    pub fn map(&self, a: u32) -> Option<u32> {
        let i = self.runs.partition_point(|r| r.a.end <= a);
        if let Some(r) = self.runs.get(i)
            && r.a.contains(&a)
            && (r.kind == RunKind::Same || r.kind == RunKind::Changed && r.a.len() == r.b.len())
        {
            return Some(r.b.start + (a - r.a.start));
        }
        self.moves
            .iter()
            .find(|m| (m.a..m.a + m.len).contains(&a))
            .map(|m| m.b + (a - m.a))
    }
}

const BASE: u64 = 0x100_0000_01B3;

fn hash(w: &[u8]) -> u64 {
    w.iter().fold(0u64, |h, &b| {
        h.wrapping_mul(BASE).wrapping_add(b as u64 + 1)
    })
}

fn plain(w: &[u8]) -> bool {
    w.iter().all(|&b| b == w[0])
}

#[derive(Debug, Clone, Copy)]
struct Match {
    a: u32,
    b: u32,
    len: u32,
}

/// The anchors: runs of equal bytes, in the second image's order and not
/// overlapping there.
fn anchors(a: &[u8], b: &[u8]) -> Vec<Match> {
    if a.len() < WINDOW || b.len() < WINDOW {
        return Vec::new();
    }
    // The first image's windows at a stride of one window, kept only where
    // unique: filler repeats, and a repeated anchor is a guess.
    let mut seen: HashMap<u64, Option<u32>> = HashMap::new();
    for p in (0..=a.len() - WINDOW).step_by(WINDOW) {
        let w = &a[p..p + WINDOW];
        if plain(w) {
            continue;
        }
        seen.entry(hash(w))
            .and_modify(|e| *e = None)
            .or_insert(Some(p as u32));
    }
    let top = (0..WINDOW - 1).fold(1u64, |t, _| t.wrapping_mul(BASE));
    let mut out: Vec<Match> = Vec::new();
    let mut i = 0usize;
    let mut h = hash(&b[..WINDOW]);
    while i + WINDOW <= b.len() {
        let hit = seen.get(&h).copied().flatten().filter(|&p| {
            let p = p as usize;
            a[p..p + WINDOW] == b[i..i + WINDOW]
        });
        if let Some(p) = hit {
            let (mut pa, mut pb) = (p as usize, i);
            let floor = out.last().map_or(0, |m| (m.b + m.len) as usize);
            while pa > 0 && pb > floor && a[pa - 1] == b[pb - 1] {
                pa -= 1;
                pb -= 1;
            }
            let mut len = i + WINDOW - pb;
            while pa + len < a.len() && pb + len < b.len() && a[pa + len] == b[pb + len] {
                len += 1;
            }
            out.push(Match {
                a: pa as u32,
                b: pb as u32,
                len: len as u32,
            });
            i = pb + len;
            if i + WINDOW <= b.len() {
                h = hash(&b[i..i + WINDOW]);
            }
            continue;
        }
        if i + WINDOW < b.len() {
            h = h
                .wrapping_sub((b[i] as u64 + 1).wrapping_mul(top))
                .wrapping_mul(BASE)
                .wrapping_add(b[i + WINDOW] as u64 + 1);
        }
        i += 1;
    }
    out
}

/// The anchors that keep their order in the first image too: the chain
/// covering the most bytes, then those that overlap in the first image
/// dropped.
fn in_order(matches: &[Match]) -> (Vec<Match>, Vec<Match>) {
    // The heaviest chain increasing in `a`, by bytes covered, with a
    // Fenwick tree of the best chain ending below each `a` rank.
    let mut ranks: Vec<u32> = matches.iter().map(|m| m.a).collect();
    ranks.sort_unstable();
    ranks.dedup();
    let mut tree: Vec<(u64, Option<usize>)> = vec![(0, None); ranks.len() + 1];
    let mut prev: Vec<Option<usize>> = vec![None; matches.len()];
    let mut best: (u64, Option<usize>) = (0, None);
    for (i, m) in matches.iter().enumerate() {
        let r = ranks.partition_point(|&a| a < m.a);
        // The best chain over ranks below `r`.
        let mut below = (0u64, None);
        let mut k = r;
        while k > 0 {
            if tree[k].0 > below.0 {
                below = tree[k];
            }
            k &= k - 1;
        }
        prev[i] = below.1;
        let total = (below.0 + m.len as u64, Some(i));
        if total.0 > best.0 {
            best = total;
        }
        let mut k = r + 1;
        while k < tree.len() {
            if total.0 > tree[k].0 {
                tree[k] = total;
            }
            k += k & k.wrapping_neg();
        }
    }
    let mut chain = Vec::new();
    let mut at = best.1;
    while let Some(i) = at {
        chain.push(i);
        at = prev[i];
    }
    chain.reverse();
    let mut kept: Vec<Match> = Vec::new();
    let mut used = vec![false; matches.len()];
    for i in chain {
        let m = matches[i];
        if kept.last().is_none_or(|k| m.a >= k.a + k.len) {
            kept.push(m);
            used[i] = true;
        }
    }
    let moved = matches
        .iter()
        .zip(used)
        // One left out at the same place is not a move: the gap there is
        // compared in place.
        .filter(|(m, u)| !u && m.len >= MIN_MOVE && m.a != m.b)
        .map(|(m, _)| *m)
        .collect();
    (kept, moved)
}

fn push(out: &mut Vec<ByteRun>, kind: RunKind, a: Range<u32>, b: Range<u32>) {
    if a.is_empty() && b.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.kind == kind
        && last.a.end == a.start
        && last.b.end == b.start
    {
        last.a.end = a.end;
        last.b.end = b.end;
        return;
    }
    out.push(ByteRun { kind, a, b });
}

/// A gap between anchors: in place when both sides are the same length,
/// else what they share at either end and a change between.
fn gap(out: &mut Vec<ByteRun>, a: &[u8], b: &[u8], ga: Range<u32>, gb: Range<u32>) {
    if ga.len() == gb.len() {
        let n = ga.len() as u32;
        let mut i = 0;
        while i < n {
            let same = a[(ga.start + i) as usize] == b[(gb.start + i) as usize];
            let mut j = i + 1;
            while j < n && (a[(ga.start + j) as usize] == b[(gb.start + j) as usize]) == same {
                j += 1;
            }
            // A short agreement inside a change stays in it.
            let kind = if same && (j - i >= MIN_SAME || i == 0 || j == n) {
                RunKind::Same
            } else {
                RunKind::Changed
            };
            push(
                out,
                kind,
                ga.start + i..ga.start + j,
                gb.start + i..gb.start + j,
            );
            i = j;
        }
        return;
    }
    let (sa, sb) = (
        &a[ga.start as usize..ga.end as usize],
        &b[gb.start as usize..gb.end as usize],
    );
    let pre = sa.iter().zip(sb).take_while(|(x, y)| x == y).count() as u32;
    let suf = sa[pre as usize..]
        .iter()
        .rev()
        .zip(sb[pre as usize..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count() as u32;
    push(
        out,
        RunKind::Same,
        ga.start..ga.start + pre,
        gb.start..gb.start + pre,
    );
    let (ma, mb) = (ga.start + pre..ga.end - suf, gb.start + pre..gb.end - suf);
    let kind = match (ma.is_empty(), mb.is_empty()) {
        (false, false) => RunKind::Changed,
        (true, false) => RunKind::Inserted,
        _ => RunKind::Deleted,
    };
    push(out, kind, ma, mb);
    push(
        out,
        RunKind::Same,
        ga.end - suf..ga.end,
        gb.end - suf..gb.end,
    );
}

/// Align `a` with `b`.
pub fn align(a: &[u8], b: &[u8]) -> ByteDiff {
    let (kept, moves) = in_order(&anchors(a, b));
    let mut runs = Vec::new();
    let (mut ca, mut cb) = (0u32, 0u32);
    for m in &kept {
        gap(&mut runs, a, b, ca..m.a, cb..m.b);
        push(&mut runs, RunKind::Same, m.a..m.a + m.len, m.b..m.b + m.len);
        ca = m.a + m.len;
        cb = m.b + m.len;
    }
    gap(&mut runs, a, b, ca..a.len() as u32, cb..b.len() as u32);
    ByteDiff {
        runs,
        moves: moves
            .into_iter()
            .map(|m| Move {
                a: m.a,
                b: m.b,
                len: m.len,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes that do not repeat, so every window is an anchor.
    fn noise(n: usize, seed: u32) -> Vec<u8> {
        let mut x = seed.wrapping_mul(2_654_435_761).max(1);
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    fn kinds(d: &ByteDiff) -> Vec<(RunKind, Range<u32>, Range<u32>)> {
        d.runs
            .iter()
            .map(|r| (r.kind, r.a.clone(), r.b.clone()))
            .collect()
    }

    #[test]
    fn a_patch_is_changed_in_place() {
        let a = noise(4096, 1);
        let mut b = a.clone();
        b[1000..1003].copy_from_slice(&[0xEA, 0xEA, 0xEA]);
        b[1005] ^= 0xFF; // two bytes on, still the same change
        let d = align(&a, &b);
        assert_eq!(
            kinds(&d),
            vec![
                (RunKind::Same, 0..1000, 0..1000),
                (RunKind::Changed, 1000..1006, 1000..1006),
                (RunKind::Same, 1006..4096, 1006..4096),
            ]
        );
        assert_eq!(d.map(2000), Some(2000));
        assert_eq!(d.count(RunKind::Changed), 6);
    }

    #[test]
    fn an_insertion_shifts_what_follows_without_changing_it() {
        let a = noise(4096, 2);
        let block = noise(100, 3);
        let b = [&a[..2000], &block[..], &a[2000..]].concat();
        let d = align(&a, &b);
        assert_eq!(
            kinds(&d),
            vec![
                (RunKind::Same, 0..2000, 0..2000),
                (RunKind::Inserted, 2000..2000, 2000..2100),
                (RunKind::Same, 2000..4096, 2100..4196),
            ]
        );
        assert_eq!(d.map(3000), Some(3100));
        // And the other way round, a deletion.
        let back = align(&b, &a);
        assert_eq!(back.count(RunKind::Deleted), 100);
        assert_eq!(back.count(RunKind::Same), 4096);
    }

    #[test]
    fn a_block_moved_elsewhere_is_found() {
        let a = noise(4096, 4);
        // The block at 1000..1200 moves to the end.
        let b = [&a[..1000], &a[1200..], &a[1000..1200]].concat();
        let d = align(&a, &b);
        assert_eq!(
            d.moves,
            vec![Move {
                a: 1000,
                b: 3896,
                len: 200
            }]
        );
        assert_eq!(d.map(1100), Some(3996));
    }

    #[test]
    fn filler_does_not_anchor_but_aligns_in_place() {
        let mut a = noise(2048, 5);
        a.extend(std::iter::repeat_n(0xFF, 4096));
        let mut b = a.clone();
        b[4000] = 0x00;
        let d = align(&a, &b);
        assert_eq!(d.count(RunKind::Changed), 1);
        assert_eq!(d.runs.len(), 3);
    }
}
