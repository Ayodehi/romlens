# Romlens Phase 0 implementation plan

## Context

Romlens is an SNES ROM study and visualization tool: macOS first, with a
portable Rust core so Windows (WinUI 3) and Linux (GTK4) shells can follow.
Design work is complete (docs 01–13, initial commit `70fad9d`); no
application code exists. Phase 0 is "Open and see": a document-based macOS
app that opens an `.sfc`/`.smc` and shows a structured hex view with a
correct dual address column, built on the Rust core from the first commit.

Acceptance criteria (docs/01, agreed 2026-09-21):

1. Opens the development ROM (`roms/SuperMetroid.F8DF.sfc`) and a homebrew
   test ROM assembled by the test suite itself.
2. Detects LoROM/HiROM/ExHiROM and copier headers; header fields, vectors
   and the mirrored checksum are shown, with tests pinned to the dev ROM.
3. Virtualized hex view scrolls smoothly through 3 MB with the dual address
   column and header/vector overlays.
4. Jump to address (⌘L) and the byte inspector work.
5. Core, FFI crate, generated Swift package and CLI exist; CI is green on
   macOS, Windows and Ubuntu.

Binding decisions: Rust core + UniFFI (docs/10 measured: flat buffers on hot
paths), Swift 6 shell, macOS 27 minimum, 0BSD, ROMs only in git-ignored
`roms/`, direct commits to `main`, GitHub account not yet chosen
(`io.github.placeholder` prefix until then).

Facts to pin tests to (docs/04, re-verified from the file): 3,145,728 bytes,
no copier header, LoROM header at `0x7FC0`, map mode `$30`, cart `$02`, ROM
size code `$0C`, RAM `$03`, region `$00`, developer `$01`, version `$00`,
complement `$0720`, checksum `$F8DF` valid only with the mirrored-sum rule;
native COP/BRK/ABORT `$8573`, NMI `$9583`, IRQ `$986A`; emulation RESET
`$841C`, others `$8573`; bytes at `0x41C` = `78 18 FB 5C 23 84 80`;
SHA-256 `12b77c4bc9c1832cee8881244659065ee1d84c70c3d29e6eaf92e6798cc2ca72`.
The `0xFFC0` slot is all `FF`; `0x40FFC0` is past the end (scorer must
bounds-check).

Toolchain: Xcode 27 / Swift 6.4, macOS 27 SDK; two Rust installs (Homebrew
`/opt/homebrew/bin/cargo` and rustup at `~/.cargo/bin`, only
`aarch64-apple-darwin` installed); UniFFI 0.31.2 proven in
`spikes/ffi-rows/uniffi/`; Docker and `gh` present; XcodeGen not installed;
`cargo-nextest` and `cargo-llvm-cov` present.

Two technical choices made here rather than asked: `FileOffset` means the
offset into the ROM payload with any copier header already stripped (the
inspector additionally shows the on-disk offset when a copier header exists),
and the SHA-256 is computed over the payload so `.smc` and `.sfc` of the same
image share one identity.

## Approach

A Cargo workspace with three crates. `romlens-core` owns ROM loading,
mapping, header, checksum, address translation and hex-row view models, with
exhaustive tests and hand-assembled homebrew fixture ROMs. `romlens-ffi`
exports one UniFFI object with chunky calls; a script builds an XCFramework
and a local Swift package. `romlens-cli` exposes the same capabilities and
produces golden files. The macOS shell is an XcodeGen-described AppKit
`NSDocument` app with SwiftUI views hosted inside; the hex view is an
`NSTableView` whose row views draw with CoreText from cached batches. A
GitHub Actions workflow is authored now; until the repo exists, Linux and
Windows are checked locally with `cargo check --target` and a Docker run.

## Repository layout after Phase 0

