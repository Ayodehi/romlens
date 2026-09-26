# Finishing Phase 3

## Progress

Written 25 September 2026 and kept as written; this section is the only part
that tracks against it.

| Task | State |
|---|---|
| F0 this document, the roadmap pointer, the checklist rows | done |
| P1 the compositor | done, 25 September 2026: `graphics::compose` draws every BG mode but hi-res (6 and 5), with scroll, 16×16 cells, offset-per-tile, mosaic, Mode 7's transform, sprites with the hardware's 32-sprite and 34-tile limits and rotated OAM priority, the main and sub screens, both windows and the colour window with their logic, colour math and brightness, and keeps what drew every pixel (`Winner`: the tilemap entry and tile, or the OAM entry). Lines come from a `LineSource`: the recorder (stream version 2) now logs every PPU register write with its scanline and dot, DMA's port writes included, and the DMA context (`VMADD`, `CGADD`, the OAM and WRAM port addresses, `K:PC`), all on stock Mesen; `recording::lines::Replay` plays a frame's writes over the previous frame's end, registers and memories both. New layer chunk `LINE`, `WLOG` kind 3, format 1.1. `romlens render frame --rec R --frame N [--at x,y] [--against dump]`. Checked with `scripts/oracle/run.sh` against Mesen's own screen: 101 frames of ten games (Super Metroid, Super Mario World, F-Zero, Chrono Trigger, Secret of Mana, Super Mario Kart, Donkey Kong Country 2, Street Fighter II, ActRaiser, Kirby's Avalanche) match pixel for pixel, and every one of 38,400 recorded frames replays exactly to its own snapshot. What the checks found, in order: brightness 0 is black; the scroll latch keeps two bytes; memory changes part way down the screen (Street Fighter II uploads in a forced blank at line 215); and Mesen's test runner skips drawing frames unless `--snes.disableFrameSkipping=true` |
| P2 the Frame and Layers views | done, 25 September 2026: Graphics › Frame and Layers. Frame draws the recording's screen at 1–4×, line by line where the recording has the writes; the status line names what drew the pixel under the pointer (the tilemap entry and tile, or the sprite, the pixel in the tile, its colour); a click keeps it, with buttons to its OAM entry, tile, tilemap cell and colour in the other views. Layers shows each background the mode has and the sprites alone, with the mode's order front to back. The frame stepper gains a scrubber. Live sessions keep each frame's line writes too. FFI: `RecordingSession::render_frame`, `frame_pixel` (a lookup in the kept frame), `render_frame_layer`, `priority_order`; `API_VERSION` 0.8.0. Tests: Rust, RomlensKit and two app tests (101 pass) |
| P3 VRAM, CGRAM and OAM words to the DMA that wrote them | done, 25 September 2026: `provenance::last_write` finds the last write to a byte of VRAM, CGRAM or OAM at or before a frame, searching back to the frame the change index says it last changed, and replaying only frames that wrote that port. `provenance::attribute` assigns a frame's port writes to its DMA starts: channels in order, each its count of bytes in its mode's register pattern, so each byte names its channel, its place in the transfer and the exact A-bus address it came from; the rest are the CPU's, or HDMA's in a horizontal blank. `pixel_parts` lists what a pixel was drawn from (its tile's row, its tilemap or OAM entry, its colour). `romlens provenance --rec R --frame N --at x,y [--rom ROM]`. On Super Metroid's gameplay: Samus's tile row came by DMA channel 1 started at `$80:93B3` straight from ROM `$9C:E264` (file 0x0E6264), her OAM entry from WRAM `$00:0370` and her colour from `$7E:C18A` by the DMAs at `$80:9372`; a background tile was written by the CPU through the port (the decompressor, for P4), 0.2 s. A synthetic stream tests both ends (`tests/provenance.rs`) |
| P4 WRAM buffers to the code that filled them, and to ROM | done, 25 September 2026: `provenance::source`. The execution log names the code that writes a WRAM buffer or a PPU port (`code_writing`), narrowest writers first so the loops that clear all of memory come last, marked. `rom_source` finds a run of bytes in the ROM as it is, or inside a Super Metroid LZ stream the log saw read: the ROM the code read is merged into intervals (a decompressor reads headers and literals with different instructions), and streams stored back to back are walked one after another; `sm_lz::decompress_traced` names the stream byte behind each output byte. The run searched is the whole stretch written in one pass with the pixel's byte (`write_run`), which a single common tile is not; then the tile or palette row alone. `provenance::chain` puts a pixel's whole chain together as data, for the CLI (`romlens provenance … --rom ROM --log session.mxlog`) and the inspector. On Super Metroid's gameplay, a background tile the CPU wrote to VRAM itself comes out as the 973 bytes decompressed from the stream at `$BA:C629` (file 0x1D4629, 0x32E8 bytes), the tile's row from its byte at `$BA:D06A`; Samus's palette row is in the ROM as it is at `$8D:DB6B` (one of 8 places); 1.3 s with the log's 253 streams |
| P5 the provenance chain in the app | done, 25 September 2026: the Frame view's pixel panel gains "Where it came from", worked out off the main thread (`Workbench::pixel_provenance`, with the project's execution log and the recording's change index). Each part lists the writes behind its bytes (Show the Code goes to the store that started the DMA; Show the Bytes to a DMA's ROM source), then the code that writes the buffer or the port (each instruction a button), and where the bytes are in the ROM: Show the Stream, Open Decompressed (into the Tile Decoder) and Mark as Compressed Graphics, a proposal applied through the undoable project edits. The wording is the core's (`chain::Link::describe` and the rest), so the CLI and the app say the same. The region is marked through the normal edit path; recording the evidence as the mark's origin waits for the tutor's Fix mode (T3), which adds accepted proposals. Tests: Rust, and an app test (102 pass) |
| P6 the recorder records where each DMA went (MesenCE) | done in P1 on stock Mesen: the DMA context and the line log need no fork change. The fork is updated only if P3 or P4 find something Mesen's Lua cannot give |
| A1 the Atlas in the core | done, 26 September 2026: `region_summary::summarize_window` gives the columns of any window (the whole map is the window of the whole image, so a window's columns are the whole map's there); `viewmodel::atlas::call_arcs` sums the calls (`JSR`, `JSL`) between the window's columns, an end outside the window as before or after it, most first; `atlas::items` lists the listing's own instructions and data rows starting in a window, up to a limit. `romlens map --from A --len N [--arcs] [--items N]`. FFI: `region_map_window` (the strip's batch layout), `atlas_call_arcs`, `atlas_items`; `API_VERSION` 0.9.0. Tests: Rust, an FFI test and a golden |
| A2 the Atlas in the app | done, 26 September 2026: View › Atlas (⌥⌘A), an editor tab drawn in one Core Graphics layer. Each bank is a row, fitted to the width; pinch, ⌘-scroll or ⌘= / ⌘− zoom continuously where the pointer is, rows growing taller up to a limit, and Zoom to Fit returns to the whole ROM. Columns come from the core a window at a time (`region_map_window`, a column a pixel); from 4 pixels a byte each instruction and data row is outlined (`atlas_items`). Overlays: kind (faint where a guess, hatched where blended), confidence, entropy and coverage. Hovering names the byte, its item or column, and draws the calls into and out of the columns near the pointer (`atlas_call_arcs`, bucketed a pixel wide). A click selects, the selection shows and is scrolled to from the other views, and a double-click opens the listing there. Tests: two app tests (104 pass); checked by eye on Super Metroid |
| D1 comparing two ROMs in the core, `romlens diff` | done, 26 September 2026: `diff::compare` over two sides (ROM, project, snapshot). Bytes: the first image's unique 32-byte windows anchor a rolling-hash scan of the second, each hit grown both ways; the chain of anchors covering the most bytes in order in both is the alignment (a Fenwick tree over the first image's order), the rest of 64 bytes or more are moved blocks; between anchors, gaps of equal length are compared in place (agreements under 4 bytes stay inside the change) and others trimmed at both ends to changed, inserted or deleted, so an insertion shifts what follows without changing it. Routines: fingerprinted by their instructions with ROM addresses masked (long operands that map to ROM, absolute operands from $8000), paired by fingerprint (same or moved), then by entry, then by where the alignment carries the entry (changed), the rest added or removed; a changed pair gets an instruction diff (longest common subsequence, a removal next to an addition read as one changed line). Data: the non-code regions with changed bytes, with their names. `romlens diff A B [--project-a P] [--project-b P] [--routines] [--json]`. Super Metroid against the four VARIA ROMs: about 125,000 bytes changed in place, none inserted, 7 moved blocks; 753–755 routines the same, 38–40 changed, 86–92 added, 6 removed; 0.1–0.3 s. One of the changed routines, `SUB_80AE7E`, is the faster door transition (`ADC #$0004` to `#$0008`, `CPX #$0040` to `#$0020`). Tests: the aligner's cases, the fixture pair (`fixtures::diff_pair`: a patched routine and 64 bytes inserted in a table), a golden |
| D2 comparing two ROMs in the app | to do |
| L1 stack-relative locals and arguments | to do |
| S1 ca65 `.dbg` import | to do |
| S2 the Source view | to do |
| T1 the tutor's client, key and tool loop | to do |
| T2 Ask mode | to do |
| T3 Fix mode | to do |
| T4 Investigate mode | to do |
| M measure and record | to do |

## Context

Phase 3's deliverable is "functions, control-flow graphs, call graph, and a
first pseudo-C rendering of a chosen routine" (`01-vision-and-roadmap.md`).
Functions, the Graph tab (`19-graphs.md`), data flow, the pseudo-C
(`18-decompiler.md`) and the explanations (`20`, `21`) are built. Six items
of the phase are not:

1. **Frame, Layers and Provenance views** (`07-memory-to-screen.md`): click
   a pixel in a recorded frame and walk back through OAM or the tilemap,
   VRAM, the DMA that filled it and the code that built its source, to the
   ROM bytes.
2. **The Atlas** (`02-ux-proposal.md`, option C): a zoomable map of the
   whole ROM. It moved here from Phase 2; only the overview strip exists.
3. **Comparing two versions of a ROM.**
4. **Stack-relative locals** in the data flow.
5. **ca65 `.dbg` import**, moved here from Phase 2 so its line records have
   a source view to land in.
6. **The tutor**: Ask mode (from Phase 1), Fix mode (from Phase 2) and
   Investigate mode (`06-ai-tutor.md`).

They are ordered by what they teach and by what they build on. Provenance
comes first: it is the centre of `07-memory-to-screen.md` and uses nearly
everything built so far. The tutor comes last so that it has the other
five to call as tools.

## What already exists

- **Recordings** (`recording/`): `MachineStateSource` gives, at each frame
  boundary, the CPU and PPU registers, WRAM (on keyframes by default), VRAM,
  CGRAM and OAM. The change index answers "when did these bytes last
  change". The Mesen pack writes a `WLOG` DMA record per `$420B` write (the
  eight channels' `$43x0–$43xB` and the scanline), but nothing reads it back
  outside the validator, and it has no PC and no VRAM, CGRAM or OAM address.
  No recorder writes a framebuffer.
- **The renderer** (`graphics/render.rs`, `mode7.rs`): one BG layer's whole
  tilemap or one sprite at a time, with no scroll, priority, main screen or
  compositing. Its header says "Phase 3 adds `compose(…)`".
- **The execution log** (`model/exec_log.rs`): aggregated per relationship,
  with counts but no frames. `AccessRun` says which PCs wrote which WRAM
  ranges; `DmaRun` has the source and B-bus register but not the VRAM
  address.
- **The decompressor** `graphics::compress::sm_lz`, verified against the
  game's routine at `$80:B119`.
- **The app's graphics views** (Tile Decoder, Palette, OAM, Tilemap) with a
  frame stepper over a recording or a live session.
- **Region data** (`Region`, `EntropyProfile`, `Coverage`) and
  `region_summary::summarize`, which always covers the whole ROM.
- **Call data**: `snap.xrefs_by_target` for every call; `graph::calls` per
  routine.
- **Projects** are keyed to one ROM by SHA-256; nothing compares two ROMs
  except `recording::delta::diff` over equal-length byte runs.
- **Stack-relative operands** lift to `MEM(S + k)`; `stack_slots` gives up
  on any routine that addresses the stack directly.
- **Symbol import** (`io/import/symbols.rs`) reads WLA, no$sns and VICE/ca65
  label files through `Project::apply_batch` with `Origin::Import`.

## Scope decisions (25 September 2026)

1. **Everything is static or from a recording.** No game code runs in
   Romlens (Phase 5 adds that).
2. **The frame is drawn by Romlens** from the recorded PPU memories and
   registers, as `07-memory-to-screen.md` leans. It is labelled "drawn from
   the PPU state", not "captured". A framebuffer layer is not added.
3. **Every link in the chain says how it was found**, as docs/07 asks:
   *exact* (from a record that names it), *matched* (the bytes agree and
   one candidate fits), or *inferred* (the best of several). The app shows
   which.
4. **The compositor covers what the lesson needs**:
   - modes 0 and 1, plus mode 7's plane at the identity transform;
   - scroll, tile size, the main screen's layer enables;
   - priority as each mode orders it, with sprite priority and the first
     sprite winning among sprites.

   Colour math, windows, hi-res, interlace and mosaic are cut; the Frame
   view says when the PPU state uses one of them.
5. **The recorder change is optional.** P3 works on today's recordings by
   matching bytes. P6 adds the VRAM, CGRAM and OAM address and the PC to
   each `WLOG` record in the MesenCE fork, issue → PR → squash-merge,
   without naming Romlens there. The recorder keeps working on stock Mesen.
6. **Comparing ROMs pairs two projects**, each keyed to its own ROM. The
   comparison works whether or not the two ROMs are the same size.
7. **The tutor uses the user's own Anthropic key** in the Keychain, with the
   loop in the Rust core as `06-ai-tutor.md` designs. Only the selection and
   tool results leave the machine. Its model and API details are checked
   against the current documentation when T1 starts, not taken from this
   document.
8. **macOS only**, with every capability also in the CLI (`08`, rule 7).

## Design

### Provenance

- **`graphics::compose`** (P1): `compose(ppu, vram, cgram, oam) ->
  Composed { bitmap, winners }`, where `winners[y][x]` names the layer and
  what drew the pixel:
  - a BG tilemap cell (layer, tilemap word address, tile number, palette,
    the pixel in the tile);
  - a sprite (OAM index, its tile, the pixel in it);
  - or the backdrop.

  Built on the existing BG and sprite decoders. Tested against fixtures and
  checked against Mesen screenshots of Super Metroid and Super Mario World
  frames.
- **The Frame and Layers views** (P2): a new graphics view with the
  composed frame over the frame stepper. Hovering names the winner.
  Clicking selects its OAM entry or tilemap cell in the existing views,
  opens its tile in the Tile Decoder, and selects its palette row. Layers
  shows each enabled layer alone, in priority order, with the mode's rules
  as a legend.
- **Backwards through VRAM** (P3): `provenance::vram_source(src, frame,
  word)`, and the same for CGRAM and OAM:
  1. The change index finds the frame the word last changed.
  2. The `WLOG` records of that frame are read (a new public reader).
  3. The DMA whose destination register is VMDATA (or CGDATA, OAMDATA),
     and whose source at that frame holds the same bytes at the right
     offset, is the writer. With P6's address this is *exact*; otherwise
     *matched* or *inferred*.
  4. A write with no DMA is named as CPU writes to the port.
- **Backwards through WRAM to ROM** (P4). For a DMA source in WRAM:
  - The execution log's `AccessRun`s name the code that writes that range.
  - If that code is a known decompressor (Super Metroid's `$80:B119`,
    found by its reads from ROM and checked by running `sm_lz` on the
    candidate source), the chain ends at the compressed bytes in ROM, with
    the offset of the byte that produced the pixel's tile.
  - A DMA straight from ROM ends there directly.
- **The chain in the app** (P5): a Provenance section in the inspector
  when a pixel, tile, OAM entry or VRAM range is selected. It is a
  breadcrumb from pixel to ROM byte; each step is a link, with its evidence
  and confidence. Following the chain to ROM also marks the region
  "graphics (compressed)" as a proposal the user accepts, with the evidence
  "provenance from recording X".

### The Atlas

- **In the core** (A1):
  - `summarize` gains a window (`start`, `len`, `buckets`), so zoom is a
    query, not a resample.
  - Call arcs come from the snapshot's call cross-references, aggregated to
    the zoom's buckets.
  - Level 2 (single instructions and data items) reuses the line index.
- **In the app** (A2): an Atlas editor tab drawn with Core Graphics in one
  layer, like the Graph tab.
  - Banks as a grid.
  - Continuous zoom by pinch and ⌘= / ⌘−.
  - Overlays for kind, confidence, entropy, execution coverage, and call
    arcs on hover.
  - Double-click opens the listing there.
  - The selection is shared with the other views.

### Comparing ROMs

- **In the core** (D1): `diff::compare(a, b)` over two sessions (ROM,
  project and snapshot):
  1. **Bytes:** runs of equal bytes, and moved blocks found with anchors (a
     rolling hash over 32-byte windows), so an insertion does not mark
     everything after it as changed.
  2. **Routines:** each routine is fingerprinted by its instructions, with
     absolute operands masked. Routines are paired across the versions as
     same, changed (with an instruction-level diff), moved, added or
     removed.
  3. **Data:** changed bytes grouped by region kind, with the regions'
     names.

  `romlens diff a.sfc b.sfc [--project-a …] [--project-b …] [--routines]
  [--json]`. Checked on Super Metroid against the four VARIA randomizer
  ROMs here (same size, patched code and data), and on fixtures with an
  inserted block.
- **In the app** (D2):
  - File › Compare With… opens a second ROM (and its project, if saved)
    beside the first, in a Compare tab.
  - The tab lists the changes, and shows the two listings side by side,
    aligned by the pairing, with the differences marked.
  - A project's names can be carried over to paired routines, as an
    undoable batch.

### Stack-relative locals (L1)

- The stack depth `stack_slots` already works out per instruction gives each
  `d,S` operand a fixed slot:
  - above the return address it is an argument (`arg3` for the byte at
    entry S + 3);
  - within what the routine pushed, it is that push's temporary.
- `(d,S),Y` reads through such a slot as a pointer.
- Signatures gain stack arguments where every caller pushes the same number
  of bytes before the call (`PEA`, `PHA`, `PHX` just before a `JSR`/`JSL`).
  The C passes them as arguments.
- A routine whose depth is not the same on every path keeps today's
  `MEM(S + k)` form, with a note.

### ca65 `.dbg` and the Source view

- **Import** (S1): the version 2 debug-info format that ld65 writes:
  - `file`, `line`, `span`, `seg` (with `ooffs`, the offset in the output
    file), `scope`, `sym` and `mod` records;
  - symbols become labels, as the other importers do;
  - lines and spans map source lines to ROM ranges, kept in the project as
    an import.

  cc65 is not installed. A small ca65 fixture is assembled once with it
  (Homebrew's `cc65`, installed only with the user's agreement), and its
  `.sfc`, `.dbg` and sources are committed, so CI needs no toolchain.
- **The Source view** (S2): an editor tab showing a source file of the
  import, found beside the `.dbg`. Selecting a line selects its bytes in the
  hex, listing and C views, and the other way round.

### The tutor

- **Client** (T1): the Messages API over HTTPS from the core, streaming,
  with its own tool loop.
  - The key is kept in the Keychain through a `CredentialStore`.
  - The cache layout, cost display and per-conversation cap are as
    `06-ai-tutor.md` designs.
  - The API shape and model are checked against the current Claude API
    documentation when this starts.
  - Tests run against recorded responses, never the network.
- **Ask** (T2): read-only tools over what Romlens already computes:
  - the listing, explanations, C, the screen setup;
  - regions and cross-references;
  - the graphics decoders and, with a recording, the provenance chain.

  Every claim cites an address. A Tutor pane in the app shows the answer,
  the tool log and the cost.
- **Fix** (T3): proposals (a label, a region type, a flag override, a
  variable) rendered as cards. Accepting applies them through `apply_batch`
  with `Origin::Accepted`.
- **Investigate** (T4): the recording tools (change index, provenance,
  frames), with a visible plan and tool log.

## Ordered tasks

| # | Task | Size (days) |
|---|---|---|
| F0 | This document, the roadmap pointer, the checklist rows | 0.3 |
| P1 | `graphics::compose` with winners; fixtures; checked against two games' frames | 3 |
| P2 | Frame and Layers views, FFI, the frame stepper as a slider | 2 |
| P3 | The `WLOG` reader; VRAM, CGRAM and OAM to their DMA; `romlens provenance` | 2 |
| P4 | WRAM to its writer and the decompressor to ROM; checked on Super Metroid's Samus tiles | 3 |
| P5 | The inspector's Provenance chain; the region proposal | 1.5 |
| P6 | MesenCE: the address and PC on each DMA record; the pack and reader | 1.5 |
| A1 | Windowed summaries, call arcs, the item level | 1 |
| A2 | The Atlas tab | 3 |
| D1 | `diff::compare`, `romlens diff`, goldens; the VARIA check | 3 |
| D2 | The Compare tab and carrying names over | 3 |
| L1 | Stack slots for `d,S`; stack arguments in signatures; differential test | 2.5 |
| S1 | `.dbg` import, fixture, goldens | 1.5 |
| S2 | The Source view | 2 |
| T1 | Client, Keychain, tool loop, recorded-response tests | 3 |
| T2 | Ask mode and the Tutor pane | 3 |
| T3 | Fix mode's cards | 2 |
| T4 | Investigate mode | 2 |
| M | Measure, record, the manual pass steps | 1 |

Each task is one or more commits, pushed.

## What is cut for now

- Colour math, windows, hi-res, interlace, mosaic and the mode 7 transform
  in the compositor.
- A framebuffer layer in recordings.
- Tracing across more than two hops (for example a buffer copied between
  WRAM buffers before its DMA), beyond naming the next writer.
- Decompressors other than Super Metroid's; others are named as "the code
  that wrote this buffer" without a ROM source.
- Three-way comparison, and comparing ROMs of different games.
- Editing source in the Source view.
- The tutor's Explain and Quiz modes, which are Phase 4.

## Risks

- **The drawn frame disagrees with the game.** P1 is checked pixel by pixel
  against Mesen screenshots, and the Frame view says when the PPU uses a
  feature the compositor leaves out.
- **Matching bytes finds the wrong DMA** where a buffer holds repeated data.
  The confidence says *inferred* whenever there is more than one candidate,
  and P6 makes the common case exact.
- **Comparing ROMs is slow** on 4 MB games. The byte pass is linear. The
  routine pass pairs by fingerprint hash first, then diffs only the pairs
  that changed.
- **The tutor costs money and sends data.** The key is the user's, the pane
  shows cost, a cap stops long investigations, and the settings say what is
  sent.

## Verification

- **Unit tests:**
  - compositor fixtures with known winners;
  - the `WLOG` reader round trip;
  - provenance on a recording made from a fixture ROM with a DMA and a
    decompression;
  - the ROM diff on a fixture with an inserted block and a patched routine;
  - `.dbg` parsing on the committed fixture;
  - the tutor loop on recorded responses.
- **The differential and compile-and-run checks** for the stack locals.
- **Goldens:** `romlens provenance`, `romlens diff`, `romlens map` with a
  window, the `.dbg` import.
- **By hand:**
  - "Find Samus" from docs/07 on a Super Metroid recording, ending at the
    compressed tiles in ROM;
  - the VARIA comparison;
  - the Atlas on both games.
- **Gates:** `make test`, `make swift`, `make app-test`.
