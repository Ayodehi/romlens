//! The generators: every question Romlens can ask about a concept at a
//! level. Each carries its claim and an explanation that teaches; the
//! caller checks and picks (`candidates`, `generate`).

use std::collections::BTreeSet;

use romlens_core::cpu65816::Mnemonic;
use romlens_core::cpu65816::opcodes::OPCODES;
use romlens_core::explain::fields::Kind;
use romlens_core::explain::glossary::{self, Kind as TermKind};
use romlens_core::model::hardware::hardware_register_named;
use romlens_tutor::lesson::{CONCEPTS, Concept, concept};
use romlens_tutor::quiz::{Ask, Claim, Question, Source};

use super::claims::{field_bits, register, same};
use super::facts::{
    self, CONCEPT_DSP, CONCEPT_MNEMONICS, CONCEPT_OPCODES, CONCEPT_REGISTERS, CONCEPT_TERMS, listed,
};
use super::{Rng, World};

pub fn all(w: &World, rng: &mut Rng, id: &str, level: u8) -> Vec<Question> {
    let Some(c) = concept(id) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    numbers(rng, c, level, &mut out);
    saids(rng, c, level, &mut out);
    match level {
        1 => {
            lines(rng, c, &mut out);
            needs(rng, c, &mut out);
            terms(rng, c, &mut out);
        }
        2 => {
            registers(rng, c, &mut out);
            dsp(rng, c, &mut out);
            mnemonics(rng, c, &mut out);
            opcodes(rng, c, &mut out);
        }
        _ => {}
    }
    super::make_rom::all(w, rng, c, level, &mut out);
    out
}

/// A question from Romlens, its id given later.
pub(super) fn question(
    c: &Concept,
    level: u8,
    generator: &str,
    prompt: String,
    ask: Ask,
    explanation: String,
    claim: Claim,
) -> Question {
    Question {
        id: String::new(),
        concept: c.id.to_owned(),
        level,
        prompt,
        ask,
        explanation,
        cite: Vec::new(),
        hint: None,
        source: Source::Romlens {
            generator: generator.to_owned(),
            claim,
        },
        focus: None,
    }
}

/// The right answer among up to three wrong ones, shuffled; None when no
/// wrong one is left once duplicates are gone.
pub(super) fn choice(rng: &mut Rng, answer: String, wrong: Vec<String>) -> Option<Ask> {
    let mut pool: Vec<String> = Vec::new();
    for x in wrong {
        if !same(&x, &answer) && !pool.iter().any(|p| same(p, &x)) {
            pool.push(x);
        }
    }
    rng.shuffle(&mut pool);
    pool.truncate(3);
    if pool.is_empty() {
        return None;
    }
    let at = rng.below(pool.len() + 1);
    pool.insert(at, answer);
    Some(Ask::Choice {
        choices: pool,
        answer: at,
    })
}

/// Wrong numbers near a right one.
fn near(a: u32) -> Vec<u32> {
    let step = (a / 4).max(1);
    [
        a * 2,
        a / 2,
        a + step,
        a.saturating_sub(step),
        a * 4,
        a + 1,
        a.saturating_sub(1),
    ]
    .into_iter()
    .filter(|x| *x != a)
    .collect()
}

/// `bit 7` or `bits 0–3`.
pub(super) fn bits_of(f: &romlens_core::explain::fields::Field) -> String {
    if f.hi == f.lo {
        format!("bit {}", f.lo)
    } else {
        format!("bits {}", f.bits())
    }
}

/// `bit 7 is` or `bits 0–3 are`.
fn bits_are(f: &romlens_core::explain::fields::Field) -> String {
    format!("{} {}", bits_of(f), if f.hi == f.lo { "is" } else { "are" })
}

pub(super) fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// A concept's name inside a sentence: "the NMI", not "The NMI".
pub(super) fn inside(name: &str) -> String {
    match name.strip_prefix("The ") {
        Some(rest) => format!("the {rest}"),
        None if name.starts_with(|c: char| c.is_uppercase())
            && !name.chars().nth(1).is_some_and(|c| c.is_uppercase()) =>
        {
            let mut c = name.chars();
            c.next()
                .map_or_else(String::new, |f| f.to_lowercase().chain(c).collect())
        }
        None => name.to_owned(),
    }
}

