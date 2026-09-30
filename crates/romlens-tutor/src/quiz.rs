//! Quizzes (docs/28): questions with answers Romlens checks, the student's
//! answers, and the store, in `<root>/Learner/quizzes/`.
//!
//! This crate holds the format; the FFI makes Romlens's own questions and
//! checks every claim against the core, as a lesson's format is here and
//! its mechanical checks there. What a quiz proved, and the points it
//! earned, are worked out from its answers (`progress`), never stored.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::lesson::Focus;
use crate::store::{StoreError, now, safe, write_atomic};

/// What a quiz is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// Five questions at one concept and level: passing proves it.
    Prove,
    /// Two questions for each proof that is due.
    Review,
    /// For its own sake: it proves nothing, but its answers earn points.
    Practice,
}

/// A fact Romlens can check against its own tables, the ROM or the
/// analysis. One field of each is the answer (`expected`); a wrong choice
/// is the same claim with that field changed, and must fail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Claim {
    /// Writing `value` to `register` sets `field` to `expect` (its meaning).
    RegisterField {
        register: String,
        value: u32,
        field: String,
        expect: String,
    },
    /// `field` of `register` is the bits the answer names.
    RegisterBits { register: String, field: String },
    /// `register` is at `expect` (`$2100`).
    RegisterAddress { register: String, expect: u32 },
    /// What a register is for: its description is `expect`.
    RegisterJob { register: String, expect: String },
    /// `what` (`vector:nmi`, `label:NAME`, `idiom:dma`) is at `expect`, a
    /// 24-bit address.
    AddressOf { what: String, expect: u32 },
    /// The instruction at `address` is `mnemonic` (with `operand` when
    /// given, as the listing writes it).
    InstructionAt {
        address: u32,
        mnemonic: String,
        #[serde(default)]
        operand: Option<String>,
    },
    /// The ROM holds `bytes` (at most four) at `address`.
    BytesAt { address: u32, bytes: Vec<u8> },
    /// The value the store at `address` writes to `register`.
    ValueReaching {
        address: u32,
        register: String,
        value: u32,
    },
    /// A DMA the game starts at `at`: its `field` (channel, source,
    /// destination or size) is `expect`.
    Dma {
        at: u32,
        field: String,
        expect: String,
    },
    /// Writing `value` to S-DSP register `register` sets `field` to `expect`.
    DspField {
        register: String,
        value: u8,
        field: String,
        expect: String,
    },
    /// A number from the facts table.
    Fact { id: String, expect: u32 },
    /// A statement from the facts table: its answer is `expect`.
    Said { id: String, expect: String },
    /// A glossary term stands for `expect`.
    Term { term: String, expect: String },
    /// A concept on the map is `expect`: its name, or its line.
    Concept { id: String, expect: String },
    /// A concept rests directly on the one named `expect`.
    Needs { id: String, expect: String },
    /// What an instruction does: `expect` is its description.
    Mnemonic { mnemonic: String, expect: String },
    /// Opcode byte `opcode` uses addressing mode `expect`.
    OpcodeMode { opcode: u8, expect: String },
    /// The opcode byte of `mnemonic` in addressing mode `mode` is `expect`.
    Opcode {
        mnemonic: String,
        mode: String,
        expect: u8,
    },
    /// At `address` the accumulator (`a`) or the index registers (`x`) are
    /// `bits` wide, 8 or 16.
    Width {
        address: u32,
        register: String,
        bits: u8,
    },
    /// The instruction at `address` is `expect` bytes long.
    Length { address: u32, expect: u8 },
    /// The ROM's mapping (`LoROM`, `HiROM`, `ExHiROM`) is `expect`.
    Mapping { expect: String },
    /// The routine at `address` holds an idiom of kind `expect`.
    IdiomAt { address: u32, expect: String },
}

