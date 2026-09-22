# FFI spike: is UniFFI fast enough for the hex table?

Measured 21 September 2026 on this machine (Apple M5 Pro, macOS 27, Rust 1.95,
Swift 6.4, UniFFI 0.31.2). Source in `spikes/ffi-rows/`. Single thread,
release builds, whole 3 MB ROM (196,608 rows) fetched in 984 batches of 200
rows; second of two runs reported.

## Results

| Path | Per 200-row batch | Per row | Whole ROM |
|---|---|---|---|
| C ABI, one call per byte ("chatty") | 2.8 µs | 14 ns | 2.7 ms |
| C ABI, row batch into a caller buffer | 1.1 µs | 5.7 ns | 1.1 ms |
| C ABI, row batch in a Rust-owned buffer, freed after | 1.1 µs | 5.7 ns | 1.1 ms |
| C ABI batch, then Swift formats each row with `String(format:)` | 1,477 µs | 7,393 ns | 1,453 ms |
| C ABI, core pre-formats text, one Swift String per batch | 30 µs | 150 ns | 30 ms |
| UniFFI, `Vec<HexRow>` records (address, bytes, ascii) | 155 µs | 774 ns | 152 ms |
| UniFFI, flat `Vec<u8>` blob per batch | 7.4 µs | 37 ns | 7.3 ms |
| UniFFI, core pre-formats text, one String per batch | 30 µs | 151 ns | 30 ms |

## What it means

1. **The FFI is not the bottleneck.** Even the worst generated shape,
   UniFFI records, costs 155 µs for 200 rows. A visible page is 40 to 80
   rows, so under 60 µs of a 8.3 ms frame at 120 Hz. The hand-written C ABI
   is 140 times cheaper per row, and it does not matter.
2. **Shell-side formatting is the bottleneck.** Building hex strings in
   Swift with `String(format:)` costs 7.4 µs per row, fifty times the
   UniFFI record cost. That is still fine per visible page (0.5 ms for 60
   rows) but rules out eager formatting of the whole ROM, and it means the
   table must format lazily per visible row or take pre-formatted text from
   the core.
3. **UniFFI serialization is real but bounded.** Records cost about 20 times
   a flat blob. The rule for hot tables is therefore: return a flat buffer or
   pre-formatted text per batch, and reserve records for everything that is
   not scrolled at 120 Hz.
4. **Never fetch the whole ROM through records.** 152 ms. Page it.

## Decision

Use UniFFI for the API. It gives generated Swift and C# bindings, async
functions and callback interfaces, which a hand-written C ABI would make us
build ourselves. Hot paths (hex rows, disassembly lines, atlas tiles, frame
images) return flat buffers or pre-formatted text; everything else returns
typed records. The raw C ABI remains underneath as an escape hatch and is
what the C# generator ultimately calls.

Shell rule confirmed for the macOS table: format per visible row, cache
formatted rows, never format ahead of scroll.

## Phase 0 measurement (shell side, 21 September 2026)

The first hex view was an `NSTableView` with one `HexRowView` per row. It
measured fine off screen (worst 40-row frame 2.2 ms) but hung the app in a
real trackpad scroll: AppKit stopped recycling row views during the long live
scroll (158,790 were attached after scrolling to row 158,700, 828 MB
resident), and SwiftUI's hosting view walked the whole subtree on every
constraint pass looking for nested hosts, so each layout pass cost seconds.
The container was swapped for the fallback the plan named: a single canvas
view as tall as the image that draws the rows in the dirty rect from the
batch cache. The row painter did not change.

Measured by `HexTableIntegrationTests` in the macOS shell's test bundle on
the development ROM (196,608 rows), debug build, M5 Pro:

| Quantity | Value |
|---|---|
| Whole ROM drawn in 40-row frames, cold cache | 3.6 s (18 µs per row) |
| Worst 40-row frame | 2.1 ms |
| Live page-by-page scroll through the image, 9,338 pages | 3.3 s, worst step 19 ms (first page) |
| 120 Hz budget per frame | 8.3 ms |
| Views in the canvas after a full scroll | 0 |

A page of fresh rows costs about a quarter of a frame in a debug build,
before AppKit's own work; cached rows cost only the draw. The Instruments
pass (Animation Hitches over a full trackpad scroll) is still a manual step.

## Phase 1 measurements (21 September 2026)

Core, release build, development ROM (3 MB), M5 Pro:

| Quantity | Value |
|---|---|
| Full analysis (two descent passes + sweep + labels) | 7 ms |
| Static reach from the twelve vectors | 3,029 instructions, 6,872 code bytes, 256 labels, 1,554 xrefs, 15 warnings (with inline arguments skipped; 2,756 instructions and 6,245 bytes before) |
| Why so little | the main loop dispatches through `JSR ($xxxx,X)` tables; jump tables are Phase 2 |

Shell, debug build, `RomlensTests` on a 3 MB padded fixture:

| Quantity | Value |
|---|---|
| Asm canvas: 196,819 lines drawn in 40-line frames, cold cache | 5.0 s (25.6 µs per line), worst frame 2.5 ms, 769 batch misses |
| Asm live page-by-page scroll | 7,790 pages, worst step 2.5 ms, 0 subviews |
| `lineForOffset` (FFI round trip, main actor) | 0.7 µs per call (budget 50 µs) |
| Navigator filter, 20,000 labels, three passes | 14 ms |
| Hex canvas (unchanged path) | worst 40-row frame 1.2 ms, live scroll worst step 13 ms |

The async `analyze()` future and the `WorkbenchListener` callback were not
timed separately: progress events are coalesced to one main-actor hop per
batch, and the whole analysis is shorter than a frame budget on this ROM.

## Phase 2 measurements (22 September 2026)

