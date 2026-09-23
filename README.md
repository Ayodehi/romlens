# Romlens — SNES ROM study tool, macOS first

A native macOS application that opens an SNES cartridge image (`.sfc`/`.smc`),
shows it as a structured hex view, disassembles it as 65816 assembly with
code and data regions marked, walks graphics from ROM bytes to pixels, and
pairs the student with an AI tutor that works only through the app's own
tools. Over time it grows into a decompiler and an educational visualization
tool in the spirit of 3Blue1Brown.

Status: **Phase 1 ("Disassemble") implemented**, 21 September 2026, after
Phase 0 ("open and see") the same day. The Rust core decodes the full 65816
with M/X width tracking, walks the ROM from its vectors, names what it finds
and stores annotations in a `.romlens` package; the CLI exposes every step;
the macOS app shows the hex view, the disassembly and both in lockstep, with
a navigator, an inspector, label/comment/region editing with undo, project
documents and asar-syntax export. Phase 2 ("Discover code vs data") is
planned in `docs/16-phase2-plan.md`. Its classification track is built:
jump tables take the code map from 0.2% of Super Metroid to 2.1%, scored
heuristics and imported emulator traces take unknown bytes from 99.8% to
82.9%, and every guess carries its evidence and its score. Graphics and
recordings are the remaining tracks; the tutor stays deferred.
`spikes/` holds the FFI measurement that shaped the API. Start with the
docs, then `docs/14-phase0-plan.md` and `docs/15-conformance-checklist.md`
for what was built and how it is checked. Romlens is a study and visualization tool
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
| [docs/09-emulator-core-licensing.md](docs/09-emulator-core-licensing.md) | Which emulator cores we may embed (permissive only), project license 0BSD |
| [docs/10-ffi-spike.md](docs/10-ffi-spike.md) | Measured UniFFI vs C ABI numbers; decision to use UniFFI with flat buffers on hot paths |
| [docs/11-naming.md](docs/11-naming.md) | Why "Romlens" should be replaced, identifier scheme without a domain, shortlist |
| [docs/12-content-policy.md](docs/12-content-policy.md) | ROM handling, third-party disassemblies, Nintendo posture, signing and distribution logistics |
| [docs/13-recording-format.md](docs/13-recording-format.md) | The .romrec format: frame-by-frame CPU and PPU memory snapshots, deltas, optional layers, producers |
| [docs/14-phase0-plan.md](docs/14-phase0-plan.md) | Phase 0 implementation plan: layout, work breakdown, tests, CI, verification |
| [docs/15-conformance-checklist.md](docs/15-conformance-checklist.md) | Frontend conformance checklist: what every shell must expose, with the CLI scenario for each item |
| [docs/16-phase2-plan.md](docs/16-phase2-plan.md) | Phase 2 implementation plan: jump tables, scored heuristics, trace and symbol imports, graphics decoders, the .romrec format |
| [docs/17-execution-log.md](docs/17-execution-log.md) | Execution logs (.mxlog) from the MesenCE fork: the format, and the references and data types Romlens takes from them |

Decisions so far are listed at the end of the interactive proposal and in
`docs/01-vision-and-roadmap.md` (Phase 0 acceptance criteria and working
agreements).

The interactive version of this proposal (UX mockups, live hex/asm demo) is
published as a Claude artifact: https://claude.ai/artifact/FPovXe1FGDZNPvcQidxtB2


## Building

Toolchain: Rust stable via rustup (`rust-toolchain.toml`; the crates need
1.88 or newer for let chains), Xcode 27 / Swift 6.4, XcodeGen
(`brew install xcodegen`). WLA-DX (`brew install wla-dx`) is optional: when
`wla-65816` is on the path, a test assembles all 256 opcodes with it and
checks the decoder against the result. `make help` lists every target.

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
R=roms/SuperMetroid.F8DF.sfc
cargo run -p romlens-cli -- info $R
cargo run -p romlens-cli -- hex $R --from '$80:841C' --rows 4
cargo run -p romlens-cli -- resolve $R '$80:841C'
cargo run -p romlens-cli -- disasm $R --from '$80:841C' --count 30 --address snes --verbose
cargo run -p romlens-cli -- disasm $R --from '$80:8423' --count 8 --flags m0x0e0   # raw decode, no analysis
cargo run -p romlens-cli -- analyze $R --stats
cargo run -p romlens-cli -- labels $R --count 40
cargo run -p romlens-cli -- xrefs $R '$80:8573'
cargo run -p romlens-cli -- inspect $R '$80:8427'
cargo run -p romlens-cli -- search $R "78 18 FB 5C"
cargo run -p romlens-cli -- registers '$420D'
cargo run -p romlens-cli -- export asm $R --out boot.asm --range '$80:841C..$80:8460'
cargo run -p romlens-cli -- export sym $R --out boot.sym --include-auto
cargo run -p romlens-cli -- project Metroid.romlens init --rom $R
cargo run -p romlens-cli -- project Metroid.romlens label '$80:841C' Boot        # `-` removes
cargo run -p romlens-cli -- project Metroid.romlens comment '$80:841C' 'disable IRQ' --line
cargo run -p romlens-cli -- project Metroid.romlens mark '$80:9000' 512 byte
cargo run -p romlens-cli -- project Metroid.romlens flags '$80:9C41' --m 1
cargo run -p romlens-cli -- disasm $R --project Metroid.romlens --from '$80:841C' --count 8
cargo run -p romlens-cli -- testrom --out /tmp/t.sfc --mapping lorom             # or --all-opcodes
```

Project commands find the ROM beside the package by hash; pass `--rom` when
it lives elsewhere. Every read command takes `--project <pkg>` to apply the
package's labels, comments, marks and flag pins.

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
