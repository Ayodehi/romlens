//! Lessons (docs/25): the tutor's Explain mode answers with steps revealed
//! one at a time, as deep as the student has reached. A fixed map of SNES
//! concepts, a five-level ladder from the idea to the bytes and cycles, and
//! a learner record kept across every conversation and ROM:
//!
//! ```text
//! <root>/Learner/learner.json                 the concepts the student marked known
//! <root>/Learner/lessons/<id>.json            a lesson, whole
//! <root>/Learner/lessons/<id>/pictures/<pic>  the pictures its steps show
//! ```
//!
//! How far the student has got is worked out from the lessons and the marks
//! each time, so deleting a lesson takes back what it taught.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::store::{StoreError, now, safe, write_atomic};

/// The ladder, the same for every topic.
pub const LEVELS: [&str; 5] = [
    "The idea",
    "The hardware",
    "In this game",
    "In the code",
    "The bytes and cycles",
];

/// What each level holds, for the prompt.
const LEVEL_LINES: [&str; 5] = [
    "what it is and why, with no registers, addresses or code",
    "the chips, memories and registers involved, for any SNES game",
    "where this game keeps it and what it holds, shown with the ROM and a recording",
    "the routines that do it, in C and assembly, with their addresses",
    "instruction-level detail, timing and the edge cases",
];

pub fn level_name(level: u8) -> &'static str {
    LEVELS[(level.clamp(1, 5) - 1) as usize]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Machine,
    Graphics,
    Timing,
    Cpu,
    Sound,
}

impl Group {
    pub const ALL: [Group; 5] = [
        Group::Machine,
        Group::Graphics,
        Group::Timing,
        Group::Cpu,
        Group::Sound,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Group::Machine => "The machine",
            Group::Graphics => "Graphics",
            Group::Timing => "Timing",
            Group::Cpu => "The CPU",
            Group::Sound => "Sound",
        }
    }
}

/// One idea on the map, and the ideas it rests on.
#[derive(Debug, Clone, Copy)]
pub struct Concept {
    pub id: &'static str,
    pub name: &'static str,
    pub group: Group,
    pub needs: &'static [&'static str],
    pub line: &'static str,
}

const fn c(
    id: &'static str,
    name: &'static str,
    group: Group,
    needs: &'static [&'static str],
    line: &'static str,
) -> Concept {
    Concept {
        id,
        name,
        group,
        needs,
        line,
    }
}

use Group::*;

