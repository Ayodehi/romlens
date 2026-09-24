# Explaining hardware writes and common idioms

## Progress

Written 24 September 2026 and kept as written. Only this section is updated to
track progress against it.

| Task | State |
|---|---|
| E0 this document, the roadmap pointer, the checklist rows | done |
| E1 the register field tables and `describe` | |
| E2 the value each store writes, `Explanations`, `romlens explain` | |
| E3 the idioms | |
| E4 the listing: explained comments, idiom notes, the toggle | |
| E5 the C: the same comments | |
| E6 FFI | |
| E7 macOS: the inspector's Explanation section, the toggle, note lines | |
| E8 measure and record | |

## Context

Romlens is for learning SNES development from real games. Most of what a game
does to the machine happens through writes to hardware registers. Right now
the listing only names them:

```
LDA #$81
STA $4200   ; NMITIMEN
```

That tells a student which register is written, but not what the write does.
They have to look up NMITIMEN, find bit 7 and bit 0, and work out that `$81`
turns on the vertical-blank interrupt and automatic controller reading. The
bits of every register are fixed by the hardware, and the value is often a
constant a few instructions back, so Romlens can do that work itself.

The same is true one level up. Every SNES game contains the same few
sequences: wait for vertical blank, set up a DMA channel and start it, clear a
block of RAM, hand bytes to the sound CPU, multiply with the hardware
multiplier. A student who can recognise them can read most of a game's setup
code. Romlens can name each one where it appears, fill in its concrete values,
and say why games do it that way.

Both are static: they come from the ROM and the analysis, not from a
recording. Both live in the core, so the listing, the inspector, the C tab and
the CLI all say the same thing.

## Scope decisions (24 September 2026)

1. **Decode what the value means, one field at a time.** A write to a register
   with a known value explains each field:
   `NMITIMEN = $81: NMI on, joypad auto-read on`. The inspector shows the full
   table (bits, field, value, meaning) and a paragraph on what the register is
   for.
2. **An unknown value still teaches.** When the value is not a constant, the
   explanation says where it came from, for example "INIDISP ← from $7E:0DAE",
   and lists the fields without values.
3. **Name the idiom, fill in the values, say why.** Each recognised sequence
   has a title ("Wait for vertical blank"), a summary with its values ("copy
   $0800 bytes from $7F:0000 to VRAM $6000 on channel 0"), and a short fixed
   paragraph on why SNES games do this.
4. **The first set of idioms:**
   - waiting for VBlank, NMI or HBlank;
   - a DMA transfer;
   - HDMA setup;
   - hardware multiply and divide;
   - clearing or filling memory;
   - the APU handshake;
   - BCD arithmetic;
   - a routine with more than one entry point.
5. **Everywhere the code is shown:**
   - The listing's automatic comment becomes the short explanation, and an
     idiom gets a note line above it (`; ▸ Wait for vertical blank`).
   - The C carries the same text as comments, and the code does not change.
   - The inspector gets an Explanation section.
   - The CLI gets `romlens explain`.
6. **A user's comment wins.** A line comment the user wrote replaces the
   automatic one, as it does today. View › Show Explanations turns the richer
   text and the note lines off.
7. **Our own words, checked.** Every field description is written for this
   project and checked against a public reference: fullsnes by Martin Korth,
   and the SNESdev wiki's register pages. The references are named here, not
   copied.

## What already exists

- `model/hardware.rs` has 200 registers, each with a name, access and one line
  of description. There are no fields and no widths.
- Register names are surfaced in three places:
  - `cpu65816/format.rs` `register_for` finds the register an instruction
    touches, and the listing's automatic comment
    (`viewmodel/asm_lines.rs::line_text`) is its name;
  - `InstructionInfo.hardware_register` feeds the inspector;
  - `decompile/emit.rs` `Namer::is_hardware` prints `INIDISP = 0x80;`.
- Nothing tracks register values across control flow:
  - the analysis's `FlagState` holds M, X, E, the data bank and the direct
    page;
  - `decompile/dataflow.rs` `merge_stores` keeps a per-block map of constant
    registers;
  - `decompile/lift.rs` has the one forward lattice (the decimal flag), the
    template for a constant pass.
- DMA appears only in recordings (`model/exec_log.rs` `DmaRun`,
  `analysis/observed.rs`), never statically.
- `decompile/loops.rs` `Counted` already finds counted loops such as
  `LDX #$0F; STZ $0200,X; DEX; BPL`.
- `decompile::function` finds each routine and its entries, and
  `decompile::cfg::Cfg` gives its blocks, reverse post-order and natural
  loops.

## Design

### Register fields (`explain::fields`)

A static table maps each register to its fields. A field has a mask and a
shift, a name, and one of three kinds of meaning:

- **A flag**, with text for on and off, for example NMITIMEN bit 7, "NMI".
- **A choice**, from a value to text, for example BGMODE bits 0–2, "mode 1:
  two 16-colour layers and a 4-colour one", or DMAPn's transfer pattern.
- **A number**, with an optional scale and unit, for example INIDISP
  brightness 0–15, BG1SC's tilemap address (`value & $FC` << 8 as a VRAM word
  address), or the size of a tilemap.