impl Claim {
    /// The answer, as the student would give it.
    pub fn expected(&self) -> Option<String> {
        Some(match self {
            Claim::RegisterField { expect, .. }
            | Claim::RegisterJob { expect, .. }
            | Claim::Dma { expect, .. }
            | Claim::DspField { expect, .. }
            | Claim::Said { expect, .. }
            | Claim::Term { expect, .. }
            | Claim::Concept { expect, .. }
            | Claim::Needs { expect, .. }
            | Claim::Mnemonic { expect, .. }
            | Claim::OpcodeMode { expect, .. }
            | Claim::Mapping { expect }
            | Claim::IdiomAt { expect, .. } => expect.clone(),
            Claim::RegisterAddress { expect, .. } | Claim::AddressOf { expect, .. } => {
                format!("${expect:X}")
            }
            Claim::Fact { expect, .. } => expect.to_string(),
            Claim::Opcode { expect, .. } => format!("${expect:02X}"),
            Claim::Length { expect, .. } => expect.to_string(),
            Claim::Width { bits, .. } => bits.to_string(),
            Claim::ValueReaching { value, .. } => format!("${value:X}"),
            Claim::InstructionAt { address, .. } => format!("${address:06X}"),
            Claim::RegisterBits { .. } | Claim::BytesAt { .. } => return None,
        })
    }

    /// The same claim with a different answer: what a wrong choice says.
    /// None when `answer` can't be one (a number that doesn't parse).
    pub fn with_expected(&self, answer: &str) -> Option<Claim> {
        let text = || answer.to_owned();
        let number = || parse_number(answer, true);
        let mut c = self.clone();
        match &mut c {
            Claim::RegisterField { expect, .. }
            | Claim::RegisterJob { expect, .. }
            | Claim::Dma { expect, .. }
            | Claim::DspField { expect, .. }
            | Claim::Said { expect, .. }
            | Claim::Term { expect, .. }
            | Claim::Concept { expect, .. }
            | Claim::Needs { expect, .. }
            | Claim::Mnemonic { expect, .. }
            | Claim::OpcodeMode { expect, .. }
            | Claim::Mapping { expect }
            | Claim::IdiomAt { expect, .. } => *expect = text(),
            Claim::RegisterAddress { expect, .. } | Claim::AddressOf { expect, .. } => {
                *expect = number()?
            }
            Claim::Fact { expect, .. } => *expect = parse_number(answer, false)?,
            Claim::Opcode { expect, .. } => *expect = u8::try_from(number()?).ok()?,
            Claim::Length { expect, .. } => {
                *expect = parse_number(answer, false)?.try_into().ok()?
            }
            Claim::Width { bits, .. } => *bits = parse_number(answer, false)?.try_into().ok()?,
            Claim::ValueReaching { value, .. } => *value = number()?,
            Claim::InstructionAt { address, .. } => *address = number()?,
            Claim::RegisterBits { .. } | Claim::BytesAt { .. } => return None,
        }
        Some(c)
    }
}

/// How a question is answered, and its answer. The answer never reaches
/// the window with the question (`public`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ask {
    /// Pick one of two to five.
    Choice { choices: Vec<String>, answer: usize },
    /// Type a number: an address or value (`hex`) or a count.
    Number { answer: u32, hex: bool },
    /// Toggle the bits of a `width`-bit register; `answer` is the set.
    Bits {
        register: String,
        width: u8,
        answer: Vec<u8>,
    },
    /// Pick a line of a listing: each line's address and text.
    Line {
        lines: Vec<(u32, String)>,
        answer: usize,
    },
    /// Write a sentence or two, marked by the model against `rubric`.
    Text { rubric: String },
}

/// An answer given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Given {
    Choice(usize),
    Number(u32),
    Bits(Vec<u8>),
    Line(usize),
    Text(String),
    Skipped,
}

/// Where a question came from, which says how sure its answer is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// Made by Romlens from its tables or the ROM.
    Romlens { generator: String, claim: Claim },
    /// Written by the tutor, its claim checked by Romlens.
    Tutor { claim: Claim },
    /// Written by the tutor and marked by the model.
    TutorMarked,
}

impl Source {
    /// Marked by Romlens, not the model.
    pub fn certain(&self) -> bool {
        !matches!(self, Source::TutorMarked)
    }

    pub fn claim(&self) -> Option<&Claim> {
        match self {
            Source::Romlens { claim, .. } | Source::Tutor { claim } => Some(claim),
            Source::TutorMarked => None,
        }
    }

