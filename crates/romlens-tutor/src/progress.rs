//! Progress (docs/28): what the student has proven, their points, streak,
//! rank and achievements. None of it is stored. It is worked out each
//! time from the lessons, the quizzes, the marks and the journal, which
//! only grow, so it cannot drift and every point has a cause.
//!
//! The journal (`<root>/Learner/journal.jsonl`) keeps the few facts that
//! have no other file: a predict question's guess, and a milestone reached
//! in a game.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::lesson::{CONCEPTS, Group, Lesson, Marks};
use crate::quiz::{Purpose, Quiz};
use crate::store::StoreError;

/// A fact with no other file, one JSON object a line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fact {
    /// A lesson step's predict question, guessed before Show; `right` when
    /// the step's claim could check it.
    PredictAnswered {
        lesson: String,
        step: u32,
        guess: String,
        #[serde(default)]
        right: Option<bool>,
        when: u64,
    },
    /// The tutor's mark for a guess Romlens couldn't check: 1, 0.5 or 0,
    /// and its line to the student.
    GuessMarked {
        lesson: String,
        step: u32,
        credit: f32,
        said: String,
        when: u64,
    },
    /// A milestone reached in a game (`reset_named`, `routines_50`, …).
    Milestone {
        rom: String,
        rom_title: String,
        id: String,
        when: u64,
    },
}

impl Fact {
    pub fn when(&self) -> u64 {
        match self {
            Fact::PredictAnswered { when, .. }
            | Fact::GuessMarked { when, .. }
            | Fact::Milestone { when, .. } => *when,
        }
    }
}

/// The journal: appended to, never rewritten.
pub struct Journal {
    path: PathBuf,
}

impl Journal {
    pub fn new(root: &Path) -> Journal {
        Journal {
            path: root.join("Learner").join("journal.jsonl"),
        }
    }

    pub fn append(&self, fact: &Fact) -> Result<(), StoreError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut line = serde_json::to_vec(fact).expect("a fact serialises");
        line.push(b'\n');
        f.write_all(&line)?;
        f.sync_data()?;
        Ok(())
    }

    /// Every fact, oldest first. A line that doesn't read (a write cut off
    /// by a crash) is skipped.
    pub fn read(&self) -> Vec<Fact> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// Removes the journal ("Reset progress").
    pub fn reset(&self) -> Result<(), StoreError> {
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

/// A day in seconds.
pub const DAY: u64 = 86_400;

/// The review boxes, in days: a proof moves up a box each time it is
/// recalled, and back to the first when it isn't.
pub const BOXES_DAYS: [u64; 5] = [1, 3, 7, 21, 60];

/// Points (docs/28, "Points, rank and achievements").
pub mod xp {
    /// Proving level L earns `PROVE * L`.
    pub const PROVE: u32 = 50;
    /// A review passed: `REVIEW + REVIEW_BOX * box`.
    pub const REVIEW: u32 = 10;
    pub const REVIEW_BOX: u32 = 5;
    /// A certain answer right; half with the hint.
    pub const ANSWER: u32 = 2;
    pub const ANSWER_HINTED: u32 = 1;
    /// A lesson finished, and each concept-level it newly taught.
    pub const LESSON: u32 = 20;
    pub const LESSON_LEVEL: u32 = 5;
    /// A predict question guessed before Show, and the guess checked right.
    pub const GUESS: u32 = 3;
    pub const GUESS_RIGHT: u32 = 2;
    /// The day's first quiz answer, lesson or guess.
    pub const DAY: u32 = 5;
    /// A streak reaching 7 and 30 days.
    pub const STREAK_7: u32 = 25;
    pub const STREAK_30: u32 = 100;
}

/// A milestone in a game: its id, what it asks, and its points. The FFI
/// checks them against the project and the analysis (`quiz::milestones`)
/// and writes each to the journal once reached.
pub struct Milestone {
    pub id: &'static str,
    pub title: &'static str,
    pub detail: &'static str,
    pub xp: u32,
}

pub const MILESTONES: &[Milestone] = &[
    Milestone {
        id: "reset_named",
        title: "Where it all starts",
        detail: "Name the routine the reset vector points at",
        xp: 20,
    },
    Milestone {
        id: "nmi_named",
        title: "Every frame",
        detail: "Name the NMI handler",
        xp: 20,
    },
    Milestone {
        id: "main_loop_named",
        title: "Round and round",
        detail: "Name the routine that waits for vertical blank",
        xp: 30,
    },
    Milestone {
        id: "sound_upload_named",
        title: "The other CPU",
        detail: "Name or comment the routine that uploads the sound driver",
        xp: 30,
    },
    Milestone {
        id: "vram_dma_named",
        title: "Pictures in",
        detail: "Name a routine that sends DMA to VRAM",
        xp: 25,
    },
    Milestone {
        id: "routines_10",
        title: "Mapmaker",
        detail: "Name 10 routines",
        xp: 15,
    },
    Milestone {
        id: "routines_50",
        title: "Cartographer",
        detail: "Name 50 routines",
        xp: 25,
    },
    Milestone {
        id: "routines_200",
        title: "Atlas",
        detail: "Name 200 routines",
        xp: 40,
    },
    Milestone {
        id: "comments_25",
        title: "Margin notes",
        detail: "Comment 25 lines",
        xp: 15,
    },
];

pub fn milestone(id: &str) -> Option<&'static Milestone> {
    MILESTONES.iter().find(|m| m.id == id)
}

