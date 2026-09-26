//! Two versions of a ROM compared, for the Compare tab (docs/22, D2).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use romlens_core::SnesAddress;
use romlens_core::diff::{self, LineOp, Pairing, RunKind, Side};
use romlens_core::model;

use crate::RomlensError;
use crate::workbench::Workbench;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ByteRunKind {
    Changed,
    Inserted,
    Deleted,
}

/// A stretch of bytes that differs. `a` is the other version, `b` this one.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ByteRunInfo {
    pub kind: ByteRunKind,
    pub a_start: u32,
    pub a_end: u32,
    pub b_start: u32,
    pub b_end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MovedBlockInfo {
    pub a: u32,
    pub b: u32,
    pub len: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RoutinePairing {
    Same,
    Moved,
    Changed,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RoutineSideInfo {
    /// 24-bit SNES address.
    pub entry: u32,
    pub offset: u32,
    pub name: String,
    pub instructions: u32,
    pub bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DiffLineOp {
    Same,
    Changed,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiffLineInfo {
    pub op: DiffLineOp,
    pub a_offset: Option<u32>,
    pub a_text: Option<String>,
    pub b_offset: Option<u32>,
    pub b_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RoutinePairInfo {
    pub pairing: RoutinePairing,
    pub a: Option<RoutineSideInfo>,
    pub b: Option<RoutineSideInfo>,
    /// A changed pair's instructions side by side.
    pub lines: Vec<DiffLineInfo>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DataChangeInfo {
    /// The region is this version's (bytes only it has), not the other's.
    pub in_b: bool,
    pub start: u32,
    pub len: u32,
    /// The region kind's name.
    pub kind: String,
    pub name: Option<String>,
    pub changed: u32,
}

/// A name the other version has for a routine this one only has an
/// automatic name for.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CarriedNameInfo {
    /// Here, 24-bit.
    pub address: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ComparisonInfo {
    pub same_bytes: u64,
    pub changed_bytes: u64,
    pub inserted_bytes: u64,
    pub deleted_bytes: u64,
    /// Every stretch that differs, in order.
    pub runs: Vec<ByteRunInfo>,
    pub moves: Vec<MovedBlockInfo>,
    pub same_routines: u32,
    /// The routines that are not the same, in the other version's order,
    /// then the added ones.
    pub routines: Vec<RoutinePairInfo>,
    pub data: Vec<DataChangeInfo>,
    pub names_to_carry: Vec<CarriedNameInfo>,
}

fn side_info(r: &diff::RoutineInfo) -> RoutineSideInfo {
    RoutineSideInfo {
        entry: r.entry.as_u24(),
        offset: r.offset,
        name: r.name.clone(),
        instructions: r.instructions as u32,
        bytes: r.bytes,
    }
}

fn info(c: &diff::Comparison, names: Vec<(SnesAddress, String)>) -> ComparisonInfo {
    ComparisonInfo {
        same_bytes: c.bytes.count(RunKind::Same),
        changed_bytes: c.bytes.count(RunKind::Changed),
        inserted_bytes: c.bytes.count(RunKind::Inserted),
        deleted_bytes: c.bytes.count(RunKind::Deleted),
        runs: c
            .bytes
            .runs
            .iter()
            .filter_map(|r| {
                let kind = match r.kind {
                    RunKind::Same => return None,
                    RunKind::Changed => ByteRunKind::Changed,
                    RunKind::Inserted => ByteRunKind::Inserted,
                    RunKind::Deleted => ByteRunKind::Deleted,
                };
                Some(ByteRunInfo {
                    kind,
                    a_start: r.a.start,
                    a_end: r.a.end,
                    b_start: r.b.start,
                    b_end: r.b.end,
                })
            })
            .collect(),
        moves: c
            .bytes
            .moves
            .iter()
            .map(|m| MovedBlockInfo {
                a: m.a,
                b: m.b,
                len: m.len,
            })
            .collect(),
        same_routines: c
            .routines
            .iter()
            .filter(|r| r.pairing == Pairing::Same)
            .count() as u32,
        routines: c
            .routines
            .iter()
            .filter(|r| r.pairing != Pairing::Same)
            .map(|r| RoutinePairInfo {
                pairing: match r.pairing {
                    Pairing::Same => RoutinePairing::Same,
                    Pairing::Moved => RoutinePairing::Moved,
                    Pairing::Changed => RoutinePairing::Changed,
                    Pairing::Added => RoutinePairing::Added,
                    Pairing::Removed => RoutinePairing::Removed,
                },
                a: r.a.as_ref().map(side_info),
                b: r.b.as_ref().map(side_info),
                lines: r
                    .lines
                    .iter()
                    .map(|l| DiffLineInfo {
                        op: match l.op {
                            LineOp::Same => DiffLineOp::Same,
                            LineOp::Changed => DiffLineOp::Changed,
                            LineOp::Added => DiffLineOp::Added,
                            LineOp::Removed => DiffLineOp::Removed,
                        },
                        a_offset: l.a.as_ref().map(|a| a.0),
                        a_text: l.a.as_ref().map(|a| a.1.clone()),
                        b_offset: l.b.as_ref().map(|b| b.0),
                        b_text: l.b.as_ref().map(|b| b.1.clone()),
                    })
                    .collect(),
            })
            .collect(),
        data: c
            .data
            .iter()
            .map(|d| DataChangeInfo {
                in_b: d.in_b,
                start: d.start,
                len: d.len,
                kind: d.kind.name().to_owned(),
                name: d.name.clone(),
                changed: d.changed,
            })
            .collect(),
        names_to_carry: names
            .into_iter()
            .map(|(a, name)| CarriedNameInfo {
                address: a.as_u24(),
                name,
            })
            .collect(),
    }
}

impl Workbench {
    fn compare_job(
        &self,
        other: &Workbench,
    ) -> impl FnOnce() -> Result<ComparisonInfo, RomlensError> + Send + 'static {
        let (rb, pb, sb) = self.parts();
        let (ra, pa, sa) = other.parts();
        move || {
            let a = Side {
                rom: &ra,
                project: &pa,
                snap: &sa,
            };
            let b = Side {
                rom: &rb,
                project: &pb,
                snap: &sb,
            };
            let c = diff::compare(a, b);
            let names = diff::names_to_carry(&c, a, b);
            Ok(info(&c, names))
        }
    }
}

#[uniffi::export]
impl Workbench {
    /// This version compared with `other`, off the calling thread: what
    /// changed from `other` (the side called `a`) to this one (`b`).
    /// Both should be analysed.
    pub async fn compare_with(
        &self,
        other: Arc<Workbench>,
    ) -> Result<ComparisonInfo, RomlensError> {
        let job = self.compare_job(&other);
        crate::future::spawn(Arc::new(AtomicBool::new(false)), job).await
    }

    pub fn compare_with_blocking(
        &self,
        other: Arc<Workbench>,
    ) -> Result<ComparisonInfo, RomlensError> {
        self.compare_job(&other)()
    }

    /// Name routines here from another version, as one undo step.
    pub fn carry_names(&self, names: Vec<CarriedNameInfo>) -> Result<(), RomlensError> {
        if names.is_empty() {
            return Ok(());
        }
        self.apply_user_batch(
            names
                .into_iter()
                .map(|n| model::Command::SetLabel {
                    address: SnesAddress::from_u24(n.address),
                    name: Some(n.name),
                })
                .collect(),
        )
    }
}
