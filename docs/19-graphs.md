# Control-flow graphs and the call graph (Phase 3)

## Progress

Written 24 September 2026 and kept as written; this section is the only part
that tracks against it.

| Task | State |
|---|---|
| G0 this document, the roadmap pointer, the checklist rows | done |
| G1 the routine graph and the call neighbourhood in the core | to do |
| G2 the layered layout | to do |
| G3 `romlens graph` | to do |
| G4 FFI | to do |
| G5 macOS Graph tab: blocks | to do |
| G6 macOS Graph tab: calls | to do |
| G7 measure and record | to do |

## Context

The roadmap's Phase 3 asks for "basic blocks; CFG view" and a "call graph
with drill-down; 'who calls this' and 'what does this call'"
(`01-vision-and-roadmap.md`). The decompiler (`18-decompiler.md`) already
finds every routine, splits it into blocks, and works out dominators and
natural loops (`decompile::function`, `decompile::cfg`), so the analysis is
there and only the picture is missing.

For a student the picture is the point. A loop in a listing is a label and a
branch forty lines further down; in a graph it is a box with an arrow coming
back up to it. The reset handler's clearing loops, a jump table's fan of
cases, a routine that is called from sixty places: each is a shape before it
is anything else.

## Scope decisions (24 September 2026)

1. **A Graph editor tab** beside Hex, Disassembly, Both and C (View › Graph,
   ⌥⌘9), with two modes: **Blocks** (the routine at the cursor as a
   control-flow graph) and **Calls** (the routine with its callers and
   callees). Like the C tab it follows the cursor to the routine it is in.
2. **Blocks show the listing's own lines**, labels and comments included,
   with the SNES address and the instruction, coloured as in the
   disassembly. Clicking a line selects that instruction, and the inspector
   follows; selecting an instruction elsewhere highlights its line and
   scrolls its block into view.
3. **Edges say how control leaves a block.** A branch taken is green, not
   taken red, a jump or fall-through grey, an edge back up to a loop header
   blue, a jump table's cases labelled with their index. A loop's blocks are
   tinted. A block that leaves the routine (a tail call, an unknown jump)
   is a small box naming where it goes.
4. **What the recording saw.** With an execution log attached, each block
   carries how many times it ran and each edge how many times it was taken;
   blocks that never ran are dimmed. Without a log nothing is shown, rather
   than zeros.
5. **Calls is a neighbourhood, not the whole program.** The routine sits in
   the middle, its callers above and its callees below, each with the number
   of call sites and whether they are direct, through a table, a tail call,
   or an indirect call only the recording saw. Double-clicking a routine
   re-centres on it; Back and Forward walk the trail, as they do in the
   editor. The whole-program zoomable map stays for later.
6. **The core lays the graph out.** The shells measure their boxes and draw;
   the core decides where boxes and edges go (a layered layout, below), so
   Windows and Linux get the same picture from the same code. The CLI prints
   the graph as Graphviz DOT and as JSON.
7. **Not editable.** No dragging boxes, no saved layouts, no marking code
   from the graph. Zoom (pinch, ⌘+ and ⌘−, and Fit) and scrolling only.

## What already exists

- `decompile::function::discover` and `containing`: a routine's
  instructions and where each one sends control (`Transfer`: branch, call
  with its `Callee`, jump, switch over a resolved table, return, halt), with
  tail calls and unknown jumps as `Dest::Tail` and `Dest::Unknown`.
- `decompile::cfg::Cfg::build`: blocks as step ranges with a `Term`, their
  predecessors, reverse post-order, dominators and post-dominators, natural
  loops with their latches, and whether the graph is irreducible. Stubs
  (empty blocks) stand for tail calls and unknown destinations.
- `decompile::entries`: every routine entry (call targets, vectors, user
  labels).
- Cross-references (`AnalysisSnapshot::xrefs_to` / `xrefs_from`), including
  the calls and jumps an execution log observed (`observed`).
- The execution log (`model::exec_log`, `17-execution-log.md`): a count per
  instruction (`INST`) and per control transfer (`FLOW`: branch taken, not
  taken, jump, call, indirect call, …).
- The listing (`viewmodel` lines, `Workbench::asm_lines`): each line's text,
  tokens and region, by line number, with `line_for_offset`. The app already
  decodes and paints these (`AsmBatch`, `AsmTokenPalette`).

## Design

### The routine graph (`graph::routine`)

`routine_graph(rom, project, snap, entry) -> RoutineGraph`:

- `blocks`: for each reachable block, in reverse post-order with the entry
  first: its instructions' file offsets, its terminator's kind, whether it
  heads a loop and the loop depth, and for a stub where it goes (a routine's
  entry and name, or why it is unknown).
- `edges`: from, to, kind (`Taken`, `NotTaken`, `Jump`, `Fall`, `Case(n)`),
  and whether it is a back edge. Back edges are those to a loop header from
  its latch, plus, where the graph is irreducible, those a depth-first walk
  finds going to a block still on its stack.
- `loops`: header and body, from `Cfg::loops`.
- With an execution log, `runs` per block (the count of its first
  instruction) and `taken` per edge (from `FLOW`: branch taken for `Taken`,
  not taken for `NotTaken`, the count of the next block's first instruction
  otherwise, when that block has one predecessor). Counts are matched by ROM
  offset, so mirrors add up.

### The call neighbourhood (`graph::calls`)

`call_neighbourhood(rom, project, snap, entry) -> CallNeighbourhood`:

- `callees`: from the routine's own transfers (direct calls, every target of
  a table call, tail calls) and from the observed calls and jumps
  `xrefs_from` its instructions carry, grouped by target routine with the
  call sites, how they call, and the recorded count.