Coverage:

- every writable PPU register, `$2100–$2133`;
- the CPU's I/O registers, `$4200–$420D`;
- the status registers that polling loops read: RDNMI, TIMEUP and HVBJOY;
- the DMA channel registers, `$43x0–$43xA`, as one template for eight
  channels.

The APU ports, the WRAM port and the joypad registers keep only their
description, since their bytes mean whatever the program says.

`describe(register, value, width)` returns a `RegisterWrite`:

- `short`: at most about 60 characters, for comments;
- `fields`: the rows (bits, name, raw value, meaning);
- `long`: a paragraph for the inspector.

Some writes need extra handling:

- **16-bit stores cover two registers.** `STA $4300` with 16-bit A writes
  DMAP0 and BBAD0, and both are decoded.
- **Write-twice registers.** BGnHOFS and BGnVOFS, the mode 7 matrix and
  CGDATA take two writes in a row. They are explained as the first or second
  write when the order is known in the block, and generically otherwise.
- **Address pairs.** VMADDL/H, the DMA channels' A1TnL/H/B and DASnL/H are
  explained together when a 16-bit store covers them.

### The value a store writes (`explain::values`)

This is a forward constant pass over one routine, using
`decompile::function::discover` for the steps and `Cfg` for the blocks, with
widths taken from the analysis (`decode_at`).

- Each of four places holds `Known(value)` or `Unknown`: A's low byte, A's high
  byte (B), X and Y. The pass joins them where blocks meet and repeats over
  reverse post-order until nothing changes.
- It models:
  - immediate loads;
  - loads from memory, which give Unknown but remember where the value came
    from;
  - the transfers and `XBA`;
  - increment and decrement;
  - `AND`, `ORA` and `EOR` with an immediate;
  - shifts of A;
  - `PHA`/`PLA` pairs within a block;
  - `STZ`, which stores 0.
- Calls and anything it does not model make what they change Unknown.
- `Value { known, source }`, where `source` is "loaded from $7E:0DAE" (with its
  name, if it has one) or "the loop counter X".

### Idioms (`explain::idioms`)

`Idiom { kind, offsets, title, summary, why }`. Each recogniser works on a
routine's blocks, loops and values:

1. **Wait for VBlank, NMI or HBlank.** A one-block loop that reads HVBJOY or
   RDNMI and branches on the bit, or a loop around `WAI`.
2. **DMA transfer.** A channel's registers written, then MDMAEN.
   - The destination comes from the B-bus address:
     - `$18`/`$19` is VRAM, at a known VMADD;
     - `$22` is CGRAM, at a known CGADD;
     - `$04` is OAM.
   - The summary reads: "copy $0800 bytes from $7F:0000 to VRAM $6000 on
     channel 0".
   - A fixed-source transfer is a fill.
   - Several channels started together are listed together.
3. **HDMA setup.** A channel's registers written, then HDMAEN: "a per-line
   table at $xx:xxxx drives BG1HOFS on channel 3".
4. **Hardware multiply and divide.** The operands are written to WRMPYA and
   WRMPYB, or to WRDIVL, WRDIVH and WRDIVB. Then RDMPY or RDDIV is read. The
   explanation includes the cycles the hardware needs before the result can be
   read.
5. **Clear or fill memory.** A counted loop storing a constant through an
   index: "clear $0200–$020F". `MVN` and `MVP` block moves are also covered.
6. **The APU handshake.** A loop comparing an APUIO port with a value: "wait
   for the sound CPU to answer".