/// An achievement: the non-exploring ones are rules over the progress; the
/// exploring ones are the milestones, once for each game.
pub struct Achievement {
    pub id: &'static str,
    pub title: &'static str,
    pub detail: &'static str,
}

const fn a(id: &'static str, title: &'static str, detail: &'static str) -> Achievement {
    Achievement { id, title, detail }
}

pub const ACHIEVEMENTS: &[Achievement] = &[
    a("first_lesson", "First steps", "Finish a lesson"),
    a("five_lessons", "Regular", "Finish five lessons"),
    a(
        "predictor",
        "Predictor",
        "Guess ten predict questions before Show",
    ),
    a(
        "down_to_the_metal",
        "Down to the metal",
        "Finish a lesson at level 4",
    ),
    a("first_proof", "Proven", "Prove a level with a quiz"),
    a(
        "clean_sheet",
        "Clean sheet",
        "Prove a level with every answer right and no hints",
    ),
    a(
        "tested_out",
        "Tested out",
        "Prove a level no lesson had taught you",
    ),
    a("one_of_each", "One of each", "Prove a level in every group"),
    a(
        "group_machine",
        "The machine",
        "Prove every concept of the machine at level 2",
    ),
    a(
        "group_graphics",
        "Graphics",
        "Prove every graphics concept at level 2",
    ),
    a(
        "group_timing",
        "Timing",
        "Prove every timing concept at level 2",
    ),
    a("group_cpu", "The CPU", "Prove every CPU concept at level 2"),
    a(
        "group_sound",
        "Sound",
        "Prove every sound concept at level 2",
    ),
    a(
        "bytes_and_cycles",
        "Bytes and cycles",
        "Prove a concept at level 5",
    ),
    a("came_back", "Came back", "Pass a review"),
    a(
        "long_memory",
        "Long memory",
        "Keep a proof until its review is 60 days away",
    ),
    a("streak_3", "Three days", "Learn on three days in a row"),
    a("streak_7", "A week", "Learn on seven days in a row"),
    a("streak_30", "A month", "Learn on thirty days in a row"),
];

fn group_achievement(g: Group) -> &'static str {
    match g {
        Group::Machine => "group_machine",
        Group::Graphics => "group_graphics",
        Group::Timing => "group_timing",
        Group::Cpu => "group_cpu",
        Group::Sound => "group_sound",
    }
}

/// A rank: what the student has shown they know, from the highest level
/// proven of each concept.
pub struct Rank {
    pub name: &'static str,
    /// What reaching it takes.
    pub needs: &'static str,
    rule: fn(&BTreeMap<&'static str, u8>) -> bool,
}

fn at_least(p: &BTreeMap<&'static str, u8>, level: u8) -> usize {
    p.values().filter(|l| **l >= level).count()
}

pub const RANKS: &[Rank] = &[
    Rank {
        name: "Power-on",
        needs: "",
        rule: |_| true,
    },
    Rank {
        name: "Reset",
        needs: "Prove a level",
        rule: |p| !p.is_empty(),
    },
    Rank {
        name: "First frame",
        needs: "Prove every concept of the machine at level 1",
        rule: |p| {
            CONCEPTS
                .iter()
                .filter(|c| c.group == Group::Machine)
                .all(|c| p.get(c.id).is_some_and(|l| *l >= 1))
        },
    },
    Rank {
        name: "In vblank",
        needs: "Prove 15 concepts at level 2",
        rule: |p| at_least(p, 2) >= 15,
    },
    Rank {
        name: "Tracer",
        needs: "Prove 10 concepts at level 3",
        rule: |p| at_least(p, 3) >= 10,
    },
    Rank {
        name: "Disassembler",
        needs: "Prove 15 concepts at level 4",
        rule: |p| at_least(p, 4) >= 15,
    },
    Rank {
        name: "Cycle counter",
        needs: "Prove 10 concepts at level 5",
        rule: |p| at_least(p, 5) >= 10,
    },
    Rank {
        name: "ROM reader",
        needs: "Prove every concept at level 3",
        rule: |p| at_least(p, 3) >= CONCEPTS.len(),
    },
    Rank {
        name: "Hardware sage",
        needs: "Prove every concept at level 5",
        rule: |p| at_least(p, 5) >= CONCEPTS.len(),
    },
];

/// A level proven, and its review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof {
    pub concept: String,
    pub level: u8,
    pub proved: u64,
    pub quiz: String,
    pub rom: Option<String>,
    /// The review box, 0 to 4.
    pub r#box: u8,
    pub due: u64,
    pub reviews: u32,
}

