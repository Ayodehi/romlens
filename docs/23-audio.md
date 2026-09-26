# Sound: how the SNES makes audio, shown and heard

## Progress

Written 26 September 2026 and kept as written. This table is the only part
that tracks progress against it.

| Task | State |
|---|---|
| A0 this document, content-policy rule 11, the licensing note, the recording layer, the roadmap pointer, the checklist rows | done |
| A1 the SPC700 instruction set: decode, format, encode, I/O names, `romlens spc disasm` | done, 26 September 2026: `spc700::OPCODES`, the 256 opcodes row by row with their operands, lengths and cycles (the cycles are checked against the single-step suite in A7); `decode` gives each operand's value (the two memory-to-memory forms take the source byte first), where control goes (`Flow`, with `TCALL`'s and `BRK`'s vectors) and the memory an operand names for either direct page; `format_instruction` prints the Sony syntax with the 65816 listing's token kinds, `$F0–$FF` by name (`MOV DSPADDR,#$4C`). `assemble` reads that syntax back with labels, equates, `.org`, `.db` and `.dw`, so tests and the sound fixture are written as code; every opcode's text assembles back to its bytes. `aram::walk` follows code from its entries through branches, calls and vectors, stopping at the boot ROM's page and at jumps through tables. `romlens spc disasm <image> [--base A] [<address>] [--count N] [--walk]`. Tests: seven (the round trip, the table, 36 instructions read by hand from fullsnes, operand order, flow, the walk, the assembler's errors) and a golden |
| A2 BRR samples: `dsp::brr`, `DataKind::Sample`, `romlens brr` | to do |
| A3 the DSP's registers and the SPC700's I/O registers explained field by field | to do |
| A4 the recorder's audio layer | to do |
| A5 audio RAM, voices, notes and the ports over a recording, and the CLI | to do |
| A6 the SPC700 execution log (MesenCE), imported | to do |
| A7 the SPC700 and its timers and ports, emulated | to do |
| A8 the S-DSP, emulated | to do |
| A9 the upload traced from the ROM, and the sound commands | to do |
| A10 the FFI and `ApuPlayer` | to do |
| A11 the Voices, Samples and Audio RAM views | to do |
| A12 playback, the transport, the port console and the Scope | to do |
| A13 the Timeline, Ports and Echo views | to do |
| A14 N-SPC songs read as notes and commands | to do |
| A15 measure and record | to do |

## Context

The picture side of the SNES is well covered in Romlens: tiles, palettes,
OAM, tilemaps, the composed frame, its layers with their colour math, the
screen a routine sets up, and a pixel's provenance back to ROM. Sound is not
covered:
- the four APU ports have names (`model/hardware.rs`) and a sentence each
  (`explain/fields.rs`);
- one idiom recognises a loop waiting on the sound CPU (`explain/idioms.rs`,
  `ApuHandshake`);
- a DMA to B-bus `$40–$43` is called "the APU".

Recordings capture nothing from the sound side. There is no SPC700, no DSP,
no BRR decoder and no playback.

Sound on the SNES is a second computer, and that is what this track teaches:

1. **How the sound processor is used.** The S-CPU cannot touch audio RAM.
   - It talks to the SPC700 only through four byte-wide ports.
   - At power-on the SPC700's boot ROM waits there for an upload.
   - The game sends its sound driver and data a byte at a time, then tells
     the driver where to start.
   - From then on the driver runs on its own. It reads commands from the
     ports and writes the S-DSP's 128 registers to play eight voices.
2. **How sound data is represented.**
   - Samples are BRR: 9-byte blocks of sixteen 4-bit values, each with a
     shift and one of four prediction filters.
   - A sample directory in audio RAM lists each sample's start and loop.
   - Each voice has a pitch, an ADSR or GAIN envelope and a volume.
   - Echo is a ring buffer in audio RAM, fed back through an 8-tap FIR
     filter.
   - The music is the driver's own data: sequences of notes and commands.
3. **What it sounds like.** Hear a sample, a voice, a song, with each note
   tied back to the DSP write that made it, the SPC700 instruction that wrote
   it, and the 65816 code that asked for it.

## Scope decisions (26 September 2026)

- **Full playback, songs from the ROM included.**
  - Romlens gets its own SPC700 and S-DSP, written in Rust.
  - A song plays either from a recording frame (audio RAM and state as
    Mesen saw them) or from the ROM, through the upload traced statically.
  - This is the first game code Romlens runs itself. `12-content-policy.md`
    says so and adds rule 11.
- **The SPC700's code is read, not decompiled.**
  - The driver gets a disassembler with the I/O and DSP registers named and
    explained.
  - The MesenCE fork gains an SPC700 execution log, which tells the driver's
    code from its data in audio RAM.
  - The pseudo-C stays 65816-only.
- **One driver family is decoded.** Nintendo's N-SPC (Super Mario World,
  Zelda, Super Metroid and many licensees) gets its songs read as notes and
  commands. Every other view works for any driver, because it reads the
  hardware (the DSP registers, the sample directory, the ports), not the
  driver.
