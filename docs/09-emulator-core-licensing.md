# Emulator core licensing: what we may embed, and what we do instead

Researched 21 September 2026. Licenses read from the projects' repositories
(GitHub API `license` field or LICENSE file); activity is the last push date.

## The problem

Phase 4 wanted an embedded stepping core and the plan said "bsnes or Mesen2
as a library." Both are GPLv3. Linking either into `romlens-core` would
make the whole application GPLv3, which is compatible with an open-source
project but would forbid permissively licensed reuse of the core by others,
complicate any future App Store distribution (already ruled out), and, more
practically, force a C++ build of a large emulator into a Rust workspace on
three platforms.

## Candidates

| Project | Language | License | Last push | Notes |
|---|---|---|---|---|
| ares | C++ | ISC (permissive) | 2026-09 | Accurate, active, permissive. No library API; built with its own `nall` and `ruby` layers; libretro support is an unofficial fork. Embedding means building the `sfc` core and writing a C shim. |
| Mesen2 | C++ | GPLv3 | 2026-06 | Best debugger and Lua API. Use out of process for recordings; never link. |
| bsnes | C++ | GPLv3 | 2026-05 | Accuracy reference. Never link. |
| bsnes-plus | C++ | GPLv3 | — | Debugger fork of an older bsnes. Out of process only. |
| snes9x | C++ | Custom, non-commercial, not OSI, GPL-incompatible | — | Do not use in any form. |
| LakeSnes | C | MIT | 2023-12 | Small, readable, runs many commercial games, easy to wrap from Rust via a C API. Not cycle-accurate. |
| super-sabicom | Rust | MIT | 2022-08 | "Almost full SNES hardware features", no enhancement chips. Pure Rust, a direct workspace dependency or fork. Unmaintained. |
| r-snes | Rust | MIT | 2026-09 | Active university project; passes test ROMs, does not yet run commercial games. Watch. |
| rsnes (nat-rix) | Rust | MIT | 2023-05 | Work in progress, unmaintained. |

## Timing note, same day

After this research the project decided that recordings come from outside
through Phase 4 (`13-recording-format.md`), and that an embedded core is
the Phase 5 debugger track rather than a Phase 4 dependency. Items 2 and 3
below are therefore the plan for Phase 5, not near-term work. Items 1, 4 and
5 apply from day one.

## Decision

1. **Recordings (Phase 2 and 3) stay out of process.** Mesen2 runs as a
   separate program with our Lua recorder script. No linking, so GPL does not
   reach us. Confirmed from Mesen2's own Lua documentation: memory
   callbacks (`read`, `write`, `exec`) accept SNES memory types including
   `snesVideoRam`, `snesSpriteRam` and `snesCgRam`, so VRAM, OAM and CGRAM
   writes can be logged at the PPU-memory level, plus CPU-address callbacks
   for `$2118`–`$2122` and `$420B`. `getScreenBuffer`, `getState`,
   `read` with a memory type and `createSavestate` (inside an exec callback)
   are available. There is no DMA channel query, so the recorder
   reconstructs channel state by reading `$43x0`–`$43x7` when `$420B` is
   written, and attributes VRAM writes that occur during that instruction
   to the transfer. This closes the Phase 2 verification item.
2. **The embedded core (Phase 5, debugger visualizer) is a permissive
   dependency, not a GPL one.** First candidate: fork `super-sabicom` into the workspace
   (Rust, MIT, near-complete), add per-instruction hooks and write
   callbacks. Second candidate: `LakeSnes` behind a small C API (MIT, C,
   proven on commercial games). Decide with a two-day spike at the start of
   Phase 5: does Super Metroid boot and reach gameplay, and can we hook
   every instruction and every PPU-memory write? `ares` is the fallback if
   accuracy proves insufficient, at the cost of a C++ shim.
3. **Our own 65816 core exists regardless,** because the decoder, flag
   tracking and the reference PPU are already planned in `romlens-core`.
   If the permissive cores disappoint, growing that into a full interpreter
   is the long-term path, under our own license.
4. **Project license: 0BSD** (decided 21 September 2026), applied to the
   core, the CLI, all shells and the docs. The user's requirement was "do
   whatever you want": 0BSD is the most permissive OSI-approved license,
   equivalent to a public-domain dedication with a warranty disclaimer and
   no attribution requirement. Nothing in the architecture forces anything
   stronger: UniFFI and its C# generator are MPL-2.0 tools whose generated
   bindings carry no license header and are ours; permissive cores (MIT,
   ISC) only require that their notices travel with the binary, which
   `THIRD-PARTY-NOTICES.md` does. If we ever chose to link a GPL core, the
   project would have to move to GPLv3; that is a deliberate future
   decision, not an accident.
5. **Opcode tables and test oracles** come from the WDC datasheet and
   hand-built cases, not from GPL emulator source. Reading GPL code to
   understand behaviour is fine; copying it is not.