- `callers`: from `xrefs_to(entry)` of kind call or jump, and the table
  calls whose targets include it, each site mapped to the routine
  containing it with `function::containing`, grouped the same way.
- Names from `Symbols`, as the listing shows them.

It needs no whole-program summary, so it costs a routine discovery per
caller rather than the second the decompiler's summaries take.

### The layout (`graph::layout`)

A layered (Sugiyama-style) layout, pure and deterministic, over nodes whose
sizes the shell gives:

1. **Layers.** Back edges are reversed; each node's layer is its longest
   path from the entry. Nodes not reachable from the entry (none in a
   routine graph; unrelated callers in a neighbourhood) start at layer 0.
2. **Dummies.** An edge spanning several layers gets a dummy node in each
   layer it crosses, so edges bend around boxes rather than through them.
3. **Order.** Within each layer, nodes start in depth-first order and move to
   the median of their neighbours' positions, sweeping down and up a fixed
   number of times; the order with the fewest crossings wins.
4. **Position.** Nodes are packed left to right with a gap, then pulled
   towards the median of their neighbours' centres in alternating passes
   without overlapping; the whole graph is shifted so its left edge is 0.
   Each layer is as tall as its tallest box, with room between layers for
   the edges.
5. **Edges.** Each edge leaves the bottom of its source, at a port spread
   across the box's width (taken to the left of not taken), passes through
   its dummies' centres, and enters the top of its target. A back edge
   leaves the bottom of its latch, climbs through its dummies, and enters
   the top of its header. The shell draws the points as a smoothed path
   with an arrowhead.

Output: a position per node, a list of points per edge, and the total size,
in the shell's units. The layout is capped (a routine is at most
`function::MAX_INSNS` instructions); past a node count it falls back to one
node per layer in reverse post-order so it never takes long.

### Surfaces

- **CLI:** `romlens graph <rom> [--project P] <address> [--calls]
  [--json | --dot]`. DOT renders with Graphviz (`dot -Tsvg`); JSON carries
  the graph and a layout with each block one unit per line.
- **FFI:** `Workbench::routine_graph(snes_address)`,
  `Workbench::call_neighbourhood(snes_address)`, and a free function
  `layout_graph(nodes, edges, spacing)`. The routine graph adds each block's
  listing line range (from `line_for_offset` on its first and last
  instruction) so the shell can fetch and paint the lines it already knows.
- **App:** `EditorTab.graph`; `GraphModel` (routine, mode, graph, layout,
  trail) beside `DecompileModel`; `GraphCanvasView`, an `NSView` in a
  magnifying `NSScrollView` that draws boxes, lines and edges; the mode
  picker, Fit and the routine's name in a header like the C tab's. The
  graph follows the cursor as the C tab does, and is rebuilt on
  `.snapshot`/`.view` changes. A live session keeps showing the graph while
  a newer one is built, as the C tab now does.

## Ordered tasks

Each task is a commit; sizes are in days.

| # | Task | Size |
|---|---|---|
| G0 | This document, the roadmap pointer, the checklist rows | 0.5 |
| G1 | `graph::routine` and `graph::calls`, with the execution log's counts; tests on the fixtures (`routines_lorom`, `dispatch_lorom`) and on a hand-made log | 1.5 |
| G2 | `graph::layout`: layers, dummies, ordering, positions, edge points; tests for no overlapping boxes, every edge ending on its boxes, determinism, and every routine on the development ROM laid out | 2 |
| G3 | `romlens graph` with `--dot` and `--json`, and a golden | 0.5 |
| G4 | FFI records and calls, with Rust and RomlensKit tests | 0.5 |
| G5 | The Graph tab, Blocks mode: canvas, lines, edges, loop tint, counts, selection in both directions, zoom and Fit, menu and context menu; app tests | 2 |
| G6 | Calls mode: the neighbourhood, re-centring, Back and Forward; app tests | 1 |
| G7 | Measurement on Super Metroid and (locally) Super Mario World: blocks and edges per routine, crossings, layout time; recorded here | 0.5 |

## What is cut for now

- The whole-program call map (zoomable, every routine), which the Atlas was
  reduced from in Phase 2.
- Editing from the graph: renaming, commenting and marking stay in the
  listing and the inspector (the selection follows, so they are one click
  away).
- Manual layout: dragging boxes, saving positions.
- Collapsing a loop or an `if` into one box.
- The graph for code the analysis did not find as a routine.

## Risks

- **Large routines draw slowly.** A few routines run to hundreds of blocks.
  Boxes are drawn only where they are visible, and the lines come from the
  listing's cache.
- **Layouts that jump.** A live session re-analyses every few seconds; a new
  graph for the same routine usually has the same shape, and the layout is
  deterministic, so it comes back the same. The view keeps its scroll and
  zoom across refreshes of the same routine.
- **Counts that mislead.** A block's count is its first instruction's; an
  interrupt in the middle of a block does not change it, but code reached
  from outside the routine (a mid-routine jump from elsewhere) does. The
  tooltip says what the count is.

## Verification

- Unit tests in the core for the graph (blocks, edge kinds, back edges,
  stubs, counts) and the layout (no overlaps, edges end on their boxes,
  layers increase along forward edges, the same input gives the same
  output).
- A golden for `romlens graph --dot` on the routines fixture.
- ROM tests (`make test-rom`): every routine on the development ROM gets a
  graph and a layout, with the time recorded.
- App tests for the tab: it follows the selection, clicking a line selects
  its instruction, Calls re-centres, Back returns.
- Gates: `make test`, `make swift`, `make app-test`.
- The manual pass in the conformance checklist (steps 39–41).