    /// What the window says a question came from.
    pub fn label(&self) -> &'static str {
        match self {
            Source::Romlens { .. } => "From Romlens's tables",
            Source::Tutor { .. } => "By the tutor, checked by Romlens",
            Source::TutorMarked => "By the tutor, marked by the model",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub concept: String,
    pub level: u8,
    pub prompt: String,
    pub ask: Ask,
    /// What the answer teaches, shown once it is given.
    pub explanation: String,
    /// Where the answer comes from: `romlens://` links or a table's name.
    #[serde(default)]
    pub cite: Vec<String>,
    #[serde(default)]
    pub hint: Option<String>,
    pub source: Source,
    #[serde(default)]
    pub focus: Option<Focus>,
}

impl Question {
    /// The right answer, in words.
    pub fn answer_text(&self) -> String {
        match &self.ask {
            Ask::Choice { choices, answer } => choices.get(*answer).cloned().unwrap_or_default(),
            Ask::Number { answer, hex: true } => format!("${answer:X}"),
            Ask::Number { answer, hex: false } => answer.to_string(),
            Ask::Bits { answer, .. } => bits_text(answer),
            Ask::Line { lines, answer } => {
                lines.get(*answer).map(|l| l.1.clone()).unwrap_or_default()
            }
            Ask::Text { rubric } => rubric.clone(),
        }
    }
}

/// `bits 0–3`, `bit 7`, or `bits 0, 2`.
pub fn bits_text(bits: &[u8]) -> String {
    let mut b = bits.to_vec();
    b.sort_unstable();
    b.dedup();
    match b.as_slice() {
        [] => "no bits".into(),
        [one] => format!("bit {one}"),
        [lo, .., hi] if b.windows(2).all(|w| w[1] == w[0] + 1) => format!("bits {lo}–{hi}"),
        _ => format!(
            "bits {}",
            b.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Whether `given` answers `ask`; None for writing, which the model marks.
pub fn grade(ask: &Ask, given: &Given) -> Option<bool> {
    Some(match (ask, given) {
        (Ask::Text { .. }, Given::Text(_)) => return None,
        (Ask::Choice { answer, .. }, Given::Choice(g)) => g == answer,
        (Ask::Number { answer, .. }, Given::Number(g)) => g == answer,
        (Ask::Bits { answer, .. }, Given::Bits(g)) => {
            let set = |v: &[u8]| {
                let mut v = v.to_vec();
                v.sort_unstable();
                v.dedup();
                v
            };
            set(answer) == set(g)
        }
        (Ask::Line { answer, .. }, Given::Line(g)) => g == answer,
        _ => false,
    })
}

/// A number as typed: `$2100`, `0x2100`, `$00:8123` or `8123` are hex when
/// `hex` (or with a prefix); otherwise decimal.
pub fn parse_number(text: &str, hex: bool) -> Option<u32> {
    let t: String = text
        .trim()
        .chars()
        .filter(|c| !matches!(c, ':' | '_' | ' ' | ','))
        .collect();
    let (digits, radix) = if let Some(h) = t.strip_prefix('$') {
        (h, 16)
    } else if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        (h, 16)
    } else if hex {
        (t.as_str(), 16)
    } else {
        (t.as_str(), 10)
    };
    if digits.is_empty() {
        return None;
    }
    u32::from_str_radix(digits, radix).ok()
}

/// One answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attempt {
    pub question: String,
    pub given: Given,
    /// 1 right, 0.5 right after the hint, 0 wrong; None while the model
    /// is marking it.
    pub credit: Option<f32>,
    #[serde(default)]
    pub hinted: bool,
    pub when: u64,
    /// The model's line on a written answer.
    #[serde(default)]
    pub feedback: Option<String>,
}

/// What a quiz came to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outcome {
    pub score: f32,
    pub questions: usize,
    /// Certain questions answered right without the hint.
    pub certain_right: usize,
    /// Every question answered, and every answer marked.
    pub done: bool,
    /// A proof's rule met: 80% or better, and three certain right.
    pub passed: bool,
}

/// At least this share of the questions right to prove a level.
pub const PASS_SHARE: f32 = 0.8;
/// And at least this many certain ones right without the hint.
pub const CERTAIN_NEEDED: usize = 3;
/// Questions Romlens asks to prove a level.
pub const PROVE_QUESTIONS: usize = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quiz {
    pub id: String,
    /// The ROM's SHA-256 and title, for a quiz about a game.
    #[serde(default)]
    pub rom: Option<String>,
    #[serde(default)]
    pub rom_title: Option<String>,
    pub concept: String,
    pub level: u8,
    pub purpose: Purpose,
    /// A review's proofs: each concept and level it asks about.
    #[serde(default)]
    pub targets: Vec<(String, u8)>,
    pub created: u64,
    pub seed: u64,
    pub questions: Vec<Question>,
    #[serde(default)]
    pub attempts: Vec<Attempt>,
    /// Questions whose hint was shown.
    #[serde(default)]
    pub hinted: Vec<String>,
    /// When the student finished it (or stopped).
    #[serde(default)]
    pub finished: Option<u64>,
    /// What the tutor's questions and marking cost.
    #[serde(default)]
    pub cost: f64,
    /// The conversation that asked for it, whose cost includes the tutor's
    /// part.
    #[serde(default)]
    pub conversation: String,
}

impl Quiz {
    pub fn question(&self, id: &str) -> Option<&Question> {
        self.questions.iter().find(|q| q.id == id)
    }

