# Vision and roadmap

## The one-sentence goal

Let a curious person open an SNES ROM, theirs or a classic, and *see* how it
works, from raw bytes, to assembly, to recovered structure, to an animated
explanation they could put in a video.

Romlens is a study and visualization tool first, not an emulator. Through
Phase 4 anything dynamic comes from a recording produced elsewhere
(`13-recording-format.md`). The natural next step after that is a debugger
visualizer with an embedded, permissively licensed core, and the views are
designed so that a live core and a recording are interchangeable sources.

## Who it is for

1. **You, first.** A tool for personal study of Super Metroid and, later, other
   ROMs. Every phase must be useful to one person before it is useful to many.
2. **Educators and video makers.** People producing content about historic
   game development who need accurate, attractive visuals of real code.
3. **Homebrew developers.** People writing their own SNES ROMs to run on
   real hardware or an emulator. They have source, symbols and a build; the
   tool shows them what their assembler produced, how their graphics reach
   the screen, and lets them record their own build to study it. Nothing in
   the app requires a commercial ROM.
4. **ROM hackers and disassembly maintainers.** A secondary audience whose
   existing tools (DiztinGUIsh, bsnes-plus, Mesen2, asar) define the file
   formats we should import and export.

## Guiding principles

- **Truth first, pretty second.** Every visual must be traceable back to bytes
  in the ROM. No hand-waving in the visualization layer.
- **Never guess silently.** Code/data classification is uncertain. The UI must
  always show *how* a region was classified (vector reach, trace log, heuristic,
  user) and at what confidence.
- **Native on every platform.** macOS first: a document-based app, multiple
  windows, keyboard navigation, Spotlight-style jump-to-address, the feel of
  Xcode or Hex Fiend. Windows and Linux follow with WinUI 3 and
  GTK4/libadwaita shells that feel like their platforms, not like a port.
- **Core is a portable library.** Everything that is not UI lives in a Rust
  core with a CLI and generated bindings, so it is testable and scriptable
  without any app, and identical on every platform (see
  `08-cross-platform.md`).
- **Ground truth drives quality.** Super Metroid has a public, complete
  human-written disassembly. We measure our classifier and disassembler
  against it and publish the number.

## Roadmap

Sizes are rough, assuming part-time solo work with AI assistance. Each phase
ends with something you would actually open and use.

### Phase 0 — Open and see (2–3 weeks)

Deliverable: a document-based macOS app that opens an `.sfc`/`.smc` and shows
a structured hex view with a correct address column, built on the portable
core from the first commit.

- Rust workspace: `romlens-core`, `romlens-ffi`, `romlens-cli`.
  First UniFFI definitions, XCFramework and Swift package build, CI matrix
  on macOS, Windows and Ubuntu running the core tests everywhere.

Acceptance criteria (agreed 21 September 2026):

1. Opens the development ROM and a homebrew test ROM assembled by the test
   suite itself.
2. Detects LoROM/HiROM/ExHiROM and copier headers; header fields, vectors
   and the mirrored checksum are shown and covered by tests pinned to the
   development ROM's facts.
3. Virtualized hex view scrolls smoothly through 3 MB with the dual address
   column and header/vector overlays.
4. Jump to address and the byte inspector work.
5. The core, FFI crate, generated Swift package and CLI exist; CI is green
   on macOS, Windows and Ubuntu.

Working agreements: direct commits to `main` for now; ROMs only in the
git-ignored `roms/` folder; macOS 27 minimum; the Anthropic key for the
tutor is always the user's own.

- Detect and strip a 512-byte copier header.
- Detect mapping mode (LoROM, HiROM, ExHiROM) by header scoring at `$7FC0`,
  `$FFC0`, `$40FFC0`.
- Parse the internal header: title, map mode, cartridge type, ROM/RAM size,
  region, developer, version, checksum and complement, native and emulation
  vectors.
- Verify checksum, including the mirrored-sum rule for non-power-of-two ROMs
  (Super Metroid is 3 MB and only validates with the last megabyte counted
  twice).
- Virtualized hex view (3 MB = 196,608 rows of 16 bytes; must scroll at 120 Hz).
- Address column shows both file offset and SNES address (`$80:841C`), with a
  toggle.
