# Romlens Phase 2 implementation plan

## Progress

Written 22 September 2026 and kept as written; this section is the only part
that tracks against it.

| Task | State |
|---|---|
| Housekeeping 1 (commit the dirty tree) | done, `78e037f` |
| Housekeeping 3 (checklist 0.12, API version in the About box) | done, `10eb4ae` |
| Housekeeping 4 (the shell job's `continue-on-error`) | done, `9c35b73`: the flag moved to the `xcodebuild` step so the rest of the job is enforced. The same commit fixed a Windows golden failure the first CI run found |
| 2A T1 foundations | done, `f213d16` |
| 2A T2 jump tables | done, `0935cfe`: 34 tables on the development ROM, code 0.2% → 2.1% |
| 2A T3 measure and record | done, `739964c` |
| 2A T4 heuristics | done: unknown 97.9% → 82.9%, region count 1,012 → 1,672 |
| Housekeeping 2 (the Phase 1 manual pass) | **still to run**, and it needs a person at the app |
| Everything from 2A T4 (heuristics) on | not started |

## Context

Phase 0 ("Open and see") and Phase 1 ("Disassemble") are implemented and
committed (21 September 2026, `1596670`…`16701dd`). Phase 1's remaining
roadmap bullet, **tutor Ask mode**, was extracted and is deferred again here:
Phase 2 ships no tutor, and `06-ai-tutor.md`'s Fix mode moves to Phase 3.

Phase 1 left one measured problem, and Phase 2 exists to fix it.
`crates/romlens-cli/tests/golden/analyze-supermetroid.txt`:

```
Code:          6872 bytes (0.2%) in 76 blocks, 3029 instructions
Data:          160 bytes (0.0%)
Unknown:       3138696 bytes (99.8%)
```

`10-ffi-spike.md` names the cause — the main loop dispatches through
`JSR ($xxxx,X)` tables — and `warnings-supermetroid.txt` shows it: four
`JSR through a table; targets not followed` warnings plus one
`jump through a pointer in RAM`, each a wall the walk stops at
(`analysis/descent.rs:518` and `:585` only `warn()`). The ROM map is
99.8% `unknown`, and Phase 3's functions, CFG and pseudo-C, the Atlas and
Phase 4's scenes are all starved until that changes.

Deliverable (`01-vision-and-roadmap.md`): *a ROM map that is mostly right,
with the evidence for each region one click away.*

### Scope decisions (agreed 22 September 2026)

| Question | Decision |
|---|---|
| Tutor | **Deferred entirely.** No Ask mode, no Fix mode. The one exception is `Project::apply_batch` + `Origin`, which the importers need anyway and which is the seam Fix mode will later use. |
| Sequencing | **2A classification → 2B graphics → 2C recordings.** Each independently shippable. |
| Atlas (`02-ux-proposal.md` puts it in Phase 2) | **Overview strip only.** The zoomable canvas Atlas moves to Phase 3, where the CFG and call-graph work needs a canvas renderer anyway. |
| Windows / Linux shells (`08-cross-platform.md` starts them after macOS Phase 1) | **Stay macOS-only.** Every capability still gets a CLI scenario (docs/08 rule 7). |

### What already exists

The data model is further along than docs/03's tree suggests. Internalise
this before starting:

- **`model/region.rs` already carries the whole Phase 2 vocabulary.**
  `DataKind::{Byte,Word,Long,Pointer,Table{stride},String,Graphics{bpp},
  Tilemap,Palette,Compressed,Struct}` with `name`/`parse`/`element_len`/
  `code`, and `Evidence::{VectorReach,Heuristic{name,score},User,
  Imported(String),Trace{file,hits}}`. The `Heuristic`, `Trace` and
  `Imported` variants exist and are nearly unused: they are the sockets.
- **"Evidence one click away" is already built.** `EvidenceKind` and
  `EvidenceInfo` cross the FFI (`romlens-ffi/src/records.rs`) and
  `RegionChip` in `InspectorView.swift` renders a "Why *kind*?" popover over
  every item. Phase 2 fills it; it does not build it.
- **User data typing is half-done.** `Command::MarkRegion` already carries
  `OverrideKind::Data(DataKind)` and `romlens project mark` accepts every
  kind; both hardcode `stride`/`bpp` (`commands/project.rs:90`). The macOS
  menu offers only code/data/unknown (`MainMenu.swift:84`).
- **The decoder already computes the jump-table base.** `decode.rs:251`
  returns `AbsoluteIndexedIndirect` as `Target { address: (pb, val), kind:
  Pointer, certain: false }` — the resolver's whole input.
- **The multi-pass loop already exists:** `MAX_PASSES = 6`
  (`analysis/mod.rs:43`), added by the inline-argument work, is the pattern
  jump tables reuse.
- **Byte search is done except the UI:** `rom/search.rs`, `romlens search`
  and `Workbench::search_bytes` all ship.
- **Greenfield:** `graphics/`, `recording/`, `analysis/heuristics/`,
  `analysis/jumptable.rs`, `io/import/`, `io/symbol_import.rs`,
  `viewmodel/region_summary.rs`, `tests/ground_truth/`.

### Housekeeping first

1. Commit the working tree (inline arguments, `ASSUMED_WIDTHS`,
   `SuspiciousFallthrough`, `analyze --warnings`; already 19 ms → 7 ms and
   2,756 → 3,029 instructions).
2. Run the Phase 1 manual pass (`15-conformance-checklist.md`, steps 6–12);
   every Phase 1 row is 🧪, not ✅. The Phase 0 Instruments pass is also open.
3. Close row 0.12: `API_VERSION` and `romlens --version` exist; only the
   macOS About panel's credits string is missing.

## Approach

Three tracks over one core. **2A** makes the map right: jump tables,
scored heuristics, trace and symbol imports, richer data typing, a minimap,
and an accuracy harness that publishes the number. **2B** adds the graphics
decoders and their four editor tabs, working on raw ROM bytes. **2C** adds
the `.romrec` format, its reader, writer, validator and change index, and
the Mesen2 recorder script.

2A is the deliverable the phase is named after and fits the roadmap's 4–8
week budget on its own. 2B and 2C are separately shippable follow-ons.

### Cross-track decisions, settled before any code

Both tracks extend `model/region.rs`:

```rust
// 2A — pointer and table semantics
pub enum BankRule { SameBank, Fixed(u8), FromEntry }
pub enum TableElem { Raw, Pointer(BankRule), Code(BankRule) }
DataKind::Pointer { bank: BankRule }
DataKind::Table   { stride: u8, elem: TableElem }

// 2B — preview parameters, on the override rather than the kind
pub struct RegionParams {
    palette: Option<SnesAddress>, columns: Option<u16>,
    screen_size: Option<ScreenSize>, char_base: Option<u16>,
}
```

`DataKind` stays `Copy + Eq + Hash` and `code()` does not change: the hex
span lane and the asm-line record's byte 10 depend on it. Both extensions
are `#[serde(default)]` so v1 packages still open.

2B owes 2A three signatures on day one so the tracks can overlap:
`graphics::palette::palette_score`, `graphics::tile::bitplane_score`,
`graphics::compress::sm_lz::try_decompress`. 2A's palette, graphics and
compressed heuristics call these rather than re-deriving bitplane layout.

`API_VERSION`: 0.3.0 after 2A, 0.4.0 after 2B, 0.5.0 after 2C.

## Repository layout after Phase 2

```
crates/romlens-core/src/
  analysis/
    jumptable.rs                  (abs,X) dispatch resolution
    heuristics/{mod,entropy,pointers,ascii,palette,graphics}.rs
  model/coverage.rs               Coverage bitsets from traces
  io/
    import/{mod,cdl,usage_map,diz}.rs
    symbol_import.rs              wla | nocash | lbl
    crc32.rs
  graphics/
    {mod,tile,palette,oam,tilemap,ppu_state,render}.rs
    compress/{mod,sm_lz}.rs
  recording/
    {mod,format,writer,reader,delta,validate,change_index,import,fixtures}.rs
    mesen/{mod,mesen_recorder.lua,savestate.rs}
  viewmodel/
    region_summary.rs             the minimap batch
    {bitmap,tile_bitmap,palette_grid,oam_rows,tilemap_rows,preview}.rs
crates/romlens-core/tests/
  ground_truth.rs  graphics_{tile,palette,oam,tilemap,render}.rs
  graphics_sm_lz.rs  romrec_{roundtrip,validate}.rs
  delta.rs  change_index.rs  machine_state_source.rs
tests/ground_truth/README.md      the schema; *.tsv is git-ignored
shells/macos/Romlens/
  Views/RegionStripView.swift
  Views/Graphics/{TileDecoder,TileSheet,Palette,OamTable,Tilemap}View.swift
  Views/Sheets/{FindSheet,DataTypeSheet}.swift
  Document/ImportController.swift
  Model/Bitmap+AppKit.swift
```

## Work breakdown

### 1. Jump tables — `analysis/jumptable.rs`

Resolution runs **between descent passes**, reusing the `MAX_PASSES` loop
exactly as inline arguments do. Mid-walk resolution would depend on worklist
order, so goldens would be unstable; between passes the input is a complete
picture of the previous pass and the result is order-independent. `Walk`
gains `jump_tables` (resolved earlier) and `found_table_sites` (seen this
pass). Raise `MAX_PASSES` to 8 for nested dispatchers; the loop already
exits early on no-growth. Add a global entry budget so a pathological image
cannot make it grow without bound.

Bounding a table — entries are 16-bit, read from and jumped to within the
program bank. Walk from the base, stopping at: end of bank; a user
`OVR_WALL`; bytes already decoded as code; an implausible target; a target
inside the table; a 256-entry cap; and the rule that actually works,
`first_target_floor`, the lowest in-table target above the table start,
since a dispatch table is essentially always immediately followed by the
routines it points at.

`plausible_entry` reuses the linear sweep's bar (decodes; first mnemonic is
not `BRK`/`WDM`/`STP`/`COP`; a second instruction decodes; no branch target
leaves ROM) but evaluated under the dispatcher's own `flags_after` — the
correct M/X state at dispatch, and the sweep's blind spot.

