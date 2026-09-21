# UX proposal

Three layout options are described below. They are not mutually exclusive:
the recommendation is to build Option A first, absorb Option B's lockstep
behaviour as a mode inside it, and add Option C in Phase 2 as the overview
and the foundation for the educational visualizations.

## Shared vocabulary

- **Address**: shown as SNES address by default (`$80:841C`), with the file
  offset (`0x00041C`) available in the inspector and as a column toggle.
  FastROM banks (`$80–$FF`) and their slow mirrors (`$00–$7F`) are shown as one
  location with a mirror badge.
- **Region**: a contiguous byte range with a *kind* (unknown, code, data
  subtypes), a *source* (vector reach, trace, heuristic, imported symbols,
  user) and a *confidence* (0–1). Color encodes kind; saturation encodes
  confidence. Unknown is neutral grey, never colored.
- **Label**: a name attached to an address. Auto labels are dimmer than user
  labels.
- **Selection** is one thing across the whole window. Selecting bytes in hex
  selects the instruction in the disassembly, the region in the map and the
  entry in the inspector.

## Option A — Workbench (recommended for Phase 0–1)

Three panes, like Xcode or Hopper.

```
┌───────────┬────────────────────────────────────────┬──────────────┐
│ Navigator │  [Hex]  [Disassembly]  [Both]          │  Inspector   │
│           │                                        │              │
│ Banks     │  $80:841C  78         SEI              │ Selected     │
│  80 ▸     │  $80:841D  18         CLC              │  $80:841C    │
│  81       │  $80:841E  FB         XCE              │  file 0x41C  │
│  …        │  $80:841F  5C 23 84 80 JML $808423     │  bank 80     │
│ Labels    │  $80:8423  E2 20      SEP #$20         │ Instruction  │
│  RESET    │  $80:8425  A9 01      LDA #$01         │  SEI 1 byte  │
│  NMI      │  …                                     │  I flag ← 1  │
│ Regions   │                                        │ Flags here   │
│  code 41% │                                        │  M=1 X=1 E=0 │
│  data 38% │                                        │ Xrefs        │
│  unk  21% │                                        │  ← vector    │
├───────────┴────────────────────────────────────────┴──────────────┤
│ Overview strip: whole ROM as a heat bar, playhead = visible range  │
└────────────────────────────────────────────────────────────────────┘
```

- **Navigator** (left): banks tree, labels list with filter, regions list,
  bookmarks, search results. Collapsible.
- **Editor** (center): tabs for Hex, Disassembly, or Both (a vertical split
  in lockstep). Each tab is a virtualized table. From Phase 2 the graphics
  views (Tile decoder, Palette, OAM, Tilemap, then Frame, Layers and
  Provenance) are further editor tabs sharing the same selection (see
  `07-memory-to-screen.md`).
- **Inspector / Tutor** (right): a segmented control switches between the
  inspector and the tutor pane; on wide windows both can be open. The tutor
  pane holds the transcript, a selection chip, proposal cards and the tool
  log (see `06-ai-tutor.md`).
- **Inspector** (right): selected byte(s) interpreted every useful way;
  selected instruction with bytes, addressing mode, effective address, flag
  state before/after; region kind, source and confidence with an edit
  control; cross-references; comments.
- **Overview strip** (bottom): the whole ROM in one bar, colored by region
  kind. Click to jump; the visible range is a playhead.

Why first: it is the fastest to build with AppKit split views and table
views, it matches muscle memory from Xcode and Hex Fiend, and every later
feature has an obvious home in one of the three panes.

## Option B — Lockstep split (absorbed into A as the "Both" tab)

Hex on the left and disassembly on the right, always scrolled together, with
a byte-to-instruction bracket drawn between them.

```
┌──────────────────────────┬──┬───────────────────────────────┐
│ 00041C 78 18 FB 5C 23 84 │──│ SEI                            │
│        80 E2 20 A9 01 8D │ ╲│ CLC                            │
│ 00042C 0D 42 85 86 C2 30 │  │ XCE                            │
│        A2 FF 1F 9A A9 00 │  │ JML $808423                    │
│ …                        │  │ SEP #$20                       │
└──────────────────────────┴──┴───────────────────────────────┘
```

Strength: the pedagogical core of the whole project is "these bytes *are*
this instruction", and this layout says it without words. Weakness: it wastes
width when the user only cares about one side, and variable-length
instructions make the two columns drift, so the bracket layer is essential.

## Option C — Atlas (Phase 2 onward; the visualization foundation)

A zoomable map of the whole ROM, rendered on a canvas.