/// The map. Ours, in the primer's words.
pub const CONCEPTS: &[Concept] = &[
    c(
        "cpu",
        "The CPU",
        Machine,
        &[],
        "the S-CPU runs the game's code and tells the other chips what to do",
    ),
    c(
        "ppu",
        "The PPU",
        Machine,
        &[],
        "the picture processor draws each frame from its own memories",
    ),
    c(
        "apu",
        "The APU",
        Machine,
        &[],
        "a separate computer that makes the sound",
    ),
    c(
        "memory_map",
        "The memory map",
        Machine,
        &["cpu"],
        "every address the CPU can reach: RAM, ROM and the chips' registers",
    ),
    c(
        "rom",
        "Cartridge ROM",
        Machine,
        &["memory_map"],
        "the game's code and data, read-only, in banks",
    ),
    c(
        "mapping",
        "LoROM and HiROM",
        Machine,
        &["rom"],
        "how the cartridge's bytes appear in the CPU's address space",
    ),
    c(
        "wram",
        "WRAM",
        Machine,
        &["memory_map"],
        "the CPU's 128 KB of working memory",
    ),
    c(
        "hw_registers",
        "Hardware registers",
        Machine,
        &["memory_map"],
        "addresses that control the PPU, DMA, the APU ports and the rest",
    ),
    c(
        "tiles",
        "Tiles",
        Graphics,
        &["ppu"],
        "8×8 pictures in bitplanes, the pieces everything on screen is built from",
    ),
    c(
        "palettes",
        "Palettes and CGRAM",
        Graphics,
        &["ppu"],
        "the colours, 15-bit, kept in the PPU's colour memory",
    ),
    c(
        "vram",
        "VRAM",
        Graphics,
        &["ppu", "tiles"],
        "the PPU's 64 KB for tiles and tilemaps",
    ),
    c(
        "backgrounds",
        "Background layers",
        Graphics,
        &["tiles", "vram"],
        "up to four layers of tiles behind and between the sprites",
    ),
    c(
        "tilemaps",
        "Tilemaps",
        Graphics,
        &["backgrounds"],
        "the grid saying which tile, palette and flip goes where on a layer",
    ),
    c(
        "bg_modes",
        "BG modes",
        Graphics,
        &["backgrounds", "palettes"],
        "how many layers there are and how many colours each has",
    ),
    c(
        "sprites",
        "Sprites",
        Graphics,
        &["ppu", "tiles"],
        "small pictures the PPU can place anywhere, over the backgrounds",
    ),
    c(
        "oam",
        "OAM",
        Graphics,
        &["sprites"],
        "the PPU's table of 128 sprites: position, tile, palette, size",
    ),
    c(
        "scrolling",
        "Scrolling",
        Graphics,
        &["backgrounds"],
        "moving a layer by changing where the PPU starts reading it",
    ),
    c(
        "mode7",
        "Mode 7",
        Graphics,
        &["bg_modes"],
        "one layer the PPU can rotate and scale",
    ),
    c(
        "color_math",
        "Colour math and windows",
        Graphics,
        &["backgrounds", "sprites"],
        "blending layers for shadows and fades, and masking parts of the screen",
    ),
    c(
        "compression",
        "Compressed data",
        Graphics,
        &["rom", "vram"],
        "graphics and maps packed in the ROM and unpacked into RAM or VRAM",
    ),
    c(
        "frame",
        "A frame",
        Timing,
        &["ppu"],
        "the PPU draws the screen line by line, 60 times a second",
    ),
    c(
        "vblank",
        "Vertical blank",
        Timing,
        &["frame"],
        "the gap between frames, when the PPU's memories may be written",
    ),
    c(
        "forced_blank",
        "Forced blank",
        Timing,
        &["vblank"],
        "turning the screen off to write VRAM at any time",
    ),
    c(
        "nmi",
        "The NMI",
        Timing,
        &["vblank", "cpu"],
        "the interrupt at the start of vertical blank, where games update the PPU",
    ),
    c(
        "irq",
        "IRQs and timers",
        Timing,
        &["frame", "cpu"],
        "interrupts at a chosen line, for effects part way down the screen",
    ),
    c(
        "dma",
        "DMA",
        Timing,
        &["cpu", "vram"],
        "the hardware copying memory to the PPU far faster than the CPU can",
    ),
    c(
        "hdma",
        "HDMA",
        Timing,
        &["dma", "frame"],
        "writing registers again on each line, for gradients and waves",
    ),
    c(
        "main_loop",
        "The main loop",
        Timing,
        &["nmi"],
        "the game's work once a frame, then a wait for the next",
    ),
    c(
        "shadows",
        "Shadow copies and buffers",
        Timing,
        &["hw_registers", "wram", "nmi"],
        "values kept in RAM and sent to the PPU in vertical blank",
    ),
    c(
        "assembly",
        "Assembly language",
        Cpu,
        &["cpu"],
        "the CPU's instructions, one line each, as the programmers wrote them",
    ),
    c(
        "cpu_registers",
        "The 65816's registers",
        Cpu,
        &["assembly"],
        "A, X, Y, the stack pointer, the direct page and the banks",
    ),
    c(
        "widths",
        "8 and 16 bits (M and X)",
        Cpu,
        &["cpu_registers"],
        "flags that make A and the index registers one byte or two",
    ),
    c(
        "addressing",
        "Addressing modes",
        Cpu,
        &["assembly", "memory_map"],
        "the ways an instruction says where its data is",
    ),
    c(
        "banks",
        "Banks and the data bank",
        Cpu,
        &["addressing"],
        "the address's top byte, and the register that supplies it",
    ),
    c(
        "stack",
        "The stack and subroutines",
        Cpu,
        &["cpu_registers"],
        "calls, returns, and saving values for later",
    ),
    c(
        "flags_branches",
        "Flags and branches",
        Cpu,
        &["cpu_registers"],
        "results set flags, and branches choose by them",
    ),
    c(
        "jump_tables",
        "Jump tables",
        Cpu,
        &["stack", "addressing"],
        "choosing a routine by number from a list of addresses",
    ),
    c(
        "reset",
        "The reset sequence",
        Cpu,
        &["cpu_registers", "forced_blank"],
        "what a game does from power-on before its first frame",
    ),
    c(
        "math_hw",
        "The multiplier and divider",
        Cpu,
        &["hw_registers"],
        "the S-CPU's arithmetic hardware",
    ),
    c(
        "joypads",
        "Reading the joypads",
        Cpu,
        &["hw_registers", "nmi"],
        "how buttons reach the game",
    ),
    c(
        "objects",
        "Game objects in RAM",
        Cpu,
        &["wram", "main_loop"],
        "tables of positions, speeds and states, one slot per object",
    ),
    c(
        "spc700",
        "The SPC700",
        Sound,
        &["apu", "assembly"],
        "the sound computer's own processor and code",
    ),
    c(
        "apu_ports",
        "The APU ports and upload",
        Sound,
        &["apu", "hw_registers"],
        "four bytes the CPU and APU talk through, and loading the sound driver",
    ),
    c(
        "dsp",
        "The DSP's voices",
        Sound,
        &["apu"],
        "eight voices mixed with pitch, volume and envelope",
    ),
    c(
        "brr",
        "BRR samples",
        Sound,
        &["dsp"],
        "the compressed sound samples the voices play",
    ),
    c(
        "sound_driver",
        "Sound drivers and songs",
        Sound,
        &["spc700", "apu_ports", "dsp"],
        "the program that plays the game's music and effects",
    ),
];

