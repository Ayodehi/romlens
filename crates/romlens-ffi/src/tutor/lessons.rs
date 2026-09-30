//! The lesson tools (docs/25): the tutor builds a lesson a step at a time
//! (`begin_lesson`, `lesson_step`, `end_lesson`) and reads what the student
//! has learned (`learner`, `lesson`). Each step's focus is resolved here, so
//! the window never points at something the ROM or recording lacks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use romlens_tutor::ToolSpec;
use romlens_tutor::agent::{ToolContext, ToolOutput};
use romlens_tutor::lesson::{FRAME_VIEWS, Focus, Lesson, LessonStore, Offer, Step, check_concepts};
use serde_json::{Value, json};

use super::tools::{addr, choice, integer, nullable, object, spec, string};
use crate::RecordingSession;

/// The tools, in the order the model uses them.
pub const NAMES: [&str; 6] = [
    "learner",
    "lesson",
    "begin_lesson",
    "lesson_step",
    "end_lesson",
    "revise_lesson_step",
];

/// The ones that build a lesson, which run in order.
pub const BUILDING: [&str; 4] = [
    "begin_lesson",
    "lesson_step",
    "end_lesson",
    "revise_lesson_step",
];

pub fn specs() -> Vec<ToolSpec> {
    let concept_level = object(&[("id", string("")), ("level", integer(""))]);
    vec![
        spec(
            "learner",
            "What the student has learned: each concept and the level reached, and the latest lessons.",
            &[],
        ),
        spec(
            "lesson",
            "An earlier lesson's steps.",
            &[("id", string(""))],
        ),
        spec(
            "begin_lesson",
            "Start a lesson in Explain mode. Returns its id for the steps.",
            &[
                ("title", string("")),
                ("from_level", integer("")),
                ("to_level", integer("")),
                (
                    "concepts",
                    json!({"type": "array", "items": concept_level.clone(), "description": "Each concept it teaches, from the map, and the level it reaches."}),
                ),
                (
                    "builds_on",
                    json!({"type": "array", "items": {"type": "string"}, "description": "Earlier lessons it rests on."}),
                ),
            ],
        ),
        spec(
            "lesson_step",
            "Add the next step. `predict` is asked before the body shows. The focus moves the main window: an address (with `focus_end` for a range; `focus_in` c shows the routine's C), or a frame of the recording in a Graphics view. `picture` is an id a tool returned.",
            &[
                ("lesson", string("Its id; empty for the one being written.")),
                ("title", string("")),
                ("predict", nullable(string(""))),
                (
                    "predict_answer",
                    nullable(string(
                        "the predict question's answer as a claim Romlens can check a guess against: a JSON object, one of quiz_question's claim kinds",
                    )),
                ),
                ("body", string("")),
                ("focus_address", nullable(string(""))),
                ("focus_end", nullable(string(""))),
                ("focus_in", nullable(choice(&["listing", "c"], ""))),
                ("focus_frame", nullable(integer(""))),
                ("focus_view", nullable(choice(&FRAME_VIEWS, ""))),
                ("picture", nullable(string(""))),
            ],
        ),
        spec(
            "end_lesson",
            "Finish the lesson and record what the student learned. `next` offers what a next lesson would teach.",
            &[
                ("lesson", string("")),
                (
                    "next",
                    json!({"type": "array", "items": object(&[("title", string("")), ("concept", string("")), ("level", integer(""))]), "description": ""}),
                ),
            ],
        ),
        spec(
            "revise_lesson_step",
            "Rewrite a step of a finished lesson that is wrong or imprecise: the whole step as it should read. Its focus and picture stay.",
            &[
                (
                    "lesson",
                    string("Its id; empty for the latest this conversation finished."),
                ),
                ("step", integer("From 1.")),
                ("title", string("")),
                ("predict", nullable(string(""))),
                ("body", string("")),
                ("reason", string("What was wrong, in a sentence.")),
            ],
        ),
    ]
}

