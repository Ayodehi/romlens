//! Romlens's own quiz questions (docs/28): made from its tables and the
//! open ROM, each with a claim checked before it is asked, so its answer is
//! certain. The format is `romlens_tutor::quiz`; this holds what needs the
//! core.

pub mod claims;
mod claims_rom;
pub mod facts;
mod make;
mod make_rom;

use std::collections::BTreeSet;
use std::sync::Arc;

use romlens_core::analysis::snapshot::AnalysisSnapshot;
use romlens_core::explain::Explanations;
use romlens_core::model::project::Project;
use romlens_core::rom::image::RomImage;
use romlens_tutor::lesson::CONCEPTS;
use romlens_tutor::quiz::Question;

use crate::workbench::Workbench;

/// What a question can be checked against.
pub struct World<'a> {
    pub rom: &'a RomImage,
    pub project: &'a Project,
    pub snap: &'a AnalysisSnapshot,
    pub explain: &'a Explanations,
}

/// A workbench's ROM, project and analysis as they are now, kept while a
/// quiz is made or marked.
pub struct Held {
    rom: RomImage,
    project: Project,
    snap: Arc<AnalysisSnapshot>,
    explain: Arc<Explanations>,
}

impl Held {
    pub fn of(wb: &Workbench) -> Held {
        let (rom, project, snap) = wb.parts();
        Held {
            rom,
            project,
            snap,
            explain: wb.explanations(),
        }
    }

    pub fn world(&self) -> World<'_> {
        World {
            rom: &self.rom,
            project: &self.project,
            snap: &self.snap,
            explain: &self.explain,
        }
    }
}

/// SplitMix64: a small, good generator, so a quiz is the same for its seed.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number below `n` (0 for none).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

/// Every question Romlens can ask about a concept at a level in this ROM,
/// each checked: one that fails its own claim is a bug in Romlens, and is
/// left out.
pub fn candidates(w: &World, rng: &mut Rng, concept: &str, level: u8) -> Vec<Question> {
    let mut seen = BTreeSet::new();
    make::all(w, rng, concept, level)
        .into_iter()
        .filter(|q| {
            let ok = claims::validate(w, q);
            debug_assert!(ok.is_ok(), "{}: {:?}", q.prompt, ok);
            ok.is_ok()
        })
        .filter(|q| seen.insert(q.prompt.clone()))
        .collect()
}

/// `n` questions about a concept at a level, the same for the same seed,
/// with ids `<prefix>-<k>`.
pub fn generate(
    w: &World,
    seed: u64,
    concept: &str,
    level: u8,
    n: usize,
    prefix: &str,
) -> Vec<Question> {
    let mut rng = Rng::new(seed);
    let mut all = candidates(w, &mut rng, concept, level);
    rng.shuffle(&mut all);
    all.truncate(n);
    for (k, q) in all.iter_mut().enumerate() {
        q.id = format!("{prefix}-{k}");
    }
    all
}

/// How many questions Romlens can ask about each concept at each level
/// in this ROM.
pub fn coverage(w: &World) -> Vec<(&'static str, [usize; 5])> {
    CONCEPTS
        .iter()
        .map(|c| {
            let mut row = [0; 5];
            for (i, n) in row.iter_mut().enumerate() {
                *n = candidates(w, &mut Rng::new(1), c.id, i as u8 + 1).len();
            }
            (c.id, row)
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use romlens_core::analysis::{AnalysisControl, analyze};

    /// The explained LoROM test program, analysed.
    pub fn held() -> Held {
        let rom = RomImage::from_bytes(romlens_core::fixtures::explain_lorom(), "t.sfc").unwrap();
        let project = Project::new(&rom);
        let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
        let explain = Explanations::build(&rom, &project, &snap);
        Held {
            rom,
            project,
            snap: Arc::new(snap),
            explain: Arc::new(explain),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use romlens_tutor::quiz::{Ask, Claim};

    #[test]
    fn every_concept_has_three_questions_at_levels_1_and_2() {
        let h = testing::held();
        let w = h.world();
        let mut short = Vec::new();
        for (c, row) in coverage(&w) {
            for level in [1, 2] {
                if row[level - 1] < 3 {
                    short.push(format!("{c} {level}: {}", row[level - 1]));
                }
            }
        }
        assert!(short.is_empty(), "too few questions:\n{}", short.join("\n"));
    }

    #[test]
    fn every_question_passes_its_claim_and_no_wrong_choice_does() {
        let h = testing::held();
        let w = h.world();
        for seed in 0..20 {
            for c in CONCEPTS {
                for level in 1..=2 {
                    for q in make::all(&w, &mut Rng::new(seed), c.id, level) {
                        claims::validate(&w, &q).unwrap_or_else(|e| panic!("{}: {e}", q.prompt));
                        let claim = q.source.claim().expect("Romlens's questions have claims");
                        if let Ask::Choice { choices, answer } = &q.ask {
                            for (i, ch) in choices.iter().enumerate() {
                                let alt = claim.with_expected(ch);
                                if i != *answer {
                                    assert!(
                                        alt.is_none_or(|a| claims::check(&w, &a).is_err()),
                                        "{}: “{ch}” passes",
                                        q.prompt
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_same_seed_asks_the_same() {
        let h = testing::held();
        let w = h.world();
        let a = generate(&w, 7, "sprites", 2, 5, "q");
        let b = generate(&w, 7, "sprites", 2, 5, "q");
        assert_eq!(a, b);
        assert_eq!(a.len(), 5);
        assert!(a.iter().all(|q| q.id.starts_with("q-")));
        let c = generate(&w, 8, "sprites", 2, 5, "q");
        assert_ne!(a, c, "another seed, another quiz");
    }

    #[test]
    fn a_false_claim_is_refused_with_a_reason() {
        let h = testing::held();
        let w = h.world();
        let e = claims::check(
            &w,
            &Claim::RegisterField {
                register: "INIDISP".into(),
                value: 0x80,
                field: "Forced blank".into(),
                expect: "off".into(),
            },
        )
        .unwrap_err();
        assert!(e.contains("INIDISP") && e.contains("Forced blank"), "{e}");
        assert!(
            claims::check(
                &w,
                &Claim::RegisterAddress {
                    register: "INIDISP".into(),
                    expect: 0x2100
                }
            )
            .is_ok()
        );
        let e = claims::check(
            &w,
            &Claim::RegisterAddress {
                register: "INIDISP".into(),
                expect: 0x2105,
            },
        )
        .unwrap_err();
        assert!(e.contains("$2100"), "{e}");
    }
}
