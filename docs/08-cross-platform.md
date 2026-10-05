# Cross-platform: one core, three native shells

## Goal

A native experience on each platform, not a lowest-common-denominator UI.
macOS ships first. Windows and Linux follow, and the macOS version is built
from day one so that adding them means writing a shell, not porting a brain.

| Platform | Shell | Toolkit | Distribution |
|---|---|---|---|
| macOS (27 and later) | Swift | SwiftUI + AppKit for heavy views | Notarized .dmg plus Homebrew cask; no App Store |
| Windows | C# / .NET 10 | WinUI 3 (Windows App SDK), Fluent | MSIX via winget and Microsoft Store |
| Linux | Rust | GTK4 + libadwaita | Flatpak on Flathub first, then .deb and .rpm |

GNOME is the default desktop on both Ubuntu (Debian family) and Fedora /
RHEL Workstation (Red Hat family), so a libadwaita app is the native feel on
both. KDE users get a well-behaved GTK4 app; a Qt shell can follow if demand
appears, and nothing in the core would need to change.

## The decision: a Rust core with a C ABI and generated bindings

Revision 1 chose a pure Swift core to keep one toolchain, with a note to
revisit if cross-platform became a goal. It has. The core is now a Rust
workspace:

```
crates/
├── romlens-core/    ROM, mapping, 65816 decoder, analyzer, regions, project,
│                       graphics decoders, reference PPU, recording index,
│                       tutor loop (HTTP client + tool registry), scene model,
│                       software rasterizer for export
├── romlens-ffi/     the public API surface: UniFFI definitions, C ABI,
│                       generated Swift and C# bindings, XCFramework build
└── romlens-cli/     headless CLI: info, disasm, analyze, export, record-index,
                        tutor (scripted), used by CI on all three OSes
shells/
├── macos/              Xcode project, Swift, SwiftUI + AppKit
├── windows/            .NET solution, WinUI 3
└── linux/              Rust crate using gtk4-rs + libadwaita-rs (no FFI needed)
```

### Why Rust for the core

- **Bindings are a solved problem.** UniFFI generates Swift bindings from the
  Rust API, including async functions and callback interfaces; a maintained
  community generator produces C# bindings from the same definitions; the
  Linux shell links the crate directly. One API definition, three consumers.
- **Performance where it matters.** The analyzer over a 4–6 MB ROM, the
  last-writer index over millions of recorded writes, and the reference PPU
  are exactly the workloads where a compiled, allocation-conscious core pays
  off on every platform.
- **Portable dependencies exist for everything the core needs:** HTTP with
  TLS for the tutor, JSON, a credential store that wraps Keychain, Windows
  Credential Manager and Secret Service behind one API, PNG encoding, a 2D
  rasterizer for exported frames.
- **CI is honest.** The same test suite runs on macOS, Windows and Linux
  runners; the CLI produces golden files that must match across all three.

### Alternatives considered

| Option | Verdict |
|---|---|
| **Rust core, native shells** | Chosen. FFI from day one, but a narrow, generated one. |
| Swift core everywhere | Swift runs on Linux and Windows, but consuming it from .NET is awkward, and GTK from Swift is immature. Would make Windows the worst experience. |
| C# core with NativeAOT, exporting a C ABI | Feasible, and keeps Windows simple, but the macOS shell would consume a hand-maintained C header and the Linux shell would fight the GC and the toolchain. Weaker binding story than Rust. |
| C++ core | Portable and fast, but binding generation and memory safety are both worse than Rust's, and the tutor's HTTP and JSON stack is heavier to assemble. |
| One cross-platform UI (Avalonia, MAUI, Qt everywhere, Electron, Tauri) | Rejected by the requirement: a native experience on each platform. |

### The cost

- Two toolchains on macOS from day one (Rust and Xcode) and an XCFramework
  build step.
- Phase 0 grows by roughly a week to set up the FFI crate, the binding
  generation and the three-OS CI matrix.
- Some things are slightly slower to iterate on than in a single Swift
  codebase. Every core change that touches the API regenerates bindings.

That week buys the guarantee that Windows and Linux are shells, not ports.

## Where the boundary is

The rule: **the core owns anything that must be identical on every platform;
the shell owns anything that must feel like the platform.**

### Core owns

- ROM loading, mapping, header, checksum, the 65816 decoder, the analyzer,
  regions, labels, comments, cross-references, functions, the IR and pseudo-C.
- Project files, recordings, imports and exports, all file formats.
- The undo stack. Shells call `undo()` / `redo()` from their native menu.
- Selection and navigation history, so that "one selection across views" is
  a core invariant rather than three reimplementations.