Confidence 0.60–0.85 by stop reason. Table bytes take a new `TAG_TABLE` and
`Evidence::Heuristic { name: "jump table, 14 entries, dispatched from
$80:9C15" }`; naming the dispatcher is what makes the popover useful. One
xref per entry, `certain: false`. New auto prefix `JTBL_` in
`model/label.rs`, ranked between `PTR` and `DATA`. `WarningKind` gains
`severity()` and an informational `JumpTable` variant replacing
`ComputedJump` at a resolved site; unresolved sites keep `ComputedJump` and
gain a reason ("table base $7E:0032 is in RAM").

User overrides: a data wall blocks resolution; a user `MarkRegion { Table
{ elem: Code(rule) } }` at the base replaces the heuristic verbatim (the
correction path); a user `Code` mark inside an extent truncates it next pass.

Out of scope: RAM pointer tables and the `LDA table,X : STA $00 : JML [$00]`
idiom need value tracking along control flow — Phase 3. Phase 2 delivers a
better failure: the warning names the RAM address and the inspector offers
"Mark a jump table here…". Long dispatch works only when the user declares
`Table { stride: 3, elem: Code(BankRule::FromEntry) }`. (`JSL [abs,X]` does
not exist on the 65816.)

### 2. Heuristics — `analysis/heuristics/`