- **Nothing is exported.** No WAV, `.spc`, BRR or rendered audio, and the CLI
  prints only digests (rule 11).

## Constraints

- **Licence.** Romlens is 0BSD. The SPC700 and the DSP are written from the
  public documentation:
  - fullsnes;
  - anomie's S-DSP and SPC700 documents;
  - the SNESdev wiki.

  No code is taken from Mesen, bsnes, higan or blargg's snes_spc (GPL and
  LGPL), as `09-emulator-core-licensing.md` item 5 already says for the
  65816; item 6 records this.
- **The IPL boot ROM is Nintendo's code.** Its 64 bytes are never in the
  repository or the app. We boot a driver in one of two ways:
  - by high-level emulation of the upload protocol;
  - with our own 64-byte IPL, written in SPC700 code by us, which follows
    the same protocol for a driver that jumps back to `$FFC0` to take a
    second upload.
- **The Gaussian interpolation table.**
  - The DSP interpolates with a fixed 512-entry table, and the sound is not
    right without it.
  - It is a hardware constant, published in fullsnes, and we include it as
    data with that citation. We accept this knowingly, as rule 9 does its
    risk.
- **Mesen.**
  - Stock 2.2.1 already exposes audio RAM (`emu.memType.spcRam`), the DSP
    registers (`spcDspRegisters`), SPC700 execute and write callbacks, and
    `spc.*` in `getState()`, so the recorder's audio layer works on stock
    builds.
  - Only the execution log needs the fork, and fork work follows its usual
    rules.
  - Mesen runs the SPC700 lazily. It catches up when the S-CPU touches a
    port, and at the end of a frame after the Lua `endFrame` event. So the
    recorder stamps every audio event with the SPC700's own cycle count, and
    each snapshot carries `spc.cycle`.

## What already exists

- **Registers:**
  - `model/hardware.rs` has `APUIO0–3`.
  - `explain/fields.rs` `describe()` gives the short/fields/long shape that
    the DSP registers will follow.
- **Idioms:** `explain/idioms.rs` `apu_loop` recognises the wait. The upload
  loop that follows it is not recognised.
- **Values:** `explain/values.rs` and `explain/setup.rs` (constants in a
  routine and back through its callers) find the pointer an upload is given.
- **The CPU module's shape:** `cpu65816/` (`decode`, `opcodes`, `mnemonic`,
  `format`, `encode`).
- **Recordings:**
  - `recording/mod.rs` `StateRegion` and `MachineStateSource`;
  - `recording/mesen/stream.rs` and `pack.rs` (the stream and `.romrec`);
  - `lines.rs` (a chunk of timestamped events: the model for `APUL`);
  - `recording/format.rs` (unknown region ids are refused, hence a version
    bump).
- **The execution log:**
  - `model/exec_log.rs`, `io/import/exec_log.rs` (CPU byte 0 only);
  - the fork's `SnesExecutionLogger`.
- **The app:**
  - `GraphicsModel` and `GraphicsMenu` and `RomViewModel.graphicsTab`: the
    pattern for an Audio picker;
  - `Bitmap+AppKit.swift` for drawing.

  There is no audio code in the app yet.
