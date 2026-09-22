//! Scoring the classifier against ground truth.
//!
//! Phase 2 adds guesses to a pipeline that previously only reported what it
//! had proved, and a guess is worth having only if someone measures it. This
//! is that measurement: precision, recall and F1 per class, plus the largest
//! ranges where the map and the truth disagree, so a number that moved is
//! actionable rather than merely alarming.
//!
//! **Precision matters more than recall** and every threshold should be tuned
//! against it first. A map that confidently calls data "code" is worse than
//! one that says "unknown": the second sends a reader looking, the first sends
//! them down a listing of nonsense.
//!
//! Truth files are never committed. They are derived from a commercial ROM, so
//! the repository carries the schema and the converter and nothing else
//! (`12-content-policy.md`). `fixtures::truth_for` is the exception and the
//! reason CI can print an accuracy number at all: the fixture builder knows
//! what it wrote, so that truth is ours.

use std::fmt;

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::error::ProjectError;
use crate::model::region::{DataKind, RegionKind};

/// The three classes the headline numbers are computed over. A palette
/// misfiled as a word table is a much smaller error than code misfiled as
/// data, and mixing the two into one score would hide both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    Unknown,
    Code,
    Data,
}

impl Class {
    pub const fn of(kind: RegionKind) -> Self {
        match kind {
            RegionKind::Unknown => Class::Unknown,
            RegionKind::Code => Class::Code,
            RegionKind::Data(_) => Class::Data,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Class::Unknown => "unknown",
            Class::Code => "code",
            Class::Data => "data",
        }
    }
}

/// A byte range someone knows the answer for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TruthRange {
    pub start: u32,
    /// Exclusive.
    pub end: u32,
    pub kind: RegionKind,
}

impl TruthRange {
    pub fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// Ground truth for one ROM.
///
/// Ranges it does not name are *unlabelled*, not "unknown": they score nothing
/// either way. Truth is nearly always partial — a bank log covers the banks
/// someone documented — and counting the rest as a miss would make a careful,
/// narrow truth file look worse than a careless, broad one.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GroundTruth {
    /// The ROM this describes, so a truth file cannot be scored against the
    /// wrong image (the same check `read_identity` makes for a project).
    pub sha256: Option<String>,
    /// Sorted and non-overlapping after `parse`.
    pub ranges: Vec<TruthRange>,
}

impl GroundTruth {
    /// Parse the tab-separated schema:
    ///
    /// ```text
    /// # anything after a hash is a comment
    /// sha256=e3b0c44298fc1c149afbf4c8996fb924…
    /// 0x000000 <TAB> 0x00000C <TAB> code
    /// 0x007FC0 <TAB> 0x008000 <TAB> struct
    /// ```
    ///
    /// Addresses are file offsets; the end is exclusive. The kind is any name
    /// `RegionKind` uses, so `code`, `unknown`, or a data kind such as
    /// `palette`.
    pub fn parse(text: &str) -> Result<Self, ProjectError> {
        let mut truth = GroundTruth::default();
        for (i, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let bad = |what: &str| {
                ProjectError::BadFormat(format!("ground truth line {}: {what}", i + 1))
            };
            if let Some(hash) = line.strip_prefix("sha256=") {
                truth.sha256 = Some(hash.trim().to_lowercase());
                continue;
            }
            let mut fields = line.split('\t').map(str::trim).filter(|f| !f.is_empty());
            let (Some(start), Some(end), Some(kind)) =
                (fields.next(), fields.next(), fields.next())
            else {
                return Err(bad("expected start, end and kind separated by tabs"));
            };
            let number = |t: &str| {
                let t = t.trim_start_matches("0x").trim_start_matches('$');
                u32::from_str_radix(t, 16).ok()
            };
            let (Some(start), Some(end)) = (number(start), number(end)) else {
                return Err(bad("offsets must be hexadecimal"));
            };
            let Some(kind) = parse_kind(kind) else {
                return Err(bad(&format!("unknown kind {kind:?}")));
            };
            if end <= start {
                return Err(bad("the end offset is exclusive and must be greater"));
            }
            truth.ranges.push(TruthRange { start, end, kind });
        }
        truth.ranges.sort_by_key(|r| (r.start, r.end));
        if let Some(pair) = truth.ranges.windows(2).find(|w| w[0].end > w[1].start) {
            return Err(ProjectError::BadFormat(format!(
                "ground truth ranges overlap: {:#08X}..{:#08X} and {:#08X}..{:#08X}",
                pair[0].start, pair[0].end, pair[1].start, pair[1].end
            )));
        }
        Ok(truth)
    }

    pub fn to_tsv(&self) -> String {
        let mut out = String::from(
            "# romlens ground truth: start\\tend\\tkind, offsets hex, end exclusive\n",
        );
        if let Some(sha) = &self.sha256 {
            out.push_str(&format!("sha256={sha}\n"));
        }
        for r in &self.ranges {
            out.push_str(&format!(
                "0x{:06X}\t0x{:06X}\t{}\n",
                r.start,
                r.end,
                r.kind.name()
            ));
        }
        out
    }

    pub fn labelled_bytes(&self) -> u64 {
        self.ranges.iter().map(|r| r.len() as u64).sum()
    }
}

fn parse_kind(name: &str) -> Option<RegionKind> {
    match name {
        "code" => Some(RegionKind::Code),
        "unknown" => Some(RegionKind::Unknown),
        other => DataKind::parse(other, None, None).map(RegionKind::Data),
    }
}

/// Precision, recall and F1 for one class.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClassScore {
    pub class: Class,
    /// Labelled bytes of this class.
    pub truth_bytes: u64,
    /// Labelled bytes the analyzer put in this class.
    pub claimed_bytes: u64,
    pub correct_bytes: u64,
}