- Graphics decoders, the reference PPU, provenance.
- The tutor: HTTP client, tool loop, transcript, proposals, cost meter,
  credential store access. Shells render events; they never talk to the API.
- The scene model and a software rasterizer, so exported video is
  pixel-identical everywhere.
- Presentation-ready view models: hex rows, disassembly lines as typed
  tokens, region summaries, atlas tiles, tile bitmaps, frame images.

### Shell owns

- Windows, documents, tabs, split views, sidebars, toolbars, menus, dialogs,
  drag and drop, recent files, printing if ever.
- Virtualized tables and canvases drawing the core's view models with
  platform fonts, colors and accessibility.
- Keyboard shortcuts in the platform idiom, mapped to core commands.
- Preferences UI, credential entry UI (storage goes through the core),
  notifications, appearance (light/dark, accent color).
- Live rendering of scenes and the atlas with the platform's GPU stack.
- Packaging, signing, updates.

## API design rules for the FFI surface

1. **Chunky, not chatty.** A hex table asks for `hex_rows(from, count)` and
   gets a batch of ready-to-draw rows. Never one call per cell; the FFI cost
   is per call, not per byte.
2. **Semantic, not styled.** The core says "region kind: code, confidence
   0.8, evidence: vector reach"; the shell chooses the color. The core says
   "token kind: hardware register, target $420D"; the shell chooses the font
   and the link style.
3. **Events, not polling.** The shell subscribes once; the core emits
   `snapshot_changed`, `selection_changed`, `analysis_progress`,
   `tutor_event`, `proposal_added`. UniFFI callback interfaces carry them.
4. **Async for anything slow.** Analysis, recording indexing, tutor turns and
   exports are async functions with cancellation tokens; shells never block
   their UI thread on the core.
5. **No platform types cross the boundary.** Bitmaps are RGBA byte buffers
   with width and height; images, fonts and colors are shell concerns.
6. **Version the API.** The FFI crate has its own semantic version; each
   shell pins the version it was built against; the CLI prints it.
7. **Everything reachable from the CLI.** If a shell can do it, the CLI can
   script it. This keeps the API honest and makes bug reports reproducible
   without a GUI.

## Per-platform idiom map

| Concern | macOS | Windows | Linux (GNOME) |
|---|---|---|---|
| Three-pane workbench | `NSSplitViewController`, sidebar style | `NavigationView` + `SplitView`, Mica material | `AdwNavigationSplitView`, `AdwHeaderBar` |
| Heavy tables | `NSTableView`, Core Text | `ItemsRepeater` / virtualized `ListView`, Win2D | `GtkListView` with `GtkSignalListItemFactory`, Cairo/Pango |
| Atlas / scenes | Metal or SwiftUI `Canvas` | Win2D / Composition | GTK4 GSK render nodes, Cairo fallback |
| Jump to address | ⌘L | Ctrl+L | Ctrl+L |
| Find | ⌘F | Ctrl+F | Ctrl+F |
| Follow / back | G / ⌫ | G / Backspace | G / BackSpace |
| Mark code / data | C / D | C / D | C / D |
| Monospace font | SF Mono | Cascadia Mono | Adwaita Mono or Source Code Pro |
| Credentials | Keychain | Credential Manager | Secret Service (libsecret) |
| Document model | `NSDocument` | custom, MRU via Windows jump list | `GtkApplication` with `GFile` handling |
| Appearance | system light/dark, accent | Fluent light/dark, accent | libadwaita light/dark, no accent yet |
| Packaging | .dmg, notarized; App Store later | MSIX, winget, Store | Flatpak (Flathub), then .deb / .rpm |
| Updates | Sparkle | winget | Flathub / distro |
| Signing | Apple Developer Program (already held) | SignPath Foundation, free for OSS | none |

The credential store is one Rust crate wrapping all three platform stores, so
only the entry UI is per shell.

## Sequencing

macOS remains first and is the reference shell. The other two start when the
core API has settled, which is the end of Phase 1 on macOS: by then the hex
view, disassembly, analysis, project files and tutor Ask mode have exercised
the FFI surface end to end.

| Track | Starts | Parity target | Rough size |
|---|---|---|---|
| macOS shell | Phase 0 | Reference | as in the roadmap |
| Windows shell (WinUI 3) | after macOS Phase 1 | Phase 0–1 parity in 4–6 weeks, then tracks macOS by phase | 40–60% of the macOS shell effort per phase, since the core does the work |
| Linux shell (GTK4/libadwaita) | after macOS Phase 1 | same | same |

