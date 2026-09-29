# Lessons: the tutor's Explain mode

## Progress

Written 28 September 2026 and kept as written. This table is the only part
that tracks progress against it.

| Task | State |
|---|---|
| L0 this document, the pointers from `06` and `24`, the checklist rows | planned |
| L1 the lesson format, the concept map and the learner record, in `romlens-tutor` | done, 28 September 2026: `romlens_tutor::lesson`: `Lesson`, `Step` (title, a predict question, the body, a `Focus` of an address range, a routine at an instruction, a frame in one of the Graphics views, or a register, and a picture), `Offer`; the ladder's five levels; `CONCEPTS`, 46 concepts in five groups, each with what it rests on and a line in our own words, checked for missing and circular needs; `LessonStore` in `<root>/Learner/` (lessons whole, with their pictures, and `learner.json` for the concepts marked known). How far the student has got (`Learner`) is worked out from the finished lessons and the marks each time, so a lesson not ended counts for nothing, a later lower level never lowers one, and deleting a lesson takes back what it taught |
| L2 the lesson tools, Explain mode's prompt, the learner summary in the prefix | done, 28 September 2026: `tutor::lessons` in the FFI: `learner`, `lesson`, `begin_lesson`, `lesson_step`, `end_lesson`. Checked there: concepts on the map, levels 1 to 5, earlier lessons that exist, and each step's focus resolved (an address in the ROM, a register, a routine's C, or a frame the open recording has, in one of the Graphics views); a picture must be one a tool returned, and its bytes are copied to the lesson at the end of the round that made it (`Lessons::settle`, at each checkpoint). The building tools are `Visual`, so a round's steps run in order, not in parallel; a step or end with no lesson named goes to the one this conversation has open. Only `end_lesson` records anything. Explain mode is `Session::explain`, kept in `meta.json`; switching it tells the model with the next question (`prompt::context`), with the learner record as it is then, and a new conversation's prefix carries the record after the ROM digest. The prompt gains a short Lessons section, and the ladder and the concept map follow the primer |
| L3 the store, `romlens tutor lessons` and `lesson show` | done, 28 September 2026: `romlens tutor lessons [--dir]` prints what the student has reached and every lesson (id, date, steps, levels, whether it ended, title); `romlens tutor lesson [latest\|id] [--dir]` prints one step by step, with what each asks first, says, points at and shows (`Lesson::describe`, which the `lesson` tool uses too); `romlens tutor ask --explain` answers with a lesson and keeps it in the learner record under `ROMLENS_TUTOR_DIR` or the app's folder |
| L4 FFI: lessons, steps and the learner record for the shell | done, 28 September 2026: `TutorSession::lesson` (finished or still being written), `lessons`, `lesson_picture`, `delete_lesson`, `learner` (every concept on the map with its group, needs, line, level reached, whether marked and by which lesson), `mark_known`, `set_explain` and `explain`; `LessonInfo` gives each step's focus as a `romlens://` link (`a/` an address, `c/` a routine's C, `f/<n>?view=` a frame in a Graphics view, `r/` a register) and in words; `lesson_id_from_result` finds the lesson a `begin_lesson` result names, so the window needs no new event: it refreshes a lesson when a lesson tool finishes |
| L5 app: the lesson card in the Tutor window, stepping, focus in the main window | done, 28 September 2026: `LessonCard`: the title and the level as a badge, the step ("2 of 5", with … while it is still being written), a predict question with the step's answer behind Show, the body as the answers are (citations are links), its picture, a "Show … in Romlens" link, Back, Next (⌘→) and Skip to the end, and at the last step the offers as Where next links, which ask for the next lesson with Explain on. It shows in the transcript where `begin_lesson` was called, and as it is written in the live answer. Each step moves the main window: `romlens://c/` opens the C tab at the instruction, `romlens://f/<n>?view=` the frame in that Graphics view. A question asked at a step carries `[The student is at step N of M of lesson … ]` (`set_lesson_step`), which the window hides like Romlens's other notes. Explain is a toggle in the status line, `/explain`, and `/learn <topic>` (Explain on, and the topic asked); it carries over to a new conversation |
| L6 app: the lesson library and the map of what the student knows | done, 28 September 2026: one sheet, from `/lessons`, `/map` or the books button in the status line, with two tabs. Lessons: every lesson, newest first (title, level, steps, date, whether it ended), the one picked shown as its card to read again, and Delete Lesson in its menu (which takes back what it taught). What you know: the map's five groups, each concept a chip shaded by the level reached with a legend of the ladder, its line and what it rests on as help, and Mark as Known at a level (or Clear the Mark) in its menu, kept after quitting. The status line's Conversations and Lessons buttons are icons, so it fits in the window's narrowest width |
| L7 live runs: a new learner's sprite lesson, then deeper, then a second topic | ready, 28 September 2026, for the user's runs in the app (no key is in the sessions' environment and Ollama was not running): the four questions of L7 below and manual rows 59–62 in `15`. Each run is read back with `romlens tutor lesson latest` and `romlens tutor show latest`, and what went wrong is recorded here |
| L8 checking lessons: this section, the pointers, the checklist rows | done, 29 September 2026 |
| L9 the lesson's revisions and check, `Session::review` | done, 29 September 2026: `Lesson.revisions` (each step's old words, the reason, when) and `Lesson.checked` (when, steps changed, cost, the last line), both defaulting so older lessons load; `LessonStore::revise` (a finished lesson's step replaced, its focus and picture kept) and `set_checked`; `tutor lesson` prints each revision and the check. `Session::review` runs `REVIEW_PROMPT`, the lesson and Romlens's findings on a copy of the conversation, read-only and within `REVIEW_CAP` ($0.50), and returns the last line and the cost; the conversation is untouched and the request keeps its prefix |
| L10 FFI: `revise_lesson_step`, Romlens's own checks, the review after the job, `LessonChecked` | done, 29 September 2026: `revise_lesson_step` (a finished lesson's step, whole; an empty lesson id is the latest this conversation finished; always declared, so the tutor can also fix an old lesson in a normal turn). `lessons::mechanical`: each line of code a step shows against the listing (the mnemonic, and a `#` or `$` operand), and each `$21xx NAME` or `$42xx`/`$43xx` against the register table, a line per finding. `end_lesson` queues the lesson; after the turn, once the session is handed back and the conversation named, each is checked from a copy with `ReviewTools` (the conversation's tool list, but edits, `generate_image` and the lesson-building tools refused), its cost charged to the conversation (the open session, its file, or when the turn that has it ends), `Lesson.checked` written and `LessonChecked` sent. `set_check_lessons`; `LessonInfo.checked` and `checking`. Tests: the mechanical checks; on the loopback server, a lesson checked, an edit refused, the step rewritten with its history, the transcript unchanged |
| L11 CLI: the check after `tutor ask --explain`, revisions in `tutor lesson` | planned |
| L12 app: the setting, Checking… and Checked on the card | planned |

## Context

The tutor answers questions (docs/24). The student wants more than answers:
to learn the SNES from the top down, at the pace of their own questions. Asked
"how does a sprite get rendered to the screen?", a beginner should first hear
what the CPU and the PPU each do and what a sprite is. Machine code comes
later, when their questions lead there. A student who has already learned
about OAM and DMA should get the same question answered from the next level
down, building on what they learned rather than repeating it.

So Explain mode answers with a **lesson**: a short sequence of steps revealed
one at a time. Each step says one thing, points the main window at what it
is about, and may ask the student to predict what comes next before showing
it. How long the lesson is depends on the topic, and how deep it goes depends
on what the student has already learned. That comes from a **learner
record**, kept across every conversation and every ROM, which the tutor reads
before it teaches and adds to when a lesson is done.

docs/06 planned Explain mode "producing scene drafts" for Phase 4. A lesson's
steps (narration, a focus, a picture) are that draft. Scenes, animation and
video export (docs/01, Phase 4) build on them later; this plan does not
include them.

## Scope decisions (28 September 2026)

1. **Depth is a ladder of five levels**, the same for every topic, so that
   "deeper" means something the student can see:

   | Level | Name | What it holds | "How is a sprite drawn?" |
   |---|---|---|---|
   | 1 | The idea | What it is and why, with no registers or code | The CPU decides, the PPU draws; a sprite is a small picture the PPU can place anywhere, on top of the backgrounds |
   | 2 | The hardware | The chips, memories and registers involved | OAM holds 128 sprites' positions, tiles and palettes; their pixels are tiles in VRAM; colours come from CGRAM; OBSEL picks sizes |
   | 3 | In this game | Where this game keeps the data, shown with a recording | Its OAM buffer in WRAM, the entries at `frame N`, the sprite drawn with `render_sprite`, its tiles decoded |
   | 4 | In the code | The routines that do it, in C and assembly | The NMI's DMA of the buffer to OAM, the object loop filling the buffer |
   | 5 | The bytes and cycles | Instruction-level detail, timing and edge cases | Why the DMA must finish in vblank, the high table's X bit 8, hiding a sprite at Y `$F0` |

   A lesson covers one or two adjacent levels of one topic. A beginner's
   first sprite lesson is levels 1–2. "Go deeper" asks for the next level.
2. **A fixed concept map.** About forty SNES concepts (the CPU, the PPU, the
   memory map, WRAM, VRAM, OAM, CGRAM, tiles, palettes, sprites, backgrounds,
   BG modes, the frame and vblank, NMI, DMA, HDMA, the 65816's widths, the
   stack, jump tables, the APU, BRR, and so on), each with an id and the
   concepts it rests on. Lessons are tagged with the concepts they teach and
   the level reached. The map is ours, written from our own primer, and
   lives in `romlens-tutor` beside it. A fixed map keeps the record
   consistent across conversations and models, and lets the app show what
   the student knows.
3. **The learner record is the student's, not a project's.** Knowing what a
   sprite is carries over from Super Mario World to F-Zero. It lives in the
   app's folder (`Tutor/Learner/`), beside the conversations and like them
   never in a package: each concept's level reached, and the lessons, each
   tagged with the ROM it used. A level-3 or level-4 lesson is about one
   game; levels 1–2 are about the SNES.
4. **The tutor decides a lesson's length and depth, within rules.** The
   prompt says: start at the next level the student has not reached for the
   concepts the question needs, and teach a concept's prerequisites first,
   briefly, if the student has not met them. Say in one line what an earlier
   lesson covered instead of teaching it again, and link to it. Use as many
   steps as the topic needs: a simple idea in three, a pipeline in eight to
   twelve. Each step says one thing.
5. **Progressive revelation within a lesson.** Steps are revealed one at a
   time, with Next. A step can open with a *predict* question ("Where do you
   think the sprite's pixels are kept?"). Its answer is in the next step, and
   the student can answer or just go on. These are not graded; checked
   questions are Quiz mode's job, later.
6. **Explain is a mode beside Ask**, switched in the composer and by
   `/learn <topic>`. With Explain on, a question is answered as a lesson.
   With it off, questions get ordinary answers, and the tutor may offer "Want
   this as a lesson?" at the end. A question asked partway through a lesson
   is answered in the conversation, with the step it was asked at, and the
   lesson waits at that step.
7. **Facts still come from tools** (docs/06 principles 1 and 2). Levels 1–2
   rest on the primer and `reference`. Levels 3–5 cite the ROM and the
   recording like any answer. Every step's focus is resolved by Romlens
   before it is shown: an address that is not in the ROM, or a frame the
   recording does not have, is refused back to the tutor.

## Design

### The lesson format (`romlens-tutor::lesson`)

```text
Lesson { id, title, rom (SHA-256, or none for levels 1–2), created,
         levels: (from, to), concepts: [(id, level)],
         builds_on: [lesson id], steps: [Step], next: [Offer] }
Step   { title, predict: Option<String>, body (Markdown, cited),
         focus: Option<Focus>, picture: Option<picture id> }
Focus  = Address(range) | Routine(entry, C line or instruction)
       | Frame(n, view: screen | layer | sprites | vram | cgram | oam)
       | Register(address)
Offer  { title, concept, level }      // "Go deeper": what the next lesson would be
```

A lesson is saved whole in `Tutor/Learner/lessons/<id>.json`, and its
pictures sit beside it. The conversation that made it holds only the tool
calls that built it (`begin_lesson`, `lesson_step`, `end_lesson`); the
window finds the lesson by those, so no new kind of transcript block is
needed and older builds still read the conversation.

### The learner record

`Tutor/Learner/learner.json`: each concept's level reached, when, and in
which lesson. It is written when a lesson ends (the tutor calls
`end_lesson`), not while it is being made. A student can mark a concept as
already known, at a level, from the map (L6), to skip the basics.

### The tools (Explain mode only)

- `learner()`: the concepts and levels reached, and the last lessons' titles.
  It is also put in the prefix, so this call is only needed after changes.
- `lesson(id)`: an earlier lesson's steps, to build on it or link to it.
- `begin_lesson(title, levels, concepts, builds_on)`, then `lesson_step(title,
  predict?, body, focus?, picture?)` once per step, then `end_lesson(next)`.
  Step by step so that the lesson appears as it is written. Each call is
  checked: a focus is resolved, a picture must be one a tool made in this
  conversation, and a concept id must be on the map.

The read-only tools (docs/24) are all there too. Edit tools follow the
permission mode as before; a lesson does not change the project.

### The prompt

A fixed section for Explain mode, after the rules and before the primer: the
ladder, how to choose where to start from the learner record, one idea per
step, predict questions, not repeating earlier lessons but linking them, and
the offers of where to go next. The concept map goes in the primer, as ids
and one line each. The learner summary is compact (only concepts the
student has reached, and the last ten lessons' titles). It goes into the
conversation's own prefix when the conversation starts, after the ROM
digest, so the cache holds across its turns.

### The app

- **The lesson card** in the transcript:
  - The title, the level as a named badge ("The idea", "The hardware"...),
    and the step as "3 of 8".
  - The step's text, and its picture.
  - Back and Next.
  - A predict question shows first, with the rest of the step behind
    "Show".
  - At the last step, the offers as buttons ("Go deeper: sprites in this
    game").
  - Stepping moves the main window to the step's focus: the address
    selected and scrolled to, the C line highlighted, the recording at the
    frame with the named view. This is the citation machinery the tutor
    already uses (`TutorModel.follow`).
- **Questions at a step:** the composer knows the step, and the question
  carries it as context (`[At step 3 of "How a sprite reaches the
  screen"]`).
- **The library** (`/lessons` and a toolbar button): lessons by concept and
  date, which reopen as cards.
- **The map:** the concepts grouped (the machine, graphics, timing, the CPU,
  sound), each shaded by the level reached. A student can mark one as known.

### CLI

`romlens tutor lessons` lists the lessons. `romlens tutor lesson show <id>`
prints one step by step, with its focuses and the learner record's changes,
for reviewing what the tutor taught (as `tutor show` does for
conversations).

## Ordered tasks (one commit each, pushed to main)

| # | Task | Days |
|---|---|---|
| L0 | This document; pointers from `06` (Explain mode) and `24` (cut list); checklist rows in `15` | 0.25 |
| L1 | `lesson.rs`: the format, the concept map with prerequisites, the learner record; serde round trips; the map's graph has no cycles, and every prerequisite is on it | 1 |
| L2 | The lesson tools and their checks; Explain mode's prompt section; the learner summary in the prefix; `Session` knows the mode. Tests on `FakeTransport`: a lesson made step by step, a bad focus refused, the record updated at `end_lesson` and not before | 1.5 |
| L3 | The store (`Tutor/Learner/`); `tutor lessons` and `tutor lesson show`; a lesson whose ROM is not open still reads (its levels 3–5 focuses shown as text) | 0.5 |
| L4 | FFI: `TutorSession` lessons, the learner record, the mode; events for a lesson begun, a step added, a lesson ended | 1 |
| L5 | App: the lesson card, Back and Next, predict and Show, the offers; focus in the main window; a question at a step; the Explain toggle and `/learn` | 2 |
| L6 | App: the library and the concept map, marking a concept known | 1.5 |
| L7 | Live: as a new learner, "how does a sprite get rendered to the screen?" in SMW (expect levels 1–2 and no code), then Go deeper twice (3 with a recording, then 4), then "how does the screen scroll?" (expect it to build on the frame and vblank, not teach them again). Record what went wrong here and tune the prompt from the transcripts | 1 |

## Checking lessons (29 September 2026)

The first live lessons were mostly right, but the words slipped: a DMA
lesson counted "8 CPU cycles per byte" where DMA takes 8 master cycles, and
said a fill cleared every VRAM word without checking how VMAIN steps the
address. A lesson is kept and read again, and it feeds the learner record,
so every finished lesson is checked in the background and corrected.

- **When.** After the turn that ended a lesson, once the student has the
  session back, on the same thread that names conversations. On by
  default; Settings › Tutor turns it off. Its cost is added to the
  conversation's and shown on the lesson's card ("Checked · $0.03").
- **Who.** The lesson's own model, on a copy of the conversation with one
  more question, so the prompt and tools are read from the cache. Its
  turns are thrown away; nothing appears in the transcript.
- **What it may do.** Only read, and call `revise_lesson_step`: the project
  cannot change and no lesson can be started during a check.
- **What it is told.** To check every fact about the ROM with the tools,
  every hardware fact against the primer and `reference`, numbers, units
  and cycle counts, and code against the ROM; to rewrite a step that is
  wrong, imprecise or claims more than the tools show, at the same level
  and length; and to leave correct steps alone. Romlens's own checks come
  first and cost nothing: each line of code a step shows as
  `$BB:AAAA  MNEMONIC operand` against the listing, and each register
  cited as `$21xx NAME` against the register table.
- **How a step changes.** Silently (the student chose this): the corrected
  step replaces the old one on the card. The old step and the reason are
  kept in the lesson's `revisions`, which `romlens tutor lesson` prints, so
  the corrections can be audited. The learner record does not change.
- `revise_lesson_step` is declared always, so the tutor can also correct
  an earlier lesson in a normal turn when the student finds an error.

## What is cut for now

- Quiz mode with checked answers (predict questions are not graded).
- Scenes: animation, camera moves, video export. Lessons are their drafts.
- Lessons shared between students, or exported as documents.
- Diagrams drawn by Romlens itself, planned in `26-diagrams.md` (until
  then a lesson can use `generate_image` and the tools' pictures).

## Risks

- **The ladder is only as good as the model's sense of level.** The prompt
  gives each level's contents per topic, and the live runs (L7) check a
  beginner gets no code. If models drift, a check in `lesson_step` can flag
  a level-1 step that cites an address.
- **The record goes stale or wrong** (a lesson abandoned, a concept marked
  too high). Only a finished lesson writes it, the student can see and
  change it on the map, and a lesson can be deleted with its record entries.
- **Cost.** A lesson is several tool calls. The step calls are small, the
  prefix is cached, and the cost is shown per lesson.
- **Long lessons get tedious.** One idea per step, a limit of about twelve
  steps before offering to continue in another lesson, and the student can
  skip to the end.

## Verification

- `make test`: the format and map (L1); the tools' checks and the record's
  timing on `FakeTransport` (L2); the store (L3); the FFI events (L4).
- `make app-test`: the card steps and moves the main window, a predict step
  hides its body until Show, the offers start the next lesson, the map shades
  what the record holds.
- L7 by hand, with the transcripts read through `romlens tutor show` and
  `romlens tutor lesson show`.
