# The glossary: the SNES's acronyms spelt out

## Progress

Written 29 September 2026 and kept as written. This table is the only part
that tracks progress against it.

| Task | State |
|---|---|
| G1 the glossary in the core, the CLI and the tutor's `reference` | done, 29 September 2026 |
| G2 app: terms linked in answers and lessons, the bubble | done, 29 September 2026 |
| G3 live runs | ready, 29 September 2026, for the user's runs in the app |

## Context

SNES writing is full of acronyms and initialisms: DMA, HDMA, OAM, CGRAM,
VRAM, NMI, BRR, and a register name in nearly every sentence (VMAIN,
INIDISP, NMITIMEN). A beginner stops at each one. The tutor spells some out,
but not every time, and a lesson read again a week later has lost the
answer where it did.

## What it is

- **The entries** (`romlens_core::explain::glossary`). Each has the term,
  its other spellings (`V-blank`, `vblank`), the words it stands for, and one
  or two sentences on what it is. The general terms (the chips, the
  memories, DMA and the blanks, the TV standards, the 65816's registers,
  BRR and ADSR) are written by hand, checked against the primer (`04`) and
  the register tables. The hardware registers come from
  `model::hardware` (their names and words) and the first sentence of their
  field layout (docs/20), with the address and whether they are read or
  written. The S-DSP registers come from `explain::sound`. A register whose
  name is a term keeps the term's entry. A test holds every entry to two
  sentences and every spelling to one entry.
- **The CLI**: `romlens glossary` lists the terms, `--registers` the
  registers too, and `romlens glossary <term>` prints one entry.
- **The tutor**: the `reference` tool's `glossary` topic gives a term's
  entry, or all the terms, so the tutor can use the same words the bubble
  will show.
- **The app**:
  - `Glossary` builds one pattern from every spelling, longest first
    (so `DSP-1` wins over `DSP`). In the tutor's prose and table cells,
    each term is linked the first time a message (an answer, or a lesson
    step) uses it, so the page is not a sea of links. Inline code that is
    just a term (`` `VMAIN` ``) is linked. Code blocks, links already in the
    text, and hex such as `$00:DMA0` are left alone. A term's link has a
    dotted underline, apart from the links to addresses and frames.
  - A click shows a bubble (an `NSPopover`) pointing at the word: the term,
    whether it is a glossary term, a register or an S-DSP register, the words,
    the sentence or two, its other spellings, and "Ask the tutor about …",
    which puts "Tell me more about …" in the composer without sending it.
    A click anywhere else closes it.

## Also in this change

Stepping through a lesson points the main window at each step's place, as
before, but no longer brings it forward: the tutor window stays in front.
A citation or "Show … in Romlens" that the student clicks still brings it
forward.
