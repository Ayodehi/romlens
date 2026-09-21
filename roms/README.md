# roms/

Put ROM images here. Everything in this folder except this file and
`.gitkeep` is ignored by git (see the repository `.gitignore`), so licensed
content cannot be committed by accident.

Romlens never copies, uploads or redistributes a ROM. Project files store a
SHA-256 and a remembered path. Opt-in tests that need a specific commercial
ROM read it from here via the `ROMLENS_ROM_DIR` environment variable and are
skipped when it is absent; CI uses the homebrew test ROM built by the test
suite itself.

Development ROM used by the ground-truth tests: `SuperMetroid.F8DF.sfc`
(3 MB, LoROM + FastROM, header checksum `F8DF`).