```
Cargo.toml                          workspace (members crates/*, exclude spikes/**)
Cargo.lock                          committed; pins uniffi 0.31.x
rust-toolchain.toml                 channel = "stable"
Makefile                            test | swift | app | ci-local
crates/
  romlens-core/src/
    lib.rs, error.rs
    memory/{address.rs, map.rs, parse.rs}
    rom/{copier.rs, header.rs, scorer.rs, checksum.rs, image.rs}
    viewmodel/{hex_rows.rs, spans.rs, inspector.rs}
    fixtures.rs                     minimal_lorom / minimal_hirom / minimal_exhirom (public, used by CLI and FFI)
  romlens-core/tests/{common/mod.rs, mapping.rs, checksum.rs, header_dev_rom.rs, homebrew_rom.rs, hex_rows.rs}
  romlens-ffi/{Cargo.toml, uniffi.toml, src/lib.rs, src/bin/uniffi-bindgen.rs, src/bin/uniffi-bindgen-swift.rs}
  romlens-cli/{src/main.rs, tests/golden.rs, tests/golden/*.txt}
bindings/swift/RomlensKit/          Package.swift committed; Sources/RomlensKit/*.swift and RomlensFFI.xcframework generated, git-ignored
scripts/{build-xcframework.sh, check-cross.sh}
shells/macos/{project.yml, Romlens/, RomlensTests/}   Romlens.xcodeproj generated, git-ignored
.github/workflows/ci.yml
```

Integration tests live per crate because Cargo only runs `tests/` inside a
package; the top-level `tests/` from docs/03 is reserved for shared data.

## Work breakdown

### 1. Workspace and core

- Workspace `Cargo.toml`: `resolver = "3"`, `members = ["crates/*"]`,
  `exclude = ["spikes/ffi-rows/uniffi", "spikes/ffi-rows/c-abi"]` (nested
  packages must be excluded or they break the workspace),
  `[workspace.package]` edition 2024, license `0BSD`, `rust-version 1.85`;
  release profile `lto`, `codegen-units = 1`. If UniFFI 0.31 macros trip an
  edition-2024 lint, drop only `romlens-ffi` to 2021.
- Cargo policy: scripts call `~/.cargo/bin/cargo` explicitly (rustup owns
  cross targets); daily commands work with either install.
- Dependencies, five runtime crates total: `thiserror` 2 (core, ffi), `sha2`
  0.10 (core), `uniffi` 0.31 with `cli` feature (ffi), `clap` 4 derive (cli),
  `anyhow` 1 (cli only). No serde, memmap, async or zstd in Phase 0. Golden
  tests use `std::process::Command` with `env!("CARGO_BIN_EXE_romlens")`.
- `memory/address.rs`: `SnesAddress(u32)` with `new(bank, offset)`,
  `from_u24`, `bank()`, `offset()`, `Display` as `$80:841C`; `FileOffset(u32)`
  with `Display` as `0x00041C`.
- `memory/map.rs`: `MappingMode { LoRom, HiRom, ExHiRom }`; `AddressMap
  { mode, rom_len, fast_rom }` with `file_offset(SnesAddress) ->
  Option<FileOffset>`, `canonical(FileOffset) -> Option<SnesAddress>` (FastROM
  bank when the header says so), `mirrors(FileOffset) -> Vec<SnesAddress>`,
  `classify(SnesAddress) -> Region { Rom, Wram, LowRam, Hardware, Sram,
  OpenBus }`, `header_offset(mode)`; free fn `mirror_offset(off, len)`: power
  of two → `off % len`; otherwise split at the largest power of two `big`,
  offsets below `big` map directly, above recurse into the remainder (3 MB:
  `0x3FFFFF → 0x2FFFFF`). Rules: LoROM upper halves of all banks except
  `$7E/$7F`, `raw = (bank & 0x7F) * 0x8000 + (addr & 0x7FFF)`; HiROM
  `$40–$7D`/`$C0–$FF` whole banks and upper halves of `$00–$3F`/`$80–$BF`,
  `raw = (bank & 0x3F) << 16 | addr`; ExHiROM `$C0–$FF` → first 4 MB,
  `$40–$7D` → `0x400000 +`, upper halves of `$80–$BF` → first 4 MB, of
  `$00–$3F` → `0x400000 +` (so CPU `$00:FFC0` is file `0x40FFC0`).
- `memory/parse.rs`: `parse_address_expr("$80:841C" | "80:841C" | "$80841C"
  | "0x41C")` → `AddressExpr::{Snes, File}`; shared by CLI, sheet and future
  shells (docs/08 rule 7).