/// One entry of the ledger: every point has a cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XpLine {
    pub when: u64,
    pub points: u32,
    pub why: String,
    /// The quiz, lesson or fact it came from.
    pub from: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Streak {
    /// Days in a row up to today (or yesterday: today may yet count).
    pub current: u32,
    pub best: u32,
    /// Today already counts.
    pub today: bool,
}

/// An achievement earned; an exploring one names its game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unlocked {
    pub id: &'static str,
    pub when: u64,
    pub rom: Option<String>,
    pub rom_title: Option<String>,
}

/// Everything, worked out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub xp: u32,
    /// Oldest first.
    pub ledger: Vec<XpLine>,
    pub proofs: BTreeMap<(String, u8), Proof>,
    pub streak: Streak,
    pub unlocked: Vec<Unlocked>,
    /// An index into `RANKS`.
    pub rank: usize,
    /// The sum of the highest level proven of each concept, and its most.
    pub corpus: (u32, u32),
}

/// What changed between two workings-out: for the window's banner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gained {
    pub xp: u32,
    pub proofs: Vec<(String, u8)>,
    pub unlocked: Vec<Unlocked>,
    pub rank: Option<&'static str>,
}

/// The inputs: everything stored.
pub struct Inputs<'a> {
    pub lessons: &'a [Lesson],
    pub quizzes: &'a [Quiz],
    pub journal: &'a [Fact],
    pub marks: &'a Marks,
    pub now: u64,
    /// The local time's offset from UTC, in seconds.
    pub offset: i32,
}

fn local_day(when: u64, offset: i32) -> i64 {
    (when as i64 + offset as i64).div_euclid(DAY as i64)
}

/// When a quiz was done: its finish, or its last answer.
fn quiz_time(q: &Quiz) -> u64 {
    q.finished
        .or_else(|| q.attempts.iter().map(|a| a.when).max())
        .unwrap_or(q.created)
}

impl Progress {
    /// Works everything out from what is stored.
    pub fn derive(i: &Inputs) -> Progress {
        let mut p = Progress::default();
        let mut days: BTreeSet<i64> = BTreeSet::new();
        let mut first_of_day: BTreeMap<i64, (u64, String)> = BTreeMap::new();
        let mut active = |when: u64, from: &str| {
            let d = local_day(when, i.offset);
            days.insert(d);
            let e = first_of_day.entry(d).or_insert((when, from.to_owned()));
            if when < e.0 {
                *e = (when, from.to_owned());
            }
        };
        let earn = |p: &mut Progress, when: u64, points: u32, why: String, from: &str| {
            if points > 0 {
                p.ledger.push(XpLine {
                    when,
                    points,
                    why,
                    from: from.to_owned(),
                });
            }
        };
        let unlock = |p: &mut Progress, id: &'static str, when: u64| {
            if !p.unlocked.iter().any(|u| u.id == id && u.rom.is_none()) {
                p.unlocked.push(Unlocked {
                    id,
                    when,
                    rom: None,
                    rom_title: None,
                });
            }
        };

        // Lessons: points for each, and what they taught, in order.
        let mut lessons: Vec<&Lesson> = i.lessons.iter().filter(|l| l.finished).collect();
        lessons.sort_by_key(|l| l.created);
        let mut taught: BTreeMap<&str, (u8, u64)> = BTreeMap::new();
        for (n, l) in lessons.iter().enumerate() {
            let mut new_levels = 0u32;
            for (c, level) in &l.concepts {
                let had = taught.get(c.as_str()).map_or(0, |t| t.0);
                if *level > had {
                    new_levels += (*level - had) as u32;
                    taught.insert(c.as_str(), (*level, l.created));
                }
            }
            earn(
                &mut p,
                l.created,
                xp::LESSON + xp::LESSON_LEVEL * new_levels,
                format!("Lesson finished: {}", l.title),
                &l.id,
            );
            active(l.created, &l.id);
            if n == 0 {
                unlock(&mut p, "first_lesson", l.created);
            }
            if n == 4 {
                unlock(&mut p, "five_lessons", l.created);
            }
            if l.concepts.iter().any(|(_, lv)| *lv >= 4) {
                unlock(&mut p, "down_to_the_metal", l.created);
            }
        }

