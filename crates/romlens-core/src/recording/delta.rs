//! Sparse runs of changed bytes, the delta frames' encoding.
//!
//! Runs rather than XOR, so unchanged memory costs nothing and a reader can
//! say what changed in a frame without decompressing anything. Runs closer
//! than [`MERGE_GAP`] bytes merge, because a run header costs eight bytes.

/// Changed bytes separated by fewer than this many unchanged ones share a
/// run; a gap of this many or more starts a new one.
pub const MERGE_GAP: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Run {
    pub offset: u32,
    pub len: u32,
}

impl Run {
    pub const fn end(self) -> u32 {
        self.offset + self.len
    }
}

/// The runs where `new` differs from `old` (equal lengths).
pub fn diff(old: &[u8], new: &[u8]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut i = 0;
    let n = old.len().min(new.len());
    while i < n {
        if old[i] == new[i] {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i + 1;
        let mut j = end;
        while j < n {
            if old[j] != new[j] {
                end = j + 1;
            } else if j + 1 - end >= MERGE_GAP {
                break;
            }
            j += 1;
        }
        runs.push(Run {
            offset: start as u32,
            len: (end - start) as u32,
        });
        i = end;
    }
    runs
}

/// Copy `bytes` (the runs' contents, concatenated) into `target`.
pub fn apply(target: &mut [u8], runs: &[Run], bytes: &[u8]) -> Result<(), String> {
    let mut at = 0usize;
    for r in runs {
        let (o, l) = (r.offset as usize, r.len as usize);
        if o + l > target.len() || at + l > bytes.len() {
            return Err(format!(
                "run {:#x}+{:#x} does not fit a {}-byte region",
                r.offset,
                r.len,
                target.len()
            ));
        }
        target[o..o + l].copy_from_slice(&bytes[at..at + l]);
        at += l;
    }
    Ok(())
}

/// The union of two sorted run lists, merged where they touch or overlap.
pub fn union(a: &[Run], b: &[Run]) -> Vec<Run> {
    let mut all: Vec<Run> = a.iter().chain(b).copied().collect();
    all.sort();
    let mut out: Vec<Run> = Vec::with_capacity(all.len());
    for r in all {
        match out.last_mut() {
            Some(last) if r.offset <= last.end() => {
                let end = last.end().max(r.end());
                last.len = end - last.offset;
            }
            _ => out.push(r),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearby_changes_merge_and_far_ones_do_not() {
        let old = vec![0u8; 100];
        let mut new = old.clone();
        new[3] = 1;
        new[10] = 1; // 6 unchanged between: merges
        new[60] = 1; // far: its own run
        let runs = diff(&old, &new);
        assert_eq!(
            runs,
            vec![Run { offset: 3, len: 8 }, Run { offset: 60, len: 1 }]
        );
        let bytes: Vec<u8> = runs
            .iter()
            .flat_map(|r| new[r.offset as usize..r.end() as usize].to_vec())
            .collect();
        let mut rebuilt = old.clone();
        apply(&mut rebuilt, &runs, &bytes).unwrap();
        assert_eq!(rebuilt, new);
        assert!(diff(&old, &old).is_empty());
    }

    #[test]
    fn property_diff_then_apply_is_identity() {
        let mut seed = 99u32;
        let mut rand = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed >> 8
        };
        for _ in 0..200 {
            let len = 1 + rand() as usize % 600;
            let old: Vec<u8> = (0..len).map(|_| rand() as u8).collect();
            let mut new = old.clone();
            for _ in 0..rand() % 20 {
                let i = rand() as usize % len;
                new[i] = new[i].wrapping_add(1 + rand() as u8 % 255);
            }
            let runs = diff(&old, &new);
            for w in runs.windows(2) {
                assert!(w[1].offset - w[0].end() >= MERGE_GAP as u32, "{w:?}");
            }
            let bytes: Vec<u8> = runs
                .iter()
                .flat_map(|r| new[r.offset as usize..r.end() as usize].to_vec())
                .collect();
            let mut rebuilt = old.clone();
            apply(&mut rebuilt, &runs, &bytes).unwrap();
            assert_eq!(rebuilt, new);
        }
    }

    #[test]
    fn union_merges_touching_runs() {
        let a = [Run { offset: 0, len: 4 }, Run { offset: 20, len: 2 }];
        let b = [Run { offset: 4, len: 2 }, Run { offset: 30, len: 1 }];
        assert_eq!(
            union(&a, &b),
            vec![
                Run { offset: 0, len: 6 },
                Run { offset: 20, len: 2 },
                Run { offset: 30, len: 1 }
            ]
        );
    }
}