- `rom/copier.rs`: `split_copier_header` by `len % 1024 == 512`.
- `rom/header.rs`: `RomHeader` with title (raw + trimmed), map mode (`is_fast_rom
  = bit 4`, low nibble → mapping), cartridge type, ROM/RAM size codes,
  region, developer id, version, complement, checksum, `native` and
  `emulation` `Vectors { cop, brk, abort, nmi, reset, irq }` at `+0x24..`
  and `+0x34..`, optional extended header when developer id is `0x33`;
  `complement_valid()`, `declared_rom_size()`.
- `rom/scorer.rs`: `score_slot(payload, mode)` bounds-checked at
  `0x7FC0`/`0xFFC0`/`0x40FFC0`; points: complement valid +4, map-mode nibble
  matches slot +3, reset `>= 0x8000` +3, printable title +2, size codes and
  region plausible +1 each, ExHiROM over HiROM +1 when both valid and
  `len > 4 MB`; `detect_mapping` picks the best, ties → map-mode nibble →
  LoROM; below a floor → `RomError::NoValidHeader`.
- `rom/checksum.rs`: `compute_checksum(payload) -> u16` using the same
  power-of-two split as `mirror_offset`, remainder repeated to fill (3 MB =
  2 MB + 2 × 1 MB). No special-casing of the checksum bytes needed.
- `rom/image.rs`: `RomImage { payload: Arc<[u8]>, copier_header, header_offset,
  header, mapping, map, sha256, computed_checksum, source_name }` with
  `load(path)`, `from_bytes(vec, name)`, `bytes()`, `len()`, `row_count()`,
  `checksum_ok()`, `sha256_hex()`, `info() -> RomInfo` (plain struct shared by
  CLI and FFI). `RomError { Io, TooSmall, TooLarge, NoValidHeader,
  UnsupportedMapping(u8) }`.
- `viewmodel/hex_rows.rs`: `encode_rows(rom, spans, start_row, count) ->
  Vec<u8>` and `format_rows_text(...)` for CLI and goldens. Batch layout, all
  little-endian: 8-byte header (`u16 version=1`, `u16 stride=64`, `u32
  rows`), then 64-byte records: `u32 file_offset`, `u32 canonical SNES
  address` (`0xFFFF_FFFF` if unmapped), `u8 byte_count`, `u8 flags`, `u16
  reserved`, `16 × u8 bytes` (zero padded), `16 × u8 span_id` (0 = none,
  else span index + 1; Phase 2 reuses the slot for region ids), `16 × u8
  ASCII`, 4 reserved. Both addresses in the record means the address-style
  toggle never refetches.
- `viewmodel/spans.rs`: `SpanKind { Title, MapMode, CartridgeType, RomSize,
  RamSize, Region, DeveloperId, Version, ChecksumComplement, Checksum,
  NativeVector, EmulationVector, ExtendedHeader }`, `Span { id, start, len,
  kind, name, value_text, target }`, `header_spans(rom)` (~24 spans) and a
  `SpanIndex` for the encoder.
- `viewmodel/inspector.rs`: `interpret(rom, off) -> ByteInterpretation
  { file_offset, snes_address, mirrors, u8, i8, u16_le, i16_le, u24_le,
  u16_as_address_in_bank, u24_as_snes_address, pointer_target_file_offset,
  ascii, span_name }`.
- `fixtures.rs`: `minimal_lorom()` 32 KB: at `$00:8000` `78 18 FB` (SEI CLC
  XCE), `E2 30`, `A9 80`, `8D 00 21` (force blank), `80 FE` (BRA self); `40`
  (RTI) at `$800E` as the catch-all; header at `0x7FC0` (title `ROMLENS
  TEST`, mode `$20`, cart `$00`, size `$05`, RAM `$00`, region `$01`),
  vectors → `$800E` except RESET → `$8000`, checksum pair computed and
  written. `minimal_hirom()` 64 KB (header `0xFFC0`, mode `$21`) and
  `minimal_exhirom()` (`0x410000` bytes, header `0x40FFC0`, mode `$25`) from
  the same builder. Public so `romlens testrom` and the FFI can emit them.

### 2. FFI crate and Apple build