        // Quizzes, in order: answers, proofs and reviews.
        let mut quizzes: Vec<&Quiz> = i.quizzes.iter().collect();
        quizzes.sort_by_key(|q| (quiz_time(q), q.created));
        for q in &quizzes {
            for a in &q.attempts {
                let certain = q.question(&a.question).is_some_and(|x| x.source.certain());
                if a.credit.is_some() {
                    active(a.when, &q.id);
                }
                let points = match (certain, a.credit) {
                    (true, Some(c)) if c >= 1.0 => xp::ANSWER,
                    (true, Some(c)) if c > 0.0 => xp::ANSWER_HINTED,
                    _ => 0,
                };
                earn(&mut p, a.when, points, "A right answer".into(), &q.id);
            }
            let o = q.outcome();
            if !o.done {
                continue;
            }
            let t = quiz_time(q);
            match q.purpose {
                Purpose::Prove if o.passed => {
                    let key = (q.concept.clone(), q.level);
                    if p.proofs.contains_key(&key) {
                        continue;
                    }
                    p.proofs.insert(
                        key,
                        Proof {
                            concept: q.concept.clone(),
                            level: q.level,
                            proved: t,
                            quiz: q.id.clone(),
                            rom: q.rom.clone(),
                            r#box: 0,
                            due: t + BOXES_DAYS[0] * DAY,
                            reviews: 0,
                        },
                    );
                    let name =
                        crate::lesson::concept(&q.concept).map_or(q.concept.as_str(), |c| c.name);
                    earn(
                        &mut p,
                        t,
                        xp::PROVE * q.level as u32,
                        format!("Proved {name}, level {}", q.level),
                        &q.id,
                    );
                    unlock(&mut p, "first_proof", t);
                    if o.score >= o.questions as f32 && q.hinted.is_empty() {
                        unlock(&mut p, "clean_sheet", t);
                    }
                    let learned = taught
                        .get(q.concept.as_str())
                        .filter(|(_, when)| *when <= t)
                        .map_or(0, |(lv, _)| *lv);
                    let marked = i.marks.known.get(&q.concept).map_or(0, |m| m.0);
                    if learned.max(marked) < q.level {
                        unlock(&mut p, "tested_out", t);
                    }
                    if q.level >= 5 {
                        unlock(&mut p, "bytes_and_cycles", t);
                    }
                    let highest = p.highest();
                    if Group::ALL.iter().all(|g| {
                        CONCEPTS
                            .iter()
                            .any(|c| c.group == *g && highest.contains_key(c.id))
                    }) {
                        unlock(&mut p, "one_of_each", t);
                    }
                    for g in Group::ALL {
                        if CONCEPTS
                            .iter()
                            .filter(|c| c.group == g)
                            .all(|c| highest.get(c.id).is_some_and(|l| *l >= 2))
                        {
                            unlock(&mut p, group_achievement(g), t);
                        }
                    }
                }
                Purpose::Review => {
                    for (concept, level) in &q.targets {
                        let key = (concept.clone(), *level);
                        let Some(proof) = p.proofs.get_mut(&key) else {
                            continue;
                        };
                        let right = q
                            .questions
                            .iter()
                            .filter(|x| x.concept == *concept && x.level == *level)
                            .filter(|x| {
                                q.attempt(&x.id)
                                    .and_then(|a| a.credit)
                                    .is_some_and(|c| c >= 1.0)
                            })
                            .count();
                        let asked = q
                            .questions
                            .iter()
                            .filter(|x| x.concept == *concept && x.level == *level)
                            .count();
                        proof.reviews += 1;
                        if asked > 0 && right == asked {
                            proof.r#box = (proof.r#box + 1).min(4);
                        } else if right == 0 {
                            proof.r#box = 0;
                        }
                        proof.due = t + BOXES_DAYS[proof.r#box as usize] * DAY;
                        let (b, long) = (proof.r#box, proof.r#box == 4);
                        if asked > 0 && right == asked {
                            let name = crate::lesson::concept(concept)
                                .map_or(concept.as_str(), |c| c.name);
                            earn(
                                &mut p,
                                t,
                                xp::REVIEW + xp::REVIEW_BOX * b as u32,
                                format!("Reviewed {name}, level {level}"),
                                &q.id,
                            );
                            unlock(&mut p, "came_back", t);
                        }
                        if long {
                            unlock(&mut p, "long_memory", t);
                        }
                    }
                }
                _ => {}
            }
        }

        // The journal: guesses and milestones, each once.
        let mut guessed: BTreeSet<(String, u32)> = BTreeSet::new();
        // Guesses already right by their claim: the tutor's mark adds nothing.
        let mut marked_right: BTreeSet<(String, u32)> = i
            .journal
            .iter()
            .filter_map(|f| match f {
                Fact::PredictAnswered {
                    lesson,
                    step,
                    right: Some(true),
                    ..
                } => Some((lesson.clone(), *step)),
                _ => None,
            })
            .collect();
        let mut reached: BTreeSet<(String, String)> = BTreeSet::new();
        let mut journal: Vec<&Fact> = i.journal.iter().collect();
        journal.sort_by_key(|f| f.when());
        for f in journal {
            match f {
                Fact::PredictAnswered {
                    lesson,
                    step,
                    right,
                    when,
                    ..
                } => {
                    if !guessed.insert((lesson.clone(), *step)) {
                        continue;
                    }
                    let points = xp::GUESS
                        + if *right == Some(true) {
                            xp::GUESS_RIGHT
                        } else {
                            0
                        };
                    earn(
                        &mut p,
                        *when,
                        points,
                        "A predict question guessed".into(),
                        lesson,
                    );
                    active(*when, lesson);
                    if guessed.len() == 10 {
                        unlock(&mut p, "predictor", *when);
                    }
                }
                Fact::GuessMarked {
                    lesson,
                    step,
                    credit,
                    when,
                    ..
                } => {
                    if *credit >= 1.0 && marked_right.insert((lesson.clone(), *step)) {
                        earn(
                            &mut p,
                            *when,
                            xp::GUESS_RIGHT,
                            "A predict question guessed right".into(),
                            lesson,
                        );
                    }
                }
                Fact::Milestone {
                    rom,
                    rom_title,
                    id,
                    when,
                } => {
                    let Some(m) = milestone(id) else { continue };
                    if !reached.insert((rom.clone(), id.clone())) {
                        continue;
                    }
                    earn(
                        &mut p,
                        *when,
                        m.xp,
                        format!("{} in {rom_title}", m.title),
                        id,
                    );
                    p.unlocked.push(Unlocked {
                        id: m.id,
                        when: *when,
                        rom: Some(rom.clone()),
                        rom_title: Some(rom_title.clone()),
                    });
                }
            }
        }

        // Days: the first of each, and the streaks.
        for (when, from) in first_of_day.values() {
            earn(&mut p, *when, xp::DAY, "The day's first".into(), from);
        }
        let today = local_day(i.now, i.offset);
        let (streak, reached_at) = streaks(&days, today);
        p.streak = streak;
        for (len, day) in reached_at {
            let when = (day * DAY as i64 - i.offset as i64).max(0) as u64;
            match len {
                3 => unlock(&mut p, "streak_3", when),
                7 => {
                    unlock(&mut p, "streak_7", when);
                    earn(
                        &mut p,
                        when,
                        xp::STREAK_7,
                        "Seven days in a row".into(),
                        "streak",
                    );
                }
                30 => {
                    unlock(&mut p, "streak_30", when);
                    earn(
                        &mut p,
                        when,
                        xp::STREAK_30,
                        "Thirty days in a row".into(),
                        "streak",
                    );
                }
                _ => {}
            }
        }

        // In time, and within a second by what they were for, so the order
        // never depends on ids.
        p.ledger
            .sort_by(|a, b| a.when.cmp(&b.when).then(a.why.cmp(&b.why)));
        p.xp = p.ledger.iter().map(|l| l.points).sum();
        p.unlocked.sort_by_key(|u| u.when);
        let highest = p.highest();
        p.rank = RANKS.iter().rposition(|r| (r.rule)(&highest)).unwrap_or(0);
        p.corpus = (
            highest.values().map(|l| *l as u32).sum(),
            CONCEPTS.len() as u32 * 5,
        );
        p
    }

    /// The highest level proven of each concept.
    pub fn highest(&self) -> BTreeMap<&'static str, u8> {
        let mut out: BTreeMap<&'static str, u8> = BTreeMap::new();
        for (c, level) in self.proofs.keys() {
            let Some(concept) = crate::lesson::concept(c) else {
                continue;
            };
            let e = out.entry(concept.id).or_insert(0);
            *e = (*e).max(*level);
        }
        out
    }

