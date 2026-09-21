# Memory to screen: which bytes are Samus?

## What this is and is not

Tile viewers, palette viewers, OAM viewers and tilemap viewers already exist
in Mesen2, bsnes-plus and a dozen ROM hacking tools. We are not rebuilding
them for their own sake. The educational question is different:

> Here is a sprite on screen. Which bytes in the cartridge are it, how are
> those bytes structured, and what path did they take to become pixels?

The answer is a **chain**, and the feature is the ability to walk that chain
in both directions: from a pixel back to ROM bytes, and from ROM bytes
forward to the frame. Every existing viewer becomes one **stage** of that
chain rather than a standalone window.

## The pipeline the app must teach

```
 ROM bytes              CPU                 WRAM buffer        DMA             PPU memories            PPU              Frame
 (often compressed) ──▶ decompression ──▶ $7E:xxxx ──────▶ channel n ──▶ VRAM  tiles + tilemaps ──▶ composition ──▶ 256×224
                        routine                            $420B          CGRAM palettes            BG modes,
                                                                          OAM   sprite table        priority,
 raw graphics ──────────────────────────────────────────▶  (direct DMA)                              windows,
                                                                                                     color math
```

Super Metroid compresses most graphics, so the two-hop path (ROM to WRAM by
a decompressor, WRAM to VRAM by DMA) is the common case. Palettes and OAM are
usually built in WRAM buffers and copied every frame. The Phase 2 tile
decoder works on any bytes; the provenance chain needs a recording.

## Provenance: the mechanism that makes the chain walkable

A **recording** (`13-recording-format.md`) is a frame-by-frame snapshot of
CPU and PPU memory and registers, produced by an emulator or any other
harness, plus optional layers: framebuffer, write log with the writing PC or
DMA channel, execution trace. Through Phase 4 Romlens embeds no emulator;
the Phase 5 debugger core will feed the same views through the same
interface.

From the snapshots alone the app can answer "when did this byte change"
exactly (binary search over frames) and "where did it come from" by
inference: the bytes that appeared in VRAM at frame N are searched for in
WRAM and ROM at frame N−1, and the captured DMA channel registers name the
last transfer. With the optional write log the answer is exact and carries
a program counter. The UI shows which confidence level each link has.
Walking back from a pixel is then a sequence of lookups:

1. pixel `(x, y)` at frame N → which layer won (BG1/2/3/4 or OBJ) and which
   tile or sprite;
2. sprite → OAM entry → tile number, palette, priority, flips, size;
3. tile number → VRAM word address (via OBSEL or BGnNBA base) → last writer:
   DMA channel 1 at frame N−12, source `$7E:xxxx`;
