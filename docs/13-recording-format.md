# Recording format (.romrec)

Decided 21 September 2026. Through Phase 4, Romlens embeds no emulator: a
recording is produced by another program and Romlens supports the format.
The format is ours, so any emulator, tracer or homebrew test harness can
produce it, and Romlens depends on none of them. The format is also the
contract a future embedded core must satisfy (see "One interface, two
sources" below), so the debugger track does not change the views.

## What a recording must contain

The minimum is a **frame-by-frame snapshot of CPU and PPU memory and
registers**. Everything the Frame, Layers, Provenance and tutor features
need can be derived from that. Optional layers add instruction-level detail
when the source can provide it.

### Required per frame

| Region | Size | Why |
|---|---|---|
| CPU registers | 16 B | A, X, Y, S, D, DB, PB, PC, P, E at the frame boundary |
| PPU state | ~256 B | Every write-only PPU register's current value as the emulator holds it: BGMODE, BGnSC, BG12NBA, BG34NBA, scroll latches, OBSEL, OAMADD, VMAIN, VMADD, CGADD, TM, TS, window and color-math registers, INIDISP, mosaic, mode 7 matrix. These are not readable from the bus, so the recorder must export them from emulator state |
| CPU/IO state | ~128 B | NMITIMEN, HDMAEN, MEMSEL, WRIO, multiplier/divider, the eight DMA channel register sets ($43x0–$43xA), joypad state, H/V counters |
| WRAM | 128 KB | $7E:0000–$7F:FFFF |
| VRAM | 64 KB | tiles and tilemaps |
| CGRAM | 512 B | palettes |
| OAM | 544 B | 512 B low table + 32 B high table |
| Frame number and timing | 16 B | frame index, master clock or field, whether the frame was interlaced or a 239-line frame |

Total raw state: about 197 KB per frame.

### Optional per frame

| Layer | Size | What it enables |
|---|---|---|
| Framebuffer | 256×224 (or 512×448 for hi-res) RGB, QOI or PNG compressed, typically 20–60 KB | The Frame view shows the emulator's own image instead of our reference render |
| APU RAM and DSP registers | 64 KB + 128 B | Audio study, SPC engine visualization |
| SRAM | up to 128 KB | Save-file structure lessons |
| Input | 4–12 B | Controller state per frame, for "what did the player do" |
| Write log | variable | Every write to WRAM, VRAM, CGRAM, OAM with the writing PC or DMA channel; gives exact provenance instead of inferred |
| Execution trace | variable, large | Every instruction with registers; enables instruction-level scrubbing inside a frame |
| Read log | variable | Which ROM bytes were read as data; the CDL-style signal for classification |

### What snapshots alone can answer

- **When did this byte change?** Binary search over frames for VRAM, CGRAM,
  OAM and WRAM. Exact, no write log needed.
- **Where did it come from?** For a VRAM region that changed at frame N,
  search WRAM of frame N−1 (and ROM) for the same bytes. A match in WRAM
  names the buffer; a match in ROM names the source directly. This
  content-match heuristic is how the two-hop provenance chain works without
  a write log. It is marked as inferred in the UI; the write log upgrades it
  to exact with a PC.
- **Which instruction wrote it?** Only with the write log or execution
  trace. The DMA channel registers captured at frame N give the last
  transfer's source, destination and size, which usually settles it for VRAM.
- **What did the PPU draw?** The reference PPU renders BG layers and OBJ
  from VRAM, CGRAM, OAM and the PPU state. The optional framebuffer is the
  ground truth to compare against.

## File layout

A single file, chunked, with an index, so it can be shared and seeked.

```
romrec file
├── header            magic "ROMREC\0", version, source emulator + version,
│                     recorder script version, ROM SHA-256, mapping mode,
│                     region list (name, size), layers present, frame count,
│                     keyframe interval, compression ("zstd"), timestamps
├── frame index       per frame: byte offset, kind (key | delta), sizes
├── frames…           keyframe: every region, zstd-compressed
│                     delta: for each region, a sparse list of changed
│                     runs (offset, length, bytes) relative to the previous
│                     frame, zstd-compressed; small records (registers,
│                     timing, input) stored in full every frame
├── optional layers   framebuffer chunks, write log chunks, trace chunks,
│                     each with its own index so a reader can skip them
└── footer            index offset, CRC32 of the header and index
```

- **Keyframe interval:** 60 frames by default (one second). Seeking decodes
  at most 59 deltas; a delta decode is a memcpy of changed runs.
- **Delta encoding:** compare the region against the previous frame, emit
  runs of changed bytes, merge runs closer than 16 bytes, compress. Sparse
  runs rather than XOR so unchanged memory costs nothing and the reader can
  answer "what changed this frame" without decompressing everything.
- **Register records** are stored raw every frame. They are tiny and random
  access to them must be instant.
- **Endianness:** little-endian throughout, matching the machine.

## Byte layout, version 1.0

Implemented 22 September 2026 (`romlens-core/src/recording/format.rs`) and
the contract for every producer. Little-endian throughout; offsets are
bytes from the start of the structure.

**Header**, 128 bytes, then a region table and a string area:

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | magic `ROMREC\0\0` |
| 8 | 2 + 2 | format version, major then minor (1.0). A reader refuses a newer major |
| 12 | 4 | header length, including the table and strings |
| 16 | 32 | SHA-256 of the ROM, without any copier header — a reader refuses a recording of another ROM |
| 48 | 1 | mapping: 0 LoROM, 1 HiROM, 2 ExHiROM, `$FF` unknown |
| 49 | 1 | flags: bit 0, WRAM is stored in keyframes only |
| 50 | 2 | keyframe interval |
| 52 | 4 | region count |
| 56 | 8 | frame count; `$FFFFFFFFFFFFFFFF` while the file is being written |
| 64 | 8 | created, Unix seconds (0 when the producer does not say) |
| 72 | 4 | layers present: bit 0 framebuffer, 1 write log, 2 trace, 3 read log |
| 76 | 4 | compression: 0 none, 1 zstd (one frame per payload) |
| 80 | 4 + 4 | producer name: offset into the string area, length |
| 88 | 4 + 4 | producer version, likewise |
| 96 | 32 | reserved, zero |

Region table entries are 16 bytes: id, size, name offset, name length.
Sizes are fixed by the id; a mismatch is an error, not a variant.

| Id | Name | Size | Contents |
|---|---|---|---|
| 0 | `cpu` | 16 | A, X, Y, S, D (u16 each), DB, PB (u8), PC (u16), P (u8), E (0 native, 1 emulation) |
| 1 | `ppu` | 256 | the PPU state block below |
| 2 | `io` | 128 | reserved for NMITIMEN, HDMAEN, the DMA channels and the rest; Phase 2 writes zeroes |
| 3 | `wram` | 131072 | `$7E:0000`–`$7F:FFFF` |
| 4 | `vram` | 65536 | byte-addressed: word *w* is bytes 2*w*, 2*w*+1 |
| 5 | `cgram` | 512 | 256 BGR15 colours |
| 6 | `oam` | 544 | low table then high table |
| 7 | `timing` | 16 | frame index (u64), then reserved |

A recording may carry any subset; one made from loose dumps typically has
`ppu`, `vram`, `cgram` and `oam` only, and a view that needs an absent
region says so rather than drawing zeroes.

**PPU state block**, 256 bytes. The CPU cannot read these registers back,
so the recorder exports them from emulator state:

| Offset | Size | Contents |
|---|---|---|
| `$00`–`$33` | 1 each | `$2100`–`$2133` at offset `address − $2100`, the last byte written. The one-write registers (`INIDISP`, `OBSEL`, `BGMODE`, `MOSAIC`, `BGnSC`, `BGnNBA`, `VMAIN`, `M7SEL`, the window and colour-math registers, `TM`/`TS`/`TMW`/`TSW`, `SETINI`) are read from here |
| `$40`–`$4F` | 2 each | `BG1HOFS`, `BG1VOFS` … `BG4VOFS`: the value both writes built |
| `$50`–`$5B` | 2 each | `M7A` `M7B` `M7C` `M7D` `M7X` `M7Y`, signed |
| `$5C` | 2 | `OAMADDL` \| `OAMADDH` << 8 |
| `$5E` | 2 | `VMADDL` \| `VMADDH` << 8 |
| `$60` | 1 | `CGADD` |
| `$62` | 2 | the fixed colour `COLDATA` built, BGR15 |
| the rest | | reserved, zero; readers ignore it |

The names are the core's hardware register table's (`model/hardware.rs`),
so there is one list of PPU registers, not two.

**Frame chunk**, one per frame in frame order:

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | `FRM\0` |
| 4 | 4 | length of everything after this field |
| 8 | 8 | frame index, counting from 0 |
| 16 | 1 | 0 keyframe, 1 delta; then 3 reserved |
| 20 | 4 | directory entries |
| 24 | 4 | run area length |
| 28 | 20 each | directory: region id, data-run count, change-run count, stored length, raw length |
| … | | run area, uncompressed: per entry, its data runs then its change runs, 8 bytes each (offset, length) |
| … | | payloads, one per entry: the data runs' bytes concatenated, compressed |

A keyframe stores every region whole, as one data run, and lists what
changed since the previous frame as change runs. A delta frame lists only
regions that changed, and their data runs *are* the changes; a region
absent from a delta frame's directory did not change. The small regions
(`cpu`, `ppu`, `io`, `timing`) are stored whole in every frame. Changed
bytes fewer than 16 apart share one run.

Putting the run tables before the payloads is the layout choice that
matters: saying what changed in a frame reads a few hundred bytes of the
file and decompresses nothing.

**Other chunks.** `FBUF`, `WLOG`, `TRCE` and `RLOG` are reserved for the
optional layers, with the same magic-and-length framing; readers skip them.

**Index**, `IDX\0`, a length, then 24 bytes per frame: frame (u64), file
offset of its chunk (u64), chunk length (u32), kind (u8), 3 reserved.

**Footer**, the last 32 bytes: index offset (u64), frame count (u64),
CRC-32 of the header (u32), CRC-32 of the index entries (u32), 4 reserved,
`ROMR`. A file whose last four bytes are not `ROMR` is still being written
or was cut short; `romlens rec info --recover` rebuilds the index by
walking the chunks from the end of the header and keeping every whole frame.

## Size estimates (to verify with a real capture)

| Scenario | Per frame compressed | Per minute at 60 fps |
|---|---|---|
| Idle gameplay, WRAM churn only | 2–8 KB | 7–30 MB |
| Scrolling with tilemap updates | 8–20 KB | 30–70 MB |
| Door transition, heavy VRAM DMA | 40–120 KB for a few frames | small overall effect |
| Keyframes | 60–120 KB each, once per second | 4–7 MB |
| Framebuffer layer | 20–60 KB | 70–220 MB |

A ten-minute session without framebuffers lands around 100–400 MB; with
framebuffers, one to two gigabytes. The framebuffer layer is therefore off by
default and the reference PPU is the normal path. The recorder should
support a "window" mode that keeps full detail only between user marks.

## Producing a recording

Romlens ships:

1. **A Mesen2 Lua recorder script.** Mesen2's Lua API (verified from its own
   documentation) provides typed memory reads for `snesWorkRam`,
   `snesVideoRam`, `snesSpriteRam` and `snesCgRam`, `getState()` for CPU and
   PPU registers, `getScreenBuffer()`, and `endFrame` events. Two capture
   strategies to measure in the first week of Phase 2: (a) read every
   region byte by byte each frame from Lua, which is simple but may be slow
   at 197 KB per frame; (b) take `createSavestate()` per frame and let the
   Romlens importer parse Mesen2's savestate format, which is fast but ties
   the importer to Mesen2's internal layout and version. A hybrid, typed
   reads for registers plus savestate blobs for memory, is the likely answer.
   Write callbacks on the PPU memories can mark dirty ranges so (a) only
   reads what changed.
