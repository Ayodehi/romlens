//! The generators for levels 3 to 5 (docs/28): questions about this game,
//! from its header, vectors, idioms, the values its code writes, its
//! listing and its DMA. What the analysis doesn't find, isn't asked.

use romlens_core::cpu65816::AddressingMode;
use romlens_core::explain::fields::Kind;
use romlens_core::explain::{DmaDest, IdiomKind};
use romlens_core::memory::map::MappingMode;
use romlens_tutor::lesson::{Concept, Focus};
use romlens_tutor::quiz::{Ask, Claim, Question};

use super::claims::register;
use super::claims_rom::{VBLANK_MASTER, cpu, dest_name, dma_cycles, idiom_title, insn, text};
use super::facts::{CONCEPT_MNEMONICS, CONCEPT_REGISTERS, listed};
use super::make::{bits_of, choice, first_sentence, question};
use super::{Rng, World};

/// At most this many questions from one family for one concept.
const EACH: usize = 6;

pub fn all(w: &World, rng: &mut Rng, c: &Concept, level: u8, out: &mut Vec<Question>) {
    match level {
        3 => {
            mapping(rng, w, c, out);
            vectors(w, c, out);
            idioms(rng, w, c, out);
            game_writes(rng, w, c, out);
            dma_where(rng, w, c, out);
        }
        4 => {
            listing(w, c, out);
            widths(w, c, out);
            values_in(w, c, out);
            dma_size(w, c, out);
        }
        5 => {
            bytes(w, c, out);
            dma_math(rng, w, c, out);
        }
        _ => {}
    }
}

fn at(a: u32) -> Option<Focus> {
    Some(Focus::Address { start: a, end: a })
}

fn place(q: &mut Question, a: u32) {
    q.focus = at(a);
    q.cite = vec![format!("romlens://a/{a:06X}")];
}

fn mapping(rng: &mut Rng, w: &World, c: &Concept, out: &mut Vec<Question>) {
    if !matches!(c.id, "mapping" | "rom") {
        return;
    }
    let m = w.rom.mapping();
    let names: Vec<String> = [MappingMode::LoRom, MappingMode::HiRom, MappingMode::ExHiRom]
        .iter()
        .map(|x| x.name().to_owned())
        .collect();
    let Some(ask) = choice(rng, m.name().into(), names) else {
        return;
    };
    out.push(question(
        c,
        3,
        "header",
        "Which mapping does this game use?".into(),
        ask,
        format!(
            "Its header says {}: {}",
            m.name(),
            match m {
                MappingMode::LoRom => "32 KB of ROM in the upper half of each bank.",
                _ => "whole 64 KB banks of ROM.",
            }
        ),
        Claim::Mapping {
            expect: m.name().into(),
        },
    ));
}

fn vectors(w: &World, c: &Concept, out: &mut Vec<Question>) {
    let h = w.rom.header();
    let (name, v, what) = match c.id {
        "reset" => (
            "reset",
            h.emulation.reset,
            "reset vector, where the game starts,",
        ),
        "nmi" => (
            "nmi",
            h.native.nmi,
            "NMI vector, the handler run at each vertical blank,",
        ),
        "irq" => ("irq", h.native.irq, "IRQ vector"),
        _ => return,
    };
    if v == 0 || v == 0xFFFF {
        return;
    }
    // Bank $00: the vectors point there.
    let a = v as u32;
    let mut q = question(
        c,
        3,
        "vector",
        format!(
            "Where does this game's {} vector point?",
            name.to_uppercase().replace("RESET", "reset")
        ),
        Ask::Number {
            answer: a,
            hex: true,
        },
        format!("The header's {what}, points at {}.", cpu(a)),
        Claim::AddressOf {
            what: format!("vector:{name}"),
            expect: a,
        },
    );
    place(&mut q, a);
    q.hint = Some("The vectors are the last 32 bytes of bank $00, just after the header.".into());
    out.push(q);
}