Five modules emitting **window-aligned spans**, never per-byte verdicts:

```rust
pub struct HeuristicHit { start: u32, len: u32, kind: RegionKind,
                          score: f32, name: &'static str, detail: String }
```

| Heuristic | Signal | Emits |
|---|---|---|
| entropy | 256-byte window, every 64 bytes; bands from docs/04 (bank `$85` 1.5 bits/byte, `$80` 4.6, `$96` 7.0) | `Compressed` above 7.3, `Byte` below 2.0; mid bands annotate only |
| pointers | runs of ≥ 8 in-range 16- or 24-bit values under a candidate bank rule | `Table { elem: Pointer(rule) }` |
| ascii | ≥ 8 printable bytes terminated by `$00`/`$FF` | `String` |
| palette | 16/32/256 consecutive u16 with bit 15 clear, ≥ 6 distinct | `Palette` (the most reliable pure-data signal, to 0.8) |
| graphics | de-interleaved nibble variety and plane sparsity | **annotation only in Phase 2** |

`graphics` is the shakiest signal, so it produces evidence and a minimap
tint but classifies nothing until the accuracy harness clears it. Flipping
that later is a one-line change.

New priority order in `analyze()`: header 100 → **trace executed 95** →
descent 90/70 → conflict −30 → **jump-table entries** → sweep 30 → data
seeds 60/90/80 → **table extents 60–85** → **imported 85** → **trace read
85** → **heuristics, best first, capped 55** → user 100.

Two invariants, both tested:

- **Heuristics only fill `cls[b] == 0`.** They never argue with the
  disassembler, a trace, or the user. This is the difference between a map
  that is mostly right and a map that fights you.
- **Trace "executed" outranks static descent** (observation beats
  inference) but never the user; trace "read" fills only unknown bytes,
  since a byte can legitimately be both.

One structural change is required: regions are cut wherever
`(cls, conf, tag)` changes, and per-byte scores would shatter the dev ROM's
76 blocks into tens of thousands of fragments. Quantize heuristic confidence
to 5% steps and add a fourth array `hid: Vec<u16>` indexing a side table of
evidence, so runs break on `(cls, conf, tag, hid)`. Add a test bounding the
dev-ROM region count.

Cost: roughly 150–400 ms on the dev ROM against a 2 s budget, versus 7 ms
today. Full re-analysis per edit survives, but cache the ROM-derived
artefacts (entropy profile, trace bitmaps, imported symbols) on the
`Workbench` outside the project-dependent path. Add `AnalysisPhase::{Tables,
Heuristics, Import}` so progress stays honest.

### 3. Trace import — `model/coverage.rs`, `io/import/`

`Coverage { executed, read, written, entry, opcode_start: BitSet, flags }` —
five bitsets over 3 MB ≈ 1.9 MB, plain `Vec<u64>`, no new dependency.

**Mesen2 CDL** (verified against `Core/Debugger/CodeDataLogger.cpp` and
`DebugTypes.h`): 9-byte header, ASCII `CDLv2` then a u32 CRC32 of the ROM;
header-less older files are also accepted. Then one byte per ROM byte in
file order, aligning 1:1 with `FileOffset`. Flags `Code=0x01, Data=0x02,
JumpTarget=0x04, SubEntryPoint=0x08`; SNES `IndexMode8=0x10,
MemoryMode8=0x20`. Verify the CRC32 as a warning — our SHA-256 is the real
check.

This is a gift beyond coverage. `SubEntryPoint` seeds a real entry with a
`SUB_` label, and `MemoryMode8`/`IndexMode8` give the **actual M/X state at
every executed opcode** — precisely what defeats the descent after `PLP` or
`XCE`. Feed them into the walk as analyzer hints, **not** user
`FlagOverride`s: they must not enter `flags.json` or the undo stack.