pub fn concept(id: &str) -> Option<&'static Concept> {
    CONCEPTS.iter().find(|c| c.id == id)
}

/// The ladder and the map, for the prompt.
pub fn concept_map_text() -> String {
    let mut o = String::from("## Lessons: the ladder and the concepts\n\nLevels:\n");
    for (i, (name, line)) in LEVELS.iter().zip(LEVEL_LINES).enumerate() {
        o.push_str(&format!("{}. {name}: {line}.\n", i + 1));
    }
    for g in Group::ALL {
        o.push_str(&format!("\n{}:\n", g.name()));
        for c in CONCEPTS.iter().filter(|c| c.group == g) {
            o.push_str(&format!("- `{}` {}: {}", c.id, c.name, c.line));
            if !c.needs.is_empty() {
                o.push_str(&format!(" (after {})", c.needs.join(", ")));
            }
            o.push('\n');
        }
    }
    o
}

/// Where a step points the main window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Focus {
    /// Bytes or instructions: 24-bit addresses, `end` inclusive.
    Address { start: u32, end: u32 },
    /// A routine's C, at an instruction in it.
    Routine { entry: u32, at: u32 },
    /// A recording's frame, in one of the graphics views.
    Frame { n: u64, view: String },
    /// A hardware register.
    Register { address: u32 },
}

/// The views a frame focus can name, as the Graphics tab has them.
pub const FRAME_VIEWS: [&str; 6] = ["frame", "layers", "tiles", "palette", "oam", "tilemap"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub title: String,
    #[serde(default)]
    pub predict: Option<String>,
    pub body: String,
    #[serde(default)]
    pub focus: Option<Focus>,
    #[serde(default)]
    pub picture: Option<String>,
    /// What the predict question's answer is, as a claim Romlens can check
    /// a guess against (docs/28).
    #[serde(default)]
    pub predict_answer: Option<crate::quiz::Claim>,
}

/// "Go deeper": what a next lesson would teach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub title: String,
    pub concept: String,
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lesson {
    pub id: String,
    pub title: String,
    /// The ROM's SHA-256, for a lesson that used one (levels 3 to 5).
    #[serde(default)]
    pub rom: Option<String>,
    pub created: u64,
    /// The levels it covers, first and last.
    pub levels: (u8, u8),
    /// Each concept it teaches and the level it reaches.
    pub concepts: Vec<(String, u8)>,
    #[serde(default)]
    pub builds_on: Vec<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub next: Vec<Offer>,
    /// The conversation that made it.
    #[serde(default)]
    pub conversation: String,
    /// Ended with `end_lesson`: only a finished lesson counts.
    #[serde(default)]
    pub finished: bool,
    /// Steps rewritten since, oldest first: what each said and why it
    /// changed (docs/25, "Checking lessons").
    #[serde(default)]
    pub revisions: Vec<Revision>,
    /// The background check, once it has run.
    #[serde(default)]
    pub checked: Option<Checked>,
}

