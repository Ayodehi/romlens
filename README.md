# Romlens — SNES ROM study tool, macOS first

A native macOS application that opens an SNES cartridge image (`.sfc`/`.smc`),
shows it as a structured hex view, disassembles it as 65816 assembly with
code and data regions marked, walks graphics from ROM bytes to pixels, and
pairs the student with an AI tutor that works only through the app's own
tools. Over time it grows into a decompiler and an educational visualization
tool in the spirit of 3Blue1Brown.

Status: **Phase 0 ("open and see") implemented**, 21 September 2026. The
Rust core, FFI crate, generated Swift package, CLI and the macOS shell exist;
the app opens an `.sfc`/`.smc`, shows the structured hex view with the dual
address column, header and vector overlays, the byte inspector and jump to
address. `spikes/` holds the FFI measurement that shaped the API. Start with
the docs, then `docs/14-phase0-plan.md` for what was built and why. Romlens is a study and visualization tool
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
| [docs/14-phase0-plan.md](docs/14-phase0-plan.md) | Phase 0 implementation plan: layout, work breakdown, tests, CI, verification |
| [docs/15-conformance-checklist.md](docs/15-conformance-checklist.md) | Frontend conformance checklist: what every shell must expose, with the CLI scenario for each item |

Decisions so far are listed at the end of the interactive proposal and in
`docs/01-vision-and-roadmap.md` (Phase 0 acceptance criteria and working
agreements).

The interactive version of this proposal (UX mockups, live hex/asm demo) is
published as a Claude artifact: https://claude.ai/artifact/FPovXe1FGDZNPvcQidxtB2


## Building

Toolchain: Rust stable via rustup (`rust-toolchain.toml`), Xcode 27 / Swift
6.4, XcodeGen (`brew install xcodegen`). `make help` lists every target.

```
make test        # cargo fmt --check, clippy -D warnings, cargo test --workspace
make test-rom    # the same plus the tests pinned to roms/SuperMetroid.F8DF.sfc
make swift       # build the XCFramework + RomlensKit Swift package, swift test
make app         # xcodegen generate + xcodebuild build (after make swift)
make app-test    # xcodebuild test for the macOS shell
make cross       # cargo check for Linux and Windows targets
make docker-test # cargo test --workspace inside rust:1.95, a real Linux run
make ci-local    # all of the above: the local stand-in for CI
```

Layout:

```
Cargo.toml                    workspace: crates/romlens-core, romlens-ffi, romlens-cli
bindings/swift/RomlensKit/    Swift package wrapping the generated bindings (generated files are git-ignored)
shells/macos/                 XcodeGen spec + Swift sources; Romlens.xcodeproj is generated
scripts/                      build-xcframework.sh, check-cross.sh
.github/workflows/ci.yml      core tests on macOS, Windows, Ubuntu; advisory macOS shell job
```

The CLI mirrors the shell so every feature has a scriptable twin:

```
cargo run -p romlens-cli -- info roms/SuperMetroid.F8DF.sfc
cargo run -p romlens-cli -- hex roms/SuperMetroid.F8DF.sfc --from '$80:841C' --rows 4
cargo run -p romlens-cli -- resolve roms/SuperMetroid.F8DF.sfc '$80:841C'
cargo run -p romlens-cli -- testrom --out /tmp/t.sfc --mapping lorom
```

If a `cargo` from Homebrew is also installed, the scripts and Makefile put
rustup's `~/.cargo/bin` first on `PATH`: cargo invokes `rustc` from `PATH`,
and only rustup's has the cross-compilation targets.

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