- `romlens-ffi/Cargo.toml`: `crate-type = ["cdylib", "staticlib"]`, lib name
  `romlens_ffi`, two bins: `uniffi-bindgen` (`uniffi::uniffi_bindgen_main()`)
  and `uniffi-bindgen-swift` (`uniffi::uniffi_bindgen_swift()`).
  `uniffi.toml`: `[bindings.swift] module_name = "RomlensKit"`,
  `ffi_module_name = "RomlensFFI"`, `generate_immutable_records = true`.
- Exported surface (`src/lib.rs`, `uniffi::setup_scaffolding!()`):
  `#[derive(uniffi::Object)] Rom` (named to avoid clashing with the Swift
  `RomDocument`) with constructors `open(path)` and `from_bytes(bytes, name)`
  returning `Result<Arc<Self>, RomlensError>`; methods `info() -> RomInfo`,
  `row_count()`, `hex_rows(start_row, count) -> Vec<u8>` (the hot path),
  `spans() -> Vec<Span>`, `inspect(file_offset) -> ByteInterpretation`,
  `resolve(text) -> Result<ResolvedAddress { file_offset, snes_address, row }>`,
  `file_offset_for(snes)`, `snes_address_for(file_offset)`,
  `mirrors(file_offset)`. Free functions `api_version()`, `hex_row_stride()`,
  `make_test_rom(mapping)`. Records/enums mirror core types via `From` impls
  so the core never depends on `uniffi`. `RomlensError { Io{msg},
  InvalidRom{msg}, BadAddress{msg} }` with `thiserror`.
- `scripts/build-xcframework.sh`: `cargo build --release -p romlens-ffi
  --target aarch64-apple-darwin` (macOS 27 is Apple-silicon-only, so arm64
  only, never universal); `cargo run -p romlens-ffi --bin
  uniffi-bindgen-swift -- <dylib> <gen-dir> --swift-sources --headers
  --modulemap --module-name RomlensFFI --modulemap-filename module.modulemap`;
  `xcodebuild -create-xcframework -library libromlens_ffi.a -headers <dir
  with RomlensFFI.h + module.modulemap> -output
  bindings/swift/RomlensKit/RomlensFFI.xcframework`; copy `RomlensKit.swift`
  into `Sources/RomlensKit/`. Verify on first run: generated filename,
  modulemap flavour (plain `module`, not `framework module`; do not pass
  `--xcframework`), and that SwiftPM picks up a rebuilt XCFramework (may need
  `xcodebuild -resolvePackageDependencies` or a DerivedData wipe; note in the
  Makefile).
- `bindings/swift/RomlensKit/Package.swift`: tools 6.0, `platforms:
  [.macOS("27.0")]`, `binaryTarget RomlensFFI`, `target RomlensKit`
  depending on it, `testTarget RomlensKitTests`. Try Swift 6 language mode
  first; fall back to `.swiftLanguageMode(.v5)` on the `RomlensKit` target
  only if the generated file complains. Generated Swift and XCFramework are
  git-ignored; `make swift` is the documented first step.

### 3. CLI

- `romlens info <rom> [--json]`, `romlens hex <rom> [--from <expr>] [--rows N]
  [--address snes|file|both]`, `romlens resolve <rom> <expr>`, `romlens
  testrom --out <path> [--mapping lorom|hirom|exhirom]`, `romlens --version`
  (crate + API version). Output prints file names, never paths, so goldens
  match on all OSes.
- `tests/golden.rs`: writes the three fixtures to a temp dir, runs the binary,
  normalises line endings, compares to `tests/golden/{info,hex,resolve}-
  {lorom,hirom,exhirom}.txt`; `UPDATE_GOLDEN=1` rewrites;
  `info-supermetroid.txt` only with `ROMLENS_ROM_DIR`.

### 4. macOS shell

- Project generation: XcodeGen (`brew install xcodegen`), `shells/macos/
  project.yml` describing target `Romlens` (application, macOS 27, Swift 6,
  `SWIFT_STRICT_CONCURRENCY = complete`, hardened runtime, automatic signing,
  local package `RomlensKit`, `CFBundleDocumentTypes` for
  `io.github.placeholder.romlens.sfc` conforming to `public.data` with
  extensions `sfc`, `smc`, `NSDocumentClass = RomDocument`, sandbox with
  user-selected read-only) and `RomlensTests` (unit-test bundle). Fallback if
  XcodeGen lags Xcode 27: generate once and commit the `.xcodeproj`. Verify
  `deploymentTarget`, `info.properties`, `entitlements.properties` keys
  against the XcodeGen spec on first use.
