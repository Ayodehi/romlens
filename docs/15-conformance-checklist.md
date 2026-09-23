# Frontend conformance checklist

Parity between the macOS, Windows and Linux shells is defined here, not by
screenshots. Each core capability lists what a shell must expose and the CLI
scenario that checks the same behaviour from a script
(`08-cross-platform.md`, rule 7). A shell "conforms" to an item when a person
following the shell column gets the same answer as the CLI column.

Status: macOS ✅ done, 🧪 built and covered by the app test bundle but not
yet verified by hand, ⬜ not yet; Windows and Linux start after macOS
Phase 1. The macOS Phase 0 manual pass was completed on 21 September 2026
(items 0.1–0.11), which also caught the NSTableView row-view leak, the
missing app delegate and the sideways-scroll bug; the Instruments pass is
still open. The Phase 1 rows were implemented the same day; their manual
pass is listed below and still to run. The Phase 2 rows were written from
`16-phase2-plan.md` on 22 September 2026; tracks 2A and 2B are built (🧪)
and track 2C is started, with the details under Phase 2.

## Phase 0

| # | Capability | Shell must expose | CLI scenario | macOS | Win | Linux |
|---|---|---|---|---|---|---|
| 0.1 | Open a ROM (`.sfc`/`.smc`) | File › Open, Open Recent, drag onto the app; copier header stripped silently | `romlens info <rom>` prints size and "copier header stripped" when present | ✅ | ⬜ | ⬜ |
| 0.2 | Refuse non-ROMs with the core's message | The error text comes from the core, verbatim | `romlens info /dev/zero`-style input exits 1 with "no valid SNES header" | ✅ | ⬜ | ⬜ |
| 0.3 | Header summary | Mapping, FastROM, header offset, title, sizes, region, developer, version, checksum with mirrored-sum verdict, SHA-256, twelve vectors | `romlens info <rom>` (and `--json`) | ✅ | ⬜ | ⬜ |
| 0.4 | Hex view, 16 bytes per row, virtualized | Scrolls the whole image smoothly; sticky column header `00`–`0F` with the selected column emphasised; last row may be partial | `romlens hex <rom> --from <expr> --rows N` | ✅ | ⬜ | ⬜ |
| 0.5 | Dual address column with toggle | Both / SNES / file, switchable without refetching | `--address both\|snes\|file` | ✅ | ⬜ | ⬜ |
| 0.6 | Header and vector overlays | Coloured spans over the header bytes; name and decoded value on hover or selection; click a span in the summary to jump | Rows carrying a span are marked `*` in `romlens hex`; `romlens info` lists the values | ✅ | ⬜ | ⬜ |
| 0.7 | Jump to address | ⌘L / Ctrl+L sheet; live preview `0x00041C = $80:841C` or the core's message; Return jumps and centres the row | `romlens resolve <rom> <expr>` | ✅ | ⬜ | ⬜ |
| 0.8 | Address expressions | `$80:841C`, `80:841C`, `$80841C`, `0x41C` all accepted | same | ✅ | ⬜ | ⬜ |
| 0.9 | Byte inspector | File offset, on-disk offset when a copier header exists, canonical SNES address, mirrors, u8/i8, u16/i16 LE, u24, ASCII, u16 in current bank and u24 as SNES address with Go, span name | `romlens resolve` gives the addresses and mirrors; readings come from the FFI `inspect` (a CLI `inspect` command is due with Phase 1) | ✅ | ⬜ | ⬜ |
| 0.10 | Keyboard navigation | Arrows move by 1 and 16 bytes, Page Up/Down by a page, Home/End, Backspace or ⌘[ goes back in jump history | n/a (interaction) | ✅ | ⬜ | ⬜ |
| 0.11 | Homebrew test ROM | The shell opens the fixture the CLI writes | `romlens testrom --out t.sfc --mapping lorom\|hirom\|exhirom` | ✅ | ⬜ | ⬜ |
| 0.12 | API version visible | About box credits and a launch log line show the core API version the shell was built against, read from the core at runtime | `romlens --version` | 🧪 | ⬜ | ⬜ |

## Phase 1

| # | Capability | Shell must expose | CLI scenario | macOS | Win | Linux |
|---|---|---|---|---|---|---|
| 1.1 | Analysis runs on open, off the main thread | Progress and phase in the toolbar with ⌘. to cancel; the hex view is usable meanwhile; percentages when done; tap to re-run | `romlens analyze <rom> --stats [--progress] [--warnings]` | 🧪 | ⬜ | ⬜ |
| 1.2 | Disassembly view, virtualized | One line per instruction or data row; section, label, comment and blank lines; tokens coloured by kind, auto labels dimmer than user labels; region gutter and tint by confidence | `romlens disasm <rom> --from <expr> --count N [--verbose]` | 🧪 | ⬜ | ⬜ |
| 1.3 | Raw decode under chosen flags | (inspector shows the analyzer's flags; a raw decode is a tutor tool) | `romlens disasm <rom> --flags m1x0e0` | n/a | ⬜ | ⬜ |
| 1.4 | Hex ↔ asm lockstep | Both tab: scrolling either side keeps the other aligned; the highlighted bytes and their line are joined by a bracket; selecting in one selects in the other | `romlens disasm --from <expr>` shows the line the bytes belong to | 🧪 | ⬜ | ⬜ |
| 1.5 | Instruction inspector | Mnemonic and mode with a one-line description, bytes, operand, effective target with Go, flags before → after, DBR/DP known or unknown, assumptions and warnings | `romlens inspect <rom> <expr>` | 🧪 | ⬜ | ⬜ |
| 1.6 | Regions and evidence | Region kind and confidence on every line and in the inspector; evidence one click away (vector reach depth, heuristic, user) | `romlens inspect` prints the region and its evidence | 🧪 | ⬜ | ⬜ |
| 1.7 | Labels | Auto labels `RESET_/NMI_/…/SUB_/CODE_/PTR_/DATA_` + address; user rename (`n`, Rename Label…, inspector field) validated live; remove restores the auto name | `romlens labels <rom> [--source]`, `romlens project <P> label <expr> <name\|->` | 🧪 | ⬜ | ⬜ |
| 1.8 | Comments | Line comment after the instruction, block comment above (`;`, Comment…, inspector fields) | `romlens project <P> comment <expr> <text\|-> --line\|--block` | 🧪 | ⬜ | ⬜ |
| 1.9 | Region marks and flag overrides | `c`/`d`/`u` and Mark as … on the highlighted range; Set Flags… pins M/X/E/DBR/DP; the analysis re-runs after a short pause | `romlens project <P> mark <expr> <len> <kind>`, `clear`, `flags <expr> --m 0\|1 …` | 🧪 | ⬜ | ⬜ |
| 1.10 | Cross-references | Referenced-by and references lists in the inspector, click to jump; double-click or `g`/⌘↩ follows a target | `romlens xrefs <rom> <expr>` | 🧪 | ⬜ | ⬜ |
| 1.11 | Navigator | Labels (filter by name or `$address` prefix, user first), regions, banks; selection jumps | `romlens labels`, `romlens analyze --json` | 🧪 | ⬜ | ⬜ |
| 1.12 | Undo and redo | ⌘Z / ⇧⌘Z with the command's title; one stack in the core; the document's edited state follows every command | `romlens project <P> history` lists the package's contents | 🧪 | ⬜ | ⬜ |
| 1.13 | Project package | Opening a ROM makes an untitled `.romlens`; Save/Save As/Revert/Duplicate; reopening finds the ROM by hash, then the remembered path, then asks (and rejects a wrong file) | `romlens project <P> init --rom <rom>`; every command takes `--project` | 🧪 | ⬜ | ⬜ |
| 1.14 | Export | File › Export ▸ Assembly Listing… (byte columns off by default, docs/12 message), Labels and Comments…, Symbol File… | `romlens export asm <rom> --out F [--range a..b]`, `export sym [--include-auto]` | 🧪 | ⬜ | ⬜ |
| 1.15 | Byte search | (see 2.7) | `romlens search <rom> "78 18 ?? 5C"` | n/a | ⬜ | ⬜ |
| 1.16 | Hardware registers | `STA $420D` carries `; MEMSEL`; the inspector shows the register's access and description | `romlens registers [address]` | 🧪 | ⬜ | ⬜ |
| 1.17 | Hex region tint | Hex rows tint code and data bytes by confidence after analysis without a refetch | (the hex-row batch's span lane) | 🧪 | ⬜ | ⬜ |
| 1.18 | Range selection | Shift+arrows and shift-click extend the highlighted range in either canvas; mouse-drag and cross-canvas drag are not in Phase 1 | n/a (interaction) | 🧪 | ⬜ | ⬜ |

## Phase 2

Planned in `16-phase2-plan.md`. Track 2A (classification) owns 2.1–2.8,
track 2B (graphics) 2.20–2.27 and track 2C (recordings) 2.28–2.36. The
numbering leaves a gap after 2.8 so 2A can grow without renumbering the other
tracks.

Track 2A landed on 22 September 2026: jump tables, scored heuristics, trace
and symbol import, typed data, the overview strip, byte and text search, and
the accuracy harness, each with goldens that need no commercial ROM. Its rows
are 🧪 — built and covered by the app test bundle, not yet verified by hand —
except 2.8, whose shell half is deliberately not built: the toolbar already
reports the classified percentage from the analysis, and the accuracy number
needs ground truth that never ships, so it stays a CLI capability.

2.3's DiztinGUIsh column is dropped rather than deferred: the format was never
verified, and a Diz user can export a bsnes usage map or a WLA `.sym`, both of
which are read.

Track 2B (graphics) landed the same day, with the part of 2C the plan pulled
forward so the views had a machine to draw: the `.romrec` reader and writer,
`testrec`, `rec info`, `rec extract`, `rec import-raw`, and File › Open
Recording… with a frame field. Its rows are 🧪. Of 2C's rows, those halves are
marked where they landed; the validator, the change index, the Mesen2 recorder,
savestate import, attaching recordings to a project and Export Frame Region…
are not started.

### 2A — classification

| # | Capability | Shell must expose | CLI scenario | macOS | Win | Linux |
|---|---|---|---|---|---|---|
| 2.1 | Jump tables resolved | Table rows render `dw CODE_…` with a "jump table" region badge; the evidence popover names the dispatching instruction and links to it; the `computed jump` warning becomes informational | `romlens tables <rom> [--json]` | 🧪 | ⬜ | ⬜ |
| 2.2 | Heuristic scores visible | The evidence popover lists every heuristic with its score, sorted descending, with the detail string ("entropy 7.4 bits/byte over `$96:0000`–`$96:8000`") | `romlens heuristics <rom> [--kind …] [--from --to] [--json]` | 🧪 | ⬜ | ⬜ |
| 2.3 | Trace and coverage import | File › Import ▸ Execution Trace…; a coverage lane in the overview strip; the analysis status reports traced bytes | `romlens import trace <P> --rom R <file> [--format cdl\|usage]` | 🧪 | ⬜ | ⬜ |
| 2.4 | Symbol import | File › Import ▸ Symbols…; imported labels visibly distinct from auto; collisions and rewritten names reported, never silently dropped | `romlens import symbols <P> --rom R <file> [--format wla\|nocash\|lbl] [--source NAME]` | 🧪 | ⬜ | ⬜ |
| 2.5 | Data typing with parameters | Mark as ▸ submenu covering every data kind, with a sheet for stride, bank rule, element kind and bpp; a pointer table renders as labelled targets with xrefs | `romlens project <P> mark <expr> <len> table --stride 2 --elem code\|pointer\|raw --bank same\|$C0\|entry` | 🧪 | ⬜ | ⬜ |
| 2.6 | Region overview strip | A minimap under the editor coloured by kind and confidence, hatched where a bucket is mixed, with the selection as a caret and a lane for traced bytes; click and drag to jump | `romlens map <rom> [--buckets N] [--json]` | 🧪 | ⬜ | ⬜ |
| 2.7 | Byte and text search | ⌘F sheet reporting the core's message; a results list under the editor showing each hit's bytes in context; ⌘G / ⇧⌘G for next and previous; Return jumps and centres | `romlens search <rom> "78 18 ?? 5C"`, `romlens search <rom> --text "Nintendo" [--ignore-case]` | 🧪 | ⬜ | ⬜ |
| 2.8 | Classifier accuracy | A line at the end of analysis reporting the classified percentage | `romlens accuracy <rom> --truth F\|--fixture [--project P] [--json]`, `romlens truth from-cdl <rom> <cdl> --out F` | ⬜ | ⬜ | ⬜ |

### 2B — graphics

| # | Capability | Shell must expose | CLI scenario | macOS | Win | Linux |
|---|---|---|---|---|---|---|
| 2.20 | Tile decoder on raw ROM bytes | Tile decoder tab; 2/4/8 bpp; a palette picker; bytes, the bitplane grids, the index grid and the zoomed tile side by side; hovering a pixel lights its bit in each plane and its byte in the strip | `romlens tiles <rom> --from <expr> --bpp 4 --text` prints the 8×8 index grid; `--json` adds the per-plane bytes | 🧪 | ⬜ | ⬜ |
| 2.21 | Tile sheet browsing | A scrolling sheet at the chosen bpp and column count; clicking a tile selects its bytes in the hex view | `romlens tiles <rom> --from <expr> --count 64 --columns 16 --text` | 🧪 | ⬜ | ⬜ |
| 2.22 | Palette view | 16×16 swatches; the entry detail shows the raw `$7FFF`, the 5-bit B/G/R fields and the 8-bit RGB; clicking a swatch selects its two bytes | `romlens palette <rom> --from <expr> [--count 256] [--json]` | 🧪 | ⬜ | ⬜ |
| 2.23 | OAM table | 128 rows with index, x, y, tile, palette, priority, flips, size in pixels from OBSEL and name table; sortable by table order, screen position or priority; selecting a row selects its low- and high-table bytes | `romlens oam <rom> --from <expr> [--obsel 0x30] [--sort table\|screen\|priority] [--json]` | 🧪 | ⬜ | ⬜ |
| 2.24 | Tilemap view | Entries decoded as `vhopppcc cccccccc`, overlaid as a grid on the rendered layer; clicking a cell selects its two bytes and reveals its tile | `romlens tilemap <rom> --from <expr> --size 32x32\|64x32\|32x64\|64x64 [--json]` | 🧪 | ⬜ | ⬜ |
| 2.25 | Previews for typed ranges | A range typed `graphics(bpp)`, `palette`, `tilemap` or `compressed` previews in the inspector with an "Open in …" button; Options… sets the palette, tiles across, and a tilemap's size and tile source on the mark | `romlens inspect <rom> <expr> --project P` prints the preview summary; `romlens project <P> preview <expr> [--palette A] [--columns N] [--size S] [--tiles A]` | 🧪 | ⬜ | ⬜ |
| 2.26 | Reference BG layer render | The Tilemap tab renders one BG layer from VRAM, CGRAM and the PPU registers (no priority, windows or colour math in Phase 2); in Mode 7 it draws the 128×128 plane untransformed, with 8-bit tile numbers in the grid | `romlens render bg --rec R --frame N --bg 1 [--ascii] [--digest]`, `romlens render sprite --rec R --frame N --index I` | 🧪 | ⬜ | ⬜ |
| 2.27 | Super Metroid decompression | Marking a range `compressed` offers "Decompress and preview", opening the tile decoder on the output | `romlens decompress <rom> --from <expr> --format sm [--stats] [--out F]` | 🧪 | ⬜ | ⬜ |

### 2C — recordings

| # | Capability | Shell must expose | CLI scenario | macOS | Win | Linux |
|---|---|---|---|---|---|---|
| 2.28 | Open a recording | File › Open Recording…; a frame field with prev/next appears and the graphics tabs read the recording's state | `romlens rec info R` | 🧪 | ⬜ | ⬜ |
| 2.29 | Validate a recording; a recording cut short offers "Open What Was Recorded" | The open path shows the validator's diagnostics verbatim and refuses a recording whose ROM hash differs | `romlens rec validate R [--rom <rom>] [--sample N] [--strict] [--recover]` | 🧪 | ⬜ | ⬜ |
| 2.30 | Extract a frame region | Export Frame Region… writes VRAM, CGRAM, OAM, WRAM or the register blocks | `romlens rec extract R --frame N --region vram [--out F] [--hex]` | 🧪 | ⬜ | ⬜ |
| 2.31 | What changed between frames (badges are exact: runs narrow, bytes decide; a sprite's own high-table bits) | The graphics views badge entries that changed since the previous frame | `romlens rec changes R --from A --to B --region vram` | 🧪 | ⬜ | ⬜ |
| 2.32 | When did this byte change (shown in each graphics view's detail pane, where VRAM, CGRAM and OAM bytes are selected, rather than the inspector) | The inspector on a VRAM/CGRAM/OAM byte reads "changed at frame N, next at M" with Go | `romlens rec when R --region vram --offset 0x4000 [--len 2] [--after N] [--backward]`, `romlens rec index R [--rebuild]` | 🧪 | ⬜ | ⬜ |
| 2.33 | Snapshot import (`.mss` savestates are not read: optional, and not done) | File › Import Snapshot… accepts loose VRAM/CGRAM/OAM dumps and a Mesen2 savestate, producing a one-frame recording | `romlens rec import-raw --vram f --cgram f --oam f --out r.romrec`, `romlens rec import-savestate s.mss --out r.romrec` | 🧪 | ⬜ | ⬜ |
| 2.34 | Ship the recorder script | Help › Save Mesen Recorder Script… writes the .lua and shows the three-step instructions | `romlens rec script --out mesen_recorder.lua`, then `romlens rec pack <stream> --rom <rom> --out r.romrec [--wram full\|keyframe\|off]` | 🧪 | ⬜ | ⬜ |
| 2.35 | Recordings referenced, never copied (the Open panel says so rather than Save, and there is no shareable export yet to omit them from; the project reattaches its recording on open only while the file is unchanged) | Attaching one stores path and hash in the project; Save shows the docs/12 notice; a shareable export omits recordings | `romlens project <P> recordings [add R \| list \| remove R]` | 🧪 | ⬜ | ⬜ |
| 2.36 | Synthetic recording fixture (done in 2B, with `--keyframe-interval`) | (n/a) | `romlens testrec --out r.romrec [--frames N]`, then every row above against it | n/a | ⬜ | ⬜ |


## Manual pass, macOS (to repeat before each release)

1. Open `roms/SuperMetroid.F8DF.sfc` and `romlens testrom` output.
2. Scroll the full 3 MB with Instruments' Animation Hitches recording.
3. Toggle the three address styles; the scroll position must not move.
4. Hover the header rows: every byte from `0x7FC0` to `0x7FFF` except the two
   unused gaps shows a span name.
5. ⌘L, type `$80:841C`, Return: the row starting `78 18 FB` is selected and
   centred; the inspector shows `0x00041C`, mirrors `$00:841C` and
   `$80:841C`; select the byte at `0x420` and the u24 reading is `$80:8423`
   with a working Go button.

Phase 1 additions (still to run):

6. Open the dev ROM: analysis progress shows in the toolbar and the hex tab
   is usable meanwhile; when it finishes the status reads about 0.2% code.
7. Disassembly tab (⌥⌘2): `RESET_80841C:` then `SEI / CLC / XCE / JML
   CODE_808423`; `STA $420D` carries `; MEMSEL`; the auto labels are dimmer
   than a renamed one.
8. Both tab (⌥⌘3): ⌘L `$80:841C`; the bracket joins `78 18 FB 5C 23 84 80`
   to their lines; scrolling either side keeps the other aligned.
9. `n` renames `RESET_80841C` to `Boot` (the label brightens, the navigator
   lists it first), `;` adds a line comment, shift+↓ then `d` marks a data
   range, Set Flags… with M=16-bit changes the widths after `LDA #$01`;
   ⌘Z four times reverts each, ⇧⌘Z redoes.
10. ⌘S saves `SuperMetroid.F8DF.romlens` beside the ROM; quit; reopening the
    package from Finder finds the ROM without a panel; move the ROM and
    reopen: the panel asks, rejects another `.sfc`, accepts the moved one.
11. File › Export writes the three files; the assembly dialog shows the
    sharing message with "Include byte columns" off.
12. Instruments Animation Hitches over a full trackpad scroll of the
    Disassembly and Both tabs (carried over from Phase 0).

Phase 2 additions (still to run):

13. Open the dev ROM: the status reads about 2.1% code and 15% data, and the
    overview strip under the editor is coloured rather than a grey bar.
    Click near the middle of the strip: the editor jumps there. Hover a
    column: the tooltip names the kind, the confidence and the entropy. A
    column that blends kinds is hatched, not painted flat.
14. ⌘L `$80:987B`, then the Disassembly tab: the dispatch reads
    `JSR (JTBL_809616,X)` rather than a number. ⌘L `$80:9616`: the region
    chip says `table 80%`, and its popover names the dispatching instruction
    with a Go link that lands back on `$80:987B`. The rows read
    `dw SUB_…`, not `db`.
15. Select a range of unclassified bytes in a graphics bank and read the
    region chip: the evidence lists the heuristics with their scores,
    strongest first, and each score is below 55%.
16. ⌘F, `78 18 ?? 5C`, Return: the results list opens under the editor with
    the matched bytes emphasised in each row's context. ⌘G and ⇧⌘G step and
    wrap, and the position in the summary follows. Switch to Text, type
    `metroid`, tick Ignore case: the header's title and the developer string
    both appear.
17. Select four bytes, Edit › Mark as ▸ Data…: pick Table, two bytes per
    entry, entries are pointers to code, same bank. The disassembly renders
    them as labels. ⌘Z reverts it as one step.
18. File › Import ▸ Symbols… with a `.sym` naming an address you have
    already named: the report says one was kept because you named it, lists
    every rewritten name, and shows the file's licence notice. ⌘Z reverts
    the whole import as one step.
19. File › Import ▸ Execution Trace… with a `.cdl` you recorded yourself:
    the report gives the executed and read byte counts and says the M/X
    widths were recorded; the map gains code and the strip gains its traced
    lane. Save and reopen: the coverage survives, and `traces/coverage.cdl`
    is inside the package.
20. Run `romlens truth from-cdl` on the same CDL and `romlens accuracy` with
    it: code precision is at least 0.98. Record the number in
    `10-ffi-spike.md`; it is the phase's pass mark and nothing in CI can
    check it, because truth for a commercial ROM never ships.

Phase 2B additions (still to run). Write the fixtures first:
`romlens testrom --fixture graphics --out g.sfc` and
`romlens testrec --out g.romrec --frames 90`.

21. Open `g.sfc`. ⌘L `$00:9000`, then ⌥⌘4: the Graphics picker reads "Tile
    Decoder", no Hex / Disassembly / Both segment is highlighted, and the
    header says `ROM $00:9000 · 0x001000`. The sheet shows shapes and a lens
    in grayscale. Click the lens's top-left quarter (tile 8): the decoder
    shows a ring, its planes and its bytes, and ⌥⌘1 shows `0x001100`–
    `0x00111F` selected in the hex view.
22. Back in the decoder, hover a pixel of the ring: its square is outlined,
    the same square is outlined in all four plane grids, its index cell
    lights, and exactly the bytes holding its bits light in the byte list —
    two in rows 0–7 of planes 0/1 and two further down for planes 2/3.
    Switch to 2 bpp and 8 bpp: the planes grid shows two and eight planes.
23. Palette menu › Colours at Tile Start is garbage on tiles, as it should
    be; ⌘L `$00:9800`, ⌥⌘5: sixteen rows of swatches, row 0 a gray ramp
    ending at white. Click a swatch: the detail gives the raw word, the
    `0 bbbbb ggggg rrrrr` bits and `#RRGGBB`, and two bytes are selected in
    the hex view.
24. ⌘L `$00:9A00`, ⌥⌘6: four sprites on screen; untick On screen only for
    128. Order by Priority puts sprite 3 first. Selecting a row selects its
    four low-table bytes.
25. File › Open Recording… with `g.romrec`: the view switches to Tilemap,
    the source reads `g.romrec, frame 0 of 90`, and BG1 is drawn — the lens
    framed by a checker field — with the grid over it. BG3 shows `ROMLENS`
    at the top; BG4 says mode 1 has none. Step to frame 30 and back: sprite
    0 in the OAM view moves a pixel a frame. Open the same recording on the
    `romlens testrom` minimal ROM: it is refused, naming both hashes.
26. Mark `0x2000` + 2048 as Tilemap, open the inspector's Preview, Options…,
    Tiles at `$00:9000`, Set: the preview draws the map with its tiles, ⌘Z
    takes it back to "set a tile address", and the analysis did not re-run.
27. Mark `0x3000` + 433 as Compressed: the preview reads "Super Metroid LZ:
    1024 bytes from 433"; Decompress and Open in Tile Decoder shows the same
    sheet as `$00:9000`, with the source reading "Decompressed, 1024 bytes".
28. On the development ROM, ⌘L `$95:80D8`, mark 9481 bytes as Compressed:
    the preview decompresses 16384 bytes. Browse them in the decoder; there
    is no way to export the image, which is deliberate (`12-content-policy.md`
    rule 5).

Phase 2C additions (still to run), with `g.sfc` and `g.romrec` from above.

29. Open `g.sfc` and `g.romrec`. Step to frame 30: in the Tile Decoder at
    VRAM 0, tile 5 is outlined in orange; in the OAM view sprite 0 has a
    dot every frame and sprite 2 never does; in the Palette view entry 17
    has a dot on the frames it changes. Select tile 5: the row under the
    sheet reads "Changed at frame 30 · No later change"; at frame 10 it
    reads "Next at frame 30", and clicking that goes there.
30. Save the project, close it, reopen it: the recording is attached again.
    Replace `g.romrec` with `romlens testrec --out g.romrec --frames 12`
    and reopen: it is not reattached. `romlens project <P> recordings`
    lists it as changed since it was attached.
31. Cut `g.romrec` short (`head -c 20000 g.romrec > cut.romrec`) and open
    it: "The recording was not finished", and Open What Was Recorded reads
    the whole frames. Damage a byte of a payload and open that: "The
    recording is damaged", with the validator's codes.
32. File › Export Frame Region…, pick CGRAM: 512 bytes, equal to
    `romlens rec extract g.romrec --frame N --region cgram`. File › Import
    Snapshot… with that file plus VRAM and OAM exports: a one-frame
    recording, attached, drawing the same layers.
33. Help › Save Mesen Recorder Script…: the file equals `romlens rec script`,
    and the three steps are shown. On a recording made with it, frames where
    the screen is off say "screen off (forced blank)" in the header.
