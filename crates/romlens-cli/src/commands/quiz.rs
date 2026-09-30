//! `romlens tutor quiz`, `tutor quiz coverage` and `tutor progress`
//! (docs/28): Romlens's own questions on the terminal, what it can ask about
//! a game, and the student's points, rank and achievements. The tutor's
//! questions are the app's.

use std::io::BufRead;
use std::path::Path;

use anyhow::{Result, anyhow};
use romlens_ffi::quiz::{Held, coverage};
use romlens_ffi::tutor::quiz::{Quizzes, start};
use romlens_tutor::lesson::{CONCEPTS, LessonStore, concept, level_name};
use romlens_tutor::progress::{ACHIEVEMENTS, milestone};
use romlens_tutor::quiz::{Ask, Given, Purpose, bits_text, parse_number};
use romlens_tutor::store::now;

pub struct QuizArgs<'a> {
    pub rom: &'a Path,
    pub project: Option<&'a Path>,
    pub concept: Option<&'a str>,
    pub level: Option<u8>,
    pub review: bool,
    pub practice: bool,
    pub seed: Option<u64>,
    /// Answers separated by `;`, instead of asking on the terminal.
    pub answers: Option<&'a str>,
    pub dir: Option<&'a Path>,
}

fn offset() -> i32 {
    0
}

/// An answer as typed: a choice's letter or number, a number, bits
/// (`0-2`, `7`, `0,2`), a line's number, or words. Empty or `-` skips.
fn read(ask: &Ask, text: &str) -> Result<Given, String> {
    let t = text.trim();
    if t.is_empty() || t == "-" {
        return Ok(Given::Skipped);
    }
    let index = |n: usize| -> Result<usize, String> {
        let k = if let Some(c) = t
            .chars()
            .next()
            .filter(|c| c.is_ascii_alphabetic() && t.len() == 1)
        {
            (c.to_ascii_uppercase() as u8 - b'A') as usize
        } else {
            t.parse::<usize>()
                .map_err(|_| format!("“{t}” is not one of them"))?
                .wrapping_sub(1)
        };
        if k < n {
            Ok(k)
        } else {
            Err(format!("“{t}” is not one of them"))
        }
    };
    Ok(match ask {
        Ask::Choice { choices, .. } => Given::Choice(index(choices.len())?),
        Ask::Line { lines, .. } => Given::Line(index(lines.len())?),
        Ask::Number { hex, .. } => {
            Given::Number(parse_number(t, *hex).ok_or(format!("“{t}” is not a number"))?)
        }
        Ask::Bits { .. } => {
            let mut bits = Vec::new();
            for part in t.split(',') {
                let p = part
                    .trim()
                    .trim_start_matches("bits")
                    .trim_start_matches("bit")
                    .trim();
                match p.split_once(['-', '–']) {
                    Some((a, b)) => {
                        let (a, b): (u8, u8) = (
                            a.trim().parse().map_err(|_| "bits are numbers")?,
                            b.trim().parse().map_err(|_| "bits are numbers")?,
                        );
                        bits.extend(a.min(b)..=a.max(b));
                    }
                    None => bits.push(p.parse().map_err(|_| "bits are numbers")?),
                }
            }
            Given::Bits(bits)
        }
        Ask::Text { .. } => Given::Text(t.to_owned()),
    })
}