- Document model: AppKit `NSDocument` (not SwiftUI `DocumentGroup`), because
  Phase 1 turns documents into `.romlens` packages that reference a ROM by
  hash (an `NSDocumentController` override `DocumentGroup` cannot express),
  multiple windows and Open Recent are native, key commands route through
  the responder chain where the `NSTableView` lives, and docs/08 already
  records `NSDocument`. `AppDelegate` + `MainMenu.swift` built in code (App,
  File, Edit, View with address-style items, Go with "Jump to Address…" ⌘L,
  Window, Help). `RomDocument.read(from:ofType:)` → `Rom.fromBytes(...)`, which
  sidesteps sandbox path questions and costs well under 200 ms.
  `RomWindowController` hosts `NSHostingController<DocumentView>` with
  `sceneBridgingOptions = [.toolbars, .title]`.
- Files: `App/{AppDelegate,MainMenu}.swift`, `Document/{RomDocument,
  RomWindowController}.swift`, `Model/{RomViewModel (@MainActor @Observable:
  rom, info, spans, rowCount, addressStyle, selectedOffset, inspection,
  history), HexRowCache (LRU of 256-row batches, cap 64 ≈ 1 MB), HexBatch
  (decodes the 64-byte records, caches a CTLine per row), SpanPalette
  (SpanKind → NSColor from the asset catalog, light/dark)}.swift`,
  `Views/{DocumentView (HSplitView: hex table + 300 pt inspector, toolbar),
  HexTableView (NSViewRepresentable → NSScrollView + NSTableView; coordinator
  is data source/delegate), HexRowView (NSView drawing address / hex / ASCII
  with CoreText, overlay background runs from span ids, selection rect),
  InspectorView, HeaderSummaryView (spans with colour chips, click → jump),
  JumpToAddressSheet}.swift`.
- Table rules: one column, fixed `rowHeight`, `usesAutomaticRowHeights =
  false`, `intercellSpacing = .zero`, `selectionHighlightStyle = .none`,
  `numberOfRows = rowCount` (196,608 for 3 MB). Row fetch on the main thread
  through `HexRowCache` (a miss is ~10 µs per 256 rows per docs/10). Never
  `String(format:)`: 256-entry hex lookup into a preallocated byte buffer,
  `String(decoding:as:)`, one cached `CTLine`, `NSFont.monospacedSystemFont`.
  Address style `both` (default) / `snes` / `file` via View menu and a toolbar
  segmented control; toggling invalidates cached lines and reloads, no FFI
  refetch. Selection by hit-testing x to a byte column; arrows move by 1 and
  16, Page Up/Down by visible rows; only affected rows reload. Jump uses
  `scrollRowToVisible` then centres. Measure with Instruments' Animation
  Hitches on day one of this task; the row view is container-independent so
  a single-canvas fallback is a container swap.
- Inspector shows `ByteInterpretation` (offsets, mirrors, u8/i8, u16/i16 LE,
  u24, "as SNES address" with Go, u16 in current bank with Go, ASCII, span
  name) or the header summary when nothing is selected.
- ⌘L: `jumpToAddress:` on the responder chain → sheet calling
  `rom.resolve(text:)` on every keystroke to show `0x00041C = $80:841C` or
  the core's message; Enter jumps and pushes history (Backspace-back comes
  cheaply).
- Concurrency: generated `Rom` is `@unchecked Sendable`; `RomViewModel` is
  `@MainActor`; nothing crosses actors in Phase 0.

### 5. Tests

