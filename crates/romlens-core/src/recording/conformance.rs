//! What every [`MachineStateSource`] must do, as one function run against
//! each implementation. About a hundred lines that decide whether Phase 5's
//! embedded core is a drop-in or a rewrite.

use crate::recording::{MachineState, MachineStateSource, RecordingError, StateRegion};

/// Check `source` against the frames it is supposed to hold. Returns the
/// first violation, worded for a test failure.
pub fn check(source: &dyn MachineStateSource, expected: &[MachineState]) -> Result<(), String> {
    let n = expected.len() as u64;
    if source.frame_count() != Some(n) {
        return Err(format!(
            "frame_count {:?}, expected {n}",
            source.frame_count()
        ));
    }
    let regions = source.regions();
    let mut sorted = regions.clone();
    sorted.sort();
    if sorted != regions {
        return Err(format!("regions not in id order: {regions:?}"));
    }
    if let Some(first) = expected.first() {
        let want: Vec<StateRegion> = first.regions.keys().copied().collect();
        if regions != want {
            return Err(format!("regions {regions:?}, expected {want:?}"));
        }
    }
    // Random access in an order that defeats a forward-only cache.
    let mut order: Vec<u64> = (0..n).rev().collect();
    order.extend(0..n);
    for f in order {
        let got = source
            .state_at(f)
            .map_err(|e| format!("state_at({f}): {e}"))?;
        if got.frame != f {
            return Err(format!("state_at({f}) says it is frame {}", got.frame));
        }
        for (r, want) in &expected[f as usize].regions {
            if let Some(have) = got.region(*r) {
                if have != want.as_slice() {
                    return Err(format!("frame {f}: {} differs", r.name()));
                }
                let single = source
                    .region_at(f, *r)
                    .map_err(|e| format!("region_at({f}, {}): {e}", r.name()))?;
                if &single != want {
                    return Err(format!(
                        "region_at({f}, {}) differs from state_at",
                        r.name()
                    ));
                }
            } else if !regions.contains(r) {
                return Err(format!("frame {f}: {} missing", r.name()));
            }
            // A source may omit a listed region at a frame (WRAM between
            // keyframes); it may never return it wrong.
        }
    }
    // changes(): every byte outside the runs is equal, and runs are sorted,
    // disjoint and inside the region.
    for from in 0..n {
        for to in from + 1..n.min(from + 5) {
            for r in &regions {
                let (Some(a), Some(b)) = (
                    expected[from as usize].region(*r),
                    expected[to as usize].region(*r),
                ) else {
                    continue;
                };
                let runs = source
                    .changes(from, to, *r)
                    .map_err(|e| format!("changes({from}, {to}, {}): {e}", r.name()))?;
                let mut covered = vec![false; a.len()];
                let mut last_end = 0u32;
                for (k, run) in runs.iter().enumerate() {
                    if (k > 0 && run.offset < last_end) || run.end() as usize > a.len() {
                        return Err(format!(
                            "changes({from}, {to}, {}) overlaps or overruns",
                            r.name()
                        ));
                    }
                    last_end = run.end();
                    covered[run.offset as usize..run.end() as usize].fill(true);
                }
                if let Some(i) = (0..a.len()).find(|&i| !covered[i] && a[i] != b[i]) {
                    return Err(format!(
                        "changes({from}, {to}, {}) misses byte {i:#x}",
                        r.name()
                    ));
                }
            }
        }
    }
    match source.state_at(n) {
        Err(RecordingError::NoSuchFrame { frame, count }) if frame == n && count == n => Ok(()),
        other => Err(format!("state_at past the end gave {other:?}")),
    }
}