/// `romlens tutor quiz`.
pub fn quiz(a: &QuizArgs) -> Result<()> {
    let root = super::tutor::root(a.dir)?;
    let wb = super::tutor::workbench(a.rom, a.project)?;
    let store = LessonStore::new(&root);
    let quizzes = Quizzes::new(&store);
    quizzes.set_offset(offset());
    let before = quizzes.progress(&store);
    let learner = store.learner();
    let purpose = if a.review {
        Purpose::Review
    } else if a.practice {
        Purpose::Practice
    } else {
        Purpose::Prove
    };
    let id = wb.rom_identity().sha256;
    let title = wb.rom().info().title.trim().to_owned();
    let mut q = start(
        &wb,
        &before,
        &|c| learner.level(c),
        a.concept,
        a.level,
        purpose,
        &id,
        &title,
        a.seed,
    )
    .map_err(|e| anyhow!(e))?;
    let name = concept(&q.concept).map_or(q.concept.clone(), |c| c.name.to_owned());
    println!(
        "{} {name}, level {} ({}): {} questions from Romlens's tables.",
        match purpose {
            Purpose::Prove => "Proving",
            Purpose::Review => "Reviewing",
            Purpose::Practice => "Practising",
        },
        q.level,
        level_name(q.level),
        q.questions.len()
    );
    let mut given = a.answers.map(|s| {
        s.split(';')
            .map(str::to_owned)
            .collect::<Vec<_>>()
            .into_iter()
    });
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let questions = q.questions.clone();
    for (k, question) in questions.iter().enumerate() {
        println!("\n{}. {}", k + 1, question.prompt);
        match &question.ask {
            Ask::Choice { choices, .. } => {
                for (i, c) in choices.iter().enumerate() {
                    println!("   {}) {c}", (b'A' + i as u8) as char);
                }
            }
            Ask::Line { lines: ls, .. } => {
                for (i, (addr, t)) in ls.iter().enumerate() {
                    println!(
                        "   {}) ${:02X}:{:04X}  {t}",
                        i + 1,
                        addr >> 16,
                        addr & 0xFFFF
                    );
                }
            }
            Ask::Number { hex: true, .. } => println!("   (a hex number)"),
            Ask::Number { .. } => println!("   (a number)"),
            Ask::Bits { register, .. } => println!("   (bits of {register}, such as 0-3 or 7)"),
            Ask::Text { .. } => println!("   (a sentence)"),
        }
        let g = loop {
            let text = match &mut given {
                Some(it) => it.next().unwrap_or_default(),
                None => {
                    print!("> ");
                    std::io::Write::flush(&mut std::io::stdout())?;
                    match lines.next() {
                        Some(l) => l?,
                        None => String::new(),
                    }
                }
            };
            if text.trim() == "?" && given.is_none() {
                match q.hint(&question.id) {
                    Ok(h) => println!("   Hint: {h}"),
                    Err(e) => println!("   {e}"),
                }
                continue;
            }
            match read(&question.ask, &text) {
                Ok(g) => break g,
                Err(e) if given.is_none() => println!("   {e}"),
                Err(_) => break Given::Skipped,
            }
        };
        let credit = q.answer(&question.id, g).map_err(|e| anyhow!(e))?.credit;
        match credit {
            Some(c) if c >= 1.0 => println!("   Right."),
            Some(c) if c > 0.0 => println!("   Right, with the hint."),
            Some(_) => println!(
                "   Not quite: {}.",
                answer(&question.ask, &question.answer_text())
            ),
            None => println!("   (A written answer: the app's tutor marks those.)"),
        }
        println!("   {}", question.explanation);
    }
    q.finished = Some(now());
    quizzes.store.save(&q).map_err(|e| anyhow!(e.to_string()))?;
    let o = q.outcome();
    let after = quizzes.progress(&store);
    let gained = after.diff(&before);
    println!(
        "\n{} of {} ({} certain right).",
        o.score, o.questions, o.certain_right
    );
    for (c, l) in &gained.proofs {
        let name = concept(c).map_or(c.clone(), |x| x.name.to_owned());
        println!("Proven: {name}, level {l}. It comes back for review tomorrow.");
    }
    if purpose == Purpose::Prove
        && gained.proofs.is_empty()
        && !after.proofs.contains_key(&(q.concept.clone(), q.level))
    {
        println!(
            "Not proven yet: it takes 4 of 5, with 3 of Romlens's questions right without a hint."
        );
    }
    println!("+{} points.", gained.xp);
    Ok(())
}

fn answer(ask: &Ask, text: &str) -> String {
    match ask {
        Ask::Bits { answer, .. } => format!("it is {}", bits_text(answer)),
        _ => format!("the answer is {text}"),
    }
}