/// What Romlens can check in a lesson without a model (docs/25, "Checking
/// lessons"): each line of code a step shows as `$BB:AAAA  MNEMONIC
/// operand` against the instruction there (`insn_at` gives its mnemonic
/// and operand), and each register a step names as `$21xx NAME` against
/// the register table. One line per finding.
pub fn mechanical(l: &Lesson, insn_at: &dyn Fn(u32) -> Option<(String, String)>) -> Vec<String> {
    let mut out = Vec::new();
    let norm = |s: &str| s.to_ascii_uppercase().replace([' ', '`'], "");
    for (i, step) in l.steps.iter().enumerate() {
        let n = i + 1;
        let mut code = false;
        for line in step.body.lines() {
            let t = line.trim();
            if t.starts_with("```") {
                code = !code;
                continue;
            }
            if code && let Some((a, mn, op)) = code_line(t) {
                match insn_at(a) {
                    None => out.push(format!(
                        "step {n}: {} is not an instruction in this ROM's listing",
                        addr(a)
                    )),
                    Some((m, o)) => {
                        let hex = |s: &str| s.starts_with('#') || s.starts_with('$');
                        let differs = !m.eq_ignore_ascii_case(&mn)
                            || hex(&op) && hex(&o) && norm(&op) != norm(&o);
                        if differs {
                            out.push(format!(
                                "step {n}: shows `{mn} {op}` at {}, but the ROM has `{m} {o}` there",
                                addr(a)
                            ));
                        }
                    }
                }
            }
            for (reg, name) in registers_named(t) {
                let Some(r) = romlens_core::model::hardware_register(reg) else {
                    out.push(format!(
                        "step {n}: ${reg:04X} is not a hardware register, but is named {name}"
                    ));
                    continue;
                };
                let theirs = romlens_core::model::hardware_register_named(&name);
                if !r.name.eq_ignore_ascii_case(&name) && theirs.is_some_and(|t| t.address != reg) {
                    out.push(format!(
                        "step {n}: calls ${reg:04X} {name}, but ${reg:04X} is {} ({name} is ${:04X})",
                        r.name,
                        theirs.map_or(0, |t| t.address)
                    ));
                }
            }
        }
    }
    out
}

/// `$00:8052  LDX #$1809  ; comment`, or with the bytes after the address
/// (`$00:8052  A2 09 18  LDX #$1809`): the address, mnemonic and operand.
fn code_line(t: &str) -> Option<(u32, String, String)> {
    let t = t.split(';').next()?.trim();
    let rest = t.strip_prefix('$')?;
    let (a, rest) = rest.split_once(char::is_whitespace)?;
    let (bank, off) = a.split_once(':')?;
    let a = (u32::from_str_radix(bank, 16).ok()? << 16) | u32::from_str_radix(off, 16).ok()?;
    let mut words = rest
        .split_whitespace()
        .skip_while(|w| w.len() == 2 && u8::from_str_radix(w, 16).is_ok());
    let mn = words.next()?;
    if mn.len() != 3 || !mn.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some((
        a,
        mn.to_ascii_uppercase(),
        words.collect::<Vec<_>>().join(" "),
    ))
}

/// Registers written as `$2118 VMDATAL` (in backticks or not).
fn registers_named(t: &str) -> Vec<(u16, String)> {
    let mut out = Vec::new();
    let b = t.replace('`', " ");
    let words: Vec<&str> = b.split_whitespace().collect();
    for w in words.windows(2) {
        let Some(h) = w[0].strip_prefix('$') else {
            continue;
        };
        let h = h.trim_end_matches([',', ':', '.', ')']);
        if h.len() != 4 {
            continue;
        }
        let Ok(a) = u16::from_str_radix(h, 16) else {
            continue;
        };
        if !(0x2100..=0x21FF).contains(&a) && !(0x4200..=0x437F).contains(&a) {
            continue;
        }
        let name: String = w[1]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if name.len() >= 4
            && name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            out.push((a, name));
        }
    }
    out
}

/// Lessons being written, and the store the finished ones go to.
#[derive(Default)]
pub struct Lessons {
    store: Mutex<Option<Arc<LessonStore>>>,
    open: Mutex<HashMap<String, Lesson>>,
    /// Finished lessons whose pictures are still to be copied, once the
    /// round that made them has put them in the conversation.
    pending: Mutex<Vec<String>>,
    /// Finished lessons still to be checked, once the turn is over.
    reviews: Mutex<Vec<String>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// An address as the model wrote it: its CPU address, its file offset if in
/// ROM, and whether it is a hardware register.
pub type Resolve<'a> = &'a dyn Fn(&str) -> Result<(u32, Option<u32>, bool), String>;

impl Lessons {
    pub fn set_store(&self, store: Option<LessonStore>) {
        *lock(&self.store) = store.map(Arc::new);
    }