pub(super) fn first_sentence(text: &str) -> &str {
    let b = text.as_bytes();
    (0..b.len().saturating_sub(2))
        .find(|&i| b[i] == b'.' && b[i + 1] == b' ' && b[i + 2].is_ascii_uppercase())
        .map_or(text, |i| &text[..=i])
}

fn numbers(rng: &mut Rng, c: &Concept, level: u8, out: &mut Vec<Question>) {
    for f in facts::NUMBERS
        .iter()
        .filter(|f| f.concept == c.id && f.level == level)
    {
        let claim = Claim::Fact {
            id: f.id.into(),
            expect: f.answer,
        };
        // At level 1 a choice; above, typed.
        let ask = if level == 1 {
            let show = |v: u32| {
                if f.hex {
                    format!("${v:X}")
                } else {
                    v.to_string()
                }
            };
            match choice(
                rng,
                show(f.answer),
                near(f.answer).into_iter().map(show).collect(),
            ) {
                Some(a) => a,
                None => continue,
            }
        } else {
            Ask::Number {
                answer: f.answer,
                hex: f.hex,
            }
        };
        let mut q = question(
            c,
            level,
            "fact",
            f.prompt.into(),
            ask,
            f.explain.into(),
            claim,
        );
        q.cite = vec!["Romlens's table of facts".into()];
        out.push(q);
    }
}

fn saids(rng: &mut Rng, c: &Concept, level: u8, out: &mut Vec<Question>) {
    for f in facts::SAIDS
        .iter()
        .filter(|f| f.concept == c.id && f.level == level)
    {
        let Some(ask) = choice(
            rng,
            f.answer.into(),
            f.wrong.iter().map(|x| x.to_string()).collect(),
        ) else {
            continue;
        };
        let claim = Claim::Said {
            id: f.id.into(),
            expect: f.answer.into(),
        };
        out.push(question(
            c,
            level,
            "said",
            f.prompt.into(),
            ask,
            f.explain.into(),
            claim,
        ));
    }
}

fn group_mates(c: &Concept) -> Vec<&'static Concept> {
    CONCEPTS
        .iter()
        .filter(|o| o.group == c.group && o.id != c.id)
        .collect()
}

fn lines(rng: &mut Rng, c: &Concept, out: &mut Vec<Question>) {
    let mates = group_mates(c);
    if let Some(ask) = choice(
        rng,
        c.name.into(),
        mates.iter().map(|o| o.name.to_owned()).collect(),
    ) {
        out.push(question(
            c,
            1,
            "line",
            format!("Which idea is this: “{}”", c.line),
            ask,
            format!("{}: {}", c.name, c.line),
            Claim::Concept {
                id: c.id.into(),
                expect: c.name.into(),
            },
        ));
    }
    if let Some(ask) = choice(
        rng,
        c.line.into(),
        mates.iter().map(|o| o.line.to_owned()).collect(),
    ) {
        out.push(question(
            c,
            1,
            "line",
            format!("What is {} about?", inside(c.name)),
            ask,
            format!("{}: {}", c.name, c.line),
            Claim::Concept {
                id: c.id.into(),
                expect: c.line.into(),
            },
        ));
    }
}

/// Everything a concept rests on, however far down.
fn beneath(c: &Concept) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    let mut todo: Vec<&str> = c.needs.to_vec();
    while let Some(n) = todo.pop() {
        if let Some(x) = concept(n)
            && out.insert(x.id)
        {
            todo.extend(x.needs.iter().copied());
        }
    }
    out
}

fn needs(rng: &mut Rng, c: &Concept, out: &mut Vec<Question>) {
    let below = beneath(c);
    let above: Vec<String> = CONCEPTS
        .iter()
        .filter(|o| o.id != c.id && !below.contains(o.id))
        .map(|o| o.name.to_owned())
        .collect();
    for n in c.needs.iter().filter_map(|n| concept(n)) {
        let mut wrong = above.clone();
        rng.shuffle(&mut wrong);
        let Some(ask) = choice(rng, n.name.into(), wrong) else {
            continue;
        };
        out.push(question(
            c,
            1,
            "needs",
            format!("Which of these does {} rest on?", inside(c.name)),
            ask,
            format!(
                "{} rests on {}. {}: {}.",
                c.name,
                c.needs
                    .iter()
                    .filter_map(|x| concept(x))
                    .map(|x| inside(x.name))
                    .collect::<Vec<_>>()
                    .join(" and "),
                n.name,
                n.line
            ),
            Claim::Needs {
                id: c.id.into(),
                expect: n.name.into(),
            },
        ));
    }
}