**bsnes-plus usage map** (verified against
`snes/cpu/debugger/debugger.hpp`): no header, no magic; the CPU block is
`1<<24` bytes indexed by **24-bit bus address**, then SMP `1<<16`, and
optionally SA-1, SuperFX, SGB. Flags `Read=0x80, Write=0x40, Exec=0x20,
Opcode=0x10, FlagE=0x04, FlagM=0x02, FlagX=0x01`. Detect by size. Fold to
ROM-offset space by iterating **our** offsets and OR-ing mirrors, and store
only the folded one-byte-per-ROM-byte form in `traces/` — never the raw
16.8 MB, which would make a package unshareable.

**DiztinGUIsh**: implement `.dizraw`/`.dizdir` only, with an actionable
error on zipped `.diz` ("Save As → .dizraw in Diz"). The zip container and
the `RepeaterCompression` / `SubstitutionCompression` layers are unverified.
First candidate to cut — a Diz user can export a bsnes usage map *and* a WLA
`.sym` from Diz, both of which the other importers read.

Two real bugs this surfaces, worth fixing regardless:
`io::project_store::write_package` has no `create_dir_all` for a `traces/`
subdirectory and `read_package` is not recursive; and
`Project::region_override_at` is a linear `.find()` over a sorted vector
called once per region — switch to `partition_point` before the region count
grows.

`PROJECT_VERSION` → 2, with `traces` and `imports` arrays in `project.json`.
Every new field `#[serde(default)]`, plus a pinned test that a v1 package
opens.

### 4. Symbol import — `io/symbol_import.rs`

Order: **WLA-DX / bsnes-plus `.sym` first** — cheapest, highest value, and
it round-trips against our own `io/symbol_export.rs` for a free correctness
test while covering asar-WLA, bsnes-plus and most public symbol packs at
once. Then nocash, then ca65 / VICE `.lbl`. Defer ca65 `.dbg` to Phase 3:
its value is line and span records, useless until there are functions and a
source view.

Two things that will bite. **Name sanitation**: `validate_label_name`
enforces `[A-Za-z_][A-Za-z0-9_]{0,63}` and refuses auto-shaped names, but
real symbol files carry dots, `@`, ca65 `::` scopes and long names. Add
`sanitize_imported_label()` that rewrites, truncates, de-duplicates and
**reports the count rewritten**; never silently drop. **Collisions**:
imported never overwrites `LabelSource::User`, replaces a previous
`Imported` from the same source, and shadows `Auto`.

Licence notices: the file's leading comment block goes into
`project.json`'s `imports[].notice` (docs/12 rule 7). PJBoy's logs are never
parsed by Romlens and never redistributed (rule 6); they are read only by
the opt-in accuracy harness, from a local path the developer supplies.

`Project::apply_batch(rom, Vec<Command>, Origin)` returning one `UndoEntry`,
with `enum Origin { User, Import(String), Accepted(String) }`. Importing
20,000 labels as 20,000 undo entries is wrong. This is also the entire seam
Phase 3's Fix mode needs — an accepted proposal is the same call with
`Origin::Accepted(..)` — so the tutor costs no rework later.

### 5. Data typing, minimap, search, accuracy

No new `Command` variant: `MarkRegion` already carries
`OverrideKind::Data(DataKind)`, so undo is free. Add the `BankRule` /
`TableElem` parameters, `--stride` / `--elem` / `--bank` on the CLI, and a
"Mark as ▸" submenu with a parameter sheet in the shell. **The payoff to
make sure lands**: `viewmodel/asm_lines.rs` must render a pointer table as
`dw CODE_808423` — the resolved label with an xref, not a bare number. That
is the most legible single result of the phase.

`viewmodel/region_summary.rs`: `summarize(snapshot, entropy, buckets)`
where `buckets` is the strip's pixel width, so the **core** reduces and
Swift draws one rect per column. A `mixed` flag keeps it honest when one
pixel covers 3 KB. Note that `Workbench::regions_summary()` returns *every*
region as a record and `NavigatorModel` filters shell-side; that does not
survive a 100× region-count increase, so the strip uses the new flat batch
and the navigator moves to a bounded query.

Byte search: core and FFI are done. Add `Pattern::from_text` with
`--text` / `--ignore-case`, a richer `SearchHit` carrying context, and
`Views/Sheets/FindSheet.swift` (⌘F, mirroring `JumpToAddressSheet`) with a
results list and ⌘G / ⇧⌘G.

Accuracy harness in `crates/romlens-core/tests/ground_truth.rs` (Cargo only
runs `tests/` inside a package). Truth is a plain TSV, `start end kind`
under a `sha256=` header line. **The repository holds the converter and the
schema, never truth content**; `.gitignore` covers `tests/ground_truth/*.tsv`.
Two sources: `romlens truth from-cdl`, converting a CDL the developer
recorded from their own session, and `fixtures::truth_for(mapping)` — the
fixture builder knows its own layout, so **CI computes and prints an
accuracy number on data we built ourselves**. `romlens accuracy` prints
precision, recall and F1 plus the ten largest disagreement ranges, so a
regression is actionable rather than a number that went down.

