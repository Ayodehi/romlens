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
| 72 | 4 | layers present: bit 0 framebuffer, 1 write log, 2 trace, 3 read log, 4 register writes by line (1.1) |
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
| 2 | `io` | 128 | the I/O state block below |
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

A producer that holds the PPU's decoded state rather than the bytes
written (Mesen does) re-encodes each one-write register from it; see "The
Mesen stream" for which registers that leaves unknown until written.

**I/O state block**, 128 bytes (`recording/io_state.rs`). Defined in
September 2026 in the space 1.0 reserved; a file written before then
carries zeroes throughout, which readers treat as unknown.

| Offset | Size | Contents |
|---|---|---|
| `$00` | 1 | `NMITIMEN` (`$4200`) |
| `$01` | 1 | `HDMAEN` (`$420C`) |
| `$02` | 1 | `MEMSEL` (`$420D`) |
| `$03` | 1 | `WRIO` (`$4201`) |
| `$04` | 2 | `HTIME` (`$4207`/`$4208`) |
| `$06` | 2 | `VTIME` (`$4209`/`$420A`) |
| `$08` | 1 | `WRMPYA` (`$4202`) |
| `$09` | 1 | `WRMPYB` (`$4203`) |
| `$0A` | 2 | `WRDIVL`/`WRDIVH` (`$4204`/`$4205`) |
| `$0C` | 1 | `WRDIVB` (`$4206`) |
| `$0E` | 2 | the H counter |
| `$10` | 2 | the V counter |
| `$12` | 8 | `JOY1`–`JOY4` (`$4218`–`$421F`) |
| `$20` | 96 | the eight DMA channels, 12 bytes each: `$43x0`–`$43xB` |
| the rest | | reserved, zero |

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
A layer chunk follows the frame chunk it belongs to.

**`WLOG`, the write log.** Phase 2 writes only its DMA subset, and sets
header layer bit 1 when it does. The body is the frame (u64), a record
count (u32), then 104 bytes a record:

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | kind: 1, general DMA started by a write to `MDMAEN` |
| 1 | 1 | the value written to `MDMAEN` (`$420B`), the channels started |
| 2 | 2 | the scanline |
| 4 | 4 | reserved |
| 8 | 96 | the eight channels' `$43x0`–`$43xB` at the moment of the write |

Kind 3 (format 1.1, recorder stream 2) follows the kind 1 it belongs to
and says where the transfer's bytes went and who started it:

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | kind: 3, the context of the DMA start before it |
| 2 | 2 | the scanline |
| 4 | 2 | master cycles into the scanline |
| 8 | 2 | `VMADD`, the VRAM word address |
| 10 | 1 | `CGADD` |
| 11 | 2 | the OAM byte address |
| 13 | 4 | `WMADD`, the WRAM port address |
| 17 | 1 | the program bank (`K`) when `MDMAEN` was written |
| 18 | 2 | the program counter then |
| the rest | | zero |

Later kinds take new kind numbers; a reader skips kinds it does not know
by their fixed length.

**`LINE`, register writes by scanline** (format 1.1). One chunk after each
frame from the second on: every write to `$2100`–`$2133` made while that
frame was drawn, DMA's bytes through the data ports included, in order.
The body is the frame (u64), the write count (u32), a compression byte (as
the frame payloads: 0 none, 1 zstd), then the writes as one block, 6 bytes
each: the scanline in the frame's own numbering (i16; 0 is the frame's
first line, the vertical blank before it negative), the dot (u16), the
register less `$2100` (u8) and the value (u8). Replayed over the previous
frame's end (`recording::lines::Replay`), they give the registers and the
VRAM, CGRAM and OAM each visible line was drawn with, and arrive exactly at
the frame's own snapshot: every frame of ten games' recordings does
(`22-phase3-finish.md`, P1).

**Index**, `IDX\0`, a length, then 24 bytes per frame: frame (u64), file
offset of its chunk (u64), chunk length (u32), kind (u8), 3 reserved.

**Footer**, the last 32 bytes: index offset (u64), frame count (u64),
CRC-32 of the header (u32), CRC-32 of the index entries (u32), 4 reserved,
`ROMR`. A file whose last four bytes are not `ROMR` is still being written
or was cut short; `romlens rec info --recover` rebuilds the index by
walking the chunks from the end of the header and keeping every whole frame.

## Size estimates

| Scenario | Per frame compressed | Per minute at 60 fps |
|---|---|---|
| Idle gameplay, WRAM churn only | 2–8 KB | 7–30 MB |
| Scrolling with tilemap updates | 8–20 KB | 30–70 MB |
| Door transition, heavy VRAM DMA | 40–120 KB for a few frames | small overall effect |
| Keyframes | 60–120 KB each, once per second | 4–7 MB |
| Framebuffer layer | 20–60 KB | 70–220 MB |

