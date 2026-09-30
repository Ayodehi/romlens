# Quizzes and progress: checked questions, proven levels, points

## Progress

Written 30 September 2026 and kept as written. This table is the only part
that tracks progress against it.

| Task | State |
|---|---|
| Q0 this document, the pointers from `06`, `24` and `25`, the checklist rows | done, 30 September 2026 |
| Q1 `romlens-tutor::quiz`: the quiz format, claims as data, grading, the store; the journal | done, 30 September 2026: `quiz`: `Quiz` (concept, level, purpose Prove, Review or Practice, a review's targets, the seed from the id, questions, answers, hints shown, cost, the conversation), `Question` (prompt, how it's answered, explanation, sources, hint, focus, and where it came from: Romlens with its claim, the tutor with its claim, or the tutor marked by the model), `Ask` (choice, number, bits, a listing's line, text), `Given`, `Attempt`, `Claim` (19 kinds, each with the field that is its answer: `expected`, and `with_expected` for what a wrong choice claims), `grade`, `parse_number` (`$`, `0x`, a bank's colon), `Quiz::answer`, `hint` (halves the credit), `mark` (the model's 0, ½ or 1), `outcome` (passed at 80% with three certain right without the hint; the model's marks never count as certain), `QuizStore` in `<root>/Learner/quizzes/`. `progress::Journal`: `journal.jsonl`, appended and synced, a torn last line skipped. Older files load |
| Q2 `romlens-tutor::progress`: proofs, reviews, XP, streaks, ranks, achievements, all derived | done, 30 September 2026: `Progress::derive(Inputs { lessons, quizzes, journal, marks, now, offset })`: lessons in order (20 points, and 5 for each concept-level newly taught), each certain answer, proofs (the first passing Prove quiz at a concept and level; 50 × the level), reviews (the boxes of 1, 3, 7, 21 and 60 days: all right moves up, none moves back to the first, else it stays), each step's first guess, each game's milestones once, the day's first activity, streaks over local days with one missed day bridged in seven (7 and 30 days earn points). `xp`, `MILESTONES` (9), `ACHIEVEMENTS` (19), `RANKS` (9, from the highest level proven of each concept), the ledger (every point with its cause, summing to `xp`), `highest`, `due` (only a concept's highest proven level), `diff` for the banner. `Learner::with_proofs`: a level proven is reached; `Reached` has `proven` and `due`; the prompt's summary says what quizzes proved. `LessonStore::progress(now, offset)` reads the quizzes and the journal beside the lessons |
| Q3 FFI `quiz`: the claim checks, the facts, the level 1–2 generators, coverage | done, 30 September 2026: `quiz::facts`: 55 numbers (each traced to the core's constant where it has one, and a test holds them equal) and 89 statements with their wrong answers, and each concept's registers, S-DSP registers, instructions, opcodes and glossary terms. `quiz::claims`: `check` for the table claims (a register's field, bits, address and job, the S-DSP's fields, a fact, a statement, a term, a concept's name or line, what it rests on, an instruction, an opcode's mode) with a line that says what is so; `validate`: the answer is the claim's, no wrong choice passes it, no choice twice, a bits question's bits are the field's. `quiz::make`: level 1 (the idea both ways, what it rests on, a term, a number as a choice, a statement), level 2 (a register's job and address, a field's bits, what a value written or read says, an instruction, an opcode's mode, a number typed, a statement). `Rng` (SplitMix64), `candidates` (each checked, duplicates dropped), `generate` (the same for the seed), `coverage`. Every concept has at least three at levels 1 and 2; twenty seeds of every question pass their claims |
| Q4 the level 3–5 generators, from the game's analysis | done, 30 September 2026: `quiz::claims_rom` checks the claims about the game: its mapping; where a vector, a label or an idiom is; the instruction at an address (its listing text); its bytes; the value a store writes and what that makes a field (`write_means`, new); a DMA's channel, destination, source, size, cycles and whether it fits in vertical blank ((262 − 225) × 1364 master cycles); A's or X's width at an instruction (refused where the analysis assumed one); an instruction's length; the idiom at an address. `quiz::make_rom`: level 3 (the mapping, the reset, NMI and IRQ vectors, what the code at an idiom does, what a value the game writes means, where a DMA copies), level 4 (the line of a routine that writes a register, A's or X's width at an immediate, the value a store writes, a DMA's channel and size), level 5 (an instruction's length and opcode byte, a DMA's master cycles and whether it fits). Each question points at its place (a focus and a `romlens://a/` link). A ROM with none of these asks nothing about them; hex answers are padded (`$0F`) |
| Q5 `TutorSession`: the quiz API, records and events, exploration milestones | done, 30 September 2026: `tutor::quiz` in the FFI: `QuizInfo`, `QuestionInfo` (how it is answered and where it came from, never the answer), `AnswerResultInfo` (the credit, the right answer, the explanation and sources, the model's line), `ProgressInfo` (points, rank and the next, the corpus, each group's standing, the streak, achievements and this game's milestones, reviews due, the latest points), `GivenInfo`; `start` (a concept named loosely; the level to prove by default is the lowest learned and not proven, else one above the highest proven; five of Romlens's questions, or two for each proof due; refused, saying so, when the game has too few), `milestones_met` (a name at the reset and NMI targets, on a routine that waits for vertical blank, on or in the sound upload, on a routine that sends DMA to VRAM; 10, 50 and 200 routines named; 25 lines commented), kept in the journal once. `TutorSession`: `set_utc_offset`, `start_quiz`, `quiz`, `quizzes`, `answer_question`, `question_hint`, `finish_quiz`, `progress`, `check_milestones`, `reset_progress`; `ConceptInfo.proven` and `due`; events `QuizChanged` and `Progress` (points gained, levels proven, achievements, a new rank), sent after each answer, a quiz's end, milestones and a turn |
| Q6 the tutor's questions: `quiz_question`, writing and grading on a copy | done, 30 September 2026: `quiz_question` (a choice, a number, bits or a written answer, with its claim as a JSON object or its text), declared always and refused except in `QuizTools`, which passes reads, refuses edits and the lesson tools, checks each question (`claims::validate`: the claim true, the answer its, no wrong choice passing it), takes at most two, a written one only after three Romlens marks, and adds each to the quiz as it comes (`QuizChanged`). `Session::write_quiz` and `grade` run on a copy of the conversation, as a lesson's check does (`Session::side`, read-only, within `QUIZ_CAP` $0.10 and `GRADE_CAP` $0.02); `QUIZ_PROMPT`, `GRADE_PROMPT` and `read_credit` (`CREDIT 1`, `0.5` or `0`, and a line to the student). `start_quiz(…, tutor)` starts the writer after Romlens's questions; a written answer is marked in the background, its credit none until then, and never counted as certain. Both costs are charged to the conversation. A "Quizzes" section in the prompt |
| Q7 CLI: `tutor quiz`, `tutor quiz coverage`, `tutor progress` | done, 30 September 2026: `romlens tutor quiz <rom> [concept] [--level N] [--review \| --practice] [--seed S] [--answers "…"] [--project] [--dir]` asks on the terminal (a choice's letter, a number, bits such as `0-3`, a line's number; `?` for the hint; empty skips) or marks the answers given, then says what was proven and the points; the tutor's questions are the app's. `romlens tutor quiz coverage <rom>` prints the questions Romlens can ask for each concept and level (Final Fantasy Mystic Quest: 129 of the 230 concept-levels can be proven offline). `romlens tutor progress [--ledger] [--json]`. The golden `tutor-quiz-lorom`. Running the generators on a real game found two gaps the test ROMs don't have, both now kept out: a register written twice in a listing window (CGDATA's two bytes), and code that is two idioms at once; a question its claim refuses is dropped, not asked. The ledger's order within a second is by cause, not id |
| Q8 app: the quiz sheet, `/quiz`, proven and due on the map | done, 30 September 2026: `QuizSheet`: the concept and level, a dot a question (right, half, wrong, being marked; a dot answered opens its question again), the question (Markdown, with the glossary and address links), a control for each kind (choices, a hex field, bit toggles, the listing's lines, a text box), Check, Skip and Hint (it halves the answer), then Right or Not quite with the right answer, the model's line for a written one, the explanation and a Show it in Romlens link; at the end what it came to: proven, and when it comes back, or what a proof takes and Try again with new questions. "The tutor is writing questions about the game…" while it does. `/quiz <topic>`, `/review`. The map: a ring on a proven concept (thicker the higher), "proven N" in its line, a clock when due; its menu: Prove the Next Level…, Practise (a level), then Mark as Known. Settings › Tutor: Add the tutor's questions about the game to quizzes (on). The tutor's questions come only with a conversation open. `tutor_test_quiz_answers` for the app's tests. App test: `/quiz sprites` proven with its five questions, the concept proven and due, a review with nothing due refused |
| Q9 predict questions answered before Show | done, 30 September 2026: `Step.predict_answer`, a claim (defaulting, so older lessons load; kept when a step is rewritten); `lesson_step` takes it (the question must be given, and `RomTools` checks the claim against the ROM first, refusing a false one with what is so). `TutorSession::answer_predict(lesson, step, guess)`: three characters or more; the first guess at a step goes in the journal and earns its points (3, and 2 more when right); Romlens marks it where the step has a claim and the guess reads as its answer (`with_expected`), else says nothing; the next question carries `[The student guessed: "…"]`. `LessonStepInfo.checks_guess`, `guessed`, `guess_right`. The card: Your guess ("Romlens checks it" when it will), Check and Show beside the question; once guessed, the guess under the question, right or not quite where it was checked. The prompt asks for `predict_answer` where the answer is a number, an address or a register's setting |
| Q10 app: the Progress tab, the status line, toasts, the setting, reviews | planned |
| Q11 live runs | planned |

## Context

Lessons (`25-lessons.md`) explain, and ask a predict question where a
guess is worth making, but nothing checks what stuck. Quiz mode was
always the next part (`06-ai-tutor.md`: "Socratic questions checked
against the core or the emulator, not against the model's memory").

A student also needs a reason to come back. So quizzes come with a light
game on top: points, a rank, achievements and streaks, tied to what the
student has shown they know. The corpus is the concept map Romlens already
has: 46 concepts in five groups, each at five levels (`25-lessons.md`),
230 cells in all.

## Scope decisions (30 September 2026)

1. **Questions come from Romlens and the tutor, and every answer is
   checked.** Romlens makes questions from its own tables and the open
   ROM: free, offline and marked with certainty. The tutor writes richer
   ones about the game, but each carries a claim Romlens checks before the
   question is asked. Free-text "explain why" answers are marked by the
   model, and say so.
2. **Learned, then proven.** A lesson still raises a concept's level: that
   is learned. Passing a quiz at a level proves it. The map shows both,
   and most points come from proving.
3. **Gentle fading.** A proven level comes due for review on a spaced
   schedule; a review keeps the streak going and earns a bonus. Levels
   never drop.
4. **Points for more than quizzes**: finding things in the ROM (per-game
   milestones checked against the analysis), reading lessons (finishing
   them, and guessing a predict question before Show), and streaks of days.

## Design

### Where things live

Everything is the student's, not a project's, as the learner record is
(`<root>/Learner/`, beside `lessons/` and `learner.json`):

- `quizzes/<id>.json`: a whole quiz, its questions and answers, written
  atomically.
- `journal.jsonl`: an append-only list of the facts with no other file,
  a predict question's guess and a milestone reached, one JSON object a
  line. A torn last line is skipped on reading.

**XP, proofs, review boxes, streaks, the rank and achievements are never
stored.** `Progress::derive(lessons, quizzes, journal, marks, now, offset)`
works them out each time, as `Learner::from` does the levels, so nothing
can drift. The inputs only grow, so the result is stable, and every point
has a cause: `romlens tutor progress --ledger` lists them. Deleting a lesson
takes back its points, as it takes back what it taught. "Reset progress"
removes the quizzes and the journal.

A quiz at level 3 or above, and a milestone, belong to a game: they keep
its SHA-256 and title.

Days are local days: the app and the CLI pass the local UTC offset.

`romlens-tutor` does not depend on the core, so the formats, the store and
the derivation are there (`quiz`, `progress`), and the generators and the
claim checks, which need the core's tables and the analysis, are in the
FFI (`quiz`), as a lesson's format is in the one and its mechanical checks
in the other.

### A question

A question has a concept and a level, the question, how it is answered,
an explanation that teaches, where the answer comes from (a `romlens://`
link or a table), an optional hint, and its source: Romlens (with the
generator's name), the tutor with a checked claim, or the tutor marked by
the model. The ways to answer:

| Kind | The student | Marked |
|---|---|---|
| Choice | picks one of two to five | by Romlens |
| Number | types an address, value or count (hex or decimal, `$` optional) | by Romlens |
| Bits | toggles bits over the register's fields | by Romlens, as a set |
| Line | picks a line of a routine's listing | by Romlens |
| Text | writes a sentence or two | by the model, and labelled so |

The answer is never sent to the window with the question; it comes back
with the result.

### Romlens's questions

Generators, each a function from a concept and level to a question with
its claim, drawn from:

| Family | Levels | Asks | From |
|---|---|---|---|
| line, gloss, fact | 1 (fact 1–2) | the idea; a term's meaning; a number (lines a frame, VRAM's size, sprites in OAM, DMA channels…) | the concept lines, the glossary, `timing` and a table of facts traced to the core |
| reg, bits, write, op, dsp, brr | 2 (write and brr also 5) | a register's address or job; a field's bits; what a value sets; the P flags, a branch's test, an addressing mode; DSP and BRR fields | `model::hardware`, `explain::fields`, `explain::sound`, the opcode table |
| header, vector, idiom, gamewrite | 3 | this game's mapping; where its NMI, reset or IRQ handler is; where it does its DMA, waits for vblank or uploads the sound driver; what it writes to BGMODE, OBSEL or NMITIMEN | the header, the vectors, the explanations (idioms, each write) |
| listing, width, valuein | 4 | the line that stores to a register or sets the data bank; whether A is 8 or 16 bits here; the value reaching a store | the listing, the analysis's flags, `explain::values` |
| bytes, dmamath | 5 | an instruction's length and encoding; a DMA's cycles and whether it fits in vertical blank | the opcode table, the ROM (four bytes at most, at run time), `timing` |

Each is seeded by the quiz, so the same quiz can be made again, and
each question goes back through the claim check before it is used: one
that fails is a Romlens bug, dropped and never shown.

Levels 1 and 2 are covered for every concept. Levels 3 to 5 depend on what
the game's analysis finds. With no generator yet: compression and objects
above level 2, and the SPC700 and the sound driver at levels 4 and 5. The
opcode table has no cycle counts, so level 5 asks about bytes and DMA
arithmetic, not an instruction's cycles. Where a cell has fewer than three
certain questions, the tutor is asked for checked ones; if none come, the
map says "Level N can't be proven in this game yet". It is said, never
hidden.

### The tutor's questions

`quiz_question` (quiz, question, kind, choices, answer, claim, explanation,
hint, citations). It is declared always, so the tool list and the cached
prefix never change, and refused except while a quiz is being written.
Each carries one claim, which Romlens checks against its own data:

| Claim | Checked against |
|---|---|
| a register field: register, value, field, meaning | `explain::fields` |
| a field's bits | `explain::fields` |
| a register's address | `model::hardware` |
| the address of a vector, a label or an idiom | the header, the labels, the idioms |
| the instruction at an address | the listing |
| the bytes at an address (four at most) | the ROM, at run time |
| the value reaching a store | `explain::values` |
| a DMA's channel, source, destination or size | the DMA idiom's transfers |
| an S-DSP field | `explain::sound` |
| a fact | the facts table |

The answer given must be the claim's, and no other choice may pass it. A
refusal is one line the model can act on ("INIDISP bit 7 is forced blank,
not brightness"). A Text question is taken only once the quiz has three
certain ones.

### A quiz

1. `/quiz [topic]`, a map chip's "Prove level N…", or Review. The topic is
   matched against the concepts' ids and names. The level is the lowest
   learned and not proven, or else one above the highest proven, to test
   out.
2. Romlens's questions at once: five to prove a level, two for each proof
   due (five proofs at most in a review). No model, no cost.
3. When the setting is on and a model is set up, or when Romlens has too
   few, the tutor writes up to two more on a copy of the conversation
   (`Session::write_quiz`, as a lesson's check does: the read tools and
   `quiz_question`, edits and lesson tools refused, within $0.10), its
   cost added to the conversation's. They arrive as the student answers.
4. Each answer is marked at once. Text answers are marked by the model on
   a copy (`Session::grade`, within $0.02), with a line of feedback.

### Marking, proving and reviews

- Right is 1; right after the hint is half; wrong or skipped is nothing.
  After each answer the right one, the explanation and its source show:
  that is the teaching. One try a question; another quiz, with new
  questions, can start at once.
- **Proven**: five questions, at least 4 of 5, and at least three certain
  (Romlens's or checked) answered right without a hint. The model's marks
  count toward the score, never toward those three. One proof per concept
  and level; a lesson is not needed first (testing out counts as learned).
- **Reviews** (boxes of 1, 3, 7, 21 and 60 days): a concept's highest
  proven level comes due. Two right moves it to the next box; one keeps
  it; none moves it back to the first. A proof never goes.

### Points, rank and achievements

| For | XP |
|---|---|
| Proving level L | 50 × L |
| A review passed | 10 + 5 × the box |
| A certain answer right | 2 (1 with the hint) |
| A lesson finished | 20, and 5 for each concept-level newly learned |
| A predict question guessed before Show | 3 (5 when the guess is checked and right) |
| A milestone in a game | 15 to 40 |
| The day's first quiz, lesson or guess | 5 |
| A streak of 7 / 30 days | 25 / 100 |

**Streaks** count local days with a quiz answer, a lesson finished or a
guess. One missed day in seven is bridged. A lost streak is never
mentioned; the best is kept.

**The rank** is what the student knows, not how much they have played:
it comes from the sum of the highest proven level of each concept (230 at
most). Power-on, Reset (a proof), First frame (every Machine concept at
level 1), In vblank (15 concepts at level 2), Tracer (10 at level 3),
Disassembler (15 at level 4), Cycle counter (10 at level 5), ROM reader
(all 46 at level 3), Hardware sage (all at level 5). Each group has a
standing, the lowest level proven across it ("Graphics 2").

**Achievements** (28: 19, and 9 for each game), each a rule over the same
inputs:

- Lessons: the first; five; Predictor (ten guesses before Show); Down to
  the metal (a lesson at level 4).
- Proofs: the first; Clean sheet (five of five, no hints); Tested out
  (proven without a lesson); One of each (a proof in every group); each
  group at level 2 (five); Bytes and cycles (a level 5 proof).
- Reviews: Came back (the first passed); Long memory (a proof in the
  60-day box).
- Streaks: 3, 7 and 30 days.
- Exploring, per game, checked against the project and the analysis and
  kept in the journal once reached: the reset handler named; the NMI
  handler named; the main loop named (the routine that waits for vertical
  blank); the sound upload named or commented; a routine that sends DMA to
  VRAM named; 10, 50 and 200 routines named; 25 lines commented. Names the
  tutor gives with the student's approval count.

**Not a trap.** No notifications, no "streak at risk", nothing ever taken
away, levels never drop, reviews due are a quiet count ("3 ready to
review"), no leaderboards, no quiz pressed on anyone. Settings › Tutor ›
Show points and achievements (on at first): off hides points, rank,
achievements and toasts, and keeps learned, proven and due on the map,
because those are about learning.

### Predict questions answered

A lesson step's predict question gets a "Your guess" field beside Show.
The first guess is kept in the journal and earns its points; the step
opens. A step may carry a claim for its answer (`lesson_step`'s
`predict_answer`): then a guess that reads as a number, an address or a
register is checked, and the card says "Right" or "Not quite". Without
one, nothing is marked. The next question tells the tutor what the
student guessed, so it can take up a wrong idea.

### The app

- **The quiz sheet**: a question at a time, with a control for each kind
  (buttons, a hex field, bit toggles over the register's fields, the
  listing's lines, a text box), Hint, Check, then the answer, the
  explanation and its links. Dots show how far through. The end says what
  was proven and when it comes back. "Writing 2 more questions…" while
  the tutor works. Each question says where it came from.
- **Progress**, a third tab beside Lessons and What you know: XP towards
  the next rank, the rank and each group's standing, the streak, reviews
  due with Review, the achievements (those not yet earned say how), and
  this game's milestones.
- **The map**: a chip's fill is the level learned, as now; a ring is the
  level proven; a clock means due. Its menu adds "Prove level N…" and
  "Practise".
- `/quiz [topic]`, `/review`, `/progress`; a small XP count and the number
  due in the status line; a banner for a proof, an achievement or a rank,
  gone after four seconds.

### CLI

- `romlens tutor quiz <rom> [concept] [--level N] [--review | --practice]
  [--seed S] [--answers "…"] [--no-tutor]`: asks on the terminal, or marks
  the answers given.
- `romlens tutor quiz coverage <rom>`: how many certain questions each
  concept and level has in this game.
- `romlens tutor progress [--ledger] [--json]`.

## Ordered tasks (one commit each, pushed to main)

| # | Task | Tests |
|---|---|---|
| Q0 | This document, the pointers, the checklist rows | — |
| Q1 | `romlens-tutor::quiz`: the format, claims as data, `grade`, `QuizStore`; `progress::Journal` | round trips, older files load; each kind marked; a torn journal line |
| Q2 | `romlens-tutor::progress`: `derive`, proving, reviews, XP, streaks, ranks, achievements (not exploring), `diff`; the learner record counts proofs | timelines: 4 of 5 with two certain fails; boxes move; a proof never goes; the ledger sums to XP; a streak over midnight at UTC+10; deleting a lesson takes only its points; every achievement reachable |
| Q3 | FFI `quiz`: the claim checks, the facts, the level 1–2 generators, coverage | every concept at levels 1–2 has three distinct questions; every answer checked again; no wrong choice passes; the same seed, the same quiz; each fact equals the core's |
| Q4 | The level 3–5 generators | on the test ROMs: the same; a coverage golden; no idiom, no question |
| Q5 | `TutorSession`: the quiz API, records, events, milestones | offline: start, answer, hint, finish, prove, the event; a question never carries its answer; a milestone stays |
| Q6 | `quiz_question`, writing and marking on a copy, the prompts | loopback: a claim that fails refused, a corrected one taken; a wrong choice that passes refused; Text before three certain refused; the model's marks labelled and not certain; the transcript unchanged; the cost capped and charged; the tool list unchanged |
| Q7 | CLI | a golden with `--answers --seed`; `--ledger` |
| Q8 | App: the quiz sheet, the commands, the map | a quiz proven in the app test rig, the ring shown; snapshots light and dark |
| Q9 | Predict guesses | only the first earns; the claim checked; the note carries the guess |
| Q10 | App: Progress, the status line, toasts, the setting, reviews | the setting off hides points, not proven or due; a review moves its box; snapshots |
| Q11 | Live runs, the user's | — |

## What is cut for now

- An instruction cycle table, and level 5 questions about cycles.
- Questions checked against a recording (the emulator): the claims are the
  static analysis's for now.
- Leaderboards, sharing, and progress synced between machines.
- Quizzes the student writes.

## Risks

- Levels 3 to 5 vary with the game: the gap is said, and the tutor's
  checked questions fill it.
- Questions that read like trivia: every generator's explanation must
  teach something, and that is the bar for adding one.
- A failed review that feels like punishment: it only moves the box, and
  says "Let's see it again soon".
- Local days at the edges: tested with offsets.

## Verification

- `cargo fmt --check`, clippy with `-D warnings`, `cargo test --workspace`,
  and the app's tests.
- Every question Romlens makes, for every concept and level and twenty
  seeds on the test ROMs, passes its claim check; every question the tool
  takes has passed it.
- Working out the progress again from the files gives the same result, and
  the ledger adds up to the XP. An older `learner.json`, older lessons and
  no journal all load.
- By hand: `romlens tutor quiz coverage` on a real game; a level proven
  with no model set up; a tutor's claim refused and then corrected, read
  with `romlens tutor show latest`; the setting off hides every score; a
  missed day says nothing.
- No ROM bytes in the repository or the goldens: the test ROMs are made by
  the tests, and a question quotes at most four bytes of the user's own
  ROM, at run time.