- Rust `tests/mapping.rs`: per-mode tables (LoROM `$80:841C → 0x41C`,
  `$00:8000 → 0`, `$3F:FFFF → 0x1FFFFF`, `$FF:FFFF → 0x3FFFFF` at 4 MB and
  `0x2FFFFF` at 3 MB, `$7E:0000 → None`, `$00:7FFF → None`; HiROM `$C0:0000
  → 0`, `$40:0000 → 0`, `$00:FFC0 → 0xFFC0`, `$00:0000 → None`; ExHiROM
  `$40:0000 → 0x400000`, `$00:FFC0 → 0x40FFC0`, `$80:FFC0 → 0xFFC0`),
  canonical fast/slow, mirrors, round trips every `0x1000`, `mirror_offset`
  for 1, 2, 2.5, 3, 4, 6 MB.
- `tests/checksum.rs`: power-of-two images, synthetic 3 MB with the expected
  value computed by explicit doubling, 2.5 and 6 MB, fixtures `checksum_ok()`.
- `tests/header_dev_rom.rs`: guarded by `ROMLENS_ROM_DIR` (prints "skipped"
  when unset); every fact in Context, the SHA-256, bytes at `0x41C`,
  `resolve("$80:841C") == 0x41C`, spans cover `0x7FC0..0x8000`, and an
  in-memory `.smc` variant (512 zero bytes prepended) parses identically with
  `has_copier_header == true`.
- `tests/homebrew_rom.rs`: three fixtures load with the right mapping, title,
  vectors, checksum; row 0 begins `78 18 FB`; random bytes → `NoValidHeader`;
  an image with both LoROM and HiROM plausible headers is decided by the
  map-mode byte.
- `tests/hex_rows.rs`: batch header, stride 64, span ids exactly over header
  bytes, last partial row `byte_count`, text formatter line shape. Unit tests
  for `parse_address_expr` and `Display`.
- CLI goldens as above. Swift: `swift test` in the package
  (`Rom.fromBytes(makeTestRom(.loRom))`, title, `hexRows(0, 4).count == 8 +
  4 * 64`); `RomlensTests` for `HexBatch` decoding, cache eviction,
  `RomViewModel.jump("$00:8000")`, and an `NSDocument` read round trip;
  `xcodebuild test -scheme Romlens -destination 'platform=macOS'`.
- Manual: open both ROMs, scroll the full 3 MB while watching for hitches,
  toggle address styles, hover overlays, ⌘L `$80:841C` lands on `78 18 FB`
  with the inspector showing `0x00041C` and mirrors `$00:841C`, `$80:841C`.

### 6. CI and local stand-ins

- `.github/workflows/ci.yml`: job `core` on `macos-latest`,
  `windows-latest`, `ubuntu-latest` with `dtolnay/rust-toolchain@stable`,
  `Swatinem/rust-cache`, `cargo fmt --check`, `cargo clippy -D warnings`,
  `cargo test --workspace`; job `macos-shell` (`continue-on-error: true`
  until runner images carry Xcode 27 / macOS 27 SDK) that installs XcodeGen,
  runs `scripts/build-xcframework.sh`, `swift test` in the package,
  `xcodegen generate`, `xcodebuild build test CODE_SIGNING_ALLOWED=NO`.
  Dev-ROM tests stay skipped in CI. Action major versions to be checked when
  the repo exists.
- `scripts/check-cross.sh` (local stand-in): `rustup target add
  x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-pc-windows-msvc`,
  fmt, clippy, test, then `cargo check --workspace --all-targets --target`
  for each (check does not link). Plus `docker run --rm -v "$PWD":/w -w /w
  rust:1.95 cargo test --workspace` for a real Linux run. `make ci-local`
  chains these with the XCFramework build and the xcodebuild smoke.
  Criterion 5 is met locally this way and confirmed on GitHub at first push.

## Ordered tasks