    pub fn attempt(&self, id: &str) -> Option<&Attempt> {
        self.attempts.iter().find(|a| a.question == id)
    }

    /// The first question not yet answered.
    pub fn next(&self) -> Option<&Question> {
        self.questions
            .iter()
            .find(|q| self.attempt(&q.id).is_none())
    }

    /// Records an answer and marks it; the model marks a written one later
    /// (`mark`). One answer a question.
    pub fn answer(&mut self, id: &str, given: Given) -> Result<&Attempt, String> {
        let q = self
            .question(id)
            .ok_or_else(|| format!("{id} is not a question of this quiz"))?;
        if self.attempt(id).is_some() {
            return Err("that question is answered".into());
        }
        let hinted = self.hinted.iter().any(|h| h == id);
        let credit = match given {
            Given::Skipped => Some(0.0),
            _ => grade(&q.ask, &given).map(|right| match (right, hinted) {
                (true, false) => 1.0,
                (true, true) => 0.5,
                (false, _) => 0.0,
            }),
        };
        self.attempts.push(Attempt {
            question: id.to_owned(),
            given,
            credit,
            hinted,
            when: now(),
            feedback: None,
        });
        Ok(self.attempts.last().expect("just pushed"))
    }

    /// The model's mark for a written answer: credit 0, 0.5 or 1.
    pub fn mark(&mut self, id: &str, credit: f32, feedback: &str) -> Result<(), String> {
        let hinted = self.hinted.iter().any(|h| h == id);
        let a = self
            .attempts
            .iter_mut()
            .find(|a| a.question == id)
            .ok_or_else(|| format!("{id} is not answered"))?;
        let c: f32 = if credit >= 0.75 {
            1.0
        } else if credit >= 0.25 {
            0.5
        } else {
            0.0
        };
        a.credit = Some(if hinted { c.min(0.5) } else { c });
        a.feedback = Some(feedback.to_owned());
        Ok(())
    }

    /// Shows a question's hint, which halves what it can earn.
    pub fn hint(&mut self, id: &str) -> Result<String, String> {
        let q = self
            .question(id)
            .ok_or_else(|| format!("{id} is not a question of this quiz"))?;
        let h = q.hint.clone().ok_or("this question has no hint")?;
        if self.attempt(id).is_none() && !self.hinted.iter().any(|x| x == id) {
            self.hinted.push(id.to_owned());
        }
        Ok(h)
    }

    pub fn outcome(&self) -> Outcome {
        let mut score = 0.0;
        let mut certain_right = 0;
        let mut marked = 0;
        for a in &self.attempts {
            let Some(c) = a.credit else { continue };
            marked += 1;
            score += c;
            let certain = self
                .question(&a.question)
                .is_some_and(|q| q.source.certain());
            if certain && c >= 1.0 && !a.hinted {
                certain_right += 1;
            }
        }
        let n = self.questions.len();
        let done = n > 0 && marked == n;
        let passed = self.purpose == Purpose::Prove
            && n >= PROVE_QUESTIONS
            && score >= PASS_SHARE * n as f32 - 1e-4
            && certain_right >= CERTAIN_NEEDED;
        Outcome {
            score,
            questions: n,
            certain_right,
            done,
            passed,
        }
    }
}

pub fn new_quiz_id() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("q{}-{:05x}", t.as_secs(), t.subsec_nanos() & 0xFFFFF)
}