**Measured**, 22 September 2026: 3,406 frames (57 seconds) of Super Metroid
gameplay, recorded with `mesen_recorder.lua` and packed by `rec pack`, came
to 8.4 MB, about 9 MB a minute. Keyframes averaged 94 KB and deltas 516
bytes with WRAM in keyframes only, 765 bytes with every frame's WRAM; the
DMA write log (11,394 transfers, stored uncompressed at 104 bytes each) was
1.2 MB of the total. That is at the low end of the estimates above, and
keeping every frame's WRAM adds under 1 MB a minute.

A ten-minute session without framebuffers lands around 100–400 MB; with
framebuffers, one to two gigabytes. The framebuffer layer is therefore off by
default and the reference PPU is the normal path. The recorder should
support a "window" mode that keeps full detail only between user marks.

## Producing a recording

Romlens ships:

1. **A Mesen Lua recorder script** (`recording/mesen/mesen_recorder.lua`,
   written by `romlens rec script`). It needs Mesen's "Allow access to I/O
   and OS functions" option, captures every frame end to a raw stream, and
   leaves everything else to `romlens rec pack <stream> --rom <rom> --out
   <rec>`: compression, the ROM hash, and the register layouts. Measured on
   Mesen 2.2.1 with Super Metroid, 22 September 2026: about 6 ms a frame
   with every region compared, over twice real time; 3,406 frames of
   gameplay make an 8.6 MB stream and an 8.4 MB recording (WRAM in
   keyframes only). The recorded VRAM, CGRAM, OAM and WRAM match direct
   dumps byte for byte at every frame checked. Strategy (a) of the Phase 2
   plan won outright; savestate blobs were never needed.
2. **A format specification and a validator** (`romlens rec validate`,
   `romlens rec info`, `romlens rec extract --frame N`) so anyone can write
   a producer: a bsnes-plus script, a libretro frontend, a hardware capture
   rig, or a homebrew developer's own test harness that emits snapshots from
   their build.

   `romlens rec validate R [--rom <rom>] [--sample N] [--strict]
   [--recover]` walks the whole file and reports every problem, not the
   first, each with a stable code (`recording/validate.rs`): `H` the
   header, `F` the footer, frame counts and both CRC-32s, `I` the index
   against the chunks, `T` the chunk walk, `K` keyframes and frame
   numbering, `D` each frame's directory, `R` runs, `P` payloads, `L` the
   layer bits and `WLOG` bodies, `W` WRAM against the keyframe-only flag,
   `M` the ROM, and `S` sampled frames rebuilt and their `changes` checked
   against what really changed. Errors break the format; warnings are
   allowed but not what a careful producer writes, and `--strict` fails
   on them too. Five of any one code are listed, then a count.
3. **Savestate import** as a one-frame recording without deltas, for people
   who just want to look at a moment.
   Loose dumps work today (`romlens rec import-raw`, File › Import
   Snapshot…); reading Mesen's `.mss` savestates is not built.

4. **Cutting a window** out of a long recording: `romlens rec convert R
   --from A --to B --out W` rebuilds each frame and renumbers from 0. The
   write log is not carried over, and WRAM kept in keyframes only is left
   out, since the cut's keyframes fall where the source has none.

## The Mesen stream

What `mesen_recorder.lua` writes and only `rec pack` reads
(`recording/mesen/stream.rs`). It is not a recording and has no stability
promise beyond its version number, since Romlens ships both ends.
Little-endian; `s1`/`s2` are strings with a u8/u16 length.

- **Header:** `RLSTREAM`, version (u16, now 2; `rec pack` reads 1 and 2), producer (`s2`), Mesen's
  ROM SHA-1 (`s2`, informational), start time (i64 Unix seconds), PRG ROM
  size (u32), 64 samples of 16 bytes taken at `size / 64 × i`, and the
  field names (u16 count, `s1` each): every numeric or boolean
  `emu.getState()` key under `cpu.`, `ppu.`, `internalRegisters.` and
  `dmaController.`, plus `frameCount`, `masterClock` and
  `memoryManager.hClock`.
