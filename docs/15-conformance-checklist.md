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
pass is listed below and still to run.

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
| 0.12 | API version visible | About box or log line shows the core API version the shell was built against | `romlens --version` | ⬜ | ⬜ | ⬜ |

## Phase 1

| # | Capability | Shell must expose | CLI scenario | macOS | Win | Linux |
|---|---|---|---|---|---|---|
| 1.1 | Analysis runs on open, off the main thread | Progress and phase in the toolbar with ⌘. to cancel; the hex view is usable meanwhile; percentages when done; tap to re-run | `romlens analyze <rom> --stats [--progress]` | 🧪 | ⬜ | ⬜ |
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
| 1.15 | Byte search | (Phase 2 UI) | `romlens search <rom> "78 18 ?? 5C"` | n/a | ⬜ | ⬜ |
| 1.16 | Hardware registers | `STA $420D` carries `; MEMSEL`; the inspector shows the register's access and description | `romlens registers [address]` | 🧪 | ⬜ | ⬜ |
| 1.17 | Hex region tint | Hex rows tint code and data bytes by confidence after analysis without a refetch | (the hex-row batch's span lane) | 🧪 | ⬜ | ⬜ |
| 1.18 | Range selection | Shift+arrows and shift-click extend the highlighted range in either canvas; mouse-drag and cross-canvas drag are not in Phase 1 | n/a (interaction) | 🧪 | ⬜ | ⬜ |

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