/// A quiz's seed, from its id (FNV-1a), so the same id asks the same.
pub fn seed_for(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The quizzes, in `<root>/Learner/quizzes/`.
pub struct QuizStore {
    dir: PathBuf,
}

impl QuizStore {
    pub fn new(root: &Path) -> QuizStore {
        QuizStore {
            dir: root.join("Learner").join("quizzes"),
        }
    }

    fn path(&self, id: &str) -> Result<PathBuf, StoreError> {
        if !safe(id) {
            return Err(StoreError::Bad(id.into(), "not a quiz id".into()));
        }
        Ok(self.dir.join(format!("{id}.json")))
    }

    pub fn save(&self, quiz: &Quiz) -> Result<(), StoreError> {
        let path = self.path(&quiz.id)?;
        std::fs::create_dir_all(&self.dir)?;
        write_atomic(
            &path,
            &serde_json::to_vec_pretty(quiz).expect("a quiz serialises"),
        )?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Quiz, StoreError> {
        let b = std::fs::read(self.path(id)?)?;
        serde_json::from_slice(&b).map_err(|e| StoreError::Bad(id.into(), e.to_string()))
    }

    /// Every quiz, oldest first; a file that doesn't read is left out.
    pub fn list(&self) -> Vec<Quiz> {
        let Ok(dir) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out: Vec<Quiz> = dir
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                (p.extension()? == "json")
                    .then(|| std::fs::read(&p).ok())
                    .flatten()
                    .and_then(|b| serde_json::from_slice(&b).ok())
            })
            .collect();
        out.sort_by(|a: &Quiz, b| a.created.cmp(&b.created).then(a.id.cmp(&b.id)));
        out
    }

    /// Removes every quiz ("Reset progress").
    pub fn reset(&self) -> Result<(), StoreError> {
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(id: &str, ask: Ask, source: Source) -> Question {
        Question {
            id: id.into(),
            concept: "sprites".into(),
            level: 1,
            prompt: "?".into(),
            ask,
            explanation: "because".into(),
            cite: vec![],
            hint: Some("look".into()),
            source,
            focus: None,
        }
    }

    fn romlens() -> Source {
        Source::Romlens {
            generator: "fact".into(),
            claim: Claim::Fact {
                id: "sprites".into(),
                expect: 128,
            },
        }
    }

    fn quiz(questions: Vec<Question>) -> Quiz {
        Quiz {
            id: "q1-00000".into(),
            rom: None,
            rom_title: None,
            concept: "sprites".into(),
            level: 1,
            purpose: Purpose::Prove,
            targets: vec![],
            created: 1,
            seed: seed_for("q1-00000"),
            questions,
            attempts: vec![],
            hinted: vec![],
            finished: None,
            cost: 0.0,
            conversation: String::new(),
        }
    }

    #[test]
    fn every_kind_is_marked() {
        let choice = Ask::Choice {
            choices: vec!["a".into(), "b".into()],
            answer: 1,
        };
        assert_eq!(grade(&choice, &Given::Choice(1)), Some(true));
        assert_eq!(grade(&choice, &Given::Choice(0)), Some(false));
        let n = Ask::Number {
            answer: 0x2100,
            hex: true,
        };
        for typed in ["$2100", "0x2100", "2100", " $21_00 "] {
            let g = Given::Number(parse_number(typed, true).unwrap());
            assert_eq!(grade(&n, &g), Some(true), "{typed}");
        }
        assert_eq!(parse_number("$00:8123", false), Some(0x8123));
        assert_eq!(parse_number("128", false), Some(128));
        assert_eq!(parse_number("", true), None);
        let bits = Ask::Bits {
            register: "INIDISP".into(),
            width: 8,
            answer: vec![0, 1, 2, 3],
        };
        assert_eq!(grade(&bits, &Given::Bits(vec![3, 2, 1, 0, 0])), Some(true));
        assert_eq!(grade(&bits, &Given::Bits(vec![0, 1, 2])), Some(false));
        let line = Ask::Line {
            lines: vec![(0x8000, "SEI".into()), (0x8001, "CLC".into())],
            answer: 0,
        };
        assert_eq!(grade(&line, &Given::Line(0)), Some(true));
        let text = Ask::Text {
            rubric: "names vblank".into(),
        };
        assert_eq!(grade(&text, &Given::Text("in vblank".into())), None);
        assert_eq!(grade(&choice, &Given::Number(1)), Some(false), "wrong kind");
        assert_eq!(bits_text(&[3, 0, 1, 2]), "bits 0–3");
        assert_eq!(bits_text(&[7]), "bit 7");
        assert_eq!(bits_text(&[0, 2]), "bits 0, 2");
    }

    #[test]
    fn a_claims_answer_can_be_swapped() {
        let c = Claim::RegisterAddress {
            register: "INIDISP".into(),
            expect: 0x2100,
        };
        assert_eq!(c.expected().as_deref(), Some("$2100"));
        assert_eq!(
            c.with_expected("$2105"),
            Some(Claim::RegisterAddress {
                register: "INIDISP".into(),
                expect: 0x2105
            })
        );
        let f = Claim::Fact {
            id: "sprites".into(),
            expect: 128,
        };
        assert_eq!(
            f.with_expected("64").and_then(|c| c.expected()).as_deref(),
            Some("64")
        );
        assert_eq!(f.with_expected("many"), None);
        assert_eq!(
            Claim::RegisterBits {
                register: "x".into(),
                field: "y".into()
            }
            .expected(),
            None
        );
    }

    #[test]
    fn a_proof_needs_the_score_and_three_certain() {
        let ask = || Ask::Choice {
            choices: vec!["a".into(), "b".into()],
            answer: 0,
        };
        let mut qs: Vec<Question> = (0..3)
            .map(|i| question(&format!("r{i}"), ask(), romlens()))
            .collect();
        qs.extend((0..2).map(|i| {
            question(
                &format!("t{i}"),
                Ask::Text { rubric: "r".into() },
                Source::TutorMarked,
            )
        }));
        // Four of five, but only two certain: not proven.
        let mut q = quiz(qs.clone());
        q.answer("r0", Given::Choice(0)).unwrap();
        q.answer("r1", Given::Choice(0)).unwrap();
        q.answer("r2", Given::Choice(1)).unwrap();
        q.answer("t0", Given::Text("x".into())).unwrap();
        q.answer("t1", Given::Text("y".into())).unwrap();
        assert!(!q.outcome().done, "the model has not marked yet");
        q.mark("t0", 1.0, "good").unwrap();
        q.mark("t1", 1.0, "good").unwrap();
        let o = q.outcome();
        assert!(
            o.done && o.score == 4.0 && o.certain_right == 2 && !o.passed,
            "{o:?}"
        );
        // Three certain right, one hinted: 4.5 of 5 and passed.
        let mut q = quiz(qs);
        assert_eq!(q.hint("t0").unwrap(), "look");
        for id in ["r0", "r1", "r2"] {
            q.answer(id, Given::Choice(0)).unwrap();
        }
        q.answer("t0", Given::Text("x".into())).unwrap();
        q.answer("t1", Given::Text("y".into())).unwrap();
        q.mark("t0", 1.0, "good").unwrap();
        q.mark("t1", 1.0, "good").unwrap();
        let o = q.outcome();
        assert!(o.passed && o.score == 4.5, "{o:?}");
        assert!(
            q.answer("r0", Given::Choice(0)).is_err(),
            "one answer a question"
        );
    }

    #[test]
    fn the_hint_halves_a_right_answer() {
        let mut q = quiz(vec![question(
            "a",
            Ask::Number {
                answer: 128,
                hex: false,
            },
            romlens(),
        )]);
        q.hint("a").unwrap();
        assert_eq!(q.answer("a", Given::Number(128)).unwrap().credit, Some(0.5));
        assert_eq!(q.outcome().certain_right, 0);
    }

    #[test]
    fn quizzes_are_kept_and_old_files_load() {
        let dir = std::env::temp_dir().join(format!("romlens-quiz-{}", std::process::id()));
        let store = QuizStore::new(&dir);
        let mut q = quiz(vec![question(
            "a",
            Ask::Number {
                answer: 128,
                hex: false,
            },
            romlens(),
        )]);
        q.answer("a", Given::Number(128)).unwrap();
        store.save(&q).unwrap();
        assert_eq!(store.load(&q.id).unwrap(), q);
        assert_eq!(store.list().len(), 1);
        // A quiz written before the later fields.
        let old = r#"{"id":"q0-00000","concept":"ppu","level":1,"purpose":"practice","created":0,"seed":1,
            "questions":[{"id":"a","concept":"ppu","level":1,"prompt":"?","ask":{"kind":"number","answer":2,"hex":false},
            "explanation":"e","source":{"kind":"tutor_marked"}}]}"#;
        std::fs::write(dir.join("Learner/quizzes/q0-00000.json"), old).unwrap();
        let o = store.load("q0-00000").unwrap();
        assert!(o.attempts.is_empty() && o.hint_free());
        assert_eq!(store.list()[0].id, "q0-00000", "oldest first");
        store.reset().unwrap();
        assert!(store.list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    impl Quiz {
        fn hint_free(&self) -> bool {
            self.hinted.is_empty() && self.questions.iter().all(|q| q.hint.is_none())
        }
    }
}