**Targets, so the phase has a pass/fail:** code-byte recall ≥ **50%** of
labelled code, up from 0.2% of the image today; code-byte precision ≥
**0.98**. Precision is what protects the user — a map that confidently calls
data "code" is worse than one that says "unknown" — so tune every threshold
against precision first.

### 6. Graphics — `graphics/`

The decode rules are the specification; each has a named test file.

**Tiles.** One rule covers 2/4/8 bpp:
`byte_index(plane b, row y) = (b/2)*16 + y*2 + (b%2)`, `bit_index(x) = 7-x`,
`index(x,y) = Σ ((bytes[byte_index(b,y)] >> (7-x)) & 1) << b`. For 4 bpp
that expands to exactly what `07-memory-to-screen.md` spells out. Mode 7 is
linear, `bytes[y*8 + x]`. The teaching view is powered by one pure function
so hovering costs zero FFI calls:
`tile_bit_source(format, x, y, plane) -> Option<BitSource>`.

**Palettes.** `0bbbbbgggggrrrrr`; `c8 = (c5 << 3) | (c5 >> 2)`. Pin that
expansion explicitly — the Mesen2 tile-viewer comparison test depends on
both sides using the same one. `PaletteRef::{Cgram, Bytes, Grayscale}` is
what makes the tile view usable on unknown ROM bytes. BG palettes are rows
0–7; OBJ palettes are rows 8–15.

**OAM.** 512-byte low table plus a 32-byte high table; byte 3 is `vhoopppn`
with `n` as tile bit 9; the high table packs four sprites per byte as (X bit
8, size). X is 9-bit signed, −256..255. All eight `OBSEL` size pairs are
tabulated; large sprites take `n, n+1, … n+16 …` wrapping within the
256-tile page.

**Tilemaps.** `vhopppcc cccccccc`; a sub-map is 32×32 entries = 2048 bytes;
`BGnSC` bits 2–7 give the base and bits 0–1 the screen size, with sub-maps
at +0x400/0x800/0xC00 words (SC0, right, below, diagonal); `BGnNBA` nibble
<< 12 is the character base; 16×16 tiles expand `n, n+1, n+16, n+17`.

**`PpuState`** is a format *we* define, because write-only registers cannot
be read back. Specify its 256-byte layout in `13-recording-format.md`
**before** writing the Lua script, and name its fields from the existing
`model/hardware.rs` register table rather than re-listing them.

**How much reference PPU.** Build `render_bg_layer(vram, palette,
&BgConfig) -> Bitmap` and `render_sprite(...)`, then stop. No priority, no
TM/TS, no windows, no colour math, no OBJ-over-BG compositing, no mode 7, no
hi-res or interlace. The reason is testability: the only honest oracle for
priority and colour math is a recorded framebuffer to diff against, and
framebuffers are a Phase 3 consumer, so a compositor written now is several
hundred lines of unverified code rewritten at the first real comparison.
docs/13 calls the reference PPU "required" precisely *because* it draws a
frame when no framebuffer was recorded — and the Frame and Layers views are
Phase 3. The seam costs nothing: Phase 3 adds `compose(layers, …)` above
this exact inner loop. Tests pin a SHA-256 of the RGBA buffer — small,
diffable, and it ships no image.

**Super Metroid decompressor** (`graphics/compress/sm_lz.rs`), in but
separable. It is the only thing that makes ROM → tiles work on the
development ROM without an emulator, and it doubles as the best
compressed-blob detector the heuristics can have. There is no shippable
ground truth, so write a matching **compressor in the test module only** and
round-trip generated data in CI; the dev-ROM corpus check is opt-in under
`ROMLENS_ROM_DIR`. Verify the command-byte encoding against the Metroid
Construction write-up and an existing open decompressor before pinning any
golden — the format details are second-hand. This is the task allowed to
slip.

**No image-file export anywhere in Phase 2.** docs/12 rule 5 says the app
has no sprite-sheet export, and a CLI that writes `sheet.png` at an
arbitrary ROM offset *is* a sprite ripper. `--text` (the 8×8 index grid as
hex digits), `--json`, `--ascii` and `--digest` satisfy the CLI-twin rule,
make better goldens, and keep the dependency budget at exactly one new
crate. A PNG encoder belongs to Phase 3/4, when the scene exporter needs one
anyway. `decompress --out` writes bytes, not an image: the one exception.

### 7. Recordings — `recording/`

**`MachineStateSource`, frozen now** so Phase 3 and Phase 5 need not
redesign it. Name the region enum `StateRegion` from the first commit —
`model::region::Region` is taken, and renaming after the format is published
is expensive. Four deliberate deviations from the docs/13 sketch:

1. `Result` everywhere. A recording can be truncated, corrupt or mid-write;
   the sketch's infallible `state_at` would force panics.
2. `regions()`. A savestate import has VRAM/CGRAM/OAM and no WRAM; without
   this, views cannot tell "absent" from "all zeroes".
3. `identity()`. Refuse a recording made from a different ROM, exactly as
   `read_identity` already refuses a mismatched project package.