4. `$7E:xxxx` → last writer: the decompression routine at `$80:xxxx`,
   which read from ROM `$9A:xxxx` (from the WRAM write log or from the
   routine's own arguments);
5. ROM `$9A:xxxx` → bytes in the hex view, region marked as
   `graphics (compressed)` with evidence "provenance from recording X".

Two hops are enough for Super Metroid. The design allows more.

### Where the recording comes from

- **Always from outside.** Romlens ships a Mesen2 Lua recorder script
  (Mesen2's Lua API, verified from its own documentation, provides typed
  reads of `snesWorkRam`, `snesVideoRam`, `snesSpriteRam` and `snesCgRam`,
  `getState`, `getScreenBuffer`, `createSavestate` and `endFrame` events),
  a published format specification, and a validator, so bsnes-plus, a
  libretro frontend, a hardware capture rig or a homebrew developer's own
  harness can all produce recordings.
- **Phase 4:** the pipeline view goes live by scrubbing a recording, frame
  by frame, and instruction by instruction when an execution trace layer is
  present.
- **Phase 5:** the embedded debugger core is just another source; the
  pipeline updates as you step.
- The file format is ours (`.romrec`), so every source produces the same
  thing and the app never depends on an emulator's internal format.

## Views

All live in the Workbench as editor tabs and all share the single selection.
Selecting a sprite in the frame selects its OAM entry, its tiles in VRAM, its
palette row in CGRAM, its transfer in the log and its bytes in the hex view.

| View | What the student sees | Phase |
|---|---|---|
| **Tile decoder** | Any 16/32/64 bytes decoded as a 2/4/8 bpp tile. Bytes, bitplanes, pixel indices and palette shown side by side; hover a pixel to see its four bits light up in the four planes. Works on raw ROM bytes with no recording. | 2 |
| **Palette** | CGRAM as 16 rows of 16. Each entry decoded from 15-bit BGR to RGB with the bit fields shown. | 2 |
| **OAM table** | 128 entries decoded from the 4-byte low table plus the 2 high bits: x, y, tile, palette, priority, flips, size, name table. Sorted by screen position or by table order. | 2 |
| **Tilemap** | BG tilemap entries decoded (`vhopppcc cccccccc`), overlaid on the rendered layer. | 2 |
| **Frame** | The recorded framebuffer with a scrubber over frames. Click a pixel. | 3 |
| **Layers** | The same frame exploded into BG1, BG2, BG3, OBJ with priority order and per-layer toggles; the mode's rules shown as a legend (Mode 1: BG1 4bpp, BG2 4bpp, BG3 2bpp). | 3 |
| **Provenance chain** | A breadcrumb from pixel to ROM byte, each step a link, each step showing the evidence that connected it. | 3 |
| **Live pipeline** | The chain updating while scrubbing a recording; a DMA animates bytes moving from WRAM into VRAM using the captured channel registers. | 4 |

### The tile decoder, because it is the most teachable piece

A 4 bpp tile is 32 bytes. Rows 0–7 each take two bytes for planes 0 and 1,
then rows 0–7 again take two bytes for planes 2 and 3. Pixel `x` of row `y`
is bit `7−x` of each of the four plane bytes, and the colour index is
`p0 | p1<<1 | p2<<2 | p3<<3`, looked up in the 16-entry palette row chosen by
OAM or the tilemap entry. Students consistently find this surprising, which
makes it the best first lesson in "how memory is structured". The view
animates it: bytes fan out into four 8×8 bit grids, the grids stack, the
stack collapses into indices, the indices pick colours.

## Reference facts the views encode

| Memory | Size | Structure |
|---|---|---|
| VRAM | 64 KB | Word-addressed. Tiles at character base (`BG12NBA`/`BG34NBA`/`OBSEL`), tilemaps at `BGnSC` |
| CGRAM | 512 B | 256 colours × 15-bit `0bbbbbgggggrrrrr` |
| OAM | 544 B | 128 × 4 bytes (x low, y, tile low, `vhoopppN`) + 32 bytes of 2-bit pairs (x high, size) |
| Tile | 16 / 32 / 64 B | 2 / 4 / 8 bpp bitplanes, 8×8 pixels |
| Tilemap entry | 2 B | `v h o ppp cc cccccccc`: flips, priority, palette, 10-bit tile |
| Sprite sizes | per `OBSEL` | Pairs such as 8×8/16×16 or 16×16/32×32; large sprites use tiles `n, n+1, n+16, n+17…` |

Registers the views name: `$2101 OBSEL`, `$2102/03 OAMADD`, `$2104 OAMDATA`,
`$2105 BGMODE`, `$2107–$210A BGnSC`, `$210B/0C BGnNBA`, `$210D–$2114`
scroll, `$2115 VMAIN`, `$2116/17 VMADD`, `$2118/19 VMDATA`, `$2121 CGADD`,
`$2122 CGDATA`, `$212C/2D TM/TS`, `$4300–$437F` DMA channels, `$420B MDMAEN`.

## The lesson this enables: "Find Samus"

1. Load a recording. Scrub to a frame where Samus is standing.
2. Click Samus. The OAM table scrolls to the entries that make her up (she is
   several sprites), the Layers view highlights OBJ, the palette view
   highlights her palette row.
3. Click one tile. VRAM address shown; the provenance chain says it arrived by
   DMA at frame N−k from a WRAM buffer.
4. Follow the chain. The WRAM buffer was filled by the decompressor, whose
   source pointer names a ROM address in a graphics bank.
5. Land in the hex view on compressed bytes. The tutor can now explain the
   compression format and, with a trace layer, walk the decompressor
   instruction by instruction in Phase 4.

## Integration with the tutor

`decode_tile`, `ppu_state` and `pixel_provenance` are tutor tools, so a
student can also type "what is at (120, 96) in frame 812 and where did it
come from?" and receive the same chain as text with citations, plus an offer
to open it in the views.

## What we import instead of building

- Raw viewers with more options than ours: link out to Mesen2.
- Savestates: import VRAM/CGRAM/OAM from Mesen2 and bsnes savestates as a
  single-frame "recording" without provenance, so the decoders work even
  before the recorder script is ready.
- Compression formats: Super Metroid's is documented on Metroid
  Construction; the decompressor is small and we implement it in
  `SNESCore` so the forward path (ROM to tiles) works without an emulator.

## Open questions

- Software PPU for the Layers view: render layers ourselves from snapshots
  (a bounded reference renderer for Modes 0, 1 and OBJ first; Mode 7 later)
  or capture per-layer framebuffers from the emulator if its API allows.
  Lean: render ourselves, because it is also the foundation for Phase 4.
- Recording size: a log of every VRAM write over minutes of play is large.
  Lean: ring-buffer the write log and take snapshots every 60 frames; keep
  full logs only for user-marked windows.