```
┌────────────────────────────────────────────────────────────┐
│  bank 80   bank 81   bank 82   bank 83   bank 84   bank 85  │
│  ▓▓▓▓░░░░  ▓▓▓▓▓▓░░  ▒▒▒▒▒▒▒▒  ░░░░░░░░  ▒▒▒▒▒▒▒▒  ░░░░░░░░  │
│  ▓▓▓░░░░░  ▓▓▓▓░░░░  ▒▒▒▒▒▒▒▒  ░░░░░░░░  ▒▒▒▒▒▒▒▒  ░░░░░░░░  │
│  code ▓   data ▒   unknown ░       zoom: 1 px = 32 bytes    │
└────────────────────────────────────────────────────────────┘
      double-click a region → opens it in the Workbench
      call arcs drawn between code regions on hover
```

- Level 0: banks as tiles. Level 1: 4 KB blocks. Level 2: individual
  instructions and data items. Zoom is continuous, like a map.
- Overlays: classification, confidence, entropy heat, execution coverage
  from a trace, call arcs, data references.
- This view becomes the stage for Phase 4 scenes: camera moves across the
  atlas, zooms into a routine, and steps through it.

Why later: it needs the classification data from Phase 2 to be interesting,
and it needs a Canvas/Metal renderer rather than table views.

## Feature list by phase

| Feature | Phase | Option |
|---|---|---|
| Open .sfc/.smc, strip copier header | 0 | A |
| Mapping detection, header parse, checksum | 0 | A |
| Structured hex view, dual address column | 0 | A |
| Header/vector overlays on hex | 0 | A |
| Byte inspector (u8/u16/u24, address, pointer) | 0 | A |
| Jump to address, bookmarks | 0 | A |
| 65816 disassembly with M/X tracking | 1 | A |
| Recursive descent from vectors | 1 | A |
| Labels, comments, xrefs | 1 | A |
| Hex ↔ asm lockstep with byte brackets | 1 | B in A |
| Project file, asm and symbol export | 1 | A |
| Trace/CDL import | 2 | A |
| Heuristic classification with visible evidence | 2 | A, C |
| Data typing, graphics and palette previews | 2 | A |
| Overview strip and Atlas map | 2 | C |
| Functions, CFG, call graph | 3 | A, C |
| Pseudo-C for one function | 3 | A |
| ROM version diff | 3 | A |
| Embedded stepping core, register view | 4 | A, C |
| Scene timeline, scrubbing, video export | 4 | C |
| Notebook documents | 4 | — |
| Tutor: Ask mode, citations, tool log | 1 | A |
| Tutor: Fix mode, proposal cards | 2 | A |
| Tutor: Investigate over recordings | 3 | A |
| Tutor: Explain to scene drafts, Quiz | 4 | A, C |
| Recording import (Mesen2 script, savestates) | 2 | A |
| Tile decoder, palette, OAM, tilemap views | 2 | A |
| Frame, Layers, Provenance chain | 3 | A |
| Live pipeline from embedded core | 4 | A, C |

## Interaction details worth deciding early

- **Keyboard first.** `⌘L` jump, `⌘F` find bytes or text, `G` follow
  reference under cursor, `⌫` back, `N` rename label, `;` comment, `C`/`D`/`U`
  mark code/data/unknown. These are the same keys IDA and Ghidra users know.
  On Windows and Linux the modifier is Ctrl; the letter keys are identical.
  The per-platform idiom map is in `08-cross-platform.md`.
- **Undo everything.** Region marks, labels and comments are all in the
  undo stack. The analyzer's automatic work is never on the undo stack; the
  user's overrides are.
- **Evidence popover.** Hovering a region's color chip shows why it was
  classified: "reached from RESET vector via 3 JSLs", "executed 1,204 times
  in trace.cdl", "entropy 7.2 bits/byte, pointer density 0", "user".
- **One selection across code and graphics.** Clicking a sprite in the
  frame selects its OAM entry, its VRAM tiles, its palette row, its DMA
  transfer and its ROM bytes. Clicking bytes in the hex view lights up any
  pixel they became.
- **The tutor proposes, the student decides.** Proposal cards carry
  Accept, Reject and Show evidence. Nothing the model says changes the
  project until a human clicks.
- **Two address systems are a feature, not a bug.** Teaching how `$80:841C`
  maps to file offset `0x41C` is exactly the kind of thing the app exists for,
  so the mapping is always one hover away.

## The same workbench on three platforms

The layout options above describe structure, not widgets. Each shell builds
the Workbench from its own toolkit: `NSSplitViewController` and
`NSTableView` on macOS, `NavigationView` and virtualized lists with Fluent
materials on Windows, `AdwNavigationSplitView` and `GtkListView` on GNOME.
What is identical everywhere comes from the core: the hex rows, the typed
disassembly tokens, region kinds and confidence, the selection, the undo
stack, the tutor transcript, and exported video.

## Visual direction

- Mono typeface with slashed zero and clear hex glyphs (SF Mono or JetBrains
  Mono). Proportional UI text is the system font.
- Region palette: code in a cool hue, data subtypes in warm hues, unknown in
  grey. Confidence lowers saturation. Colorblind-safe pairings checked.
- Respect system light/dark appearance and accent color.
- Motion only where it carries information: the selection bracket sliding
  between hex and asm, the overview playhead, scene playback.
