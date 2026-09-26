//! Comparing two versions of a ROM (docs/22, D1): their bytes aligned,
//! their routines paired, and the data that changed.

pub mod bytes;
pub mod routines;

pub use bytes::{ByteDiff, ByteRun, Move, RunKind, align};
pub use routines::{DataChange, DiffLine, LineOp, Pairing, RoutineInfo, RoutinePair};

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::model::project::Project;
use crate::rom::image::RomImage;

/// One version: its ROM, its project (names), its analysis.
#[derive(Clone, Copy)]
pub struct Side<'a> {
    pub rom: &'a RomImage,
    pub project: &'a Project,
    pub snap: &'a AnalysisSnapshot,
}

/// What changed from `a` to `b`.
#[derive(Debug, Clone)]
pub struct Comparison {
    pub bytes: ByteDiff,
    /// Every routine of either version, paired where they match, in the
    /// first version's order and then the added ones.
    pub routines: Vec<RoutinePair>,
    /// The data regions with changed bytes.
    pub data: Vec<DataChange>,
}

/// The names to carry from `a` to `b`: each paired routine that has a
/// user's or an imported name in `a`, and none but an automatic one in
/// `b`, as `b`'s entry and `a`'s name.
pub fn names_to_carry(
    c: &Comparison,
    a: Side<'_>,
    b: Side<'_>,
) -> Vec<(crate::SnesAddress, String)> {
    let mut out = Vec::new();
    for r in &c.routines {
        let (Some(ra), Some(rb)) = (&r.a, &r.b) else {
            continue;
        };
        let Some(label) = a.project.labels.get(&ra.entry) else {
            continue;
        };
        if b.project.labels.contains_key(&rb.entry) {
            continue;
        }
        out.push((rb.entry, label.name.clone()));
    }
    out
}

/// Compare two versions of a ROM.
pub fn compare(a: Side<'_>, b: Side<'_>) -> Comparison {
    let bytes = align(a.rom.bytes(), b.rom.bytes());
    let routines = routines::pair(a, b, &bytes);
    let data = routines::data_changes(a, b, &bytes);
    Comparison {
        bytes,
        routines,
        data,
    }
}
