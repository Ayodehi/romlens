//! Checking a claim against Romlens's own data (docs/28), and a question
//! against its claim: the answer given must be the claim's, and no wrong
//! choice may pass it.

use romlens_core::cpu65816::Mnemonic;
use romlens_core::cpu65816::opcodes::OPCODES;
use romlens_core::explain::fields::{self, Field, Layout};
use romlens_core::explain::{glossary, sound};
use romlens_core::model::hardware::hardware_register_named;
use romlens_tutor::lesson::{CONCEPTS, concept};
use romlens_tutor::quiz::{Ask, Claim, Question, parse_number};

use super::World;
use super::facts;

/// A register's layout by name: a hardware register (or a pair such as
/// VMADD) or an S-DSP register, with its width in bits and its address as
/// written.
pub struct Found {
    pub name: String,
    pub layout: Layout,
    pub width: u8,
    pub address: String,
    pub dsp: bool,
}

pub fn register(name: &str) -> Option<Found> {
    if let Some((name, layout)) = fields::layout_named(name) {
        let pair = layout.pair.is_some() && layout.pair.is_some_and(|p| name.starts_with(p));
        return Some(Found {
            name,
            address: format!("${:04X}", layout.address),
            width: if pair { 16 } else { 8 },
            layout,
            dsp: false,
        });
    }
    let reg = sound::dsp_register_named(name)?;
    Some(Found {
        name: sound::dsp_register_name(reg),
        layout: sound::dsp_layout(reg),
        width: 8,
        address: format!("${reg:02X}"),
        dsp: true,
    })
}

pub fn field<'a>(f: &'a Found, name: &str) -> Option<&'a Field> {
    f.layout
        .fields
        .iter()
        .find(|x| x.name.eq_ignore_ascii_case(name.trim()))
}

/// The bits a field covers, lowest first.
pub fn field_bits(f: &Field) -> Vec<u8> {
    (f.lo..=f.hi).collect()
}

/// The same answer written two ways: case and spacing aside, or the same
/// number.
pub fn same(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    if norm(a) == norm(b) {
        return true;
    }
    matches!((parse_number(a, false), parse_number(b, false)), (Some(x), Some(y)) if x == y)
}

fn is(what: &str, got: &str, want: &str) -> Result<(), String> {
    if same(got, want) {
        Ok(())
    } else {
        Err(format!("{what} is “{got}”, not “{want}”"))
    }
}