- **CLI goldens:** `crates/romlens-cli/tests/golden.rs`. Images are pinned as
  digests, and so will sound be.

## Design

### The SPC700 instruction set (`spc700/`)

- An opcode table of 256 entries: mnemonic, operand mode, length and cycles.
- `decode_at` over a 64 KB image; `format` in the usual syntax
  (`MOV A,#$12`, `MOV $F2,#$4C`, `BBS $12.3,label`).
- `encode`, used by the fixtures and the tests.
- `names.rs` names `$F0–$FF`:
  - TEST, CONTROL, DSPADDR, DSPDATA;
  - CPUIO0–3, AUXIO4/5;
  - T0–2TARGET, T0–2OUT.
- `aram.rs` lists an audio RAM image. It walks the reachable code from the
  driver's entry and marks the rest data, unless an execution log says
  otherwise.

### BRR (`dsp/brr.rs`)

- A block is a header (shift 0–12, filter 0–3, loop, end) and sixteen signed
  nibbles.
- Decoding returns the samples and, for teaching, each step: the nibble, the
  shifted value, the two previous samples, the filter's prediction, and the
  clamped and wrapped result.
- A new `DataKind::Sample` classifies BRR in the ROM when the upload traced
  in A9 places it. Until then it is only a user mark.

### The DSP and I/O registers explained (`explain/dsp_fields.rs`)

- All 128 DSP registers use the `describe()` shape. Per voice:
  - VOL L/R, signed;
  - PITCH (with the rate it plays at);
  - SRCN (the directory entry and its address);
  - ADSR1/2 (each rate in milliseconds);
  - GAIN (direct, or the four slopes);
  - ENVX, OUTX.