/// A step as it was before it was rewritten.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    /// The step's index, from 0.
    pub step: usize,
    pub before: Step,
    pub reason: String,
    pub when: u64,
}

/// What checking a finished lesson did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checked {
    pub when: u64,
    /// Steps it rewrote.
    pub changed: u32,
    pub cost: f64,
    /// The reviewer's last line.
    #[serde(default)]
    pub note: String,
}

impl Lesson {
    pub fn new(title: &str, levels: (u8, u8), concepts: Vec<(String, u8)>) -> Lesson {
        Lesson {
            id: new_lesson_id(),
            title: title.into(),
            rom: None,
            created: now(),
            levels,
            concepts,
            builds_on: Vec::new(),
            steps: Vec::new(),
            next: Vec::new(),
            conversation: String::new(),
            finished: false,
            revisions: Vec::new(),
            checked: None,
        }
    }
}

impl Lesson {
    /// The lesson as text: for the tutor's `lesson` and for review.
    pub fn describe(&self) -> String {
        let mut o = format!(
            "{} ({}): {}\n",
            self.id,
            levels_text(self.levels),
            self.title
        );
        let concepts: Vec<String> = self
            .concepts
            .iter()
            .map(|(c, lv)| format!("{} {lv}", concept(c).map_or(c.as_str(), |c| c.name)))
            .collect();
        o.push_str(&format!("Teaches: {}\n", concepts.join(", ")));
        if !self.builds_on.is_empty() {
            o.push_str(&format!("Builds on: {}\n", self.builds_on.join(", ")));
        }
        for (i, s) in self.steps.iter().enumerate() {
            o.push_str(&format!("\n{}. {}\n", i + 1, s.title));
            if let Some(p) = &s.predict {
                o.push_str(&format!("   (asks first: {p})\n"));
            }
            for line in s.body.lines() {
                o.push_str(&format!("   {line}\n"));
            }
            if let Some(f) = &s.focus {
                o.push_str(&format!("   [focus: {}]\n", f.text()));
            }
            if let Some(p) = &s.picture {
                o.push_str(&format!("   [picture {p}]\n"));
            }
            for r in self.revisions.iter().filter(|r| r.step == i) {
                let was: String = r.before.body.chars().take(240).collect();
                o.push_str(&format!(
                    "   [revised: {}]\n   [it said: {}{}]\n",
                    r.reason,
                    was.replace('\n', " "),
                    if r.before.body.chars().count() > 240 {
                        "…"
                    } else {
                        ""
                    }
                ));
            }
        }
        if let Some(c) = &self.checked {
            o.push_str(&format!(
                "\nChecked: {} step{} rewritten, ${:.4}{}\n",
                c.changed,
                if c.changed == 1 { "" } else { "s" },
                c.cost,
                if c.note.is_empty() {
                    String::new()
                } else {
                    format!(": {}", c.note)
                }
            ));
        }
        for n in &self.next {
            o.push_str(&format!(
                "\nNext: {} ({} {}, {})",
                n.title,
                n.concept,
                n.level,
                level_name(n.level)
            ));
        }
        o
    }
}

/// `level 2, The hardware` or `levels 1 to 2, The idea to the hardware`.
pub fn levels_text((from, to): (u8, u8)) -> String {
    if from == to {
        format!("level {from}, {}", level_name(from))
    } else {
        format!(
            "levels {from} to {to}, {} to {}",
            level_name(from),
            level_name(to).to_lowercase()
        )
    }
}

fn cpu(a: u32) -> String {
    format!("${:02X}:{:04X}", a >> 16, a & 0xFFFF)
}

impl Focus {
    pub fn text(&self) -> String {
        match self {
            Focus::Address { start, end } if start == end => cpu(*start),
            Focus::Address { start, end } => format!("{} to {}", cpu(*start), cpu(*end)),
            Focus::Routine { at, .. } => format!("the C at {}", cpu(*at)),
            Focus::Frame { n, view } => format!("frame {n}, {view}"),
            Focus::Register { address } => format!("register {}", cpu(*address)),
        }
    }
}

pub fn new_lesson_id() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("l{}-{:05x}", t.as_secs(), t.subsec_nanos() & 0xFFFFF)
}