- **`F`, a frame end:** frame (u32); the fields that changed (u16 count,
  then u16 index and i64 value each); 52 bytes, the last byte written to
  each of `$2100`–`$2133`; 52 bytes, whether each has been written; the 128
  bytes `$4300`–`$437F`; then for VRAM, CGRAM, OAM and WRAM in that order,
  the 256-byte blocks that changed (u16 count, then u16 block number and
  the block each; OAM's last block is 32 bytes).
- **`D`, a DMA start:** frame (u32), the byte written to `$420B`, the
  scanline (u16), and `$4300`–`$437F` at that moment. From version 2, then
  master cycles into the line (u16), `VMADD` (u16), `CGADD` (u8), the OAM
  address (u16), the WRAM port address (u32), `K` (u8) and `PC` (u16), read
  from `getState()` in the same callback.
- **`R`, the register writes of a frame** (version 2): frame (u32), count
  (u32), then 6 bytes a write as in the `LINE` chunk. Written just before the
  frame's `F`, from the second frame on. The script notes `emu.getMasterClock()`
  at each write, and at each frame end reads `masterClock`, `ppu.scanline` and
  `memoryManager.hClock`; 1,364 master cycles a line place each write on its
  scanline and dot, on stock Mesen as on the fork.
- **`L`:** frame (u32); a savestate was loaded before it.
- **`E`:** frames written (u32), the clean end. A stream without it was
  cut short; `rec pack` keeps every whole frame and says so.
- **`X`, an execution log:** length (u32), then an `.mxlog`
  (`17-execution-log.md`). Sent on a live connection only: the whole log on
  connecting, then once a second what the CPU did since, and a last one just
  before `E`. A file recording keeps its log beside it instead, so `rec pack`
  skips these.

The ROM check is the size and samples, because Mesen offers only SHA-1 and
Romlens hashes with SHA-256. `rec pack` builds the PPU state block from the
decoded fields, re-encoding each one-write register the way the game would
have written it, so a recording that starts from a savestate is right from
its first frame. Mesen does not export the window mask logic or `CGWSEL`'s
two window fields, so `WBGLOG`, `WOBJLOG` and `CGWSEL` come from the last
byte written and read as zero until the game writes them; `rec pack` names
them. Wherever both exist, it also checks each rebuilt register against the
byte the game last wrote and warns on any disagreement: none in 17,850
comparisons over Super Metroid's boot, nor in the 3,406-frame gameplay
recording.

## Live sessions

Added 23 September 2026. The recorder script can also send its stream to
Romlens while the game runs. File › Start Live Session (⌥⌘L) listens on
`127.0.0.1`, port 7462; `romlens rec live --rom <rom>` does the same from the
command line and reports what arrives. With Mesen's script settings allowing
both I/O and network access, the script tries to connect every two seconds,
using the LuaSocket library Mesen bundles, which stock releases have too.

- **The same bytes as the file.** A new connection gets the stream header and
  one full frame (every field and every memory block), then the deltas the
  file gets. So Romlens can connect at any point in a session, and one decoder
  (`StreamDecoder`) serves `rec pack` and live sessions alike. Checked by
  connecting at stream frame 840: the live session's last frame matched the
  packed file's VRAM, CGRAM and OAM byte for byte.
- **Never slows the game.** Sends are non-blocking. A connection that falls
  16 MB behind is dropped, and the file recording carries on regardless.
- **A window, not a file.** A `LiveSource` is a `MachineStateSource` like a
  `.romrec`, so the Tilemap, Tiles, Palette and OAM views read it unchanged. It
  keeps the latest 600 frames (ten seconds, about 40 MB) without WRAM, and
  numbers frames itself, so a script that reconnects carries on the count.
  The views follow the newest frame; stepping back pauses that, and stepping
  to the newest resumes it. Updates reach the views at most 30 times a
  second.
- **Local and checked.** Only loopback connections are accepted, and a stream
  recorded from another ROM is refused on its header. The sandboxed app has
  the `network.server` entitlement for this, and no other network access.

- **Code discovery live.** With the MesenCE fork's
  `emu.takeExecutionLogDelta()`, the connection also carries the execution
  log as `X` records. Romlens merges each into the open project
  (`Workbench::merge_live_log`, which keeps the undo history, unlike an
  import) and re-analyzes after its usual short pause, so the disassembly,
  the overview strip and the "seen" references fill in as the game plays;
  the header counts the instructions found. The deltas' counts are the
  increases since the previous one, so the merge is exact: over 1,500 frames
  of Super Metroid, 26 records merged to the same 2,953 instructions, 809
  transfers and access and DMA runs as the full log the script wrote at the
  end.

## The change index

`<recording>.romrec.idx`, beside the recording (`recording/change_index.rs`;
built by `romlens rec index`, and by `rec when` the first time it is
needed). It answers "which frames changed these bytes" without reading every
frame: each indexed region is cut into 256-byte blocks, and each block keeps
the frames whose change runs touch it, as delta-encoded LEB128 varints or,
when more than a quarter of the frames are in the list, a bitmap. Frame 0
counts as a change to everything, since it is where every byte's history
starts.

The index narrows and never answers: a candidate frame is returned only after
its own change runs are read and found to touch the bytes asked about.
WRAM is indexed only when every frame carries it. The file is `ROMRIDX\0`,
a u32 version (1), the key it belongs to (the recording's length, frame
count and the CRC-32 of its index entries: content, not a modification time),
then per region its id, block count, and per block a kind byte (0 varints,
1 bitmap), the entry count, the body length and the body. A sidecar whose key
does not match is rebuilt. Measured on the 18,478-frame session: built in
0.3 s, 45 KB, loaded in 5 ms.

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