- Header fields and vectors overlaid on the hex as named, colored spans.
- Inspector for the selected byte(s): u8/u16/u24 little-endian, signed, as
  SNES address, as pointer target, as ASCII.
- Jump to file offset or SNES address (`⌘L`).

### Phase 1 — Disassemble (3–5 weeks)

Deliverable: a synchronized disassembly view with labels, cross-references,
and a saved project file.

- Full 65816 decoder: 256 opcodes, 24 addressing modes, correct operand
  widths under the M and X flags, emulation-mode awareness.
- Flag tracking: REP/SEP propagation along control flow, with per-address
  overrides when the analyzer is wrong.
- Recursive-descent analysis from the six native and six emulation vectors,
  following JSR/JSL/JMP/JML/branches, stopping at RTS/RTL/RTI/JMP/BRA/BRK.
- Linear sweep of leftover space, marked low-confidence.
- Auto labels (`CODE_80841C`, `DATA_...`), user renames, comments (line and
  block), cross-reference panel.
- Hex ↔ disassembly lockstep: selecting bytes in one highlights the other.
- Project file (`.romlens` package) storing ROM hash, mapping, labels,
  comments, region classification, flag overrides.
- Export: plain `.asm` in asar-compatible syntax, and a symbol file.
- **Tutor, Ask mode:** questions about the current selection answered
  through read-only tools with address citations (see `06-ai-tutor.md`).
  *Extracted and not built; deferred again past Phase 2.*

### Phase 2 — Discover code vs data (4–8 weeks)

Deliverable: a ROM map that is mostly right, with the evidence for each region
one click away.

- Import execution traces / coverage: Mesen2 CDL files, bsnes-plus usage
  maps, DiztinGUIsh projects. Executed bytes become high-confidence code;
  read-only bytes become high-confidence data.
- Heuristics with visible scores: entropy per window, pointer-table detection
  (runs of in-range 16/24-bit addresses), jump tables after `JMP (abs,X)`,
  ASCII runs, 2bpp/4bpp graphics signatures, palette signatures (15-bit BGR).
- Data typing by the user: byte, word, long, pointer (with bank rule), table
  with stride, struct, string, graphics, tilemap, palette, compressed blob.
- Previews: tiles at 2/4/8 bpp with a chosen palette, palettes as swatches,
  tilemaps rendered.
- Import public symbol lists so a known game becomes readable immediately;
  use PJBoy's Super Metroid logs to compute classifier accuracy.
- Import assembler symbol and listing files (asar, ca65, WLA-DX) so a
  homebrew developer's own labels appear in the disassembly from the first
  open.
- Region overview strip (minimap) colored by classification and confidence.
- **Recording import** from the Mesen2 recorder script we ship, plus
  single-frame imports from savestates.
- **Tile decoder, palette, OAM and tilemap views** working on raw bytes and
  on recording snapshots (see `07-memory-to-screen.md`).
- **Tutor, Fix mode:** reclassification, flag-override and data-type
  proposals rendered as cards the user accepts or rejects.

Scope decisions (22 September 2026), recorded in `16-phase2-plan.md`:
the tutor is deferred out of Phase 2 entirely, so Ask mode (carried over
from Phase 1) and Fix mode both move to Phase 3; the Atlas is reduced to the
overview strip, with the zoomable map joining Phase 3's CFG work; the
Windows and Linux shells stay unstarted. Phase 2 is sequenced as three
independently shippable tracks — 2A classification, 2B graphics, 2C
recordings — and only 2A fits the four-to-eight-week estimate above.

**Track 2A shipped on 22 September 2026.** Jump tables took the code map from
0.2% of Super Metroid to 2.1%; scored heuristics and imported traces took
unknown bytes from 99.8% to 82.9%. What changed against the bullets above:
DiztinGUIsh import is dropped rather than deferred, because the format was
never verified and Diz exports two that are read; PJBoy's logs are not used to
compute accuracy, because they are never redistributed
(`12-content-policy.md`) — `romlens truth from-cdl` builds truth from a
recording the developer makes themselves instead; ca65 `.dbg` moves to Phase 3,
where its line and span records have a source view to land in. Previews, the
graphics views and recording import are tracks 2B and 2C, still to start.