7. **BCD arithmetic.** `SED … ADC/SBC … CLD`: decimal digits, as in a score,
   a timer or a lives display.
8. **More than one entry point.** A routine whose body holds another routine's
   entry, or that other routines jump into partway: "F606 sets $7D, then falls
   into F60A".

### `Explanations`

`Explanations::build(rom, project, snapshot)` runs the value pass and the
recognisers over every routine once. It gives two maps from file offsets:

- to register writes;
- to the idioms that cover them.

The listing, the inspector and the C read from these maps. They are rebuilt
when the analysis or the project's names change. An instruction outside every
routine gets a scan within its own block.

### Surfaces

- **Listing:**
  - The automatic comment becomes the short explanation.
  - An idiom's first instruction gets a note line, a new `LineKind::Note`.
  - With Show Explanations off, the listing reads as it does today.
  - The asar export keeps only the register name.
- **C:** a comment after each explained store, and a block comment before an
  idiom's statement or loop. The code itself does not change.
- **CLI:**
  - `romlens explain <rom> [--project P] <address> [--json]` explains the
    instruction there, and the idioms it belongs to.
  - `--routine` explains a whole routine.
  - `romlens registers <address>` lists the register's fields.
- **FFI:**
  - records `RegisterWriteInfo` and `IdiomInfo`;
  - `Workbench::explain_at(file_offset)` and `explanations_in_routine(entry)`.
  - `API_VERSION` 0.7.0.
- **macOS:**
  - The inspector gets an Explanation section after Instruction. It shows the
    write's fields in a grid, the idiom's title, summary and "Why games do
    this", and where an unknown value came from, as a link.
  - View › Show Explanations.
  - Clicking a note line selects the idiom's instructions.

## Ordered tasks

Each task is a commit. Sizes are in days.

| # | Task | Size |
|---|---|---|
| E0 | This document, the roadmap pointer, the checklist rows | 0.5 |
| E1 | `explain::fields` and `describe`; `romlens registers` lists fields; tests on hand-checked values | 2 |
| E2 | `explain::values`, `Explanations`, `romlens explain`; goldens on the fixtures and a new one with DMA, a VBlank wait and a multiply | 2 |
| E3 | `explain::idioms`: each kind with a test that matches and one that must not | 3 |
| E4 | The listing's comments, note lines and the toggle; goldens refreshed | 1 |
| E5 | The C's comments; the C still passes `cc -fsyntax-only` | 1 |
| E6 | FFI records and exports, API 0.7.0, a RomlensKit test | 1 |
| E7 | macOS: the Explanation section, the toggle, note lines; app tests | 2 |
| E8 | Measure and record: how many hardware stores have a known value, the idioms found by kind, the build time; check RESET, `SUB_008E1A` and the NMI handler in Super Mario World by hand | 0.5 |

## What is cut for now

- **Reads in general.** Only the status registers that polling loops read are
  covered.
- **Values through calls.** A call makes what it changes Unknown.
- **Rewriting the C into helpers** such as `dma_copy(…)`. Explanations are
  comments only.
- **Explanations from the tutor.** Everything here is fixed text.
- **The Phase 4 scenes and animations.**
- **SA-1 and Super FX registers.**

## Risks

- **A wrong explanation teaches the wrong thing.** Every field is checked
  against a named reference, tests pin decoded values, and E8 checks the Super
  Mario World routines already studied by hand.
- **Unknown values may be common.** E8 measures how often the value is known.
  The source ("from $7E:0DAE") keeps the unknown case useful.
- **Longer comments crowd the listing.** The short form is capped, and the
  toggle brings back the terse listing.
- **Note lines move line numbers.** The line index is already rebuilt whenever
  the analysis or the project changes. The app's selection tests run with
  explanations on and off.

## Verification

- **Unit tests** (`tests/explain.rs`):
  - field decodes;
  - the value pass: joins, calls, 8- and 16-bit A;
  - each idiom, matching and not matching.
- **CLI goldens** for `explain`, `registers`, `disasm` and `decompile`,
  refreshed with `UPDATE_GOLDEN=1` and every diff read.
- **A development-ROM test** (`ROMLENS_ROM_DIR`): `Explanations::build` over
  every routine, with no panic and the counts reported.
- **Gates:** `make test`, `make swift`, `make app-test`.
- **The manual pass** in `15-conformance-checklist.md`.
