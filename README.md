# Romlens — SNES ROM study tool, macOS first

A native macOS application that opens an SNES cartridge image (`.sfc`/`.smc`),
shows it as a structured hex view, disassembles it as 65816 assembly with
code and data regions marked, walks graphics from ROM bytes to pixels, and
pairs the student with an AI tutor that works only through the app's own
tools. Over time it grows into a decompiler and an educational visualization
tool in the spirit of 3Blue1Brown.

Status: **proposal stage**. No application code yet; `spikes/` holds the FFI
measurement. Start with the docs. Romlens is a study and visualization tool
first: it needs no commercial ROM, works on homebrew builds, and its dynamic
views come from recordings in our own format. A debugger visualizer with an
embedded core is the planned next step after that, feeding the same views.

| Doc | What it answers |
|---|---|
| [docs/01-vision-and-roadmap.md](docs/01-vision-and-roadmap.md) | Why, for whom, and what ships in each phase |
| [docs/02-ux-proposal.md](docs/02-ux-proposal.md) | Three UX layout options, feature list, recommendation |
| [docs/03-architecture.md](docs/03-architecture.md) | Core and shell architecture, module layout, project file format |
| [docs/04-snes-technical-primer.md](docs/04-snes-technical-primer.md) | Memory mapping, 65816 quirks that shape the disassembler, facts about the bundled ROM |
| [docs/05-references.md](docs/05-references.md) | Public resources to lean on |
| [docs/06-ai-tutor.md](docs/06-ai-tutor.md) | The AI tutor: modes, tool surface, grounding rules, evaluation |
| [docs/07-memory-to-screen.md](docs/07-memory-to-screen.md) | Sprites and backgrounds as a walkable chain from ROM bytes to pixels |
| [docs/08-cross-platform.md](docs/08-cross-platform.md) | One Rust core, native shells for macOS, Windows and Linux; boundary, bindings, sequencing |
| [docs/09-emulator-core-licensing.md](docs/09-emulator-core-licensing.md) | Which emulator cores we may embed (permissive only), project license MIT OR Apache-2.0 |
| [docs/10-ffi-spike.md](docs/10-ffi-spike.md) | Measured UniFFI vs C ABI numbers; decision to use UniFFI with flat buffers on hot paths |
| [docs/11-naming.md](docs/11-naming.md) | Why "Romlens" should be replaced, identifier scheme without a domain, shortlist |
| [docs/12-content-policy.md](docs/12-content-policy.md) | ROM handling, third-party disassemblies, Nintendo posture, signing and distribution logistics |
| [docs/13-recording-format.md](docs/13-recording-format.md) | The .romrec format: frame-by-frame CPU and PPU memory snapshots, deltas, optional layers, producers |

Decisions so far are listed at the end of the interactive proposal and in
`docs/01-vision-and-roadmap.md` (Phase 0 acceptance criteria and working
agreements).

The interactive version of this proposal (UX mockups, live hex/asm demo) is
published as a Claude artifact: https://claude.ai/artifact/FPovXe1FGDZNPvcQidxtB2


## Test ROM

`SuperMetroid.F8DF.sfc` (3 MB, LoROM/FastROM, US/JP release, header checksum
`F8DF`) is the development ROM. It is a good first target because a complete,
public, human-written disassembly exists (PJBoy's bank logs), which gives us
ground truth to measure our automatic code/data classification against.

ROM images are not redistributed by this project. They live in the
git-ignored `roms/` folder; project files store only a hash of the ROM plus
the user's annotations.

## License

0BSD: do whatever you want, no attribution required. See `LICENSE` and
`THIRD-PARTY-NOTICES.md`.