/// Whether concept ids and levels are ones the map and the ladder have.
pub fn check_concepts(concepts: &[(String, u8)]) -> Result<(), String> {
    if concepts.is_empty() {
        return Err("name at least one concept the lesson teaches".into());
    }
    for (id, level) in concepts {
        if concept(id).is_none() {
            return Err(format!("`{id}` is not on the concept map"));
        }
        if !(1..=5).contains(level) {
            return Err(format!("`{id}`: levels are 1 to 5"));
        }
    }
    Ok(())
}

/// How far the student has got with one concept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reached {
    pub level: u8,
    pub when: u64,
    /// The lesson that took it there; `None` where the student marked it.
    pub lesson: Option<String>,
    pub marked: bool,
    /// The highest level a quiz proved (docs/28), 0 for none.
    pub proven: u8,
    /// When that proof's review is due.
    pub due: Option<u64>,
}

/// What the student knows: each concept reached, and the latest lessons.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Learner {
    pub concepts: BTreeMap<String, Reached>,
    /// The latest finished lessons, newest first: id and title.
    pub recent: Vec<(String, String)>,
}

impl Learner {
    /// Worked out from the finished lessons and the concepts marked known.
    pub fn from(lessons: &[Lesson], marks: &Marks) -> Learner {
        let mut concepts: BTreeMap<String, Reached> = BTreeMap::new();
        for (id, (level, when)) in &marks.known {
            concepts.insert(
                id.clone(),
                Reached {
                    level: *level,
                    when: *when,
                    lesson: None,
                    marked: true,
                    proven: 0,
                    due: None,
                },
            );
        }
        let mut done: Vec<&Lesson> = lessons.iter().filter(|l| l.finished).collect();
        done.sort_by_key(|l| l.created);
        for l in &done {
            for (id, level) in &l.concepts {
                let higher = concepts.get(id).is_none_or(|r| *level > r.level);
                if higher {
                    concepts.insert(
                        id.clone(),
                        Reached {
                            level: *level,
                            when: l.created,
                            lesson: Some(l.id.clone()),
                            marked: false,
                            proven: 0,
                            due: None,
                        },
                    );
                }
            }
        }
        let recent = done
            .iter()
            .rev()
            .take(10)
            .map(|l| (l.id.clone(), l.title.clone()))
            .collect();
        Learner { concepts, recent }
    }

    /// Adds what quizzes proved. A level proven is a level reached, so a
    /// student who tests out has learned it.
    pub fn with_proofs(mut self, progress: &crate::progress::Progress) -> Learner {
        for (id, level) in progress.highest() {
            let proof = &progress.proofs[&(id.to_owned(), level)];
            let r = self.concepts.entry(id.to_owned()).or_insert(Reached {
                level,
                when: proof.proved,
                lesson: None,
                marked: false,
                proven: 0,
                due: None,
            });
            r.level = r.level.max(level);
            r.proven = level;
            r.due = Some(proof.due);
        }
        self
    }

    pub fn level(&self, id: &str) -> u8 {
        self.concepts.get(id).map_or(0, |r| r.level)
    }

    /// For the prompt: only what the student has reached.
    pub fn summary(&self) -> String {
        if self.concepts.is_empty() {
            return "The student has no lessons yet and has marked nothing known: start at level 1.".into();
        }
        let mut o = String::from("What the student has reached (concept, level):\n");
        for (id, r) in &self.concepts {
            let mut how = match (&r.lesson, r.marked) {
                (_, true) => " (marked known)".to_owned(),
                (Some(l), _) => format!(" (lesson {l})"),
                _ => String::new(),
            };
            if r.proven > 0 {
                how.push_str(&format!(" (proven to level {} in a quiz)", r.proven));
            }
            o.push_str(&format!("- {id} {}{how}\n", r.level));
        }
        if !self.recent.is_empty() {
            o.push_str("Latest lessons:\n");
            for (id, title) in &self.recent {
                o.push_str(&format!("- {id}: {title}\n"));
            }
        }
        o
    }
}

/// The concepts the student marked known, and the level: the only part of
/// the record kept as itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marks {
    /// Concept id: level and when.
    #[serde(default)]
    pub known: BTreeMap<String, (u8, u64)>,
}

/// The lessons and the marks, in `<root>/Learner/`.
pub struct LessonStore {
    dir: PathBuf,
}

impl LessonStore {
    pub fn new(root: &Path) -> LessonStore {
        LessonStore {
            dir: root.join("Learner"),
        }
    }

