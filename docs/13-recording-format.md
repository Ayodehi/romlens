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