fn terms(rng: &mut Rng, c: &Concept, out: &mut Vec<Question>) {
    let others: Vec<String> = glossary::entries()
        .into_iter()
        .filter(|e| e.kind == TermKind::Term)
        .map(|e| e.words)
        .collect();
    for t in listed(CONCEPT_TERMS, c.id) {
        let Some(e) = glossary::lookup(t) else {
            continue;
        };
        let mut wrong = others.clone();
        rng.shuffle(&mut wrong);
        let Some(ask) = choice(rng, e.words.clone(), wrong) else {
            continue;
        };
        let mut q = question(
            c,
            1,
            "gloss",
            format!("What is “{}”?", e.term),
            ask,
            format!("{}: {}. {}", e.term, e.words, e.about),
            Claim::Term {
                term: e.term.clone(),
                expect: e.words.clone(),
            },
        );
        q.cite = vec!["Romlens's glossary".into()];
        out.push(q);
    }
}

/// All the registers the concepts name, for wrong answers.
fn every_register() -> Vec<&'static str> {
    let mut v: Vec<&str> = CONCEPT_REGISTERS
        .iter()
        .flat_map(|(_, r)| r.iter().copied())
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

const REGISTER_HINT: &str = "The PPU's registers are at $2100–$213F, the APU's ports at $2140–$2143, the CPU's at $4016 and $4200–$421F, and DMA's at $4300–$437F.";

fn registers(rng: &mut Rng, c: &Concept, out: &mut Vec<Question>) {
    for name in listed(CONCEPT_REGISTERS, c.id) {
        let Some(hw) = hardware_register_named(name) else {
            continue;
        };
        let cite = format!("romlens://r/{:04X}", hw.address);
        let about = register(name)
            .map(|f| first_sentence(f.layout.about).to_owned())
            .unwrap_or_default();
        let explain = format!(
            "{} (${:04X}): {}. {about}",
            hw.name, hw.address, hw.description
        );
        // What it is for.
        let wrong: Vec<String> = every_register()
            .into_iter()
            .filter(|r| r != name)
            .filter_map(hardware_register_named)
            .map(|r| r.description.to_owned())
            .collect();
        let mut wrong = wrong;
        rng.shuffle(&mut wrong);
        if let Some(ask) = choice(rng, hw.description.into(), wrong) {
            let mut q = question(
                c,
                2,
                "reg",
                format!("What is {} for?", hw.name),
                ask,
                explain.clone(),
                Claim::RegisterJob {
                    register: hw.name.into(),
                    expect: hw.description.into(),
                },
            );
            q.cite = vec![cite.clone()];
            out.push(q);
        }
        // Where it is.
        let mut q = question(
            c,
            2,
            "reg",
            format!("At which address is {}?", hw.name),
            Ask::Number {
                answer: hw.address as u32,
                hex: true,
            },
            explain.clone(),
            Claim::RegisterAddress {
                register: hw.name.into(),
                expect: hw.address as u32,
            },
        );
        q.cite = vec![cite.clone()];
        q.hint = Some(REGISTER_HINT.into());
        out.push(q);
        fields(rng, c, name, &cite, out);
    }
}

fn dsp(rng: &mut Rng, c: &Concept, out: &mut Vec<Question>) {
    for name in listed(CONCEPT_DSP, c.id) {
        fields(rng, c, name, "S-DSP register table", out);
    }
}