| # | Task | Size | Depends on |
|---|---|---|---|
| 0 | Workspace `Cargo.toml` (with spike exclusion), `rust-toolchain.toml`, `.gitignore` additions (generated Swift, xcframework, xcodeproj, build/), `Makefile`, cargo path policy | 0.5 h | — |
| 1 | `memory/`: address types, `AddressMap` × 3, `mirror_offset`, `parse_address_expr`, `tests/mapping.rs` | 1 d | 0 |
| 2 | `rom/`: copier, header, scorer, checksum, `RomImage`, `fixtures.rs`, checksum/dev-ROM/homebrew tests | 1.5 d | 1 |
| 3 | `viewmodel/`: row batch encoder, text formatter, spans, inspector, tests | 0.5–1 d | 2 |
| 4 | CLI: four commands, `--version`, golden tests | 0.5–1 d | 3 |
| 5 | FFI: object, records, errors, `uniffi.toml`, bindgen bins, `build-xcframework.sh`, `Package.swift`, `swift test` | 1–1.5 d | 3 |
| 6 | Shell skeleton: XcodeGen spec, AppDelegate, menu, `RomDocument`, window showing `info()` | 1 d | 5 |
| 7 | Hex table: cache, batch decoder, CoreText row view, table, address toggle, overlays, hitch measurement | 2–3 d | 6 |
| 8 | Selection, keyboard navigation, inspector, ⌘L sheet, history | 1–1.5 d | 7 |
| 9 | Swift tests (package + app), `xcodebuild test` from the terminal | 0.5 d | 8 |
| 10 | `ci.yml`, `check-cross.sh`, Docker run, record measurements in docs/10 | 0.5 d | 4, 9 |
| 11 | Docs: README build steps, docs/03 module deltas, conformance checklist seed (hex, jump, inspector, address styles), commit | 0.5 d | 10 |

Ten to thirteen working days, inside the roadmap's two to three weeks.
Tasks 4 and 5 are independent once 3 lands; task 7 is the critical path and
gets measured on its first day.

## Risks and mitigations

- **UniFFI 0.31 vs 0.32.** Stay on 0.31.x (pinned by `Cargo.lock`) because
  the C# generator tracks it; bindgen binaries build from the same crate
  graph so generator and library cannot skew; bindings regenerate every
  build and are never committed, which avoids UniFFI's checksum-mismatch
  panic. Re-evaluate at the end of Phase 1.
- **Swift 6 strict concurrency with generated code.** Objects are
  `@unchecked Sendable`, records `Sendable`; if v6 still complains, only the
  `RomlensKit` target drops to language mode 5; the app stays in 6.
- **NSTableView at 120 Hz.** Fixed row height, no `NSTextField`s, CoreText
  caching, per-visible-row formatting; the row view is container-independent
  so a custom canvas is a cheap fallback. Measured before building further.
- **macOS 27 SDK on GitHub runners** may not exist yet; the shell job is
  advisory, the three core jobs are the gate.
- **XCFramework modulemap flavour and SwiftPM binary caching.** Two variants
  documented in the script; verify once.
- **XcodeGen behind Xcode 27.** Fallback: generate once, commit the project.
- **Two Rust toolchains.** Scripts reference `~/.cargo/bin/cargo`.
- **`Vec<u8>` arrives as `[UInt8]`, not `Data`.** Decoder uses
  `withUnsafeBytes`; either works.
- **Header scorer heuristics.** Weights are tunable; tests assert outcomes
  on the dev ROM and fixtures only.

## Verification (end to end)

1. `cargo test --workspace` passes; `ROMLENS_ROM_DIR=$PWD/roms cargo test
   --workspace` also passes the dev-ROM pins and the Super Metroid golden.
2. `cargo run -p romlens-cli -- info roms/SuperMetroid.F8DF.sfc` prints
   LoROM, FastROM, checksum `$F8DF` valid (mirrored), RESET `$841C`, SHA-256
   `12b77c4b…`.
3. `cargo run -p romlens-cli -- testrom --out /tmp/t.sfc` then `info /tmp/t.sfc`
   shows a valid homebrew header; `hex /tmp/t.sfc --rows 1` starts `78 18 FB`.
4. `scripts/build-xcframework.sh` produces the package; `swift test` in
   `bindings/swift/RomlensKit` passes; `xcodegen generate` and `xcodebuild …
   build test` succeed from the terminal.
5. Launch the app, open both ROMs, scroll the full 3 MB with no hitches in
   Instruments, toggle address styles, see header and vector overlays, ⌘L
   `$80:841C` selects `78 18 FB` and the inspector shows `0x00041C`, mirrors
   `$00:841C` and `$80:841C`, `JML` target bytes interpretable as `$808423`.
6. `scripts/check-cross.sh` passes for Linux and Windows targets; the Docker
   Linux `cargo test` passes; the CI file is committed for first push.