**Execution logs (23 September 2026).** Coverage says a byte was executed or
read; telling code from data reliably needs to know *what* read it and where
control went. The MesenCE fork now records an execution log — instructions
with their widths, each instruction's reads and writes, every control
transfer and every DMA — and Romlens imports it as a trace
(`17-execution-log.md`). It adds the references the game made, including
indirect calls no static walk can follow, and types ROM by where DMA sent it.
The next uses, in order: follow decompressor → RAM buffer → DMA chains to type
compressed sources, map RAM-resident code back to its ROM origin, and use the
recorded width states to settle flag conflicts.

### Phase 3 — Recover structure (8–12 weeks)

Deliverable: functions, control-flow graphs, call graph, and a first
pseudo-C rendering of a chosen routine.

- Function boundaries from analysis and user marks; basic blocks; CFG view.
- Call graph with drill-down; "who calls this" and "what does this call".
- Data-flow: register/flag state at each instruction, direct-page and data-bank
  assumptions, stack-relative locals.
- Lifting to an intermediate representation, then to readable pseudo-C for one
  function at a time. Not a whole-program compiler; a reading aid.
- Diffing two ROM versions (JP vs US, revision 0 vs 1).
- **Frame, Layers and Provenance views:** click a pixel in a recorded frame
  and walk back through OAM, VRAM, the DMA transfer and the decompressor to
  the ROM bytes.
- **Tutor, Investigate mode:** multi-step analysis over recordings with a
  visible plan and tool log.

### Phase 4 — Teach (ongoing)

Deliverable: exportable, animated explanations of real code.

- Recording scrubber: step a recording frame by frame with register and
  memory views, and instruction by instruction where the recording carries
  an execution trace. No game code runs inside Romlens in this phase.
- Scene system: a timeline of highlights, camera moves, callouts and register
  animations over the hex/asm/CFG views; scrubbable in the app.
- Export to video frames via AVFoundation, and to a scene description that can
  be rendered externally (Motion Canvas or manim style), for people who want
  to composite in their own pipeline.
- Notebook-style documents mixing prose, live views and scenes, publishable as
  HTML.
- **Live memory-to-screen:** the pipeline view updates while scrubbing a
  recording; DMA transfers animate from the captured channel state.
- **Tutor, Explain and Quiz modes:** walkthroughs that become scene drafts,
  and Socratic questions checked against the core or emulator.

## Platform tracks

macOS is the reference shell. The Windows (WinUI 3) and Linux
(GTK4/libadwaita) shells start once the core API has settled at the end of
macOS Phase 1, reach Phase 0–1 parity in roughly four to six weeks each, and
then track macOS phase by phase. A frontend conformance checklist in the
repo defines parity; CLI-scripted scenarios check it the same way on all
three. Details, boundary and idiom map in `08-cross-platform.md`.

## Two cross-cutting features

The AI tutor and the memory-to-screen pipeline each span several phases
rather than owning one. They have their own documents:

- `06-ai-tutor.md`: the tutor works only through tools over the core, cites
  every claim, and can only propose edits.
- `07-memory-to-screen.md`: graphics are taught as a chain from ROM bytes
  through decompression, DMA and PPU memories to pixels, with provenance
  from recordings making the chain walkable in both directions.

### Phase 5 — Debug (after Phase 4, the natural next step)

Deliverable: a debugger visualizer. An embedded, permissively licensed SNES
core (candidates in `09-emulator-core-licensing.md`) runs a ROM inside
Romlens as a live `MachineStateSource`, records to `.romrec` as it goes,
and adds breakpoints, watchpoints, stepping and memory editing on top of the
same views. Because every view already consumes recordings, this phase adds
a source, not a UI. Homebrew developers get an edit, build, run, inspect
loop in one window.

## What is deliberately out of scope for now

- Playing games. Even with the Phase 5 core, Romlens is a debugger, not a
  player: no controller mapping polish, no shaders, no netplay.
- General-purpose tile and sprite editors. We decode and explain; Mesen2 and
  the ROM hacking tools already edit.
- Other consoles. The core is designed with a `Platform` protocol so NES,
  Game Boy and Genesis can follow, but nothing is built for them until the
  SNES path works end to end.
- Automatic whole-program decompilation to compilable C. The goal is
  readable pseudo-C for study, not a rebuildable source tree.