    /// The folder the student's record is in: quizzes and the journal are
    /// kept beside the lessons (docs/28).
    pub fn root(&self) -> PathBuf {
        self.dir.parent().unwrap_or(&self.dir).to_path_buf()
    }

    fn lessons_dir(&self) -> PathBuf {
        self.dir.join("lessons")
    }

    fn path(&self, id: &str) -> Result<PathBuf, StoreError> {
        if !safe(id) {
            return Err(StoreError::Bad(id.into(), "not a lesson id".into()));
        }
        Ok(self.lessons_dir().join(format!("{id}.json")))
    }

    /// Saves a lesson whole, with the pictures its steps show.
    pub fn save(&self, lesson: &Lesson, pictures: &[(String, Vec<u8>)]) -> Result<(), StoreError> {
        let path = self.path(&lesson.id)?;
        std::fs::create_dir_all(self.lessons_dir())?;
        if !pictures.is_empty() {
            let dir = self.lessons_dir().join(&lesson.id).join("pictures");
            std::fs::create_dir_all(&dir)?;
            for (id, bytes) in pictures {
                let p = dir.join(id);
                if safe(id) && !p.exists() {
                    write_atomic(&p, bytes)?;
                }
            }
        }
        write_atomic(
            &path,
            &serde_json::to_vec_pretty(lesson).expect("a lesson serialises"),
        )?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Lesson, StoreError> {
        let b = std::fs::read(self.path(id)?)?;
        serde_json::from_slice(&b).map_err(|e| StoreError::Bad(id.into(), e.to_string()))
    }

    /// Replaces step `step` (from 0) of a finished lesson, keeping what it
    /// said and why it changed in the lesson's revisions. Its focus and
    /// picture stay.
    pub fn revise(&self, id: &str, step: usize, new: Step, reason: &str) -> Result<Lesson, String> {
        let mut l = self.load(id).map_err(|e| e.to_string())?;
        if !l.finished {
            return Err(format!(
                "{id} is still being written; change it with lesson_step"
            ));
        }
        let Some(old) = l.steps.get(step).cloned() else {
            return Err(format!("{id} has steps 1 to {}", l.steps.len()));
        };
        let new = Step {
            focus: old.focus.clone(),
            picture: old.picture.clone(),
            predict_answer: old.predict_answer.clone(),
            ..new
        };
        if new == old {
            return Err("that is the step as it is".into());
        }
        l.revisions.push(Revision {
            step,
            before: old,
            reason: reason.to_owned(),
            when: now(),
        });
        l.steps[step] = new;
        self.save(&l, &[]).map_err(|e| e.to_string())?;
        Ok(l)
    }

    /// Records that the lesson has been checked.
    pub fn set_checked(&self, id: &str, checked: Checked) -> Result<(), String> {
        let mut l = self.load(id).map_err(|e| e.to_string())?;
        l.checked = Some(checked);
        self.save(&l, &[]).map_err(|e| e.to_string())
    }

    /// A picture a lesson's step shows.
    pub fn picture(&self, lesson: &str, picture: &str) -> Option<Vec<u8>> {
        if !safe(lesson) || !safe(picture) {
            return None;
        }
        std::fs::read(
            self.lessons_dir()
                .join(lesson)
                .join("pictures")
                .join(picture),
        )
        .ok()
    }

    /// Every lesson, newest first.
    pub fn list(&self) -> Vec<Lesson> {
        let mut out: Vec<Lesson> = std::fs::read_dir(self.lessons_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let id = name.strip_suffix(".json")?.to_owned();
                self.load(&id).ok()
            })
            .collect();
        out.sort_by(|a, b| b.created.cmp(&a.created).then(b.id.cmp(&a.id)));
        out
    }

    /// Deletes a lesson and its pictures; what it taught goes with it.
    pub fn delete(&self, id: &str) -> Result<(), StoreError> {
        let path = self.path(id)?;
        std::fs::remove_file(path)?;
        let _ = std::fs::remove_dir_all(self.lessons_dir().join(id));
        Ok(())
    }