4. Live extras (`step_frame`, breakpoints, `poke`) go to a separate
   `LiveSource: MachineStateSource` in Phase 5, so `.romrec` implements no
   stubs and the read-only trait stays small and object-safe. That is the
   whole point of freezing it now.

Ship a `MemorySource` test double and a **shared conformance function run
against both it and `RomrecSource`** — about a hundred lines that decide
whether Phase 5's embedded core is a drop-in or a rewrite.

**`.romrec` layout.** A 128-byte fixed header plus a region table and string
area; per-frame `FRM\0` chunks with a region directory and one zstd frame
per payload; a 24-byte-per-frame index; layer chunks (`FBUF`, `WLOG`,
`TRCE`, `RLOG`) each with their own index; a 32-byte footer ending in `ROMR`
so truncation is detectable by reading four bytes.

The one layout choice that matters: **put every region's run table at the
head of the uncompressed payload** (`runs_area_len` in the chunk header).
That means `changes()` and `ChangeIndex` construction read a few hundred
bytes per frame instead of materialising 197 KB, and it is what makes the
index fast.

Write the header with `frame_count = u64::MAX`, stream frames, append the
index and footer, then seek back and patch. A file without a valid footer is
*in progress*; `--recover` rebuilds the index by scanning for chunk magics.
A crashed emulator session is the common failure, so this is worth the hour.

`ChangeIndex`: 256-byte blocks, delta-varint frame lists switching to a
bitmap above about 25% density, in a sidecar keyed on file size, footer
CRC32 and mtime. **The index narrows; it never answers** — candidates are
confirmed exactly by reading that frame's run table. Pin that with a
brute-force equivalence test.

The validator's eleven check families are the heart of 2C's test value: take
a valid file and corrupt it a dozen specific ways, asserting exact
diagnostic codes.

**Mesen2 capture: strategy (a)**, pure Lua typed reads, with dirty-range
narrowing and WRAM keyframe-only, and the format arranged so (b) can be
added later without a format change.

Strategy (b) would make `.romrec` production depend on parsing a GPLv3
project's *internal* savestate layout — that becomes our compatibility
surface and breaks on every Mesen2 release. Strategy (a)'s only risk is
throughput, and the reframing that removes most of the pressure is that
**throughput is not correctness**: Mesen2's Lua runs synchronously with
emulation, so if capture costs 30 ms per frame the emulator runs at 20 fps
and the recording is still a perfectly correct sequence of consecutive
frames. "Recording — the game will run slowly" is acceptable for a study
tool.

Two choices make it fast anyway. **Dirty-range narrowing**: write callbacks
on `snesVideoRam`, `snesCgRam` and `snesSpriteRam` (verified available, per
docs/09) accumulate touched ranges, and `endFrame` reads only those, plus a
full read every 60 frames for the keyframe — steady-state volume is about
2 KB per frame, not 197 KB. **WRAM keyframe-only** (`--wram
full|keyframe|off`, default `keyframe`, flagged in the header) removes most
of the rest; Phase 2 needs WRAM for nothing, since provenance is Phase 3.

Measurement, week one of 2C, before anything else: six variants from a
do-nothing `endFrame` baseline up to `createSavestate()` per frame, at three
scripted places, 600 frames each. **Check first** whether the targeted
Mesen2 version exposes a *bulk* memory read rather than byte-at-a-time
`emu.read`; if it does, the question evaporates. Decision rule: dirty-range
≥ 30 fps ships as designed; 10–30 fps ships documented; below 10 escalates
to `--every N` and window mode, then opaque savestate blobs, then a
different producer.

**Capture DMA events now**, even though the Provenance view is Phase 3.
docs/09 records that Mesen2 has no DMA channel query, so the recorder
reconstructs channel state by reading `$43x0`–`$437F` when `$420B` is
written and attributes VRAM writes during that instruction to the transfer.
It is free at record time and **impossible to add retroactively** to
recordings already made.

**Pull forward out of 2C, before 2B's shell work:** the trait, the writer,
and `rec import-raw` / `testrec`, about four days. That gives the four
graphics views a realistic VRAM/CGRAM/OAM to render before the Lua recorder
exists — exactly the role docs/13 assigns savestate import.

`zstd` is the only new dependency: feature-gate it behind `romlens-core`'s
`recording` feature (default on), add the `THIRD-PARTY-NOTICES.md` row under
the BSD-3-Clause grant, and run `scripts/check-cross.sh` on day one of 2C.
`ruzstd` (pure Rust, decode-only) is the fallback if the cross build bites.
CRC32 is about twenty lines, not a dependency.

Recordings are **referenced, never copied** into a project (docs/12 rule 4):
`recordings.json` holds `{ path, sha256, frameCount, addedAt }`.
`PROJECT_VERSION` stays 1 for this, because `from_files` ignores unknown
files.

### 8. macOS shell

