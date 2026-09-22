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

Core, release build, development ROM (3 MB), M5 Pro. Jump tables only; the
heuristics, trace import and graphics work is not in these numbers.

| Quantity | Phase 1 | With jump tables |
|---|---|---|
| Full analysis | 5 ms | 12 ms |
| Instructions | 3,029 | 28,320 |
| Code bytes | 6,872 (0.2%) | 65,495 (2.1%) |
| Code blocks | 76 | 485 |
| Auto labels | 256 | 2,998 |
| Xrefs | 1,554 | 14,722 |
| Warnings | 15 | 137 (34 informational jump tables) |

`romlens analyze --no-tables` reproduces the Phase 1 column exactly, which is
what makes the gain attributable rather than merely coincident. The Phase 1
figure above is 5 ms rather than the 7 ms recorded below because the pass loop
now exits as soon as a pass adds nothing.

Tables resolved: 34, holding 521 entries over 1,042 bytes. The largest has 45
entries. 28 were bounded by the first routine they point at and 6 by code the
descent had already claimed, so no table on this ROM fell back to a weak stop
reason. 54 dispatch sites remain unresolved and every one of them builds its
table in RAM.

The analyzer runs the descent up to eight times now instead of six, and a pass
costs about 1.5 ms, which is where the 5 ms → 12 ms goes. Still two orders of
magnitude inside the 2 s budget.

## Not measured, still to check in Phase 0

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
