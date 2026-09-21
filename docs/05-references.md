# References

Public resources this project leans on. Verify links before relying on them;
several are community sites that move.

## SNES hardware and memory maps

- **fullsnes** by nocash — the single most complete SNES hardware reference,
  including memory maps for every mapping mode and special chip.
  https://problemkaputt.de/fullsnes.htm
- **SNESdev wiki** — community-maintained, readable, with header layout and
  memory map pages. https://snes.nesdev.org/wiki/
- **Anomie's SNES documents** (register, timing, memory map docs) — mirrored
  on romhacking.net and elsewhere; search "anomie snes docs".
- **Official Nintendo SNES Development Manual** — widely archived; search
  "SNES development manual book I".

## The 65816 CPU

- **W65C816S datasheet** (Western Design Center) — the opcode matrix and
  addressing modes. https://www.westerndesigncenter.com/wdc/documentation/w65c816s.pdf
- **Programming the 65816** (Eyes & Lichty) — the standard book; the WDC
  site distributes a PDF.
- **6502.org 65816 opcode tables** — quick reference for the 256 opcodes and
  their cycle counts. http://6502.org/tutorials/65c816opcodes.html

## Super Metroid specifically

- **PJBoy's Super Metroid disassembly (bank logs)** — a complete,
  hand-annotated disassembly of every bank. Our ground truth.
  https://patrickjohnston.org/bank/index.html
- **Metroid Construction** — community wiki, RAM/ROM maps, data format
  documentation (rooms, tiles, compression). https://metroidconstruction.com
- **Super Metroid compression format** — documented on Metroid Construction
  and in several open decompressors; needed for graphics previews in Phase 2.

## SNES graphics specifically

- **fullsnes PPU chapters** — VRAM, CGRAM, OAM layouts, BG modes, OBJ sizes
  and the register map the views name.
- **SNESdev wiki: Tiles, Backgrounds, Sprites, DMA** — approachable pages
  on bitplane formats, tilemap entries and the OAM high table.
- **Super Metroid compression** — Metroid Construction documents the LZ
  variant; several open decompressors exist to validate ours against.

## Existing tools (to import from, export to, and learn from)

- **DiztinGUIsh (Diz)** — SNES-specific interactive disassembler; its project
  format is a natural import source. https://github.com/IsoFrieze/DiztinGUIsh
- **Mesen2** — emulator with a debugger, trace logger and CDL (code/data log)
  export, plus a Lua scripting API with memory callbacks that our recorder
  script will use. Verify the callback surface for PPU ports and DMA before
  Phase 2. https://github.com/SourMesen/Mesen2
- **bsnes-plus** — debugger-oriented bsnes fork with usage maps and a trace
  logger. https://github.com/devinacker/bsnes-plus
- **bsnes / higan** — accuracy reference; bsnes can be built as a library for
  an embedded stepping core. https://github.com/bsnes-emu/bsnes
- **asar** — the de facto SNES assembler; our `.asm` export should assemble
  under it. https://github.com/RPGHacker/asar
- **Ghidra** — general reverse-engineering framework; community 65816
  processor modules exist. Useful as a comparison for CFG and decompiler
  output. https://github.com/NationalSecurityAgency/ghidra
- **Hex Fiend** — the macOS hex editor whose performance and feel we should
  match for the hex view. https://github.com/HexFiend/HexFiend

## Educational visualization

- **manim** (3Blue1Brown's animation engine, Python) — the reference for the
  visual language: camera moves, highlights, transforms between
  representations. https://github.com/3b1b/manim (community fork:
  https://www.manim.community)
- **Motion Canvas** — TypeScript, web-rendered, procedural animation with a
  timeline; a strong model for our in-app scene system.
  https://motioncanvas.io
- **Retro Game Mechanics Explained** (YouTube) — the best existing examples
  of SNES internals explained visually; several videos use Super Metroid.
- **Ben Eater** (YouTube) — for pacing and the "show the bytes" style of
  teaching CPU behaviour.

## Cross-platform building

- **UniFFI** (Mozilla) — Rust to Swift/Kotlin/Python bindings from one
  definition; community generators add C# and Go.
  https://github.com/mozilla/uniffi-rs
- **gtk4-rs and libadwaita-rs** — GTK4 and libadwaita from Rust; **Relm4**
  is the idiomatic app framework on top. https://gtk-rs.org
- **GNOME Human Interface Guidelines** — what "native" means on GNOME.
  https://developer.gnome.org/hig/
- **Windows App SDK / WinUI 3** and the **Fluent design** guidance.
  https://learn.microsoft.com/windows/apps/winui/
- **Flatpak and Flathub** — cross-distro Linux packaging.
  https://docs.flatpak.org
- **keyring crate** — one API over Keychain, Credential Manager and Secret
  Service. https://crates.io/crates/keyring

## Licensing, policy and distribution (researched 2026-09-21)

- ares LICENSE (ISC): https://github.com/ares-emulator/ares/blob/master/LICENSE
- Mesen2 (GPLv3) and its Lua documentation:
  https://github.com/SourMesen/Mesen2/blob/master/UI/Debugger/Documentation/LuaDocumentation.json
- LakeSnes (MIT): https://github.com/angelo-wf/LakeSnes
- super-sabicom (MIT, Rust): https://github.com/tanakh/super-sabicom
- r-snes (MIT, Rust, active): https://github.com/r-snes/r-snes
- snes9x license (non-commercial): https://github.com/snes9xgit/snes9x/blob/master/LICENSE
- Nintendo IP and piracy FAQ:
  https://en-americas-support.nintendo.com/app/answers/detail/a_id/55888
- Nintendo Game Content Guidelines: https://www.nintendo.co.jp/networkservice_guideline/en/
- GitHub DMCA notice, yuzu network (section 1201 grounds):
  https://github.com/github/dmca/blob/master/2024/04/2024-04-29-nintendo.md
- Flathub app ID requirements: https://docs.flathub.org/docs/for-app-authors/requirements
- SignPath Foundation OSS signing terms: https://signpath.org/terms.html
- Apple notarization requirement: https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution
- uniffi-bindgen-cs (C# generator): https://github.com/NordSecurity/uniffi-bindgen-cs
- Romlens CF font (name collision): https://connary.com/fonts/romlens/

## Claude API (for the tutor)

- Anthropic Messages API docs: tool use, streaming, prompt caching,
  structured outputs. https://docs.anthropic.com
- No official Swift or Rust SDK; the core calls the HTTPS API directly. The
  TypeScript SDK's tool runner is the reference for the loop we hand-write.

## macOS application building

- Apple: document-based apps in SwiftUI (`DocumentGroup`,
  `FileDocument`/`ReferenceFileDocument`), `NSViewRepresentable`, `NSTableView`
  view-based rows, Core Text for fast glyph runs, Metal for the Atlas.
- Swift 6 concurrency: actors for the analyzer, `Sendable` snapshots to the
  UI.