| Item | Where |
|---|---|
| About credits carrying `apiVersion()` (row 0.12) | `App/MainMenu.swift`, `AppDelegate` |
| Find sheet and results list (rows 1.15 / 2.7) | `Views/Sheets/FindSheet.swift`, `Views/SearchResultsView.swift` |
| Mark as ▸ submenu and parameter sheet | `MainMenu.swift`, `Views/Sheets/DataTypeSheet.swift` |
| Import menu, file panels, async progress | `Document/ImportController.swift`, mirroring `ExportController` |
| Region strip | `Views/RegionStripView.swift`, wired into `EditorView.swift` |
| Evidence popover: scores sorted, warning severity, "go to the dispatching instruction" | `Views/Inspector/InspectorView.swift` |
| Four graphics tabs | `Views/Graphics/*.swift` |
| `BitmapInfo` → `CGImage` | `Model/Bitmap+AppKit.swift` |
| Open Recording…, frame stepper, changed badges, the docs/12 notice | `ProjectDocument`, toolbar |

Three shell decisions worth stating. **Tabs**: seven items in the principal
segmented control is too wide, so keep Hex / Disassembly / Both and add a
second `.menu`-style "Graphics" picker beside it, leaving Phase 0–1 muscle
memory untouched. **Shared selection**: widen `RomViewModel`'s selection to
`bytes(range) | tile(i) | oamEntry(i) | paletteEntry(i) | tilemapEntry(i)`
where every graphics case **also carries the byte range it came from**, so
the byte range stays the canonical join key and docs/02's "one selection
across code and graphics" falls out without a cross-view identity model.
**The strip** is a plain `NSView` outside any `NSScrollView`: it is a
whole-ROM overview and never scrolls.

One latent encoding problem to fix when per-data-kind tinting lands: the hex
span lane packs `0x80 | kind << 4 | confidence4` into one byte
(`workbench.rs`), leaving three bits for the kind, so `AsmTokenPalette`
collapses all eleven data kinds to `.byte`. docs/02 wants "data subtypes in
warm hues", so widen the encoding — three bits of confidence is plenty, or
use a second lane.

## Ordered tasks

Track 2A, about 17–20 days:

| # | Task | Size | Depends on |
|---|---|---|---|
| 1 | Foundations: `WarningKind::severity`, `apply_batch` + `Origin`, `write_package` subdirectories, `PROJECT_VERSION = 2` with a v1-opens test, the `hid` side table, `region_override_at` binary search | 0.5 d | — |
| 2 | Jump tables: `jumptable.rs`, the two descent sites, the pass loop, `TAG_TABLE`, `JTBL_`, xrefs, new warnings, fixture, synthetic tests, `romlens tables`, goldens | 2–3 d | 1 |
| 3 | **Measure and record.** Run the dev ROM; put the new coverage into docs/10 and docs/03. Not optional: this number is the phase's thesis | 1 d | 2 |
| 4 | Heuristics: five modules, priority insertion, `EntropyProfile`, `romlens heuristics`, tests | 2 d | 1 |
| 5 | Accuracy harness: truth TSV, `romlens accuracy`, `truth from-cdl`, fixture truth, CI wiring — deliberately before the importers, so everything after is measured | 1.5 d | 3 |
| 6 | Trace import: `coverage.rs`, CDL and usage map, trace flag hints, `traces/`, tests; then re-run 5 | 2 d | 5 |
| 7 | Symbol import: wla → nocash → lbl, sanitizer, batch command, notices, round-trip test | 1.5 d | 1 |
| 8 | Data typing: `BankRule` / `TableElem`, project JSON, FFI, labelled pointer rows with xrefs, CLI | 1.5 d | 2 |
| 9 | `region_summary.rs`, FFI batches, `romlens map` | 1 d | 4 |
| 10 | macOS: About, Find, Mark as ▸, Import menu, region strip, evidence popover, warning severity, XCTests | 3–4 d | 6–9 |
| 11 | Docs: docs/15 rows, docs/03 deltas, docs/10 measurements, manual-pass additions, final accuracy numbers | 1 d | 10 |

Track 2B, about 15–19 days: bitmap and crc32 skeleton plus **the three 2A
signatures first** (0.5) → tile (1.5) → palette (1) → oam (1.5) → tilemap
(1.5) → ppu_state (1) → render (2) → view models and previews (2) →
`graphics_lorom()` fixture (1) → CLI and goldens (2) → FFI (1.5) → macOS
tabs (3–4) → separable: `sm_lz` (2–3).

Track 2C, about 17–22 days: **Mesen2 measurement in week one, parallel with
2B** (1) → trait, `MemorySource` and the conformance suite (1) → format and
writer (2) → reader and `RomrecSource` (2) → delta and property tests (1) →
**`import-raw` and `testrec`, pulled forward to unblock 2B** (1) → validator,
CLI and goldens (2.5) → `ChangeIndex` (2.5) → **`mesen_recorder.lua`, the
highest uncertainty** (3–4) → optional `.mss` import (2–3) → FFI
`RecordingSession` and `recordings.json` (1.5) → macOS recording UI (2–3) →
`rec convert --window` (0.5).

Total 49–61 days. The roadmap's Phase 2 budget is four to eight weeks, so
**2A alone fits it; 2B and 2C are the overrun.** Treat them as separately
shippable follow-ons rather than pretending the whole phase fits.

## What is deliberately cut