/// The concepts an idiom teaches.
fn idiom_concepts(k: IdiomKind, dest: Option<DmaDest>) -> &'static [&'static str] {
    match (k, dest) {
        (IdiomKind::Wait, _) => &["vblank", "main_loop"],
        (IdiomKind::Dma, Some(DmaDest::Vram(_))) => &["dma", "vram"],
        (IdiomKind::Dma, Some(DmaDest::Cgram(_))) => &["dma", "palettes"],
        (IdiomKind::Dma, Some(DmaDest::Oam)) => &["dma", "oam"],
        (IdiomKind::Dma, _) => &["dma"],
        (IdiomKind::Hdma, _) => &["hdma"],
        (IdiomKind::Multiply | IdiomKind::Divide, _) => &["math_hw"],
        (IdiomKind::ClearMemory | IdiomKind::BlockMove, _) => &["wram"],
        (IdiomKind::ApuHandshake, _) => &["apu_ports"],
        (IdiomKind::ApuUpload, _) => &["apu_ports", "spc700", "sound_driver"],
        (IdiomKind::Decimal, _) => &["flags_branches"],
        (IdiomKind::ShadowRegister, _) => &["shadows"],
        (IdiomKind::DataBank, _) => &["banks"],
        (IdiomKind::SharedEntry, _) => &[],
    }
}

const TITLES: &[IdiomKind] = &[
    IdiomKind::Wait,
    IdiomKind::Dma,
    IdiomKind::Hdma,
    IdiomKind::Multiply,
    IdiomKind::Divide,
    IdiomKind::ClearMemory,
    IdiomKind::BlockMove,
    IdiomKind::ApuHandshake,
    IdiomKind::Decimal,
    IdiomKind::ShadowRegister,
    IdiomKind::DataBank,
    IdiomKind::ApuUpload,
];

fn idioms(rng: &mut Rng, w: &World, c: &Concept, out: &mut Vec<Question>) {
    let mut n = 0;
    for i in w.explain.idioms() {
        let dest = i.transfers.first().map(|t| t.dest);
        if n >= EACH || !idiom_concepts(i.kind, dest).contains(&c.id) {
            continue;
        }
        let (Some(title), Some(a)) = (idiom_title(i.kind), w.rom.snes_address_for(i.first()))
        else {
            continue;
        };
        let a = a.as_u24();
        // Every idiom the code here is part of is right: none is a wrong
        // choice.
        let here: Vec<IdiomKind> = w
            .explain
            .idioms_at(i.first())
            .iter()
            .map(|x| x.kind)
            .collect();
        let mut wrong = TITLES
            .iter()
            .filter(|k| !here.contains(k))
            .filter_map(|k| idiom_title(*k))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        rng.shuffle(&mut wrong);
        let Some(ask) = choice(rng, title.into(), wrong) else {
            continue;
        };
        let mut q = question(
            c,
            3,
            "idiom",
            format!("What does the code at {} do?", cpu(a)),
            ask,
            format!("{}: {} {}", i.title, i.summary, i.why),
            Claim::IdiomAt {
                address: a,
                expect: title.into(),
            },
        );
        place(&mut q, a);
        out.push(q);
        n += 1;
    }
}

/// The game's stores of known values to the concept's registers: the
/// address, the register, the value.
fn stores(w: &World, c: &Concept) -> Vec<(u32, String, u32)> {
    let mut all = parts(w, c);
    // A 16-bit store writes two registers: ask about the first.
    all.dedup_by_key(|s| s.0);
    all
}

/// Every register part of every such store.
fn parts(w: &World, c: &Concept) -> Vec<(u32, String, u32)> {
    let regs = listed(CONCEPT_REGISTERS, c.id);
    let any = c.id == "hw_registers";
    let mut out = Vec::new();
    for e in w.explain.writes() {
        if e.indexed {
            continue;
        }
        let Some(a) = w.rom.snes_address_for(e.offset) else {
            continue;
        };
        for p in &e.write.parts {
            if let Some(v) = p.value
                && (any || regs.iter().any(|r| r.eq_ignore_ascii_case(&p.name)))
            {
                out.push((a.as_u24(), p.name.clone(), v));
            }
        }
    }
    out
}

