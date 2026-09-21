# Frontend conformance checklist

Parity between the macOS, Windows and Linux shells is defined here, not by
screenshots. Each core capability lists what a shell must expose and the CLI
scenario that checks the same behaviour from a script
(`08-cross-platform.md`, rule 7). A shell "conforms" to an item when a person
following the shell column gets the same answer as the CLI column.

Status: macOS ✅ done, ⬜ not yet; Windows and Linux start after macOS
Phase 1.

## Phase 0

| # | Capability | Shell must expose | CLI scenario | macOS | Win | Linux |
|---|---|---|---|---|---|---|
| 0.1 | Open a ROM (`.sfc`/`.smc`) | File › Open, Open Recent, drag onto the app; copier header stripped silently | `romlens info <rom>` prints size and "copier header stripped" when present | ✅ | ⬜ | ⬜ |
| 0.2 | Refuse non-ROMs with the core's message | The error text comes from the core, verbatim | `romlens info /dev/zero`-style input exits 1 with "no valid SNES header" | ✅ | ⬜ | ⬜ |
| 0.3 | Header summary | Mapping, FastROM, header offset, title, sizes, region, developer, version, checksum with mirrored-sum verdict, SHA-256, twelve vectors | `romlens info <rom>` (and `--json`) | ✅ | ⬜ | ⬜ |
| 0.4 | Hex view, 16 bytes per row, virtualized | Scrolls the whole image smoothly; last row may be partial | `romlens hex <rom> --from <expr> --rows N` | ✅ | ⬜ | ⬜ |
| 0.5 | Dual address column with toggle | Both / SNES / file, switchable without refetching | `--address both\|snes\|file` | ✅ | ⬜ | ⬜ |
| 0.6 | Header and vector overlays | Coloured spans over the header bytes; name and decoded value on hover or selection; click a span in the summary to jump | Rows carrying a span are marked `*` in `romlens hex`; `romlens info` lists the values | ✅ | ⬜ | ⬜ |
| 0.7 | Jump to address | ⌘L / Ctrl+L sheet; live preview `0x00041C = $80:841C` or the core's message; Return jumps and centres the row | `romlens resolve <rom> <expr>` | ✅ | ⬜ | ⬜ |
| 0.8 | Address expressions | `$80:841C`, `80:841C`, `$80841C`, `0x41C` all accepted | same | ✅ | ⬜ | ⬜ |
| 0.9 | Byte inspector | File offset, on-disk offset when a copier header exists, canonical SNES address, mirrors, u8/i8, u16/i16 LE, u24, ASCII, u16 in current bank and u24 as SNES address with Go, span name | `romlens resolve` gives the addresses and mirrors; readings come from the FFI `inspect` (a CLI `inspect` command is due with Phase 1) | ✅ | ⬜ | ⬜ |
| 0.10 | Keyboard navigation | Arrows move by 1 and 16 bytes, Page Up/Down by a page, Home/End, Backspace or ⌘[ goes back in jump history | n/a (interaction) | ✅ | ⬜ | ⬜ |
| 0.11 | Homebrew test ROM | The shell opens the fixture the CLI writes | `romlens testrom --out t.sfc --mapping lorom\|hirom\|exhirom` | ✅ | ⬜ | ⬜ |
| 0.12 | API version visible | About box or log line shows the core API version the shell was built against | `romlens --version` | ⬜ | ⬜ | ⬜ |

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