1. DiztinGUIsh import beyond `.dizraw`/`.dizdir`, or entirely.
2. ca65 `.dbg` → Phase 3; its value is line and span records.
3. `DataKind::Struct` editing → Phase 3; Phase 2 marks it as an opaque blob.
4. RAM-pointer and hand-rolled long dispatch tables → Phase 3.
5. Incremental invalidation stays deferred, with a *measured*
   re-justification recorded in docs/03 rather than an assumption carried on.
6. The compositing reference PPU → Phase 3, framebuffer diff test first.
7. Framebuffer, execution-trace and read-log layers: format-reserved, reader
   and validator handle them, nothing produces or displays them.
8. The full write log → reduced to the DMA-event subset.
9. `recording::provenance` entirely → Phase 3. **But keep DMA capture.**
10. Any image-file export (PNG, QOI, sprite sheets).
11. Mesen2 `.mss` parsing: optional, last, version-gated.
12. A frame scrubber; Phase 2 gets a frame field with prev/next.
13. The Atlas and tutor Fix mode, per the scope decisions above.

Two additions the roadmap does not name, both small and high-leverage:
`romlens testrec`, the recording twin of `romlens testrom`, so every golden
and bug report works with no ROM and no committed binary; and the
`MemorySource` conformance suite for `MachineStateSource`.

## Risks and mitigations

- **A false-positive jump table poisons the map.** One wrong table seeds
  code across data and the descent propagates it. Mitigated by the high
  entry-plausibility bar, the `first_target_floor` ceiling, entry budgets, a
  precision-first accuracy gate in CI, evidence naming the dispatcher on
  every table byte so one "Mark as Data" undoes it, and `--no-tables` for
  A/B comparison.
- **Descent blow-up** as tables multiply the worklist. Per-pass and global
  entry budgets; the existing cancellation check; a progress phase so a long
  run is visible rather than mysterious.
- **Region fragmentation** from per-byte scores. Quantized confidence,
  window-aligned spans, the `hid` side table, and a test bounding the
  dev-ROM region count.
- **bsnes usage maps are 16.8 MB and bus-indexed.** Fold at import; store
  only the folded form.
- **Imported names break `validate_label_name`.** `sanitize_imported_label`
  with a reported rewrite count; never silently drop.
- **`PROJECT_VERSION` bump strands old packages.** Every new field
  `#[serde(default)]`, plus a pinned v1-opens test.
- **Content-policy slip** — a trace or symbol pack landing in the repo.
  `.gitignore` covers `tests/ground_truth/*.tsv` and `traces/`; the export
  path warns; a CI check that no large binary enters the tree.
- **Analysis cost makes the edit loop feel slow** at roughly 300 ms. Cache
  ROM-derived artefacts outside the project-dependent path; re-measure and
  record in docs/03.
- **Mesen2's Lua is too slow, or its API differs from the docs.** The
  week-one spike; the "recording below full speed is fine" reframing;
  dirty ranges; WRAM keyframe-only; `import-raw` as the always-works path; a
  producer-agnostic format; the three-step fallback ladder.
- **zstd adds C to a pure-Rust workspace.** Feature-gated; cross-check on
  day one; `ruzstd` as the decode-only fallback.
- **Hundred-megabyte recordings.** Window mode and `rec convert --window` in
  Phase 2 rather than Phase 3; WRAM keyframe-only; framebuffers off; `rec
  info` reporting per-region totals so the cost is visible before it hurts.
- **The reference PPU quietly grows into a full PPU** with nothing to check
  it against. `render_bg_layer` takes a `BgConfig`, not a compositing
  context; the compositor waits for the framebuffer diff test.
- **The SM decompressor's details are second-hand.** Verify before pinning
  goldens; test against a test-local compressor; keep the task separable.

## Verification (end to end)

1. `make test` green; `make test-rom` green with the dev-ROM pins.
2. `romlens analyze roms/SuperMetroid.F8DF.sfc --stats` shows code well
   above 0.2% — the headline check. Compare with `--no-tables` to attribute
   the gain.
3. `romlens analyze --warnings` shows the four `JSR through a table`
   warnings replaced by informational `jump table` entries.
4. `romlens tables`, `heuristics`, `map`, `accuracy`, `tiles --text`,
   `palette`, `oam`, `tilemap`, `rec info|validate|extract` all produce
   goldens on the fixtures, with no commercial ROM required.
5. `romlens accuracy` on the fixture truth runs in CI at precision ≥ 0.98;
   on the dev ROM (opt-in) recall ≥ 50%.
6. `make swift && make app-test` green; `make cross` and `make docker-test`
   still pass with `zstd` added.
7. In the app: open the dev ROM and watch the toolbar percentages move; the
   minimap shows structure instead of a grey bar; a region chip's popover
   carries a heuristic score; ⌘F finds `78 18 ?? 5C`; a range marked as a
   pointer table renders `dw CODE_…` with xrefs; the tile decoder on a known
   graphics range lights four plane bits under the cursor.
8. Import a CDL recorded from your own play session; confirm coverage lifts
   the map further and re-run `romlens accuracy`.
9. Re-run the manual pass in `15-conformance-checklist.md`, extended with
   the Phase 2 steps, and update the marks.