    pub fn marks(&self) -> Marks {
        std::fs::read(self.dir.join("learner.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Marks a concept known at a level, or clears the mark (`None`).
    pub fn mark_known(&self, concept_id: &str, level: Option<u8>) -> Result<(), StoreError> {
        if concept(concept_id).is_none() {
            return Err(StoreError::Bad(
                concept_id.into(),
                "not on the concept map".into(),
            ));
        }
        let mut m = self.marks();
        match level {
            Some(l) => {
                m.known.insert(concept_id.into(), (l.clamp(1, 5), now()));
            }
            None => {
                m.known.remove(concept_id);
            }
        }
        std::fs::create_dir_all(&self.dir)?;
        write_atomic(
            &self.dir.join("learner.json"),
            &serde_json::to_vec_pretty(&m).expect("marks serialise"),
        )?;
        Ok(())
    }

    /// What the student knows now: the lessons, the marks and the proofs.
    pub fn learner(&self) -> Learner {
        let lessons = self.list();
        let marks = self.marks();
        Learner::from(&lessons, &marks).with_proofs(&self.progress_with(&lessons, &marks, now(), 0))
    }

    /// Points, proofs, streak and achievements now (docs/28), with days
    /// counted in the local time `offset` seconds from UTC.
    pub fn progress(&self, now: u64, offset: i32) -> crate::progress::Progress {
        self.progress_with(&self.list(), &self.marks(), now, offset)
    }

    fn progress_with(
        &self,
        lessons: &[Lesson],
        marks: &Marks,
        now: u64,
        offset: i32,
    ) -> crate::progress::Progress {
        let root = self.dir.parent().unwrap_or(&self.dir);
        let quizzes = crate::quiz::QuizStore::new(root).list();
        let journal = crate::progress::Journal::new(root).read();
        crate::progress::Progress::derive(&crate::progress::Inputs {
            lessons,
            quizzes: &quizzes,
            journal: &journal,
            marks,
            now,
            offset,
        })
    }
}

/// Concepts whose `needs` lead back to themselves, for the test.
pub fn cycles() -> Vec<&'static str> {
    fn visit(
        id: &'static str,
        path: &mut Vec<&'static str>,
        done: &mut BTreeSet<&'static str>,
        out: &mut Vec<&'static str>,
    ) {
        if done.contains(id) {
            return;
        }
        if path.contains(&id) {
            out.push(id);
            return;
        }
        path.push(id);
        if let Some(c) = concept(id) {
            for n in c.needs {
                visit(n, path, done, out);
            }
        }
        path.pop();
        done.insert(id);
    }
    let mut out = Vec::new();
    let mut done = BTreeSet::new();
    for c in CONCEPTS {
        visit(c.id, &mut Vec::new(), &mut done, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_map_holds_together() {
        let mut ids = BTreeSet::new();
        for c in CONCEPTS {
            assert!(ids.insert(c.id), "{} twice", c.id);
            for n in c.needs {
                assert!(concept(n).is_some(), "{} needs {n}, not on the map", c.id);
            }
        }
        assert!(cycles().is_empty(), "{:?}", cycles());
        assert!(CONCEPTS.len() >= 40);
        let text = concept_map_text();
        assert!(
            text.contains("- `oam` OAM: the PPU's table of 128 sprites"),
            "{text}"
        );
        assert!(text.contains("1. The idea: what it is and why"), "{text}");
    }

    #[test]
    fn concepts_are_checked() {
        assert!(check_concepts(&[("sprites".into(), 2)]).is_ok());
        assert!(check_concepts(&[("sprite".into(), 2)]).is_err());
        assert!(check_concepts(&[("sprites".into(), 6)]).is_err());
        assert!(check_concepts(&[]).is_err());
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("romlens-lessons-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_record_follows_the_finished_lessons() {
        let root = scratch("record");
        let store = LessonStore::new(&root);
        assert!(store.learner().summary().contains("start at level 1"));

        let mut first = Lesson::new(
            "What a sprite is",
            (1, 2),
            vec![("sprites".into(), 2), ("ppu".into(), 1)],
        );
        first.created = 10;
        first.steps.push(Step {
            title: "Two chips".into(),
            predict: Some("Which chip draws?".into()),
            body: "The CPU decides; the PPU draws.".into(),
            focus: Some(Focus::Frame {
                n: 3,
                view: "oam".into(),
            }),
            picture: Some("tool-ab".into()),
            predict_answer: None,
        });
        // Not finished: it counts for nothing.
        store
            .save(&first, &[("tool-ab".into(), b"PNG".to_vec())])
            .unwrap();
        assert_eq!(store.learner().level("sprites"), 0);
        first.finished = true;
        store.save(&first, &[]).unwrap();
        assert_eq!(store.load(&first.id).unwrap(), first);
        assert_eq!(
            store.picture(&first.id, "tool-ab").as_deref(),
            Some(&b"PNG"[..])
        );

        let mut deeper = Lesson::new("Sprites in this game", (3, 3), vec![("sprites".into(), 3)]);
        deeper.created = 20;
        deeper.finished = true;
        deeper.builds_on = vec![first.id.clone()];
        store.save(&deeper, &[]).unwrap();
        let l = store.learner();
        assert_eq!((l.level("sprites"), l.level("ppu")), (3, 1));
        assert_eq!(l.recent[0].1, "Sprites in this game");
        assert!(
            l.summary().contains("- sprites 3 (lesson"),
            "{}",
            l.summary()
        );

        // A lower level later never lowers it.
        let mut again = Lesson::new("Sprites again", (1, 1), vec![("sprites".into(), 1)]);
        again.created = 30;
        again.finished = true;
        store.save(&again, &[]).unwrap();
        assert_eq!(store.learner().level("sprites"), 3);

        // Deleting a lesson takes back what it taught.
        store.delete(&deeper.id).unwrap();
        assert_eq!(store.learner().level("sprites"), 2);

        // Marked known.
        store.mark_known("dma", Some(2)).unwrap();
        assert!(store.mark_known("dmaa", Some(2)).is_err());
        let l = store.learner();
        assert!(l.concepts["dma"].marked && l.level("dma") == 2);
        store.mark_known("dma", None).unwrap();
        assert_eq!(store.learner().level("dma"), 0);
        assert_eq!(store.list().len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_rewritten_step_keeps_what_it_said() {
        let root = scratch("revise");
        let store = LessonStore::new(&root);
        let mut l = Lesson::new("DMA", (1, 3), vec![("dma".into(), 3)]);
        l.steps.push(Step {
            title: "The cost".into(),
            predict: None,
            body: "About 8 CPU cycles a byte.".into(),
            focus: Some(Focus::Address {
                start: 0x8052,
                end: 0x806B,
            }),
            picture: Some("draw-1".into()),
            predict_answer: None,
        });
        store.save(&l, &[]).unwrap();
        let fixed = Step {
            title: "The cost".into(),
            predict: None,
            body: "8 master cycles a byte.".into(),
            focus: None,
            picture: None,
            predict_answer: None,
        };
        // Only a finished lesson is revised, and only a step it has.
        assert!(
            store
                .revise(&l.id, 0, fixed.clone(), "units")
                .unwrap_err()
                .contains("still being written")
        );
        l.finished = true;
        store.save(&l, &[]).unwrap();
        assert!(store.revise(&l.id, 3, fixed.clone(), "units").is_err());
        let r = store
            .revise(
                &l.id,
                0,
                fixed,
                "DMA takes 8 master cycles a byte, not CPU cycles",
            )
            .unwrap();
        assert_eq!(r.steps[0].body, "8 master cycles a byte.");
        // Its focus and picture stay; the old words are kept with the reason.
        assert_eq!(r.steps[0].picture.as_deref(), Some("draw-1"));
        assert!(r.steps[0].focus.is_some());
        assert_eq!(r.revisions[0].before.body, "About 8 CPU cycles a byte.");
        store
            .set_checked(
                &l.id,
                Checked {
                    when: 1,
                    changed: 1,
                    cost: 0.02,
                    note: "Fixed the units.".into(),
                },
            )
            .unwrap();
        let back = store.load(&l.id).unwrap();
        assert_eq!(
            back,
            Lesson {
                checked: back.checked.clone(),
                ..r
            }
        );
        let text = back.describe();
        assert!(
            text.contains("[revised: DMA takes 8 master cycles"),
            "{text}"
        );
        assert!(
            text.contains("[it said: About 8 CPU cycles a byte.]"),
            "{text}"
        );
        assert!(
            text.contains("Checked: 1 step rewritten, $0.0200: Fixed the units."),
            "{text}"
        );
        // A lesson saved before revisions existed still reads.
        let old =
            r#"{"id":"l1-aaaaa","title":"Old","created":1,"levels":[1,1],"concepts":[["cpu",1]]}"#;
        let parsed: Lesson = serde_json::from_str(old).unwrap();
        assert!(parsed.revisions.is_empty() && parsed.checked.is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
