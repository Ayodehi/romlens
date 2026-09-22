# Third-party notices

Romlens itself is released under the 0BSD license (see `LICENSE`): do
whatever you want with it, no attribution required. Code and assets from
other projects keep their own licenses, listed here as they are added. None
of them is copyleft over our code; permissive licenses require that their
notices be preserved, which this file does.

| Component | License | Used for |
|---|---|---|
| [ruzstd](https://github.com/KillingSpark/zstd-rs) 0.8, Copyright (c) 2019 Moritz Borcherding | MIT | Compressing and decompressing `.romrec` payloads (zstd frames), behind `romlens-core`'s `recording` feature |
| [twox-hash](https://github.com/shepmaster/twox-hash) 2.1, Copyright (c) 2015 Jake Goulding | MIT | ruzstd's frame checksums |

Both are MIT: permission to use, copy, modify and distribute, provided the
copyright notice and the permission notice are included in copies. The full
text is in each crate's `LICENSE` file as published on crates.io.

The pure-Rust zstd was chosen over the `zstd` crate (BSD-3-Clause, a
binding to the C library) because the C build needs a cross-compiler for
every target, which `make cross` and CI's Windows and Linux jobs do not have
(`16-phase2-plan.md` 2C.7 named it as the fallback).

Anticipated when the corresponding phase lands:

- UniFFI (Mozilla) — MPL-2.0 for the tool; generated bindings are ours to
  license. Recorded here for completeness.
- A permissively licensed SNES core (super-sabicom MIT, LakeSnes MIT, or
  ares ISC), Phase 5 only, with its notice reproduced in full.
- Imported symbol packs keep their authors' terms and are never bundled
  without permission.