fn game_writes(rng: &mut Rng, w: &World, c: &Concept, out: &mut Vec<Question>) {
    let mut n = 0;
    for (a, name, v) in parts(w, c) {
        let Some(reg) = register(&name) else { continue };
        for f in reg.layout.fields {
            if n >= EACH {
                return;
            }
            let wrong: Vec<String> = match f.kind {
                Kind::Flag { on, off, .. } => vec![on.into(), off.into()],
                Kind::Choice { names, .. } if names.len() >= 2 => {
                    names.iter().map(|x| x.to_string()).collect()
                }
                _ => continue,
            };
            let meaning = f.meaning(v);
            let Some(ask) = choice(rng, meaning.clone(), wrong) else {
                continue;
            };
            let mut q = question(
                c,
                3,
                "gamewrite",
                format!(
                    "At {}, this game writes {name}. What does its value make “{}”?",
                    cpu(a),
                    f.name
                ),
                ask,
                format!(
                    "The store writes ${v:02X} to {name}; {} {} {}, so {} is {meaning}. {}",
                    bits_of(f),
                    if f.hi == f.lo { "is" } else { "are" },
                    f.raw(v),
                    f.name,
                    first_sentence(reg.layout.about)
                ),
                Claim::WriteMeans {
                    address: a,
                    register: name.clone(),
                    field: f.name.into(),
                    expect: meaning,
                },
            );
            place(&mut q, a);
            q.hint = Some("Look at the value loaded just before the store.".into());
            out.push(q);
            n += 1;
        }
    }
}

/// The DMA transfers with a concept, and where they start.
fn transfers(w: &World, c: &Concept, math: bool) -> Vec<(u32, romlens_core::explain::DmaTransfer)> {
    let mut out = Vec::new();
    for i in w
        .explain
        .idioms()
        .iter()
        .filter(|i| i.kind == IdiomKind::Dma)
    {
        for t in &i.transfers {
            if !idiom_concepts(IdiomKind::Dma, Some(t.dest)).contains(&c.id)
                && !(math && c.id == "vblank" && t.bytes.is_some())
            {
                continue;
            }
            if let Some(a) = w.rom.snes_address_for(t.at) {
                out.push((a.as_u24(), *t));
            }
        }
    }
    out.truncate(EACH);
    out
}