/// A register's fields: which bits each is, and what a value sets it to.
fn fields(rng: &mut Rng, c: &Concept, name: &str, cite: &str, out: &mut Vec<Question>) {
    let Some(reg) = register(name) else { return };
    let many = reg.layout.fields.len() >= 2;
    for f in reg.layout.fields {
        let bits = field_bits(f);
        let explain = format!(
            "{} ({}): {} {}. {}",
            reg.name,
            reg.address,
            bits_are(f),
            f.name,
            first_sentence(reg.layout.about)
        );
        if many {
            let mut q = question(
                c,
                2,
                "bits",
                format!("Which bits of {} are “{}”?", reg.name, f.name),
                Ask::Bits {
                    register: reg.name.clone(),
                    width: reg.width,
                    answer: bits.clone(),
                },
                explain.clone(),
                Claim::RegisterBits {
                    register: reg.name.clone(),
                    field: f.name.into(),
                },
            );
            q.cite = vec![cite.into()];
            out.push(q);
        }
        // What a value sets it to: flags and choices only, whose meanings
        // are a short list.
        let raw = match f.kind {
            Kind::Flag { .. } => rng.below(2) as u32,
            Kind::Choice { names, .. } if names.len() >= 2 => rng.below(names.len()) as u32,
            _ => continue,
        };
        let value = raw << f.lo;
        let meaning = f.meaning(value);
        let wrong: Vec<String> = match f.kind {
            Kind::Flag { on, off, .. } => vec![on.into(), off.into()],
            Kind::Choice { names, .. } => names.iter().map(|n| n.to_string()).collect(),
            Kind::Number(_) => Vec::new(),
        };
        let Some(ask) = choice(rng, meaning.clone(), wrong) else {
            continue;
        };
        let digits = if reg.width == 16 { 4 } else { 2 };
        // A register the CPU only reads is read, not written.
        let read = !reg.dsp
            && hardware_register_named(&reg.name).is_some_and(|r| r.access.as_str() == "R");
        let (verb, prep) = if read {
            ("reads", "from")
        } else {
            ("writes", "to")
        };
        let mut q = question(
            c,
            2,
            "write",
            format!(
                "A game {verb} ${value:0digits$X} {prep} {}. What does that say about “{}”?",
                reg.name, f.name
            ),
            ask,
            format!(
                "{} of ${value:0digits$X} {} {raw}, so {} is {meaning}. {}",
                capital(&bits_of(f)),
                if f.hi == f.lo { "is" } else { "are" },
                f.name,
                first_sentence(reg.layout.about)
            ),
            if reg.dsp {
                Claim::DspField {
                    register: reg.name.clone(),
                    value: value as u8,
                    field: f.name.into(),
                    expect: meaning,
                }
            } else {
                Claim::RegisterField {
                    register: reg.name.clone(),
                    value,
                    field: f.name.into(),
                    expect: meaning,
                }
            },
        );
        q.cite = vec![cite.into()];
        out.push(q);
    }
}

const COMMON: &[&str] = &[
    "LDA", "STA", "JSR", "RTS", "BRA", "INC", "PHA", "CMP", "SEP", "TAX",
];

fn mnemonics(rng: &mut Rng, c: &Concept, out: &mut Vec<Question>) {
    let list = listed(CONCEPT_MNEMONICS, c.id);
    for m in list {
        let Some(mn) = Mnemonic::parse(m) else {
            continue;
        };
        let wrong: Vec<String> = list
            .iter()
            .chain(COMMON)
            .filter(|o| *o != m)
            .filter_map(|o| Mnemonic::parse(o))
            .map(|o| o.describe().to_owned())
            .collect();
        let Some(ask) = choice(rng, mn.describe().into(), wrong) else {
            continue;
        };
        let mut q = question(
            c,
            2,
            "op",
            format!("What does `{m}` do?"),
            ask,
            format!("`{m}`: {}.", mn.describe()),
            Claim::Mnemonic {
                mnemonic: (*m).into(),
                expect: mn.describe().into(),
            },
        );
        q.cite = vec!["65816 opcode table".into()];
        out.push(q);
    }
}

fn opcodes(rng: &mut Rng, c: &Concept, out: &mut Vec<Question>) {
    let list = listed(CONCEPT_OPCODES, c.id);
    for op in list {
        let o = &OPCODES[*op as usize];
        let wrong: Vec<String> = list
            .iter()
            .filter(|x| *x != op)
            .map(|x| OPCODES[*x as usize].mode.describe().to_owned())
            .collect();
        let Some(ask) = choice(rng, o.mode.describe().into(), wrong) else {
            continue;
        };
        let mut q = question(
            c,
            2,
            "op",
            format!(
                "Opcode ${op:02X} is `{}`. Which addressing mode does it use?",
                o.mnemonic.as_str()
            ),
            ask,
            format!(
                "${op:02X} is {} {}: {}.",
                o.mnemonic.as_str(),
                o.mode.describe(),
                o.mnemonic.describe()
            ),
            Claim::OpcodeMode {
                opcode: *op,
                expect: o.mode.describe().into(),
            },
        );
        q.cite = vec!["65816 opcode table".into()];
        out.push(q);
    }
}