Parity is defined by a **frontend conformance checklist** kept in the repo:
each core capability lists the shell features that expose it, and the CLI
provides a scripted scenario per item so the three shells can be checked the
same way.

## What changes in Phase 0 because of this

- Initialize the Rust workspace, not a Swift package. The first tests (header,
  mapping, checksum pinned to the bundled ROM) are Rust tests.
- Add `romlens-ffi` with the first UniFFI definitions (`open_rom`,
  `rom_info`, `hex_rows`, `resolve_address`) and a script that builds an
  XCFramework and a Swift package wrapper.
- The macOS shell consumes the Swift package from its first commit.
- CI matrix on macOS, Windows and Ubuntu runners builds the core and CLI and
  runs the tests; only the macOS job builds the shell for now.
- The CLI grows alongside the shell, one command per new capability.

## Risks

- **Binding generator maturity.** The Swift generator is mature; the C#
  generator is community-maintained. Mitigation: the API is small and the C
  ABI underneath is always available for hand-written P/Invoke if needed.
- **FFI overhead in tables.** Mitigated by chunky calls and by caching row
  batches on the shell side; measured in Phase 0 against the 120 Hz budget.
- **Three shells drift.** Mitigated by the conformance checklist, the
  CLI-driven scenarios, and the rule that features land in the core and the
  reference shell first.
- **Contributors.** Each shell needs someone who cares about that platform.
  The core-first design means a Windows or Linux contributor can start with a
  working brain and a checklist.

## Linux shell decisions (October 2026)

- **Menus.** A GNOME hamburger menu, not a menu bar. The macOS menu tree is
  flattened into sections and Import, Export, Edit, View and Go submenus.
  Every command is a named `gio` action, so a menu item, a shortcut and a
  button reach it the same way; a command whose feature is not built yet is
  listed and insensitive.
- **The workspace (docs/29, October 2026).** The window follows the macOS
  workspace: the sidebar (`sidebar.rs`) lists every view by its chip, then
  the symbols; the editor is tab groups in a grid (`tabgrid.rs`), each an
  `AdwTabBar` over an `AdwTabView`, the grid nested `gtk::Paned`s built from
  the workspace's tree (`model/workspace.rs`), which is the truth: the tab
  widgets tell the document what the person did and are made to match what
  it then says. A tab dragged out of a bar carries its `AdwTabPage`, which a
  drop target on each group's view takes for the edge splits; a tab is never
  torn off into a window. The TabView's own Alt+digit and Ctrl+Page keys are
  off, since Alt+digit are the views. The header bar holds the panels, Back
  and Forward, the jump bar (`jumpbar.rs`), Open Quickly and the menu; the
  window's title keeps the project's name. The strip and the analysis are a
  status bar under the groups (`statusbar.rs`). The layout is kept in
  `local.json` in the macOS shell's JSON, with the macOS bookmark written
  back as it was read.
- **Shortcuts.** Ctrl stands for Command and Alt for Option, with the view
  tabs on Alt+digit (as GNOME terminals and browsers switch tabs) and the
  tutor, live session and compare on Alt+Shift, because Ctrl+Alt combinations
  collide with desktop bindings. The one table is `ACCELS` in
  `shells/linux/src/actions.rs`; the Keyboard Shortcuts window is built from
  it, and tests fail if two commands share a shortcut or the window omits one.
  The workspace's keys: Close Tab Ctrl+W and Close Window Ctrl+Shift+W (⌘W
  and ⇧⌘W); Split Right Ctrl+\ and Split Down Ctrl+Shift+\ (⌘\ and ⌥⌘\);
  Next and Previous Tab Ctrl+Page Down and Up, as GNOME does (⇧⌘] and ⇧⌘[);
  Focus Group 1 to 4 Ctrl+Alt+1 to 4 (⌃1 to ⌃4: Ctrl+digit is already the
  address columns and Alt+digit the views); Open Quickly Ctrl+P (⇧⌘O, which
  is Open Project here).
- **Projects.** A `.romlens` package is a folder, so Open ROM and Open Project
  are separate commands (a file chooser cannot offer both). A project that has
  a file is saved when its window closes and every 30 seconds, as
  `autosavesInPlace` does; an untitled one asks.
- **Minimum versions.** GTK 4.14 and libadwaita 1.6 (the accent colour). Ubuntu
  24.04 LTS ships libadwaita 1.5, so a `.deb` for it needs a fixed accent
  colour; the Flatpak is unaffected.
- **Compare.** Compare With… takes a ROM file; Compare With Project… takes a
  `.romlens` folder, because a file chooser cannot offer both. The Source and
  Compare tabs appear only when there are imported sources or a comparison, and
  View › Source and Compare are greyed out otherwise, as on macOS.
