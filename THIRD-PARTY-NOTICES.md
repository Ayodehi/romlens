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
| [resvg and usvg](https://github.com/linebender/resvg) 0.48, the Resvg Authors | Apache-2.0 OR MIT | Reading and drawing the diagrams' SVG (`romlens-draw`, docs/26) |
| [tiny-skia](https://github.com/linebender/tiny-skia) 0.12, Yevhenii Reizner and the Linebender authors | BSD-3-Clause | resvg's rasterizer, and the diagrams' PNG |
| [fontdb](https://github.com/RazrFalcon/fontdb), [harfrust](https://github.com/harfbuzz/harfrust), [skrifa, read-fonts and font-types](https://github.com/googlefonts/fontations) | MIT; MIT; MIT OR Apache-2.0 | Loading, shaping and measuring the diagrams' text |
| resvg's other dependencies: roxmltree, svgtypes, simplecss, kurbo, polycool, xmlwriter, strict-num, rgb, bytemuck, arrayref, arrayvec, euclid, float-cmp, imagesize, data-url, base64, png, fdeflate, flate2, miniz_oxide, zlib-rs, adler2, crc32fast, simd-adler32, slotmap, smallvec, tinyvec, unicode-bidi, unicode-script, unicode-vo, pico-args, log | Each MIT, Apache-2.0, BSD-2-Clause, Zlib or a choice of these (listed with `cargo metadata`) | Parsing SVG and CSS, geometry, PNG and deflate |
| [IBM Plex Sans and IBM Plex Mono](https://github.com/IBM/plex), Copyright © 2017 IBM Corp., with Reserved Font Name "Plex" | SIL Open Font License 1.1 (`crates/romlens-draw/fonts/OFL.txt`) | The diagrams' text, shipped unmodified so every machine draws the same picture |

ruzstd and twox-hash are MIT: permission to use, copy, modify and distribute, provided the
copyright notice and the permission notice are included in copies. The full
text is in each crate's `LICENSE` file as published on crates.io; the
same holds for the diagram crates' MIT, Apache-2.0, BSD and Zlib notices.
The fonts are bundled, not modified, and the OFL's text travels with them;
the OFL lets them be embedded in software under any licence, as long as
they are not sold on their own.

The pure-Rust zstd was chosen over the `zstd` crate (BSD-3-Clause, a
binding to the C library) because the C build needs a cross-compiler for
every target, which `make cross` and CI's Windows and Linux jobs do not have
(`16-phase2-plan.md` 2C.7 named it as the fallback).

## The Linux shell

`shells/linux` is dynamically linked against the system's GTK 4, libadwaita,
GLib, Cairo, Pango, libsecret and ALSA, which are LGPL-2.1-or-later: the
packages depend on the distribution's copies (the Flatpak and snap carry the
runtime's), and a person can replace them. Its Rust crates keep their own
licenses:

| Component | License | Used for |
|---|---|---|
| [gtk4-rs, libadwaita-rs, libsecret-rs and the gtk-rs family](https://github.com/gtk-rs) | MIT | Rust bindings to the libraries above |
| [cpal](https://github.com/RustAudio/cpal) 0.18 | Apache-2.0 | The sound output |
| [UniFFI](https://github.com/mozilla/uniffi-rs) 0.31 (`uniffi`, `uniffi_core`, `uniffi_macros` and friends) | MPL-2.0 | The core's public API, which the macOS shell reads through generated Swift. The Linux build links the runtime crates and not the binding generators. MPL-2.0 is file-level: the crates are unmodified, and their source is on crates.io under the versions in `Cargo.lock` |
| [rustls](https://github.com/rustls/rustls), [ring](https://github.com/briansmith/ring), rustls-webpki, rustls-native-certs, untrusted | Apache-2.0 OR ISC OR MIT; Apache-2.0 AND ISC; ISC | The tutor's HTTPS connections to a model provider |
| [webpki-roots](https://github.com/rustls/webpki-roots) | CDLA-Permissive-2.0 | A built-in list of CA certificates for the HTTPS connections |

The rest of the shell's dependency tree is MIT, Apache-2.0, Zlib, BSD-2-Clause,
BSD-3-Clause, ISC, Unlicense, or a choice among them (`cargo metadata` in
`shells/linux` lists each crate's license). No dependency is GPL or AGPL.

Anticipated when the corresponding phase lands:

- UniFFI (Mozilla) — MPL-2.0 for the tool; generated bindings are ours to
  license. Recorded here for completeness.
- A permissively licensed SNES core (super-sabicom MIT, LakeSnes MIT, or
  ares ISC), Phase 5 only, with its notice reproduced in full.
- Imported symbol packs keep their authors' terms and are never bundled
  without permission.