- The globals:
  - MVOL, EVOL;
  - KON, KOF, ENDX (as voice lists);
  - FLG (reset, mute, echo writes off, noise clock);
  - EFB, PMON, NON, EON;
  - DIR (the directory's address);
  - ESA and EDL (the buffer's address, its size and the delay in ms);
  - FIR0–7.
- The SPC700's I/O registers the same way. For example, CONTROL's timer
  enables, port clears and IPL enable, and a timer's period from its target.
- Every entry is checked against fullsnes or anomie, and the tests pin the
  decoded values, as in 20-explanations.md.
- SPC700 idioms follow the `idioms.rs` pattern:
  - a DSP write (`MOV $F2,#reg` then `MOV $F3,A`);
  - a key-on;
  - a timer wait;
  - a port command read and echoed.

### The recording's audio layer

- **Regions.** Three new `StateRegion`s:
  - `Aram` (64 KB, changed 256-byte blocks as for WRAM);
  - `DspRegs` (128 B);
  - `SpcState`: A, X, Y, SP, PC, PSW; the four ports each way; CONTROL; the
    timers; the DSP address latch; the IPL enable; `spc.cycle`.
- **The `APUL` chunk: timestamped events.** Each event carries the SPC700
  cycle and PC, and the S-CPU master clock where it is the writer.
  - The S-CPU's writes to `$2140–$2143`, on the `snesRegister` callback.
  - The SPC700's writes to `$F4–$F7`.
  - Its DSP writes: `$F3` with the `$F2` latch.
- **The recorder** (`mesen_recorder.lua`):
  - adds `spc.` to its field prefixes;
  - compares audio RAM in blocks with `read32`;
  - registers the three callbacks, only when `emu.memType.spcRam` exists.
- **Versions.** The stream and `.romrec` versions go up: a reader older than
  the audio layer refuses the new regions. Files without it open as before.
- **Reading it.** `MachineStateSource` gains the regions and
  `apu_events(frame)`.

### The SPC700 execution log (MesenCE)

- An `SpcExecutionLogger` in the fork's `SpcDebugger`, with the same MXLG
  sections under CPU byte 1.
- Its `ACCS` kinds tell the SPC700's own accesses from the DSP's sample
  fetches and echo writes. Those mark BRR and the echo buffer exactly.
  - Whether `Spc.cpp` can tell the two apart at the hook is checked first.
  - If it cannot, the logger reads the fetch addresses from the DSP's
    state.
- `io/import/exec_log.rs` accepts CPU 1 into an audio RAM coverage map. The
  map is kept per recording, because audio RAM is not the ROM.

### The machine (`apu/`, `dsp/`)

- `apu::cpu`: the SPC700 interpreter.
  - PSW, with P selecting the direct page.
  - The edge cases of `MUL`, `DIV`, `DAA`/`DAS`, `TCALL`, `BRK` and
    `SLEEP`/`STOP`.
  - Cycle counts from the table.
- `apu::io`:
  - CONTROL;
  - three timers (8 kHz, 8 kHz and 64 kHz, with 4-bit counters cleared on
    read);
  - the port latches, both ways.
- `apu::ipl`: our IPL and the high-level boot.
- `Apu { cpu, aram, io, dsp }`:
  - `step(cycles)`;
  - `write_port` / `read_port` for the S-CPU side;
  - `from_snapshot` (a recording frame);
  - `from_upload` (blocks and entry).
- `dsp`, at 32 kHz, one sample every 32 SPC700 cycles:
  - voices: the pitch counter, pitch modulation from the previous voice,
    noise, the Gaussian interpolation, the ADSR/GAIN envelope with its rate
    table, ENVX and OUTX, ENDX;
  - echo: ESA/EDL, the FIR, EFB, ECEN;
  - the mix with MVOL, EVOL and FLG.
- `render(n)` gives stereo 16-bit frames, with per-voice mute and solo and a
  tap per voice for the scopes.
- Every KON, KOF, pitch and sample change is an event for the timeline, so
  an emulated run and a recorded one give the same data.

### From the ROM (`audio/upload.rs`)

- **An IPL upload idiom in `explain/idioms.rs`:**
  - the first write of `$CC`;
  - the loop that writes a byte and its index and waits for the index to
    come back;
  - the new-block step (index plus 2 or more);
  - the final jump.
- **The block list.** Where the idiom is given a pointer (found with
  `values`/`setup`), the standard list is parsed: length and address, then
  the bytes, ending with a zero length and the entry.
- **The result.**
  - `Upload { blocks: [(aram, rom_offset, len)], entry }` gives each audio
    RAM byte's origin in the ROM, the sound side of pixel provenance.
  - ROM bytes the upload places are classified as uploaded: driver code,
    song data or `Sample`, as the audio RAM map says. Until now they were
    plain `Byte` at confidence 85.
- **"Send a sound command"**, an idiom on the 65816 side: a constant stored
  to a port at a call site. It lists the values a game sends, and the port
  console offers them.

### Audio RAM, notes and ports (`audio/`)

- **`aram_map`:**
  - pages 0 and 1 (direct page and stack), the I/O registers and the IPL
    page;
  - the sample directory at DIR×$100, each entry's start and loop;
  - each sample, walked to its END block;
  - the echo buffer at ESA×$100, EDL×2 KB;
  - driver code (reachability, or the log);
  - the rest song or unknown data.

  Each part keeps its ROM origin when the upload is known.
- **`notes`:**
  - pitch to frequency: $1000 plays the sample at 32 kHz;
  - each sample's tuning estimated from its loop's period, so a note is
    named with cents;
  - a `Timeline` of note events per voice.
- **`ports`:** the conversation both ways, each message linked to the 65816
  instruction that sent it (from the recording or the `.mxlog`) and to the
  SPC700 instruction that read or answered it.
- **`nspc`**, last:
  - recognises the N-SPC driver by its code;
  - decodes the song table, the patterns and each track's commands (notes,
    ties, rests, instrument, volume, pan, tempo, loops) as text beside the
    timeline.

### The FFI (API 0.11.0)

- Records:
  - `VoiceInfo`, `DspRegisterInfo`;
  - `BrrBlockInfo` with its steps;
  - `SampleInfo`, `AramRegionInfo`;
  - `NoteEventInfo`, `PortEventInfo`;
  - `UploadInfo`, `SpcLineInfo`.
- `RecordingSession` gains:
  - `voices(frame)`, `dsp_registers(frame)`;
  - `aram_map(frame)`, `samples(frame)`;
  - `note_timeline()`, `port_events(range)`;
  - `spc_listing(frame, range)`.
- `Workbench` gains `upload()`, `sound_commands()` and `brr_at(offset)`.
- An `ApuPlayer` object:
  - built from a recording frame or the upload;
  - `send_port`, `render`, `mute`, `solo`, `voice_scopes`, `seek`.

  It is `Send + Sync`, so the app fills a ring buffer from a background queue
  and never calls into Rust on the audio thread.

### The app

An Audio menu in the toolbar works like the Graphics one (`audioTab`, then
`AudioEditorView`), over a recording frame or the ROM's upload.

1. **Voices.** Eight channel strips. Each has:
   - the sample, its note and pitch;
   - the volume left and right;
   - its envelope drawn as a curve with the current point, and an ENVX
     meter;
   - key on/off, echo, noise and pitch modulation;
   - mute and solo.
2. **Timeline.** A piano roll per voice. A note opens its frame, the DSP
   write, the SPC700 instruction that wrote it, and the port command that
   started the song.
3. **Samples.**
   - The directory.
   - The waveform with its loop point.
   - The BRR block grid: header fields, then each nibble to its sample, with
     the filter's formula filled in with that sample's numbers.
   - Play at a pitch, or from an on-screen keyboard.
   - The ROM origin, which links to the listing.
4. **Audio RAM.** The 64 KB map coloured by part, and the SPC700 listing and
   hex of the selected part, with the upload blocks that filled it.
5. **Echo & effects.**
   - The FIR's taps and its frequency response.
   - The delay in ms and the feedback.
   - Where the buffer sits in audio RAM.
   - Noise and pitch modulation.
6. **Ports.** The messages both ways with links into both CPUs' code, and an
   upload's progress block by block.
7. **Scope.** Each voice's waveform and the output, while playing.

**The inspector** explains:
- DSP and I/O register writes in the SPC700 listing;
- the upload and sound-command idioms in the 65816 listing, with Play This
  Command.

**Playback.**
- `AVAudioEngine` with an `AVAudioSourceNode` at 32 kHz, fed from a
  lock-free ring.
- Play and pause, follow the recording, and a port console for sending a
  command byte.

### The CLI

- `romlens spc disasm <rec> --frame N [<addr>] [--count N]`
- `romlens apu voices|dsp|map|samples --rec R --frame N [--json]`
- `romlens apu timeline --rec R` and `romlens apu ports --rec R --frames A..B`
- `romlens apu upload <rom> [--project P]`
- `romlens apu song <rec|rom> …` (N-SPC)
- `romlens brr <rom> <offset> [--blocks] [--ascii]`
- `romlens apu render (--rec R --frame N | <rom> --command 0=$xx) --seconds S --digest`,
  a SHA-256 of the PCM and never the audio.

### Fixtures

`fixtures/sound.rs` builds a small ROM of our own:
- a 65816 upload through the standard protocol;
- a driver assembled with `spc700::encode`;
- a generated square-wave BRR sample.

The driver keys voice 0 on with an ADSR and echo, and answers one port
command. It runs on real Mesen (with the real IPL) and on ours (with our
IPL), and it is the recording for the goldens.

## Ordered tasks

Each task is one commit and is pushed. Sizes are in days.

| # | Task | Size |
|---|---|---|
| A0 | this document, rule 11, the licensing note, docs/13's layer, the roadmap pointer, checklist rows 5A | 0.5 |
| A1 | `spc700/`: the table, decode, format, encode, names; every opcode tested, encode and decode round trip; `romlens spc disasm` over a raw image | 2 |
| A2 | `dsp/brr.rs`, `DataKind::Sample`, `romlens brr`; hand-worked blocks for filters 0–3, clamping, loop and end | 1 |
| A3 | `explain/dsp_fields.rs` and the I/O registers, each checked against a named reference and tested; the SPC700 idioms | 1.5 |
| A4 | the recorder's audio layer: Lua with feature detection, `APUL`, the regions, version bumps; the cost per frame measured; recordings of Super Mario World and Final Fantasy III for development (not committed) | 2 |
| A5 | `audio/aram_map`, `notes`, `ports` over a recording; `romlens apu voices\|dsp\|map\|samples\|timeline\|ports` with goldens on the fixture's recording | 2 |
| A6 | the fork's SPC700 execution log (issue, PR, squash-merge) and its import; the map uses it | 2 |
| A7 | `apu/`: the interpreter, timers, ports, our IPL, the high-level boot. Checked against Tom Harte's SPC700 single-step tests (MIT, read from `ROMLENS_SPC_TESTS`, not committed). A differential run replays a recording's port writes from a snapshot and compares audio RAM, the DSP registers and the DSP write log with Mesen's frame by frame | 4 |
| A8 | the rest of `dsp/`: voices, envelopes, noise, modulation, echo, mix. Tested with hand-computed envelopes and pitches; ENVX, OUTX and ENDX compared with the recording; a render digest golden | 3 |
| A9 | `audio/upload`: the upload idiom, the block list, audio RAM's ROM origins, the sound commands, ROM classification; `romlens apu upload` and `apu render <rom>` | 2 |
| A10 | the FFI and `ApuPlayer`, API 0.11.0, RomlensKit tests | 1.5 |
| A11 | the app's AudioModel, the Audio menu, Voices, Samples, Audio RAM, the inspector, tests | 3 |
| A12 | playback, the transport, the port console, the Scope | 2 |
| A13 | the Timeline, Ports, and Echo & effects, linked to both CPUs' code | 2.5 |
| A14 | `audio/nspc` in the Timeline and `romlens apu song` | 2 |
| A15 | measure on Super Mario World, Super Metroid, Final Fantasy III, Chrono Trigger and one game not on N-SPC: uploads traced, samples found, the timeline against Mesen's, how long ours runs before it drifts from the recording; the manual pass | 1 |

The order puts first what teaches without an emulator: the SPC700's code,
BRR, the registers, and a recording's voices (A1–A6). The machine and
playback follow (A7–A12), and the one driver-specific piece comes last. The
Phase 3 tutor (T1–T4) and M in `22-phase3-finish.md` are still open. The
tutor gains audio tools once A10 exists.

## What is cut

- Any export of sound: WAV, `.spc`, BRR or rendered audio (rule 11).
- Decoding songs of drivers other than N-SPC.
- Running the game's own 65816 upload code. The standard block list is
  traced statically; any other upload starts from a recording frame.
- MSU-1, and sound through the SA-1 or SuperFX.
- Editing samples or songs.

## Risks

- **An emulator that is slightly wrong sounds wrong.** Countered by:
  - the single-step suite;
  - the frame-by-frame differential run against Mesen recordings;
  - the drift A15 measures.
- **Wrong explanations teach wrong things.** Every DSP field is checked
  against a named reference and pinned by a test.
- **Comparing audio RAM in Lua every frame costs time.** It is measured in
  A4. If it is too slow, audio RAM is captured every N frames; the event log
  rebuilds the frames between.
- **The fork cannot tell the DSP's fetches from the SPC700's reads.** Checked
  first in A6, with the DSP state as the fallback.
- **Real-time audio on macOS.** No allocation and no calls into Rust on the
  render thread; the ring is filled ahead.
- **Nintendo's material.** The IPL is never shipped, because ours replaces
  it. The Gaussian table is included as a cited hardware constant.

## Verification

- `make test`:
  - unit tests for `spc700`, `brr`, `dsp` and the explanations;
  - the single-step suite when `ROMLENS_SPC_TESTS` is set;
  - upload tracing on the dev ROMs with `ROMLENS_ROM_DIR`;
  - the differential run against recordings.
- A golden for every new command, refreshed with `UPDATE_GOLDEN=1` and each
  diff read. Rendered sound is pinned as a digest.
- `make swift` and `make app-test`, with a test for each audio view.
- The manual pass (`15-conformance-checklist.md`, 5A), in Super Mario World:
  - the upload idiom in RESET;
  - the Audio RAM map and the directory behind it;
  - a BRR block's filter worked through;
  - the title screen's voices from a recording;
  - the title song played from the ROM, one voice soloed, and a note
    followed back to the SPC700 and 65816 code.