2. **A format specification and a validator** (`romlens rec validate`,
   `romlens rec info`, `romlens rec extract --frame N`) so anyone can write
   a producer: a bsnes-plus script, a libretro frontend, a hardware capture
   rig, or a homebrew developer's own test harness that emits snapshots from
   their build.
3. **Savestate import** as a one-frame recording without deltas, for people
   who just want to look at a moment.

## One interface, two sources

Every dynamic view (Frame, Layers, Provenance, register panels, the tutor's
recording tools, the scene player) consumes a `MachineStateSource`:

```
trait MachineStateSource {
    fn frame_count(&self) -> Option<u64>;        // None for a live source
    fn state_at(&self, frame: u64) -> MachineState;
    fn changes(&self, from: u64, to: u64, region: Region) -> Vec<Run>;
    fn layers(&self) -> Layers;                  // framebuffer, write log, trace
    // live sources add: step_frame, step_instruction, breakpoints, poke
}
```

A `.romrec` file is one implementation. A future embedded emulator core is
another, and it records to the same format as it runs, so a debugging
session is a recording you can scrub backwards. That is the path to a
debugger visualizer without redesigning any view.

## Implications for the rest of the plan

- No emulator core is embedded through Phase 4. The permissively licensed
  cores identified in `09-emulator-core-licensing.md` (super-sabicom,
  LakeSnes, ares) are the candidates for the debugger track after that;
  GPL cores stay out of process regardless.
- The Phase 4 "live stepping" feature is **scrubbing a recording** at frame
  granularity, and at instruction granularity when the execution trace layer
  is present. Instruction-level stepping of live code is the debugger
  track's job.
- Provenance has two confidence levels: inferred from snapshots (content
  match) and exact from the write log. The UI shows which.
- The reference PPU is required, not optional, because it is how a frame is
  drawn from a snapshot when no framebuffer layer was recorded.
- Homebrew developers can produce recordings from their own toolchain's
  emulator and get every feature, since nothing depends on a commercial ROM.