    pub fn store(&self) -> Option<Arc<LessonStore>> {
        lock(&self.store).clone()
    }

    /// Copies the pictures finished lessons show, now the conversation has
    /// them.
    pub fn settle(&self, pictures: &HashMap<String, Vec<u8>>) {
        let Some(store) = self.store() else { return };
        for id in std::mem::take(&mut *lock(&self.pending)) {
            let Ok(lesson) = store.load(&id) else {
                continue;
            };
            let pics: Vec<(String, Vec<u8>)> = lesson
                .steps
                .iter()
                .filter_map(|s| s.picture.as_ref())
                .filter_map(|p| pictures.get(p).map(|b| (p.clone(), b.clone())))
                .collect();
            let _ = store.save(&lesson, &pics);
        }
    }

    /// The lessons ended since last asked, to check.
    pub fn take_reviews(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.reviews))
    }

    /// A lesson being written, for the window.
    pub fn open_lesson(&self, id: &str) -> Option<Lesson> {
        lock(&self.open).get(id).cloned()
    }

    pub fn run(
        &self,
        name: &str,
        v: &Value,
        cx: &ToolContext,
        resolve: Resolve,
        rec: Option<Arc<RecordingSession>>,
        rom: &str,
    ) -> ToolOutput {
        let Some(store) = self.store() else {
            return ToolOutput::error("lessons are not kept in this session");
        };
        let r = match name {
            "learner" => Ok(store.learner().summary()),
            "lesson" => self.read(&store, v),
            "begin_lesson" => self.begin(&store, v, cx),
            "lesson_step" => self.step(v, cx, resolve, rec),
            "end_lesson" => self.end(&store, v, cx, rom),
            "revise_lesson_step" => revise(&store, v, cx),
            _ => Err(format!("no tool {name}")),
        };
        match r {
            Ok(t) => ToolOutput::text(t),
            Err(e) => ToolOutput::error(e),
        }
    }

    fn read(&self, store: &LessonStore, v: &Value) -> Result<String, String> {
        let id = v["id"].as_str().unwrap_or_default();
        let l = self
            .open_lesson(id)
            .or_else(|| store.load(id).ok())
            .ok_or_else(|| format!("no lesson {id}"))?;
        Ok(l.describe())
    }

    fn begin(&self, store: &LessonStore, v: &Value, cx: &ToolContext) -> Result<String, String> {
        let title = v["title"].as_str().unwrap_or_default().trim();
        if title.is_empty() {
            return Err("give the lesson a title".into());
        }
        let from = v["from_level"].as_u64().unwrap_or(0) as u8;
        let to = v["to_level"].as_u64().unwrap_or(0) as u8;
        if !(1..=5).contains(&from) || !(from..=5).contains(&to) {
            return Err("levels are 1 to 5, from_level no higher than to_level".into());
        }
        let concepts: Vec<(String, u8)> = v["concepts"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                (
                    c["id"].as_str().unwrap_or_default().to_owned(),
                    c["level"].as_u64().unwrap_or(0) as u8,
                )
            })
            .collect();
        check_concepts(&concepts)?;
        let mut builds_on = Vec::new();
        for b in v["builds_on"].as_array().into_iter().flatten() {
            let b = b.as_str().unwrap_or_default();
            if store.load(b).is_err() {
                return Err(format!("no earlier lesson {b}; `learner` lists them"));
            }
            builds_on.push(b.to_owned());
        }
        let mut l = Lesson::new(title, (from, to), concepts);
        l.builds_on = builds_on;
        l.conversation = cx.conversation.to_owned();
        let id = l.id.clone();
        lock(&self.open).insert(id.clone(), l);
        Ok(format!(
            "Lesson {id} begun ({}). Add its steps with lesson_step, lesson \"{id}\", then end_lesson.",
            romlens_tutor::lesson::levels_text((from, to))
        ))
    }

    /// The lesson a call names, or with none named, the one this
    /// conversation has open.
    fn which(&self, v: &Value, cx: &ToolContext) -> Result<String, String> {
        let named = v["lesson"].as_str().unwrap_or_default().trim();
        let open = lock(&self.open);
        if open.contains_key(named) {
            return Ok(named.to_owned());
        }
        let mine: Vec<&String> = open
            .iter()
            .filter(|(_, l)| l.conversation == cx.conversation)
            .map(|(id, _)| id)
            .collect();
        match (named.is_empty(), mine.as_slice()) {
            (true, [one]) => Ok((*one).clone()),
            (true, []) | (false, _) if mine.is_empty() => {
                Err("no lesson is being written; begin_lesson first".into())
            }
            _ => Err(format!(
                "no lesson {named} is being written; the open ones are {}",
                mine.iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    fn step(
        &self,
        v: &Value,
        cx: &ToolContext,
        resolve: Resolve,
        rec: Option<Arc<RecordingSession>>,
    ) -> Result<String, String> {
        let id = &self.which(v, cx)?;
        let title = v["title"].as_str().unwrap_or_default().trim().to_owned();
        let body = v["body"].as_str().unwrap_or_default().trim().to_owned();
        if title.is_empty() || body.is_empty() {
            return Err("a step needs a title and a body".into());
        }
        let focus = focus(v, resolve, rec)?;
        let picture = v["picture"]
            .as_str()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned);
        if let Some(p) = &picture
            && !["tool-", "draw-", "svg-", "shot-"]
                .iter()
                .any(|k| p.starts_with(k))
        {
            return Err(format!("{p} is not a picture a tool returned"));
        }
        let predict = v["predict"]
            .as_str()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned);
        // Checked against the ROM by `RomTools` before the step comes here.
        let predict_answer = super::quiz::read_claim(&v["predict_answer"])?;
        if predict_answer.is_some() && predict.is_none() {
            return Err(
                "predict_answer is the answer to a predict question: give the question".into(),
            );
        }
        let mut open = lock(&self.open);
        let l = open
            .get_mut(id)
            .ok_or_else(|| format!("no lesson {id} is being written"))?;
        l.steps.push(Step {
            title,
            predict,
            body,
            focus,
            picture,
            predict_answer,
        });
        Ok(format!("Step {} of {id} added.", l.steps.len()))
    }

    fn end(
        &self,
        store: &LessonStore,
        v: &Value,
        cx: &ToolContext,
        rom: &str,
    ) -> Result<String, String> {
        let id = &self.which(v, cx)?;
        let mut next = Vec::new();
        for o in v["next"].as_array().into_iter().flatten() {
            let c = o["concept"].as_str().unwrap_or_default();
            let level = o["level"].as_u64().unwrap_or(0) as u8;
            check_concepts(&[(c.to_owned(), level)])?;
            next.push(Offer {
                title: o["title"].as_str().unwrap_or_default().to_owned(),
                concept: c.to_owned(),
                level,
            });
        }
        let mut l = lock(&self.open)
            .remove(id)
            .ok_or_else(|| format!("no lesson {id} is being written"))?;
        if l.steps.is_empty() {
            lock(&self.open).insert(id.clone(), l);
            return Err("add its steps before ending it".into());
        }
        let before = store.learner();
        l.next = next;
        l.finished = true;
        if l.steps.iter().any(|s| s.focus.is_some()) {
            l.rom = Some(rom.to_owned());
        }
        store
            .save(&l, &[])
            .map_err(|e| format!("the lesson was not saved: {e}"))?;
        lock(&self.pending).push(l.id.clone());
        lock(&self.reviews).push(l.id.clone());
        let after = store.learner();
        let raised: Vec<String> = l
            .concepts
            .iter()
            .filter(|(c, _)| after.level(c) > before.level(c))
            .map(|(c, _)| format!("{c} {}", after.level(c)))
            .collect();
        Ok(format!(
            "Lesson {id} ended, {} steps. {}",
            l.steps.len(),
            if raised.is_empty() {
                "The student had already reached these levels.".to_owned()
            } else {
                format!("The student has now reached: {}.", raised.join(", "))
            }
        ))
    }
}

fn revise(store: &LessonStore, v: &Value, cx: &ToolContext) -> Result<String, String> {
    let named = v["lesson"].as_str().unwrap_or_default().trim();
    // None named: the latest this conversation finished.
    let latest = || {
        store
            .list()
            .into_iter()
            .filter(|l| l.finished && l.conversation == cx.conversation)
            .max_by_key(|l| l.created)
            .map(|l| l.id)
    };
    let id = if named.is_empty() {
        latest().ok_or("this conversation has finished no lesson")?
    } else {
        named.to_owned()
    };
    let id = id.as_str();
    let n = v["step"].as_u64().unwrap_or(0) as usize;
    let text = |k: &str| {
        v[k].as_str()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    let (Some(title), Some(body)) = (text("title"), text("body")) else {
        return Err("a step needs a title and a body".into());
    };
    let reason = text("reason").ok_or("say in `reason` what was wrong")?;
    if n == 0 {
        return Err("steps are numbered from 1".into());
    }
    let step = Step {
        title,
        predict: text("predict"),
        body,
        focus: None,
        picture: None,
        predict_answer: None,
    };
    store.revise(id, n - 1, step, &reason)?;
    Ok(format!("Step {n} of {id} rewritten."))
}

/// A step's focus from its arguments, checked.
fn focus(
    v: &Value,
    resolve: Resolve,
    rec: Option<Arc<RecordingSession>>,
) -> Result<Option<Focus>, String> {
    if let Some(n) = v["focus_frame"].as_u64() {
        let rec = rec.ok_or("no recording is open, so a step cannot show a frame")?;
        let i = rec.info();
        let last = i.first_frame + i.frame_count.saturating_sub(1);
        if n < i.first_frame || n > last {
            return Err(format!(
                "frame {n} is not in the recording (frames {} to {last})",
                i.first_frame
            ));
        }
        let view = v["focus_view"].as_str().unwrap_or("frame").to_owned();
        return Ok(Some(Focus::Frame { n, view }));
    }
    let Some(a) = v["focus_address"].as_str().filter(|a| !a.trim().is_empty()) else {
        return Ok(None);
    };
    let (start, offset, register) = resolve(a)?;
    if register {
        return Ok(Some(Focus::Register { address: start }));
    }
    if offset.is_none() {
        return Err(format!(
            "{} is not in the ROM; name a register or ROM address",
            addr(start)
        ));
    }
    if v["focus_in"].as_str() == Some("c") {
        return Ok(Some(Focus::Routine {
            entry: start,
            at: start,
        }));
    }
    let end = match v["focus_end"].as_str().filter(|e| !e.trim().is_empty()) {
        Some(e) => {
            let (end, _, _) = resolve(e)?;
            if end < start {
                return Err("focus_end is before focus_address".into());
            }
            end
        }
        None => start,
    };
    Ok(Some(Focus::Address { start, end }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Workbench;
    use crate::tutor::tools::RomTools;
    use romlens_tutor::agent::Tools;

    fn fixture(name: &str) -> (RomTools, std::path::PathBuf) {
        let rom = crate::Rom::from_bytes(romlens_core::fixtures::explain_lorom(), "e.sfc".into())
            .unwrap();
        let wb = Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        let t = RomTools::new(wb);
        let root = std::env::temp_dir().join(format!(
            "romlens-lesson-tools-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        t.lessons.set_store(Some(LessonStore::new(&root)));
        (t, root)
    }

    fn call(t: &RomTools, name: &str, input: Value) -> ToolOutput {
        let on = |_: romlens_tutor::agent::Event| {};
        let cancel = romlens_tutor::http::Cancel::new();
        let cx = ToolContext {
            mode: romlens_tutor::agent::Mode::ReadOnly,
            conversation: "c1",
            turn: 1,
            events: &on,
            approver: &romlens_tutor::agent::AcceptAll,
            cancel: &cancel,
        };
        t.run("t", name, &input, &cx)
    }

    fn text(o: &ToolOutput) -> String {
        o.parts
            .iter()
            .map(|p| match p {
                romlens_tutor::Part::Text { text } => text.clone(),
                _ => String::new(),
            })
            .collect()
    }

    fn step(lesson: &str, title: &str, extra: Value) -> Value {
        let mut v = json!({"lesson": lesson, "title": title, "predict": null, "body": "Some words.",
            "focus_address": null, "focus_end": null, "focus_in": null,
            "focus_frame": null, "focus_view": null, "picture": null});
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        v
    }

    #[test]
    fn a_predict_answer_must_be_true() {
        let (t, root) = fixture("predict");
        let begun = text(&call(
            &t,
            "begin_lesson",
            json!({"title": "The screen", "from_level": 2, "to_level": 2,
            "concepts": [{"id": "forced_blank", "level": 2}], "builds_on": []}),
        ));
        assert!(begun.contains("begun"), "{begun}");
        let claim = |expect: u32| {
            json!({"kind": "register_address", "register": "INIDISP", "expect": expect}).to_string()
        };
        let asked = |c: String| {
            step(
                "",
                "Where",
                json!({"predict": "Where is INIDISP?", "predict_answer": c}),
            )
        };
        let bad = call(&t, "lesson_step", asked(claim(0x2105)));
        assert!(
            bad.is_error && text(&bad).contains("$2100"),
            "{}",
            text(&bad)
        );
        let ok = call(&t, "lesson_step", asked(claim(0x2100)));
        assert!(!ok.is_error, "{}", text(&ok));
        let no_question = call(
            &t,
            "lesson_step",
            step("", "No question", json!({"predict_answer": claim(0x2100)})),
        );
        assert!(no_question.is_error, "an answer needs its question");
        let open = t.lessons.open.lock().unwrap();
        let l = open.values().next().unwrap();
        assert!(l.steps[0].predict_answer.is_some());
        drop(open);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_lesson_is_written_a_step_at_a_time() {
        let (t, root) = fixture("steps");
        assert_eq!(
            t.kind("lesson_step"),
            romlens_tutor::agent::ToolKind::Visual
        );
        assert_eq!(t.kind("learner"), romlens_tutor::agent::ToolKind::Read);
        assert!(text(&call(&t, "learner", json!({}))).contains("start at level 1"));

        // Checked: the map's concepts, the ladder's levels.
        let bad = call(
            &t,
            "begin_lesson",
            json!({"title": "Sprites", "from_level": 1, "to_level": 2,
            "concepts": [{"id": "sprite", "level": 2}], "builds_on": []}),
        );
        assert!(
            bad.is_error && text(&bad).contains("not on the concept map"),
            "{}",
            text(&bad)
        );
        let bad = call(
            &t,
            "begin_lesson",
            json!({"title": "Sprites", "from_level": 3, "to_level": 2,
            "concepts": [{"id": "sprites", "level": 2}], "builds_on": []}),
        );
        assert!(bad.is_error);

        let begun = call(
            &t,
            "begin_lesson",
            json!({"title": "How a sprite reaches the screen",
            "from_level": 1, "to_level": 2,
            "concepts": [{"id": "sprites", "level": 2}, {"id": "ppu", "level": 1}], "builds_on": []}),
        );
        assert!(!begun.is_error, "{}", text(&begun));
        let id = text(&begun).split_whitespace().nth(1).unwrap().to_owned();

        // Steps: a predict question, a register, a routine's C; a frame with
        // no recording open, and an address outside the ROM, are refused.
        assert!(
            !call(
                &t,
                "lesson_step",
                step(&id, "Two chips", json!({"predict": "Which chip draws?"}))
            )
            .is_error
        );
        let r = call(
            &t,
            "lesson_step",
            step(&id, "OBSEL", json!({"focus_address": "$2101"})),
        );
        assert!(!r.is_error, "{}", text(&r));
        let r = call(
            &t,
            "lesson_step",
            step(
                &id,
                "The code",
                json!({"focus_address": "$00:8000", "focus_in": "c"}),
            ),
        );
        assert!(!r.is_error, "{}", text(&r));
        let r = call(
            &t,
            "lesson_step",
            step(
                &id,
                "A frame",
                json!({"focus_frame": 3, "focus_view": "oam"}),
            ),
        );
        assert!(
            r.is_error && text(&r).contains("no recording"),
            "{}",
            text(&r)
        );
        let r = call(
            &t,
            "lesson_step",
            step(&id, "RAM", json!({"focus_address": "$7E:0000"})),
        );
        assert!(
            r.is_error && text(&r).contains("not in the ROM"),
            "{}",
            text(&r)
        );
        let r = call(
            &t,
            "lesson_step",
            step(&id, "A picture", json!({"picture": "made-up"})),
        );
        assert!(r.is_error, "{}", text(&r));

        // Nothing is recorded until it ends.
        assert!(t.lessons.store().unwrap().list().is_empty());
        let open = t.lessons.open_lesson(&id).unwrap();
        assert_eq!(open.steps.len(), 3);
        assert_eq!(
            open.steps[1].focus,
            Some(Focus::Register { address: 0x2101 })
        );
        assert!(matches!(open.steps[2].focus, Some(Focus::Routine { .. })));

        let ended = call(
            &t,
            "end_lesson",
            json!({"lesson": id,
            "next": [{"title": "Sprites in this game", "concept": "oam", "level": 3}]}),
        );
        assert!(!ended.is_error, "{}", text(&ended));
        assert!(text(&ended).contains("sprites 2"), "{}", text(&ended));
        let store = t.lessons.store().unwrap();
        let saved = store.load(&id).unwrap();
        assert!(saved.finished && saved.rom.is_some() && saved.conversation == "c1");
        assert_eq!(store.learner().level("sprites"), 2);

        // A later lesson builds on it, and `lesson` reads it back.
        assert!(text(&call(&t, "lesson", json!({"id": id}))).contains("1. Two chips"));
        let next = call(
            &t,
            "begin_lesson",
            json!({"title": "OAM", "from_level": 3, "to_level": 3,
            "concepts": [{"id": "oam", "level": 3}], "builds_on": [id]}),
        );
        assert!(!next.is_error, "{}", text(&next));
        let orphan = call(
            &t,
            "begin_lesson",
            json!({"title": "OAM", "from_level": 3, "to_level": 3,
            "concepts": [{"id": "oam", "level": 3}], "builds_on": ["l1-00000"]}),
        );
        assert!(orphan.is_error);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_step_shows_a_diagram_romlens_drew() {
        let (t, root) = fixture("diagram");
        let drawn = call(
            &t,
            "draw_diagram",
            json!({"kind": "fields", "spec": {"register": "INIDISP", "value": "$80"}}),
        );
        assert!(!drawn.is_error, "{}", text(&drawn));
        let (image, bytes) = drawn.pictures[0].clone();
        assert!(image.id.starts_with("draw-"));
        let begun = call(
            &t,
            "begin_lesson",
            json!({"title": "Forced blank", "from_level": 2, "to_level": 2,
            "concepts": [{"id": "forced_blank", "level": 2}], "builds_on": []}),
        );
        let id = text(&begun).split_whitespace().nth(1).unwrap().to_owned();
        let r = call(
            &t,
            "lesson_step",
            step(&id, "Bit 7", json!({"picture": image.id})),
        );
        assert!(!r.is_error, "{}", text(&r));
        assert!(!call(&t, "end_lesson", json!({"lesson": id, "next": []})).is_error);
        // At the end of the round the picture goes with the lesson.
        let pictures = HashMap::from([(image.id.clone(), bytes.clone())]);
        t.lessons.settle(&pictures);
        let store = t.lessons.store().unwrap();
        assert_eq!(store.picture(&id, &image.id), Some(bytes));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn romlens_checks_a_lessons_code_and_registers() {
        let mut l = Lesson::new("DMA", (3, 3), vec![("dma".into(), 3)]);
        l.steps.push(Step {
            title: "The code".into(),
            predict: None,
            body: "```asm\n$00:8052  LDX #$1809   ; DMAP0\n$00:8055  A2 00 43  STA $4300\n$00:8060  LDA #$00\n```\nThen `$2118 VMDATAL` and `$2118 CGDATA` and $4300 DMAP0.".into(),
            focus: None,
            picture: None,
            predict_answer: None,
        });
        let insn_at = |a: u32| match a {
            0x8052 => Some(("LDX".to_string(), "#$1809".to_string())),
            0x8055 => Some(("STX".to_string(), "$4300".to_string())),
            _ => None,
        };
        let found = mechanical(&l, &insn_at);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(
            found[0].contains("shows `STA $4300` at $00:8055, but the ROM has `STX $4300`"),
            "{found:?}"
        );
        assert!(
            found[1].contains("$00:8060 is not an instruction"),
            "{found:?}"
        );
        assert!(
            found[2].contains("calls $2118 CGDATA, but $2118 is VMDATAL (CGDATA is $2122)"),
            "{found:?}"
        );
    }
}
