//! Quizzes and progress for the shell (docs/28): the records the window
//! shows, starting a quiz from Romlens's questions, answering, and the
//! milestones found in the game. `TutorSession` exports them.

use std::sync::Mutex;

use romlens_tutor::lesson::{CONCEPTS, Concept, Group, LessonStore, concept, level_name};
use romlens_tutor::progress::{ACHIEVEMENTS, Fact, Journal, MILESTONES, Progress, milestone};
use romlens_tutor::quiz::{
    Ask, Given, PROVE_QUESTIONS, Purpose, Question, Quiz, QuizStore, new_quiz_id, parse_number,
    seed_for,
};
use romlens_tutor::store::now;

use crate::quiz::{Held, claims, generate};
use crate::workbench::Workbench;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum QuizPurposeInfo {
    Prove,
    Review,
    Practice,
}

/// A bit field a bits question shows over the register.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BitFieldInfo {
    pub name: String,
    pub lo: u8,
    pub hi: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ListingLineInfo {
    pub address: u32,
    pub text: String,
}

/// How a question is answered; never its answer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum QuestionKindInfo {
    Choice {
        choices: Vec<String>,
    },
    Number {
        hex: bool,
    },
    Bits {
        register: String,
        width: u8,
        fields: Vec<BitFieldInfo>,
    },
    Line {
        lines: Vec<ListingLineInfo>,
    },
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QuestionInfo {
    pub id: String,
    pub concept: String,
    pub level: u8,
    pub prompt: String,
    pub kind: QuestionKindInfo,
    /// Where it came from, in words.
    pub source: String,
    /// Marked by Romlens rather than the model.
    pub certain: bool,
    pub has_hint: bool,
    /// A `romlens://` link to what it asks about.
    pub focus: Option<String>,
}