fn dma_where(rng: &mut Rng, w: &World, c: &Concept, out: &mut Vec<Question>) {
    for (a, t) in transfers(w, c, false) {
        if t.dest == DmaDest::Unknown {
            continue;
        }
        let answer = dest_name(t.dest);
        let wrong = ["VRAM", "CGRAM", "OAM", "WRAM"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let Some(ask) = choice(rng, answer.clone(), wrong) else {
            continue;
        };
        let mut q = question(
            c,
            3,
            "dma",
            format!("The DMA this game starts at {} copies to where?", cpu(a)),
            ask,
            format!(
                "Channel {} writes {} (B-bus register set in BBAD{}): {answer}.",
                t.channel,
                if t.reverse {
                    "from the B bus"
                } else {
                    "to the B bus"
                },
                t.channel
            ),
            Claim::Dma {
                at: a,
                field: "destination".into(),
                expect: answer,
            },
        );
        place(&mut q, a);
        q.hint = Some(
            "BBADn names the B-bus register: $18 is VRAM, $22 CGRAM, $04 OAM, $80 WRAM.".into(),
        );
        out.push(q);
    }
}

fn dma_size(w: &World, c: &Concept, out: &mut Vec<Question>) {
    for (a, t) in transfers(w, c, false) {
        let mut q = question(
            c,
            4,
            "dma",
            format!("Which channel does the DMA started at {} use?", cpu(a)),
            Ask::Number {
                answer: t.channel as u32,
                hex: false,
            },
            format!(
                "Its registers are at $43{}x, and MDMAEN bit {} starts it.",
                t.channel, t.channel
            ),
            Claim::Dma {
                at: a,
                field: "channel".into(),
                expect: t.channel.to_string(),
            },
        );
        place(&mut q, a);
        out.push(q);
        if let Some(b) = t.bytes {
            let mut q = question(
                c,
                4,
                "dma",
                format!("How many bytes does the DMA started at {} copy?", cpu(a)),
                Ask::Number {
                    answer: b,
                    hex: false,
                },
                format!(
                    "DAS{} holds the count: {b} bytes (0 means 65536).",
                    t.channel
                ),
                Claim::Dma {
                    at: a,
                    field: "size".into(),
                    expect: b.to_string(),
                },
            );
            place(&mut q, a);
            q.hint = Some(format!("Look for the store to DAS{}L.", t.channel));
            out.push(q);
        }
    }
}

fn dma_math(rng: &mut Rng, w: &World, c: &Concept, out: &mut Vec<Question>) {
    for (a, t) in transfers(w, c, true) {
        let Some(b) = t.bytes else { continue };
        let cycles = dma_cycles(b);
        let mut q = question(
            c,
            5,
            "dmamath",
            format!(
                "The DMA at {} copies {b} bytes. At 8 master cycles a byte, how many master cycles is that?",
                cpu(a)
            ),
            Ask::Number {
                answer: cycles,
                hex: false,
            },
            format!(
                "{b} × 8 = {cycles} master cycles, about {} lines of 1364.",
                cycles.div_ceil(1364)
            ),
            Claim::Dma {
                at: a,
                field: "cycles".into(),
                expect: cycles.to_string(),
            },
        );
        place(&mut q, a);
        out.push(q);
        let fits = if cycles <= VBLANK_MASTER { "yes" } else { "no" };
        let Some(ask) = choice(rng, fits.into(), vec!["yes".into(), "no".into()]) else {
            continue;
        };
        let mut q = question(
            c,
            5,
            "dmamath",
            format!(
                "Does the {b}-byte DMA at {} fit in one NTSC vertical blank?",
                cpu(a)
            ),
            ask,
            format!(
                "Vertical blank is lines 225 to 261: 37 × 1364 = {VBLANK_MASTER} master cycles. The DMA takes {cycles}, so {}.",
                if fits == "yes" {
                    "it fits"
                } else {
                    "it doesn't: the game must split it or use forced blank"
                }
            ),
            Claim::Dma {
                at: a,
                field: "fits".into(),
                expect: fits.into(),
            },
        );
        place(&mut q, a);
        out.push(q);
    }
}

/// Up to seven instructions around `a`, one after another, for a listing.
fn around(w: &World, a: u32) -> Vec<(u32, String)> {
    let Some(off) = w
        .rom
        .file_offset_for(romlens_core::SnesAddress::from_u24(a))
    else {
        return Vec::new();
    };
    let recs = &w.snap.instructions;
    let Some(k) = recs.iter().position(|r| r.offset == off.0) else {
        return Vec::new();
    };
    let mut lo = k;
    while lo > 0 && k - lo < 3 && recs[lo - 1].end() == recs[lo].offset {
        lo -= 1;
    }
    let mut hi = k;
    while hi + 1 < recs.len() && hi - lo < 6 && recs[hi].end() == recs[hi + 1].offset {
        hi += 1;
    }
    recs[lo..=hi]
        .iter()
        .filter_map(|r| {
            let i = w.snap.decode_at(w.rom, r)?;
            Some((i.address.as_u24(), text(w, &i)))
        })
        .collect()
}

fn listing(w: &World, c: &Concept, out: &mut Vec<Question>) {
    let mut n = 0;
    for (a, name, _) in stores(w, c) {
        if n >= EACH {
            return;
        }
        let Some(i) = insn(w, a) else { continue };
        let lines = around(w, a);
        let Some(answer) = lines.iter().position(|l| l.0 == a) else {
            continue;
        };
        if lines.len() < 3 {
            continue;
        }
        let t = text(w, &i);
        let (m, operand) = t.split_once(' ').unwrap_or((t.as_str(), ""));
        // A register written twice here (CGDATA's two bytes) has two right
        // lines: not a question.
        let also =
            |l: &(u32, String)| l.0 != a && l.1.split_once(' ').is_some_and(|(_, o)| o == operand);
        if lines.iter().any(also) {
            continue;
        }
        let mut q = question(
            c,
            4,
            "listing",
            format!(
                "Which line of the code at {} writes {name}?",
                cpu(lines[0].0)
            ),
            Ask::Line {
                lines: lines.clone(),
                answer,
            },
            format!("`{t}` at {} stores to {name}.", cpu(a)),
            Claim::InstructionAt {
                address: a,
                mnemonic: m.into(),
                operand: Some(operand.into()),
            },
        );
        place(&mut q, a);
        out.push(q);
        n += 1;
    }
}

fn values_in(w: &World, c: &Concept, out: &mut Vec<Question>) {
    for (a, name, v) in stores(w, c).into_iter().take(EACH) {
        let mut q = question(
            c,
            4,
            "valuein",
            format!("What value does the store at {} write to {name}?", cpu(a)),
            Ask::Number {
                answer: v,
                hex: true,
            },
            format!("Romlens follows the value to the store: ${v:02X}."),
            Claim::ValueReaching {
                address: a,
                register: name.clone(),
                value: v,
            },
        );
        place(&mut q, a);
        q.hint = Some("Follow A (or the register stored) back to where it was loaded.".into());
        out.push(q);
    }
}

fn widths(w: &World, c: &Concept, out: &mut Vec<Question>) {
    if !matches!(c.id, "widths" | "cpu_registers") {
        return;
    }
    let mut n = 0;
    for r in &w.snap.instructions {
        if n >= EACH {
            return;
        }
        let Some(i) = w.snap.decode_at(w.rom, r) else {
            continue;
        };
        // An immediate operand as wide as A, or as X and Y.
        let reg = match i.mode {
            AddressingMode::ImmediateM => "a",
            AddressingMode::ImmediateX => "x",
            _ => continue,
        };
        if i.assumptions != 0 {
            continue;
        }
        let eight = if reg == "a" {
            i.flags_before.m
        } else {
            i.flags_before.x
        } || i.flags_before.e;
        let bits = if eight { 8 } else { 16 };
        let a = i.address.as_u24();
        let Some(ask) = choice(
            &mut super::Rng::new(a as u64),
            bits.to_string(),
            vec!["8".into(), "16".into()],
        ) else {
            continue;
        };
        let who = if reg == "a" { "A" } else { "X and Y" };
        let mut q = question(
            c,
            4,
            "width",
            format!(
                "At {}, `{}`: how many bits wide is {who} here?",
                cpu(a),
                text(w, &i)
            ),
            ask,
            format!(
                "The operand is {}, so {who} is {bits} bits: {} is {} there.",
                if i.len == 2 { "one byte" } else { "two bytes" },
                if reg == "a" { "M" } else { "X" },
                if eight { "set" } else { "clear" }
            ),
            Claim::Width {
                address: a,
                register: reg.into(),
                bits,
            },
        );
        place(&mut q, a);
        out.push(q);
        n += 1;
    }
}

fn bytes(w: &World, c: &Concept, out: &mut Vec<Question>) {
    let mnemonics = listed(CONCEPT_MNEMONICS, c.id);
    let picks: Vec<u32> = if !mnemonics.is_empty() || matches!(c.id, "assembly" | "addressing") {
        w.snap
            .instructions
            .iter()
            .filter_map(|r| w.snap.decode_at(w.rom, r))
            .filter(|i| {
                matches!(c.id, "assembly" | "addressing")
                    || mnemonics.contains(&i.mnemonic.as_str())
            })
            .filter(|i| i.assumptions == 0)
            .map(|i| i.address.as_u24())
            .take(EACH)
            .collect()
    } else {
        stores(w, c).into_iter().map(|s| s.0).take(EACH).collect()
    };
    for a in picks {
        let Some(i) = insn(w, a) else { continue };
        let t = text(w, &i);
        let mut q = question(
            c,
            5,
            "bytes",
            format!("How many bytes is `{t}` at {}?", cpu(a)),
            Ask::Number {
                answer: i.len as u32,
                hex: false,
            },
            format!(
                "One opcode byte and {}: {}.",
                match i.len - 1 {
                    0 => "no operand".to_owned(),
                    1 => "one byte of operand".to_owned(),
                    n => format!("{n} bytes of operand"),
                },
                i.mode.describe()
            ),
            Claim::Length {
                address: a,
                expect: i.len,
            },
        );
        place(&mut q, a);
        out.push(q);
        let mut q = question(
            c,
            5,
            "bytes",
            format!("What is the opcode byte of `{t}` at {}?", cpu(a)),
            Ask::Number {
                answer: i.opcode as u32,
                hex: true,
            },
            format!(
                "${:02X}: {} in {} mode.",
                i.opcode,
                i.mnemonic.as_str(),
                i.mode.describe()
            ),
            Claim::Opcode {
                mnemonic: i.mnemonic.as_str().into(),
                mode: i.mode.describe().into(),
                expect: i.opcode,
            },
        );
        place(&mut q, a);
        out.push(q);
    }
}
