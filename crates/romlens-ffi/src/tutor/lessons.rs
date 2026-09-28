//! The lesson tools (docs/25): the tutor builds a lesson a step at a time
//! (`begin_lesson`, `lesson_step`, `end_lesson`) and reads what the student
//! has learned (`learner`, `lesson`). Each step's focus is resolved here, so
//! the window never points at something the ROM or recording lacks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use romlens_tutor::ToolSpec;
use romlens_tutor::agent::{ToolContext, ToolOutput};
use romlens_tutor::lesson::{
    FRAME_VIEWS, Focus, LEVELS, Lesson, LessonStore, Offer, Step, check_concepts, concept,
    level_name,
};
use serde_json::{Value, json};

use super::tools::{addr, choice, integer, nullable, object, spec, string};
use crate::RecordingSession;

/// The tools, in the order the model uses them.
pub const NAMES: [&str; 5] = [
    "learner",
    "lesson",
    "begin_lesson",
    "lesson_step",
    "end_lesson",
];

/// The ones that build a lesson, which run in order.
pub const BUILDING: [&str; 3] = ["begin_lesson", "lesson_step", "end_lesson"];

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
    ]
}

/// Lessons being written, and the store the finished ones go to.
#[derive(Default)]
pub struct Lessons {
    store: Mutex<Option<Arc<LessonStore>>>,
    open: Mutex<HashMap<String, Lesson>>,
    /// Finished lessons whose pictures are still to be copied, once the
    /// round that made them has put them in the conversation.
    pending: Mutex<Vec<String>>,
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
        Ok(describe(&l))
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
            levels((from, to))
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
            && !(p.starts_with("tool-") || p.starts_with("shot-"))
        {
            return Err(format!("{p} is not a picture a tool returned"));
        }
        let predict = v["predict"]
            .as_str()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned);
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

fn levels((from, to): (u8, u8)) -> String {
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

/// A lesson as text, for `lesson` and for review.
pub fn describe(l: &Lesson) -> String {
    let mut o = format!("{} ({}): {}\n", l.id, levels(l.levels), l.title);
    let concepts: Vec<String> = l
        .concepts
        .iter()
        .map(|(c, lv)| format!("{} {lv}", concept(c).map_or(c.as_str(), |c| c.name)))
        .collect();
    o.push_str(&format!("Teaches: {}\n", concepts.join(", ")));
    if !l.builds_on.is_empty() {
        o.push_str(&format!("Builds on: {}\n", l.builds_on.join(", ")));
    }
    for (i, s) in l.steps.iter().enumerate() {
        o.push_str(&format!("\n{}. {}\n", i + 1, s.title));
        if let Some(p) = &s.predict {
            o.push_str(&format!("   (asks first: {p})\n"));
        }
        for line in s.body.lines() {
            o.push_str(&format!("   {line}\n"));
        }
        if let Some(f) = &s.focus {
            o.push_str(&format!("   [focus: {}]\n", focus_text(f)));
        }
        if let Some(p) = &s.picture {
            o.push_str(&format!("   [picture {p}]\n"));
        }
    }
    for n in &l.next {
        o.push_str(&format!(
            "\nNext: {} ({} {}, {})",
            n.title,
            n.concept,
            n.level,
            LEVELS[(n.level.clamp(1, 5) - 1) as usize]
        ));
    }
    o
}

pub fn focus_text(f: &Focus) -> String {
    match f {
        Focus::Address { start, end } if start == end => addr(*start),
        Focus::Address { start, end } => format!("{} to {}", addr(*start), addr(*end)),
        Focus::Routine { at, .. } => format!("the C at {}", addr(*at)),
        Focus::Frame { n, view } => format!("frame {n}, {view}"),
        Focus::Register { address } => format!("register {}", addr(*address)),
    }
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
}