/// An answer given, as the window sends it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum GivenInfo {
    Choice { index: u32 },
    Number { text: String },
    Bits { bits: Vec<u8> },
    Line { index: u32 },
    Text { text: String },
    Skipped,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AnswerResultInfo {
    pub question: String,
    /// 1, 0.5 or 0; none while the model marks a written answer.
    pub credit: Option<f32>,
    pub hinted: bool,
    pub right_answer: String,
    pub explanation: String,
    pub cite: Vec<String>,
    /// The model's line on a written answer.
    pub feedback: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct OutcomeInfo {
    pub score: f32,
    pub questions: u32,
    pub certain_right: u32,
    pub done: bool,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct QuizInfo {
    pub id: String,
    pub concept: String,
    pub concept_name: String,
    pub level: u8,
    pub level_name: String,
    pub purpose: QuizPurposeInfo,
    pub questions: Vec<QuestionInfo>,
    pub results: Vec<AnswerResultInfo>,
    /// The tutor is still writing questions for it.
    pub writing: bool,
    pub finished: bool,
    pub outcome: OutcomeInfo,
    pub cost: f64,
    pub created: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GroupStandingInfo {
    pub name: String,
    /// The lowest level proven across the group: 0 until every concept in
    /// it has a proof.
    pub level: u8,
    pub proven: u32,
    pub learned: u32,
    pub concepts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AchievementInfo {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub unlocked: Option<u64>,
    /// For a game's milestone: the game.
    pub rom_title: Option<String>,
    /// One of a game's milestones, not an achievement for the student.
    pub game: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DueInfo {
    pub concept: String,
    pub concept_name: String,
    pub level: u8,
    pub due: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct XpLineInfo {
    pub when: u64,
    pub points: u32,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProgressInfo {
    pub xp: u32,
    pub rank: String,
    pub next_rank: Option<String>,
    pub next_needs: Option<String>,
    pub corpus: u32,
    pub corpus_max: u32,
    pub groups: Vec<GroupStandingInfo>,
    pub streak: u32,
    pub best_streak: u32,
    pub today: bool,
    /// The student's achievements, then this game's milestones.
    pub achievements: Vec<AchievementInfo>,
    /// Reviews due now, the longest waiting first.
    pub due: Vec<DueInfo>,
    /// The latest points, newest first.
    pub recent: Vec<XpLineInfo>,
}

/// Where the quizzes and the journal are, and the local time.
pub struct Quizzes {
    pub store: QuizStore,
    pub journal: Journal,
    offset: Mutex<i32>,
    /// Quizzes the tutor is writing questions for.
    pub writing: Mutex<std::collections::HashSet<String>>,
}

impl Quizzes {
    pub fn new(lessons: &LessonStore) -> Quizzes {
        let root = lessons.root();
        Quizzes {
            store: QuizStore::new(&root),
            journal: Journal::new(&root),
            offset: Mutex::new(0),
            writing: Mutex::new(Default::default()),
        }
    }

    pub fn set_offset(&self, seconds: i32) {
        *self.offset.lock().unwrap_or_else(|e| e.into_inner()) = seconds;
    }

    pub fn offset(&self) -> i32 {
        *self.offset.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn progress(&self, lessons: &LessonStore) -> Progress {
        lessons.progress(now(), self.offset())
    }
}

/// A concept named loosely: its id, its name, or a word of either.
pub fn find_concept(text: &str) -> Option<&'static Concept> {
    let t = text.trim().to_lowercase().replace(' ', "_");
    let words = text.trim().to_lowercase();
    CONCEPTS
        .iter()
        .find(|c| c.id == t || c.name.to_lowercase() == words)
        .or_else(|| {
            CONCEPTS
                .iter()
                .find(|c| c.id.contains(&t) || c.name.to_lowercase().contains(&words))
        })
}

/// The level a proof quiz asks by default: the lowest level learned and
/// not yet proven, or the one above the highest proven (testing out).
pub fn default_level(p: &Progress, learned: u8, id: &str) -> u8 {
    let proven = |l: u8| p.proofs.contains_key(&(id.to_owned(), l));
    (1..=learned.max(1))
        .find(|l| !proven(*l))
        .unwrap_or_else(|| (p.highest().get(id).copied().unwrap_or(0) + 1).min(5))
}

fn purpose_info(p: Purpose) -> QuizPurposeInfo {
    match p {
        Purpose::Prove => QuizPurposeInfo::Prove,
        Purpose::Review => QuizPurposeInfo::Review,
        Purpose::Practice => QuizPurposeInfo::Practice,
    }
}

pub fn question_info(q: &Question, fields: &dyn Fn(&str) -> Vec<BitFieldInfo>) -> QuestionInfo {
    QuestionInfo {
        id: q.id.clone(),
        concept: q.concept.clone(),
        level: q.level,
        prompt: q.prompt.clone(),
        kind: match &q.ask {
            Ask::Choice { choices, .. } => QuestionKindInfo::Choice {
                choices: choices.clone(),
            },
            Ask::Number { hex, .. } => QuestionKindInfo::Number { hex: *hex },
            Ask::Bits {
                register, width, ..
            } => QuestionKindInfo::Bits {
                register: register.clone(),
                width: *width,
                fields: fields(register),
            },
            Ask::Line { lines, .. } => QuestionKindInfo::Line {
                lines: lines
                    .iter()
                    .map(|(a, t)| ListingLineInfo {
                        address: *a,
                        text: t.clone(),
                    })
                    .collect(),
            },
            Ask::Text { .. } => QuestionKindInfo::Text,
        },
        source: q.source.label().into(),
        certain: q.source.certain(),
        has_hint: q.hint.is_some(),
        focus: q.focus.as_ref().map(super::session::focus_link),
    }
}

/// A register's fields, for a bits question's toggles.
pub fn bit_fields(register: &str) -> Vec<BitFieldInfo> {
    claims::register(register)
        .map(|r| {
            r.layout
                .fields
                .iter()
                .map(|f| BitFieldInfo {
                    name: f.name.into(),
                    lo: f.lo,
                    hi: f.hi,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn result_info(quiz: &Quiz, id: &str) -> Option<AnswerResultInfo> {
    let q = quiz.question(id)?;
    let a = quiz.attempt(id)?;
    Some(AnswerResultInfo {
        question: id.to_owned(),
        credit: a.credit,
        hinted: a.hinted,
        right_answer: q.answer_text(),
        explanation: q.explanation.clone(),
        cite: q.cite.clone(),
        feedback: a.feedback.clone(),
    })
}

pub fn quiz_info(quiz: &Quiz, writing: bool) -> QuizInfo {
    let o = quiz.outcome();
    QuizInfo {
        id: quiz.id.clone(),
        concept: quiz.concept.clone(),
        concept_name: concept(&quiz.concept).map_or(quiz.concept.clone(), |c| c.name.to_owned()),
        level: quiz.level,
        level_name: level_name(quiz.level).into(),
        purpose: purpose_info(quiz.purpose),
        questions: quiz
            .questions
            .iter()
            .map(|q| question_info(q, &bit_fields))
            .collect(),
        results: quiz
            .attempts
            .iter()
            .filter_map(|a| result_info(quiz, &a.question))
            .collect(),
        writing,
        finished: quiz.finished.is_some(),
        outcome: OutcomeInfo {
            score: o.score,
            questions: o.questions as u32,
            certain_right: o.certain_right as u32,
            done: o.done,
            passed: o.passed,
        },
        cost: quiz.cost,
        created: quiz.created,
    }
}

/// An answer as the quiz keeps it, read as the question needs.
pub fn given(ask: &Ask, g: GivenInfo) -> Result<Given, String> {
    Ok(match (ask, g) {
        (_, GivenInfo::Skipped) => Given::Skipped,
        (Ask::Choice { choices, .. }, GivenInfo::Choice { index }) => {
            if index as usize >= choices.len() {
                return Err("that is not one of the choices".into());
            }
            Given::Choice(index as usize)
        }
        (Ask::Number { hex, .. }, GivenInfo::Number { text }) => Given::Number(
            parse_number(&text, *hex).ok_or_else(|| format!("“{text}” is not a number"))?,
        ),
        (Ask::Bits { .. }, GivenInfo::Bits { bits }) => Given::Bits(bits),
        (Ask::Line { lines, .. }, GivenInfo::Line { index }) => {
            if index as usize >= lines.len() {
                return Err("that is not one of the lines".into());
            }
            Given::Line(index as usize)
        }
        (Ask::Text { .. }, GivenInfo::Text { text }) => {
            if text.trim().is_empty() {
                return Err("write an answer, or skip".into());
            }
            Given::Text(text)
        }
        _ => return Err("that answer doesn't fit this question".into()),
    })
}

/// A new quiz of Romlens's questions: for a proof or practice, five about a
/// concept at a level; for a review, two for each proof due (five at most).
/// `rom` and `rom_title` are the open game's.
#[allow(clippy::too_many_arguments)]
pub fn start(
    wb: &Workbench,
    progress: &Progress,
    learned: &dyn Fn(&str) -> u8,
    concept_text: Option<&str>,
    level: Option<u8>,
    purpose: Purpose,
    rom: &str,
    rom_title: &str,
) -> Result<Quiz, String> {
    let id = new_quiz_id();
    let seed = seed_for(&id);
    let held = Held::of(wb);
    let w = held.world();
    let (concept_id, level, targets, questions) = match purpose {
        Purpose::Review => {
            let due = progress.due(now());
            if due.is_empty() {
                return Err("Nothing is due for review.".into());
            }
            let mut qs = Vec::new();
            let mut targets = Vec::new();
            for (k, p) in due.iter().take(5).enumerate() {
                let got = generate(
                    &w,
                    seed.wrapping_add(k as u64),
                    &p.concept,
                    p.level,
                    2,
                    &format!("{id}-{k}"),
                );
                if got.len() == 2 {
                    targets.push((p.concept.clone(), p.level));
                    qs.extend(got);
                }
            }
            if targets.is_empty() {
                return Err(
                    "Romlens can't make review questions for what is due in this game.".into(),
                );
            }
            (targets[0].0.clone(), targets[0].1, targets, qs)
        }
        _ => {
            let text = concept_text.ok_or("name a concept to quiz")?;
            let c = find_concept(text).ok_or_else(|| format!("{text} is not on the map"))?;
            let level = level.unwrap_or_else(|| default_level(progress, learned(c.id), c.id));
            if !(1..=5).contains(&level) {
                return Err("levels are 1 to 5".into());
            }
            let qs = generate(&w, seed, c.id, level, PROVE_QUESTIONS, &id);
            if qs.len() < PROVE_QUESTIONS {
                return Err(format!(
                    "Level {level} of {} can't be proven in this game yet: Romlens has {} question{} about it here.",
                    c.name,
                    qs.len(),
                    if qs.len() == 1 { "" } else { "s" }
                ));
            }
            (c.id.to_owned(), level, Vec::new(), qs)
        }
    };
    let game = level >= 3 || purpose == Purpose::Review;
    Ok(Quiz {
        id,
        rom: game.then(|| rom.to_owned()),
        rom_title: game.then(|| rom_title.to_owned()),
        concept: concept_id,
        level,
        purpose,
        targets,
        created: now(),
        seed,
        questions,
        attempts: Vec::new(),
        hinted: Vec::new(),
        finished: None,
        cost: 0.0,
        conversation: String::new(),
    })
}

pub fn progress_info(
    p: &Progress,
    lessons: &LessonStore,
    rom: &str,
    rom_title: &str,
) -> ProgressInfo {
    let learner = lessons.learner();
    let highest = p.highest();
    let groups = Group::ALL
        .iter()
        .map(|g| {
            let cs: Vec<&Concept> = CONCEPTS.iter().filter(|c| c.group == *g).collect();
            GroupStandingInfo {
                name: g.name().into(),
                level: cs
                    .iter()
                    .map(|c| highest.get(c.id).copied().unwrap_or(0))
                    .min()
                    .unwrap_or(0),
                proven: cs.iter().filter(|c| highest.contains_key(c.id)).count() as u32,
                learned: cs.iter().filter(|c| learner.level(c.id) > 0).count() as u32,
                concepts: cs.len() as u32,
            }
        })
        .collect();
    let mut achievements: Vec<AchievementInfo> = ACHIEVEMENTS
        .iter()
        .map(|a| AchievementInfo {
            id: a.id.into(),
            title: a.title.into(),
            detail: a.detail.into(),
            unlocked: p
                .unlocked
                .iter()
                .find(|u| u.id == a.id && u.rom.is_none())
                .map(|u| u.when),
            rom_title: None,
            game: false,
        })
        .collect();
    achievements.extend(MILESTONES.iter().map(|m| {
        AchievementInfo {
            id: m.id.into(),
            title: m.title.into(),
            detail: m.detail.into(),
            unlocked: p
                .unlocked
                .iter()
                .find(|u| u.id == m.id && u.rom.as_deref() == Some(rom))
                .map(|u| u.when),
            rom_title: Some(rom_title.into()),
            game: true,
        }
    }));
    ProgressInfo {
        xp: p.xp,
        rank: p.rank().name.into(),
        next_rank: p.next_rank().map(|r| r.name.into()),
        next_needs: p.next_rank().map(|r| r.needs.into()),
        corpus: p.corpus.0,
        corpus_max: p.corpus.1,
        groups,
        streak: p.streak.current,
        best_streak: p.streak.best,
        today: p.streak.today,
        achievements,
        due: p
            .due(now())
            .iter()
            .map(|d| DueInfo {
                concept: d.concept.clone(),
                concept_name: concept(&d.concept).map_or(d.concept.clone(), |c| c.name.into()),
                level: d.level,
                due: d.due,
            })
            .collect(),
        recent: p
            .ledger
            .iter()
            .rev()
            .take(20)
            .map(|l| XpLineInfo {
                when: l.when,
                points: l.points,
                why: l.why.clone(),
            })
            .collect(),
    }
}

/// The milestones this game's project and analysis now meet (docs/28): a
/// name at the reset and NMI targets, on the main loop, on the sound
/// upload, on a routine sending DMA to VRAM; routines named; lines
/// commented. Labels and comments are the user's (or the tutor's, which
/// the user approves).
pub fn milestones_met(wb: &Workbench) -> Vec<&'static str> {
    use romlens_core::decompile::function;
    use romlens_core::explain::{DmaDest, IdiomKind};
    use romlens_core::memory::address::SnesAddress;
    use romlens_core::model::comment::CommentKind;

    let held = Held::of(wb);
    let w = held.world();
    let named = |a: SnesAddress| {
        w.project
            .labels
            .contains_key(&romlens_core::model::project::Project::canonical(w.rom, a))
    };
    let h = w.rom.header();
    let mut out = Vec::new();
    if h.emulation.reset != 0 && named(SnesAddress::new(0, h.emulation.reset)) {
        out.push("reset_named");
    }
    if h.native.nmi != 0 && named(SnesAddress::new(0, h.native.nmi)) {
        out.push("nmi_named");
    }
    let entries = function::entries(w.snap);
    let routine_named = |off: romlens_core::memory::address::FileOffset| {
        function::containing(w.rom, w.snap, &entries, off).is_some_and(|f| named(f.entry))
    };
    let routine_marked = |off: romlens_core::memory::address::FileOffset| {
        function::containing(w.rom, w.snap, &entries, off).is_some_and(|f| {
            named(f.entry)
                || f.steps.iter().any(|s| {
                    w.project
                        .comment_at(s.insn.address, CommentKind::Line)
                        .is_some()
                        || w.project
                            .comment_at(s.insn.address, CommentKind::Block)
                            .is_some()
                })
        })
    };
    let idioms = w.explain.idioms();
    if idioms
        .iter()
        .any(|i| i.kind == IdiomKind::Wait && routine_named(i.first()))
    {
        out.push("main_loop_named");
    }
    if idioms
        .iter()
        .any(|i| i.kind == IdiomKind::ApuUpload && routine_marked(i.first()))
    {
        out.push("sound_upload_named");
    }
    if idioms.iter().any(|i| {
        i.kind == IdiomKind::Dma
            && i.transfers
                .iter()
                .any(|t| matches!(t.dest, DmaDest::Vram(_)))
            && routine_named(i.first())
    }) {
        out.push("vram_dma_named");
    }
    let routines = entries.iter().filter(|a| named(**a)).count();
    for (n, id) in [
        (10, "routines_10"),
        (50, "routines_50"),
        (200, "routines_200"),
    ] {
        if routines >= n {
            out.push(id);
        }
    }
    let comments = w
        .project
        .comments
        .keys()
        .filter(|(_, k)| *k == CommentKind::Line)
        .count();
    if comments >= 25 {
        out.push("comments_25");
    }
    out
}

/// Journals the milestones newly met; returns their titles.
pub fn record_milestones(q: &Quizzes, wb: &Workbench, rom: &str, rom_title: &str) -> Vec<String> {
    let have: std::collections::HashSet<String> = q
        .journal
        .read()
        .into_iter()
        .filter_map(|f| match f {
            Fact::Milestone { rom: r, id, .. } if r == rom => Some(id),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    for id in milestones_met(wb) {
        if have.contains(id) {
            continue;
        }
        let fact = Fact::Milestone {
            rom: rom.into(),
            rom_title: rom_title.into(),
            id: id.into(),
            when: now(),
        };
        if q.journal.append(&fact).is_ok()
            && let Some(m) = milestone(id)
        {
            out.push(m.title.to_owned());
        }
    }
    out
}