fn field_claim(r: &str, value: u32, f: &str, expect: &str) -> Result<(), String> {
    let reg = register(r).ok_or_else(|| format!("{r} is not a register Romlens knows"))?;
    let fl = field(&reg, f).ok_or_else(|| {
        format!(
            "{} has no field “{f}”; its fields are {}",
            reg.name,
            reg.layout
                .fields
                .iter()
                .map(|x| x.name)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    is(
        &format!("${value:02X} in {}'s {}", reg.name, fl.name),
        &fl.meaning(value),
        expect,
    )
}

/// Whether a claim is true, or why not, in a line the model can act on.
pub fn check(w: &World, c: &Claim) -> Result<(), String> {
    match c {
        Claim::RegisterField {
            register,
            value,
            field,
            expect,
        } => field_claim(register, *value, field, expect),
        Claim::DspField {
            register,
            value,
            field,
            expect,
        } => field_claim(register, *value as u32, field, expect),
        Claim::RegisterBits {
            register: r,
            field: f,
        } => {
            let reg = register(r).ok_or_else(|| format!("{r} is not a register Romlens knows"))?;
            field(&reg, f)
                .map(|_| ())
                .ok_or_else(|| format!("{} has no field “{f}”", reg.name))
        }
        Claim::RegisterAddress {
            register: r,
            expect,
        } => {
            let reg = hardware_register_named(r)
                .ok_or_else(|| format!("{r} is not a hardware register"))?;
            if reg.address as u32 == expect & 0xFFFF {
                Ok(())
            } else {
                Err(format!(
                    "{} is at ${:04X}, not ${expect:X}",
                    reg.name, reg.address
                ))
            }
        }
        Claim::RegisterJob {
            register: r,
            expect,
        } => {
            let reg = hardware_register_named(r)
                .ok_or_else(|| format!("{r} is not a hardware register"))?;
            is(&format!("{} is for", reg.name), reg.description, expect)
        }
        Claim::Fact { id, expect } => {
            let f = facts::number(id).ok_or_else(|| format!("no fact {id}"))?;
            if f.answer == *expect {
                Ok(())
            } else {
                Err(format!("{} is {}, not {expect}", f.prompt, f.answer))
            }
        }
        Claim::Said { id, expect } => {
            let f = facts::said(id).ok_or_else(|| format!("no fact {id}"))?;
            is(f.prompt, f.answer, expect)
        }
        Claim::Term { term, expect } => {
            let e =
                glossary::lookup(term).ok_or_else(|| format!("{term} is not in the glossary"))?;
            is(&format!("{} stands for", e.term), &e.words, expect)
        }
        Claim::Concept { id, expect } => {
            let c = concept(id).ok_or_else(|| format!("{id} is not on the map"))?;
            if same(c.name, expect) || same(c.line, expect) {
                Ok(())
            } else {
                Err(format!("{id} is {} (“{}”)", c.name, c.line))
            }
        }
        Claim::Needs { id, expect } => {
            let c = concept(id).ok_or_else(|| format!("{id} is not on the map"))?;
            let rests = c
                .needs
                .iter()
                .filter_map(|n| concept(n))
                .any(|n| same(n.name, expect));
            if rests {
                Ok(())
            } else {
                Err(format!("{} does not rest directly on {expect}", c.name))
            }
        }
        Claim::Mnemonic { mnemonic, expect } => {
            let m = Mnemonic::parse(mnemonic)
                .ok_or_else(|| format!("{mnemonic} is not a 65816 instruction"))?;
            is(&format!("{mnemonic} does"), m.describe(), expect)
        }
        Claim::OpcodeMode { opcode, expect } => {
            let o = &OPCODES[*opcode as usize];
            is(
                &format!("${opcode:02X} ({})", o.mnemonic.as_str()),
                o.mode.describe(),
                expect,
            )
        }
        Claim::Opcode {
            mnemonic,
            mode,
            expect,
        } => {
            let o = &OPCODES[*expect as usize];
            if o.mnemonic.as_str().eq_ignore_ascii_case(mnemonic) && same(o.mode.describe(), mode) {
                Ok(())
            } else {
                Err(format!(
                    "${expect:02X} is {} {}, not {mnemonic} {mode}",
                    o.mnemonic.as_str(),
                    o.mode.describe()
                ))
            }
        }
        _ => super::claims_rom::check(w, c),
    }
}

/// Whether a question stands: its concept and level are on the map, its
/// claim is true, the answer it gives is the claim's, and no wrong choice
/// passes the claim.
pub fn validate(w: &World, q: &Question) -> Result<(), String> {
    if concept(&q.concept).is_none() {
        return Err(format!(
            "{} is not on the map; the concepts are {}",
            q.concept,
            CONCEPTS.iter().map(|c| c.id).collect::<Vec<_>>().join(", ")
        ));
    }
    if !(1..=5).contains(&q.level) {
        return Err("levels are 1 to 5".into());
    }
    if q.prompt.trim().is_empty() || q.explanation.trim().is_empty() {
        return Err("a question needs its prompt and an explanation".into());
    }
    let claim = match (&q.ask, q.source.claim()) {
        (Ask::Text { .. }, None) => return Ok(()),
        (_, None) => return Err("a question Romlens marks needs a claim".into()),
        (_, Some(c)) => c,
    };
    check(w, claim)?;
    match &q.ask {
        Ask::Choice { choices, answer } => {
            if !(2..=5).contains(&choices.len()) {
                return Err("give two to five choices".into());
            }
            let right = choices
                .get(*answer)
                .ok_or("the answer is not one of the choices")?;
            let e = claim
                .expected()
                .ok_or("that claim can't answer a choice; use bits or a number")?;
            if !same(right, &e) {
                return Err(format!("the claim's answer is “{e}”, not “{right}”"));
            }
            for (i, ch) in choices.iter().enumerate() {
                if i == *answer {
                    continue;
                }
                let also = same(ch, &e)
                    || claim
                        .with_expected(ch)
                        .is_some_and(|alt| check(w, &alt).is_ok());
                if also {
                    return Err(format!("the choice “{ch}” is right too"));
                }
                if choices[..i].iter().any(|x| same(x, ch)) {
                    return Err(format!("“{ch}” is given twice"));
                }
            }
            Ok(())
        }
        Ask::Number { answer, .. } => {
            let e = claim.expected().ok_or("that claim has no number to type")?;
            match parse_number(&e, false) {
                Some(v) if v == *answer => Ok(()),
                _ => Err(format!("the claim's answer is {e}, not {answer}")),
            }
        }
        Ask::Bits { answer, .. } => {
            let Claim::RegisterBits {
                register: r,
                field: f,
            } = claim
            else {
                return Err("a bits question needs a register_bits claim".into());
            };
            let reg = register(r).ok_or("no such register")?;
            let fl = field(&reg, f).ok_or("no such field")?;
            let mut given = answer.clone();
            given.sort_unstable();
            given.dedup();
            if given == field_bits(fl) {
                Ok(())
            } else {
                Err(format!("{} of {} is bits {}", fl.name, reg.name, fl.bits()))
            }
        }
        Ask::Line { lines, answer } => {
            let Claim::InstructionAt { address, .. } = claim else {
                return Err("a line question needs an instruction_at claim".into());
            };
            let right = lines
                .get(*answer)
                .ok_or("the answer is not one of the lines")?;
            if right.0 != *address {
                return Err(format!(
                    "the claim is about ${address:06X}, not ${:06X}",
                    right.0
                ));
            }
            for (i, (a, text)) in lines.iter().enumerate() {
                if i != *answer
                    && claim
                        .with_expected(&format!("${a:06X}"))
                        .is_some_and(|alt| check(w, &alt).is_ok())
                {
                    return Err(format!("the line “{text}” is right too"));
                }
            }
            Ok(())
        }
        Ask::Text { .. } => Ok(()),
    }
}