Core, release build, development ROM (3 MB), M5 Pro. Track 2A only; the
graphics and recording tracks are not started.

`romlens analyze --no-tables` and `--no-heuristics` exist so the gain is
attributable rather than merely coincident, and the first column below is
exactly what Phase 1 produced.

| Quantity | Phase 1 | Jump tables | Heuristics | Both |
|---|---|---|---|---|
| Full analysis | 7 ms | 10 ms | 35 ms | 57 ms |
| Instructions | 3,029 | 28,320 | 3,029 | 28,320 |
| Code bytes | 6,872 (0.2%) | 65,495 (2.1%) | 6,872 (0.2%) | 65,495 (2.1%) |
| Code blocks | 76 | 485 | 76 | 485 |
| Data bytes | 160 (0.0%) | 1,625 (0.1%) | 471,645 (15.0%) | 473,012 (15.0%) |
| Unknown bytes | 3,138,696 (99.8%) | 3,078,608 (97.9%) | 2,667,211 (84.8%) | 2,607,221 (82.9%) |
| Regions | 136 | 1,012 | 806 | 1,672 |
| Auto labels | 256 | 2,998 | 256 | 2,998 |
| Xrefs | 1,554 | 14,722 | 1,554 | 14,722 |
| Warnings | 15 | 137 | 15 | 137 |

The two are independent by construction. Jump tables add code and everything
that follows from it; heuristics only fill bytes nothing else claimed, so they
never change the code figures.

### Jump tables

34 resolved, holding 521 entries over 1,042 bytes; the largest has 45 entries.
28 were bounded by the first routine they point at and 6 by code the descent
had already claimed, so no table on this ROM fell back to a weak stop reason.

54 dispatch sites remain unresolved and **every one of them builds its table in
RAM**, which needs value tracking along control flow (Phase 3). Each warning
names the address, so the manual fix is one `Mark as ▸ Table` with the entries
declared as code.

The descent now runs up to eight passes instead of six, at about 1.5 ms each,
which is where 7 ms → 10 ms goes.

### Heuristics

| Heuristic | Spans | Bytes | |
|---|---|---|---|
| entropy | 333 | 440,576 | mostly constant fill, plus one high-entropy band |
| pointers | 94 | 324,352 | |
| palette | 54 | 20,992 | |
| ascii | 2 | 1,536 | |
| graphics | 778 | 263,424 | annotates only; sets no region's kind |

The sums exceed the 473,012 bytes actually painted because the guesses overlap
and the strongest one on a byte wins.

The region count is the number to watch. Scoring per byte would have turned a
thousand regions into millions; scoring per 256-byte window and merging
neighbours that agree to within a 5% step gives 1,672, and
`tests/heuristics.rs` bounds it so a future heuristic cannot quietly regress
that. The entropy profile is one pass over the image and is cached on the
`Workbench`, so the edit loop pays about 45 ms rather than 57.

Two known false positives, both left standing until the accuracy harness can
price them rather than tuned away by eye: a block of Super Metroid tilemap
entries at `0x052100` is 75% printable and reads as a string, and constant fill
is claimed as byte data at 50% — defensible, since padding is certainly not
code, and the reason "unknown" fell as far as it did.

### Imports

| Quantity | Value |
|---|---|
| Reading a 3 MB Mesen2 CDL and writing the package | 16 ms |
| Analysis with a 20%-coverage trace applied | 182 ms |
| `romlens truth from-cdl` over 3 MB | 21 ms |

The 182 ms is with roughly thirteen thousand observed opcode starts seeded into
the walk, which is the shape a real play session has. Still an order of
magnitude inside the 2 s budget.

### Accuracy

The fixtures score 1.000 precision and 1.000 recall in CI, which is a
regression guard rather than a challenge — they were built to be recognisable.
The development-ROM target (code precision ≥ 0.98, recall ≥ 50%) is unmeasured
until someone records a CDL from their own play session: truth for a commercial
ROM is derived from it and never ships (`12-content-policy.md`).

## Track 2B measurements (22 September 2026)

Release build, the development machine, wall time of the whole CLI process
(which is most of it: `romlens --version` alone is about 15–20 ms warm).

| What | Result |
|---|---|
| Decompress `$95:80D8` (Super Metroid LZ, 9,481 → 16,384 bytes) | 27 ms |
| A 1,024-tile 4 bpp sheet from the dev ROM, rendered and hashed | 23 ms |
| `testrec`, 600 frames of the graphics fixture's machine, written | 89 ms; 346,902 bytes |
| Keyframes / deltas in that file | 5,567 bytes / 468 bytes on average, compressed |
| Render BG1 at frame 599 (a keyframe at 540, then 59 deltas) | 19 ms |
| Render BG1 at frame 0 | 18 ms |

The recording numbers are the synthetic machine's, whose deltas are a few
bytes of OAM, a scroll register and a colour; real play is docs/13's estimate
until the Mesen2 recorder exists. The decompressor's two dev-ROM streams come
out at exactly 16 KB and 12 KB, which is the evidence that its reading of the
format matches the game (`tests/graphics_sm_lz.rs`, opt-in).

## Not measured, still to check in Phase 0## Not measured, still to check in Phase 0

- Async call and callback-interface overhead (event delivery from the core's
  thread pool to the main thread).
- C# bindings from `uniffi-bindgen-cs` (community-maintained, tracks UniFFI
  0.31, a 0.32 upgrade was in progress as of June 2026, and maintainers were
  asked in April 2026 whether the project is still a priority). The C ABI is
  the fallback if it stalls.
- Memory: the generated record path allocates per row; the blob path
  allocates per batch. Fine at page granularity.
- Generated Swift for four functions is 823 lines; acceptable, and it is
  regenerated by the build, not maintained by hand.