- **Recordings and live sessions.** Open Recording starts in
  `~/Documents/Romlens/Recordings`, where the recorder script writes (named
  from `$HOME`, as the script does, not the XDG documents folder), and takes a
  `.romrec`, or an `.rlstream`, which is packed into
  `$XDG_DATA_HOME/romlens/Recordings` first and may then be moved to the Trash.
  A recording is read where it is and never copied; the project keeps its path
  and fingerprint. The live session listens on the loopback address only, so a
  Flatpak needs no extra permission for it. Live Session is a check item in the
  menu, not a command that changes its own label.
- **Audio output.** `cpal` over ALSA, which PipeWire and PulseAudio serve, so
  the Flatpak needs the PulseAudio socket and the snap the `audio-playback`
  plug. The DSP runs at 32 kHz and is rendered ahead into a lock-free ring on
  its own thread; the device callback resamples to the device's rate and never
  allocates, locks or calls into the core.
- **Tutor.** No window of its own (docs/29): the tutor is a tab of the
  inspector's drawer (`drawer.rs`) and can be opened as a tab of the grid, two
  views of the document's one conversation. Keys live in the Secret Service through libsecret (schema
  `io.github.ayodehi.Romlens.tutor`, attribute `endpoint`), never in the
  settings file `$XDG_CONFIG_HOME/romlens/tutor.json` or a project; where no
  Secret Service is running no key can be saved. Conversations are kept under
  `$XDG_DATA_HOME/romlens/Tutor`, not in the project. Answers are Markdown drawn
  as GTK labels: prose as Pango markup with each citation a link into the
  window's tabs (beside the tutor's tab when that has focus; pointing at a
  paragraph outlines the lines it cites) and each glossary term a link to a bubble, code in the C view's
  colours, tables as grids. The Settings window carries the macOS Settings pages
  (General, Providers, Tutor, Images, Privacy) as an `AdwPreferencesDialog`.
  Lessons, the map, progress and the quiz are dialogs over the project window.
- **Packaging.** The shell's data lives in `shells/linux/data`: the desktop file,
  AppStream metainfo, a shared-mime-info file for `.romrec`, `.rlstream` and
  `.spclog` (the ROM type is shared-mime-info's own; a `.romlens` project is a
  folder, which a mime type cannot describe) and the icons. Every format is
  built from those files and the one release binary, natively on each
  architecture:
  - `.deb` and `.rpm`: `cargo deb` and `cargo generate-rpm`, driven by
    `scripts/package-linux.sh`, which also writes a tarball with an
    `install.sh` and `SHA256SUMS`. The `.deb` takes its library dependencies
    from the binary, so it needs the libraries and the glibc of the system it
    was built on: the CI and release builds run in an Ubuntu 26.04 container
    (libadwaita 1.9), which is also what the runner images (24.04, libadwaita
    1.5) cannot build, because the shell needs 1.6. A package for an older
    distribution is the Flatpak.
  - Flatpak: `shells/linux/flatpak/io.github.ayodehi.Romlens.yaml` on the GNOME
    50 runtime with the Rust SDK extension. `scripts/cargo-sources.py` lists
    every crate in `Cargo.lock`, so the build is offline and checked against
    the lockfile; `scripts/build-flatpak.sh` makes `dist/romlens-<arch>.flatpak`.
    Permissions: Wayland and X11, the GPU, PulseAudio (cpal reaches PipeWire
    through it), the network (the tutor's providers, and the live session's
    loopback listener) and `org.freedesktop.secrets`. Files come through the
    portal, so a ROM chosen there is the only file the sandbox sees: a sibling
    `.dbg` or ROM is asked for. Flathub takes the same manifest with a `git`
    source at a release tag in place of the `dir` source.
  - Snap: `snap/snapcraft.yaml` on the `gnome` extension, with the `home`,
    `network`, `network-bind`, `audio-playback` and `password-manager-service`
    plugs, and ALSA routed to PulseAudio. It has not been built by the
    authors (no snapcraft where it was written), so its `grade` is `devel`.
  - `.github/workflows/release.yml` builds all of it for a `v*` tag and
    leaves a draft GitHub release for a person to read and publish. An apt
    repository needs a signing key and somewhere to host it, which are not
    decided; until then the `.deb` is a release download.
  - The shell links the core's `romlens-ffi` with the `bindgen` feature off,
    so it does not build UniFFI's binding generators (askama, goblin and
    their tree), which only the Swift shell needs. OpenSSL is not needed: TLS
    is rustls.