    /// The proofs to review: each concept's highest proven level, once due.
    pub fn due(&self, now: u64) -> Vec<&Proof> {
        let highest = self.highest();
        let mut out: Vec<&Proof> = self
            .proofs
            .values()
            .filter(|p| highest.get(p.concept.as_str()) == Some(&p.level) && p.due <= now)
            .collect();
        out.sort_by_key(|p| p.due);
        out
    }

    pub fn rank(&self) -> &'static Rank {
        &RANKS[self.rank]
    }

    pub fn next_rank(&self) -> Option<&'static Rank> {
        RANKS.get(self.rank + 1)
    }

    /// What is new since `before`.
    pub fn diff(&self, before: &Progress) -> Gained {
        Gained {
            xp: self.xp.saturating_sub(before.xp),
            proofs: self
                .proofs
                .keys()
                .filter(|k| !before.proofs.contains_key(*k))
                .cloned()
                .collect(),
            unlocked: self
                .unlocked
                .iter()
                .filter(|u| {
                    !before
                        .unlocked
                        .iter()
                        .any(|b| b.id == u.id && b.rom == u.rom)
                })
                .cloned()
                .collect(),
            rank: (self.rank > before.rank).then(|| self.rank().name),
        }
    }
}

/// The current and best streaks over the active days, and each length a
/// run reached (3, 7, 30) with the day it did. One missed day is bridged
/// when no other was in the last seven days of the run.
fn streaks(days: &BTreeSet<i64>, today: i64) -> (Streak, Vec<(u32, i64)>) {
    let mut best = 0u32;
    let mut run = 0u32;
    let mut last: Option<i64> = None;
    let mut bridged_at: Option<i64> = None;
    let mut reached = Vec::new();
    for &d in days {
        run = match last {
            Some(l) if d - l == 1 => run + 1,
            Some(l) if d - l == 2 && bridged_at.is_none_or(|b| d - b > 7) => {
                bridged_at = Some(d);
                run + 1
            }
            _ => {
                bridged_at = None;
                1
            }
        };
        for mark in [3, 7, 30] {
            if run == mark {
                reached.push((mark, d));
            }
        }
        best = best.max(run);
        last = Some(d);
    }
    let alive = last.is_some_and(|l| {
        today - l <= 1 || (today - l == 2 && bridged_at.is_none_or(|b| today - b > 7))
    });
    (
        Streak {
            current: if alive { run } else { 0 },
            best,
            today: last == Some(today),
        },
        reached,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::lesson::Lesson;
    use crate::quiz::{Ask, Attempt, Claim, Given, Question, Quiz, Source};

    /// Midnight UTC, 16 January 2027.
    const T0: u64 = 20_834 * DAY;

    fn romlens_q(id: &str, concept: &str, level: u8) -> Question {
        Question {
            id: id.into(),
            concept: concept.into(),
            level,
            prompt: "?".into(),
            ask: Ask::Choice {
                choices: vec!["a".into(), "b".into()],
                answer: 0,
            },
            explanation: "e".into(),
            cite: vec![],
            hint: None,
            source: Source::Romlens {
                generator: "fact".into(),
                claim: Claim::Fact {
                    id: "x".into(),
                    expect: 1,
                },
            },
            focus: None,
        }
    }

    /// A quiz whose `right` first questions were answered right at `t`.
    fn quiz(
        id: &str,
        purpose: Purpose,
        concept: &str,
        level: u8,
        n: usize,
        right: usize,
        t: u64,
    ) -> Quiz {
        let questions: Vec<Question> = (0..n)
            .map(|k| romlens_q(&format!("{id}-{k}"), concept, level))
            .collect();
        let attempts = questions
            .iter()
            .enumerate()
            .map(|(k, q)| Attempt {
                question: q.id.clone(),
                given: Given::Choice(if k < right { 0 } else { 1 }),
                credit: Some(if k < right { 1.0 } else { 0.0 }),
                hinted: false,
                when: t,
                feedback: None,
            })
            .collect();
        Quiz {
            id: id.into(),
            rom: None,
            rom_title: None,
            concept: concept.into(),
            level,
            purpose,
            targets: if purpose == Purpose::Review {
                vec![(concept.into(), level)]
            } else {
                vec![]
            },
            created: t,
            seed: 1,
            questions,
            attempts,
            hinted: vec![],
            finished: Some(t),
            cost: 0.0,
            conversation: String::new(),
        }
    }

    fn lesson(id: &str, concepts: &[(&str, u8)], t: u64) -> Lesson {
        Lesson {
            id: id.into(),
            title: format!("Lesson {id}"),
            rom: None,
            created: t,
            levels: (1, 2),
            concepts: concepts.iter().map(|(c, l)| (c.to_string(), *l)).collect(),
            builds_on: vec![],
            steps: vec![],
            next: vec![],
            conversation: String::new(),
            finished: true,
            revisions: vec![],
            checked: None,
        }
    }

    fn derive(
        lessons: &[Lesson],
        quizzes: &[Quiz],
        journal: &[Fact],
        now: u64,
        offset: i32,
    ) -> Progress {
        Progress::derive(&Inputs {
            lessons,
            quizzes,
            journal,
            marks: &Marks::default(),
            now,
            offset,
        })
    }

    fn has(p: &Progress, id: &str) -> bool {
        p.unlocked.iter().any(|u| u.id == id)
    }

    #[test]
    fn a_proof_takes_the_rule_and_is_never_lost() {
        // Four of five, all certain: proven, 100 points for level 2.
        let p = derive(
            &[],
            &[quiz("q1", Purpose::Prove, "sprites", 2, 5, 4, T0)],
            &[],
            T0,
            0,
        );
        let proof = &p.proofs[&("sprites".to_string(), 2)];
        assert_eq!((proof.r#box, proof.due), (0, T0 + DAY));
        assert!(
            p.ledger
                .iter()
                .any(|l| l.points == 100 && l.why.contains("Sprites, level 2")),
            "{:?}",
            p.ledger
        );
        assert!(has(&p, "first_proof") && has(&p, "tested_out") && !has(&p, "clean_sheet"));
        // Three of five fails; so does a practice quiz, however good.
        let p = derive(
            &[],
            &[quiz("q1", Purpose::Prove, "sprites", 2, 5, 3, T0)],
            &[],
            T0,
            0,
        );
        assert!(p.proofs.is_empty());
        let p = derive(
            &[],
            &[quiz("q1", Purpose::Practice, "sprites", 2, 5, 5, T0)],
            &[],
            T0,
            0,
        );
        assert!(p.proofs.is_empty());
        // A later failed attempt at the same level takes nothing away.
        let p = derive(
            &[],
            &[
                quiz("q1", Purpose::Prove, "sprites", 1, 5, 5, T0),
                quiz("q2", Purpose::Prove, "sprites", 1, 5, 0, T0 + 10),
            ],
            &[],
            T0 + 20,
            0,
        );
        assert_eq!(p.proofs.len(), 1);
        assert!(has(&p, "clean_sheet"));
        assert_eq!(p.xp, p.ledger.iter().map(|l| l.points).sum::<u32>());
    }

    #[test]
    fn reviews_move_the_box() {
        let proof = quiz("q1", Purpose::Prove, "vblank", 1, 5, 5, T0);
        let r =
            |id: &str, right: usize, t: u64| quiz(id, Purpose::Review, "vblank", 1, 2, right, t);
        let key = ("vblank".to_string(), 1);
        let p = derive(
            &[],
            &[proof.clone(), r("r1", 2, T0 + DAY)],
            &[],
            T0 + DAY,
            0,
        );
        assert_eq!(p.proofs[&key].r#box, 1);
        assert_eq!(p.proofs[&key].due, T0 + DAY + 3 * DAY);
        assert!(has(&p, "came_back"));
        let p = derive(
            &[],
            &[
                proof.clone(),
                r("r1", 2, T0 + DAY),
                r("r2", 1, T0 + 4 * DAY),
            ],
            &[],
            T0,
            0,
        );
        assert_eq!(p.proofs[&key].r#box, 1, "one of two keeps its box");
        let p = derive(
            &[],
            &[
                proof.clone(),
                r("r1", 2, T0 + DAY),
                r("r2", 0, T0 + 4 * DAY),
            ],
            &[],
            T0,
            0,
        );
        assert_eq!(p.proofs[&key].r#box, 0, "none moves it back");
        assert_eq!(p.proofs.len(), 1, "but the proof stays");
        let mut all = vec![proof];
        for k in 0..5 {
            all.push(r(&format!("r{k}"), 2, T0 + (k + 1) * 70 * DAY));
        }
        let p = derive(&[], &all, &[], T0, 0);
        assert_eq!(p.proofs[&key].r#box, 4);
        assert!(has(&p, "long_memory"));
        // Due: only the highest proven level, once its day comes.
        let p = derive(
            &[],
            &[
                quiz("a", Purpose::Prove, "vblank", 1, 5, 5, T0),
                quiz("b", Purpose::Prove, "vblank", 2, 5, 5, T0),
            ],
            &[],
            T0,
            0,
        );
        assert!(p.due(T0).is_empty());
        let due = p.due(T0 + DAY);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].level, 2);
    }

    #[test]
    fn lessons_guesses_and_milestones_earn_once() {
        let lessons = [
            lesson("l1", &[("sprites", 2), ("oam", 1)], T0),
            lesson("l2", &[("sprites", 2)], T0 + 5),
        ];
        let guess = |step: u32, right: Option<bool>| Fact::PredictAnswered {
            lesson: "l1".into(),
            step,
            guess: "vblank".into(),
            right,
            when: T0 + 1,
        };
        let m = Fact::Milestone {
            rom: "abc".into(),
            rom_title: "TEST".into(),
            id: "nmi_named".into(),
            when: T0 + 2,
        };
        let journal = [
            guess(0, Some(true)),
            guess(0, Some(true)),
            guess(1, None),
            m.clone(),
            m,
        ];
        let p = derive(&lessons, &[], &journal, T0 + 10, 0);
        let points = |why: &str| {
            p.ledger
                .iter()
                .filter(|l| l.why.contains(why))
                .map(|l| l.points)
                .collect::<Vec<_>>()
        };
        // The first taught three concept-levels; the second nothing new.
        assert_eq!(points("Lesson finished"), vec![20 + 5 * 3, 20]);
        assert_eq!(
            points("predict"),
            vec![5, 3],
            "each step's first guess only"
        );
        assert_eq!(points("Every frame"), vec![20], "a milestone once per game");
        assert_eq!(points("The day's first"), vec![5], "one day");
        assert!(has(&p, "first_lesson") && has(&p, "nmi_named"));
        let nmi = p.unlocked.iter().find(|u| u.id == "nmi_named").unwrap();
        assert_eq!(nmi.rom_title.as_deref(), Some("TEST"));
        // Deleting a lesson takes only its points.
        let fewer = derive(&lessons[..1], &[], &journal, T0 + 10, 0);
        assert_eq!(p.xp - fewer.xp, 20);
        let before = derive(&[], &[], &[], T0, 0);
        let g = p.diff(&before);
        assert_eq!(g.xp, p.xp);
        assert!(g.unlocked.iter().any(|u| u.id == "first_lesson"));
    }

    #[test]
    fn streaks_count_local_days_and_bridge_one() {
        let at = |day: u64, hour: u64| T0 + day * DAY + hour * 3600;
        let guesses = |times: &[u64]| -> Vec<Fact> {
            times
                .iter()
                .enumerate()
                .map(|(k, t)| Fact::PredictAnswered {
                    lesson: format!("l{k}"),
                    step: 0,
                    guess: "x".into(),
                    right: None,
                    when: *t,
                })
                .collect()
        };
        // 20:00 and 23:00 UTC are the next morning at UTC+10.
        let j = guesses(&[at(0, 1), at(0, 20), at(1, 23)]);
        let utc = derive(&[], &[], &j, at(1, 23), 0);
        let east = derive(&[], &[], &j, at(1, 23), 10 * 3600);
        assert_eq!(utc.streak.current, 2);
        assert_eq!(
            east.streak.current, 3,
            "at UTC+10 the evening guesses fall a day later"
        );
        // Seven days with one missed: bridged. Two missed: broken.
        let week = guesses(&[
            at(0, 1),
            at(1, 1),
            at(2, 1),
            at(4, 1),
            at(5, 1),
            at(6, 1),
            at(7, 1),
        ]);
        let p = derive(&[], &[], &week, at(7, 2), 0);
        assert_eq!(p.streak.current, 7);
        assert!(has(&p, "streak_7") && has(&p, "streak_3"));
        assert!(p.ledger.iter().any(|l| l.points == xp::STREAK_7));
        let broken = guesses(&[at(0, 1), at(1, 1), at(4, 1)]);
        let p = derive(&[], &[], &broken, at(4, 2), 0);
        assert_eq!((p.streak.current, p.streak.best), (1, 2));
        // Nothing today yet, but yesterday counted: still going.
        let p = derive(&[], &[], &guesses(&[at(0, 1), at(1, 1)]), at(2, 1), 0);
        assert_eq!(p.streak.current, 2);
        assert!(!p.streak.today);
    }

    #[test]
    fn ranks_follow_what_is_proven() {
        let machine: Vec<Quiz> = CONCEPTS
            .iter()
            .filter(|c| c.group == Group::Machine)
            .enumerate()
            .map(|(k, c)| {
                quiz(
                    &format!("m{k}"),
                    Purpose::Prove,
                    c.id,
                    1,
                    5,
                    5,
                    T0 + k as u64,
                )
            })
            .collect();
        let p = derive(&[], &machine[..1], &[], T0, 0);
        assert_eq!(p.rank().name, "Reset");
        let p = derive(&[], &machine, &[], T0, 0);
        assert_eq!(p.rank().name, "First frame");
        assert_eq!(p.next_rank().unwrap().name, "In vblank");
        assert_eq!(p.corpus, (8, 230));
        let all: Vec<Quiz> = CONCEPTS
            .iter()
            .enumerate()
            .flat_map(|(k, c)| {
                (1..=5).map(move |l| quiz(&format!("a{k}-{l}"), Purpose::Prove, c.id, l, 5, 5, T0))
            })
            .collect();
        let p = derive(&[], &all, &[], T0, 0);
        assert_eq!(p.rank().name, "Hardware sage");
        assert_eq!(p.corpus, (230, 230));
        for g in Group::ALL {
            assert!(has(&p, group_achievement(g)));
        }
        assert!(has(&p, "one_of_each") && has(&p, "bytes_and_cycles"));
    }

    #[test]
    fn every_achievement_can_be_earned() {
        let at = |day: u64| T0 + day * DAY;
        let lessons: Vec<Lesson> = (0..5)
            .map(|k| lesson(&format!("l{k}"), &[("cpu", 4)], at(k)))
            .collect();
        let mut quizzes: Vec<Quiz> = CONCEPTS
            .iter()
            .enumerate()
            .flat_map(|(k, c)| {
                (1..=5)
                    .map(move |l| quiz(&format!("p{k}-{l}"), Purpose::Prove, c.id, l, 5, 5, at(1)))
            })
            .collect();
        for k in 0..5 {
            quizzes.push(quiz(
                &format!("r{k}"),
                Purpose::Review,
                "cpu",
                5,
                2,
                2,
                at(2 + k * 70),
            ));
        }
        let mut journal: Vec<Fact> = (0..10)
            .map(|k| Fact::PredictAnswered {
                lesson: "l0".into(),
                step: k,
                guess: "x".into(),
                right: None,
                when: at(0),
            })
            .collect();
        journal.extend((0..31).map(|d| Fact::PredictAnswered {
            lesson: "daily".into(),
            step: 100 + d as u32,
            guess: "x".into(),
            right: None,
            when: at(1000 + d),
        }));
        let p = derive(&lessons, &quizzes, &journal, at(1031), 0);
        for a in ACHIEVEMENTS {
            assert!(has(&p, a.id), "{} was not earned", a.id);
        }
    }

    #[test]
    fn the_journal_keeps_facts_and_skips_a_torn_line() {
        let dir = std::env::temp_dir().join(format!("romlens-journal-{}", std::process::id()));
        let j = Journal::new(&dir);
        assert!(j.read().is_empty(), "no journal is an empty one");
        let a = Fact::PredictAnswered {
            lesson: "l1-00000".into(),
            step: 2,
            guess: "in vblank".into(),
            right: None,
            when: 10,
        };
        let b = Fact::Milestone {
            rom: "abc".into(),
            rom_title: "TEST".into(),
            id: "nmi_named".into(),
            when: 20,
        };
        j.append(&a).unwrap();
        j.append(&b).unwrap();
        // A crash in the middle of a write.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("Learner/journal.jsonl"))
            .unwrap();
        f.write_all(br#"{"kind":"milestone","rom":"ab"#).unwrap();
        assert_eq!(j.read(), vec![a, b]);
        j.reset().unwrap();
        assert!(j.read().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