impl ClassScore {
    /// Of what we claimed, how much was right. This is the number that
    /// protects a reader, so it is the one to tune against.
    pub fn precision(&self) -> f64 {
        ratio(self.correct_bytes, self.claimed_bytes)
    }

    /// Of what is there, how much we found.
    pub fn recall(&self) -> f64 {
        ratio(self.correct_bytes, self.truth_bytes)
    }

    pub fn f1(&self) -> f64 {
        let (p, r) = (self.precision(), self.recall());
        if p + r == 0.0 {
            0.0
        } else {
            2.0 * p * r / (p + r)
        }
    }
}

fn ratio(n: u64, d: u64) -> f64 {
    if d == 0 { 0.0 } else { n as f64 / d as f64 }
}

/// A maximal run where the map and the truth disagree.
#[derive(Debug, Clone, PartialEq)]
pub struct Disagreement {
    pub start: u32,
    pub end: u32,
    pub truth: RegionKind,
    pub found: RegionKind,
}

impl Disagreement {
    pub fn len(&self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

impl fmt::Display for Disagreement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "0x{:06X}..0x{:06X}  {:>8} bytes  truth {:<10} found {}",
            self.start,
            self.end,
            self.len(),
            self.truth.name(),
            self.found.name()
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Accuracy {
    pub labelled_bytes: u64,
    /// Labelled bytes whose class matches.
    pub correct_bytes: u64,
    /// Labelled bytes whose exact kind matches, which is stricter: a palette
    /// filed as a word table is correct here only under `correct_bytes`.
    pub exact_bytes: u64,
    pub classes: Vec<ClassScore>,
    /// Largest first.
    pub disagreements: Vec<Disagreement>,
}

impl Accuracy {
    pub fn class(&self, class: Class) -> Option<&ClassScore> {
        self.classes.iter().find(|c| c.class == class)
    }

    /// Labelled bytes classified correctly, as a fraction.
    pub fn overall(&self) -> f64 {
        ratio(self.correct_bytes, self.labelled_bytes)
    }
}

/// Score a snapshot against ground truth. Unlabelled bytes are skipped.
pub fn score(snapshot: &AnalysisSnapshot, truth: &GroundTruth, worst: usize) -> Accuracy {
    let found_kind = |off: u32| {
        snapshot
            .region_at(crate::memory::address::FileOffset(off))
            .map(|r| r.kind)
            .unwrap_or(RegionKind::Unknown)
    };
    let mut scores: Vec<ClassScore> = [Class::Code, Class::Data, Class::Unknown]
        .into_iter()
        .map(|class| ClassScore {
            class,
            truth_bytes: 0,
            claimed_bytes: 0,
            correct_bytes: 0,
        })
        .collect();
    let mut labelled = 0u64;
    let mut correct = 0u64;
    let mut exact = 0u64;
    let mut disagreements: Vec<Disagreement> = Vec::new();

    for range in &truth.ranges {
        // Walk byte by byte, but collapse runs: the truth range is one kind
        // and the map changes only at region boundaries, so this is a handful
        // of steps per range in practice.
        let mut off = range.start;
        while off < range.end {
            let found = found_kind(off);
            let mut end = off + 1;
            while end < range.end && found_kind(end) == found {
                end += 1;
            }
            let len = (end - off) as u64;
            labelled += len;
            let truth_class = Class::of(range.kind);
            let found_class = Class::of(found);
            for s in &mut scores {
                if s.class == truth_class {
                    s.truth_bytes += len;
                }
                if s.class == found_class {
                    s.claimed_bytes += len;
                    if truth_class == found_class {
                        s.correct_bytes += len;
                    }
                }
            }
            if truth_class == found_class {
                correct += len;
                if range.kind == found {
                    exact += len;
                }
            } else {
                disagreements.push(Disagreement {
                    start: off,
                    end,
                    truth: range.kind,
                    found,
                });
            }
            off = end;
        }
    }

    disagreements.sort_by(|a, b| b.len().cmp(&a.len()).then(a.start.cmp(&b.start)));
    disagreements.truncate(worst);
    Accuracy {
        labelled_bytes: labelled,
        correct_bytes: correct,
        exact_bytes: exact,
        classes: scores,
        disagreements,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_round_trips_the_schema() {
        let text =
            "# a comment\nsha256=ABCD\n0x000000\t0x00000C\tcode\n0x007FC0\t0x008000\tstruct\n";
        let truth = GroundTruth::parse(text).unwrap();
        assert_eq!(truth.sha256.as_deref(), Some("abcd"));
        assert_eq!(truth.ranges.len(), 2);
        assert_eq!(truth.ranges[0].kind, RegionKind::Code);
        assert_eq!(truth.ranges[1].kind, RegionKind::Data(DataKind::Struct));
        assert_eq!(truth.labelled_bytes(), 12 + 0x40);
        assert_eq!(GroundTruth::parse(&truth.to_tsv()).unwrap(), truth);
    }

    #[test]
    fn refuses_what_it_cannot_score() {
        assert!(
            GroundTruth::parse("0x10\t0x08\tcode").is_err(),
            "end before start"
        );
        assert!(
            GroundTruth::parse("0x00\t0x10\tsideways").is_err(),
            "unknown kind"
        );
        assert!(GroundTruth::parse("0x00\t0x10").is_err(), "missing kind");
        assert!(GroundTruth::parse("zz\t0x10\tcode").is_err(), "not hex");
        assert!(
            GroundTruth::parse("0x00\t0x20\tcode\n0x10\t0x30\tbyte").is_err(),
            "overlapping ranges cannot both be true"
        );
        assert_eq!(GroundTruth::parse("").unwrap(), GroundTruth::default());
    }
}
