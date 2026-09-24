# What the screen is set up to be

## Progress

Written 24 September 2026 and kept as written. Only this section is updated
to track progress against it.

| Task | State |
|---|---|
| S0 this document, the roadmap pointer, the checklist rows | done |
| S1 the registers reaching an instruction, through calls | |
| S2 the setup, decoded; `romlens screen` | |
| S3 where VRAM and the palette were filled from | |
| S4 FFI | |
| S5 macOS: the Screen section | |
| S6 measure and record | |

## Context

The explanations (`20-explanations.md`) say what each write to a register
does. A routine that sets up a level, such as Super Mario World's RESET or
the code that loads a stage, writes thirty or more PPU registers, and each
comment describes one of them. What a student wants is the combined picture
those writes produce. Which background mode is it? Where are each layer's
tilemap and tiles? Which layers are on? Where do the sprites come from?

The registers' values are often constants in the code, so Romlens can work
this out without running the game. It can also say where the graphics in
VRAM came from, because the DMA transfers that fill VRAM are already
recognised. That links the code being read to the tiles and palettes in the
graphics viewers.

## Scope decisions (24 September 2026)

1. **At an instruction.** The setup is what the registers hold when that
   instruction runs, as far as the code before it on every path says. It
   is shown in the inspector's new Screen section and by
   `romlens screen <rom> <address>`.
2. **Through calls.** Setup code calls helpers that write registers, so a
   call counts as the writes its routine makes on every way out of it.
   A register a routine writes on only some paths is "set differently on
   each path", never a guess.
3. **What it covers:**
   - the display (forced blank, brightness, interlace and overscan);
   - the background mode;
   - for each layer the mode has: its colour depth, tilemap address and
     size, tile address, tile size, and whether it is on the main screen
     and the sub screen;
   - the sprites (sizes, tile addresses, main and sub screen);
   - colour math;
   - interrupts.
4. **Every value says where it came from.** Each value names the
   instruction that set it, which can be selected. A value can also be a
   variable ("from $7E:0D9F"), set differently on different paths, or not
   set on the way here.
5. **Where VRAM was filled from.** The DMA transfers to VRAM and the
   palette that the idioms find, with known destinations and sources, are
   indexed. A layer's tiles or tilemap names the transfer that uploaded
   them, and its source in ROM opens in the Tile Decoder or Tilemap viewer
   with the layer's colour depth. The palette opens in the Palette viewer.
6. **Static only.** No recording is needed. With a recording, the graphics
   viewers already show the real VRAM, which is the next step and out of
   scope here.

## Design

- **`explain::setup`, the registers reaching an instruction.** This is a
  walk back through the routine's blocks, like the idioms' walk. Where
  paths meet it keeps what they agree on, and marks as "varies" a register
  set on some paths and not others. At a call it takes the callee's
  summary, then carries on back past it.
  - A routine's summary is its register writes at each `RTS`, `RTL` or
    `RTI` and each tail call, joined.
  - Summaries are memoised, and a routine already being summarised (a
    cycle) counts as writing nothing known.
- **`ScreenSetup`, the decoded picture.** It is built from those
  registers, using `explain::fields` for the words. Each row has a label,
  its text, the instructions that set it, and optionally a link to a
  graphics view.
- **Uploads.** An `Idiom` for a DMA gains a structured `Transfer`
  (destination, source, byte count). `Explanations` indexes the VRAM and
  palette uploads by destination range.

## Ordered tasks

| # | Task | Size |
|---|---|---|
| S0 | This document, the roadmap pointer, the checklist rows | 0.2 |
| S1 | `explain::setup::registers_at` with call summaries; tests on the explain fixture and one with a helper routine | 1 |
| S2 | `ScreenSetup` and its rows; `romlens screen`; goldens | 1 |
| S3 | `Transfer` on DMA idioms, the upload index, links on rows | 0.5 |
| S4 | FFI records and `Workbench::screen_at`, a RomlensKit test | 0.5 |
| S5 | The inspector's Screen section: rows that select their instruction and buttons that open the viewers; app tests | 1 |
| S6 | Check Super Mario World's RESET and level loading by hand; record | 0.3 |

## What is cut for now

- **Scroll positions.** They are written twice and usually come from
  variables.
- **Window shapes.**
- **HDMA's line-by-line changes.**
- **Anything a recording would show.**

## Risks

- **Summaries through deep call chains could be slow.** They are memoised
  per request and capped in depth.
- **A register set before this routine was called** shows as "set before
  this routine". That is the truth, and it tells the student where to
  look next.

## Verification

- **Unit tests:** a fixture's setup, a helper routine's writes seen
  through the call, a register set on only one path.
- **Goldens:** `romlens screen` output.
- **The app:** Super Mario World's RESET and its level loading, checked by
  eye.
- **Gates:** `make test`, `make swift`, `make app-test`.