/// `romlens tutor quiz coverage`.
pub fn quiz_coverage(rom: &Path, project: Option<&Path>) -> Result<()> {
    let wb = super::tutor::workbench(rom, project)?;
    let held = Held::of(&wb);
    let rows = coverage(&held.world());
    println!("Questions Romlens can ask in this game, by level (a proof takes 5):");
    println!(
        "{:<16} {:>3} {:>3} {:>3} {:>3} {:>3}",
        "concept", "1", "2", "3", "4", "5"
    );
    for (c, row) in &rows {
        println!(
            "{:<16} {:>3} {:>3} {:>3} {:>3} {:>3}",
            c, row[0], row[1], row[2], row[3], row[4]
        );
    }
    let provable: usize = rows
        .iter()
        .map(|(_, r)| r.iter().filter(|n| **n >= 5).count())
        .sum();
    println!(
        "{provable} of {} concept-levels can be proven here.",
        rows.len() * 5
    );
    Ok(())
}

/// `romlens tutor progress`.
pub fn progress(dir: Option<&Path>, ledger: bool, json: bool) -> Result<()> {
    let store = LessonStore::new(&super::tutor::root(dir)?);
    let p = store.progress(now(), offset());
    let highest = p.highest();
    if json {
        let v = serde_json::json!({
            "xp": p.xp,
            "rank": p.rank().name,
            "next_rank": p.next_rank().map(|r| r.name),
            "corpus": [p.corpus.0, p.corpus.1],
            "streak": {"current": p.streak.current, "best": p.streak.best},
            "proven": highest.iter().map(|(c, l)| serde_json::json!({"concept": c, "level": l})).collect::<Vec<_>>(),
            "due": p.due(now()).iter().map(|d| serde_json::json!({"concept": d.concept, "level": d.level, "due": d.due})).collect::<Vec<_>>(),
            "unlocked": p.unlocked.iter().map(|u| serde_json::json!({"id": u.id, "when": u.when, "game": u.rom_title})).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    println!("{} points. Rank: {}.", p.xp, p.rank().name);
    if let Some(n) = p.next_rank() {
        println!("Next: {} ({}).", n.name, n.needs);
    }
    println!(
        "Proven: {} of {} levels across the map.",
        p.corpus.0, p.corpus.1
    );
    let days = |n: u32| {
        if n == 1 {
            "1 day".to_owned()
        } else {
            format!("{n} days")
        }
    };
    println!(
        "Streak: {} (best {}).",
        days(p.streak.current),
        days(p.streak.best)
    );
    let due = p.due(now());
    if !due.is_empty() {
        println!("Ready to review:");
        for d in due {
            println!(
                "- {} level {}",
                concept(&d.concept).map_or(d.concept.as_str(), |c| c.name),
                d.level
            );
        }
    }
    if !highest.is_empty() {
        println!("Proven levels:");
        for c in CONCEPTS.iter().filter(|c| highest.contains_key(c.id)) {
            println!("- {} {}", c.name, highest[c.id]);
        }
    }
    let earned: Vec<String> = p
        .unlocked
        .iter()
        .map(|u| {
            ACHIEVEMENTS
                .iter()
                .find(|a| a.id == u.id)
                .map(|a| a.title.to_owned())
                .or_else(|| {
                    milestone(u.id).map(|m| {
                        format!("{} ({})", m.title, u.rom_title.clone().unwrap_or_default())
                    })
                })
                .unwrap_or_else(|| u.id.to_owned())
        })
        .collect();
    println!(
        "Achievements: {} of {}{}",
        p.unlocked.iter().filter(|u| u.rom.is_none()).count(),
        ACHIEVEMENTS.len(),
        if earned.is_empty() {
            String::new()
        } else {
            format!(": {}", earned.join(", "))
        }
    );
    if ledger {
        println!("Points:");
        for l in &p.ledger {
            println!("  {:>4}  {}", l.points, l.why);
        }
    }
    Ok(())
}
