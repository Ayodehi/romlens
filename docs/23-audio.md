# Sound: how the SNES makes audio, shown and heard

## Progress

Written 26 September 2026 and kept as written. This table is the only part
that tracks progress against it.

| Task | State |
|---|---|
| A0 this document, content-policy rule 11, the licensing note, the recording layer, the roadmap pointer, the checklist rows | done |
| A1 the SPC700 instruction set: decode, format, encode, I/O names, `romlens spc disasm` | done, 26 September 2026: `spc700::OPCODES`, the 256 opcodes row by row with their operands, lengths and cycles (the cycles are checked against the single-step suite in A7); `decode` gives each operand's value (the two memory-to-memory forms take the source byte first), where control goes (`Flow`, with `TCALL`'s and `BRK`'s vectors) and the memory an operand names for either direct page; `format_instruction` prints the Sony syntax with the 65816 listing's token kinds, `$F0–$FF` by name (`MOV DSPADDR,#$4C`). `assemble` reads that syntax back with labels, equates, `.org`, `.db` and `.dw`, so tests and the sound fixture are written as code; every opcode's text assembles back to its bytes. `aram::walk` follows code from its entries through branches, calls and vectors, stopping at the boot ROM's page and at jumps through tables. `romlens spc disasm <image> [--base A] [<address>] [--count N] [--walk]`. Tests: seven (the round trip, the table, 36 instructions read by hand from fullsnes, operand order, flow, the walk, the assembler's errors) and a golden |
| A2 BRR samples: `dsp::brr`, `DataKind::Sample`, `romlens brr` | done, 26 September 2026: `dsp::brr::decode_block` decodes a block and keeps each value's steps: the nibble, the shift (13–15 give 0 or -2048), the previous two results, the filter's prediction in the DSP's integer form, the 16-bit clamp, the 15-bit wrap, and the doubled output; `decode_sample` walks from a start to the end flag, carrying the history from block to block and marking the loop point's block. A new data kind `sample` (batch code 13) marks BRR in the ROM, in the core, the FFI, the CLI's map (`a`) and the app's mark sheet, strip and Atlas. `romlens brr <rom> <at> [--loop L] [--blocks] [--ascii] [--max N]`: the blocks with their headers and ranges, every value's decoding, and a text waveform. `fixtures::sound` holds a sample of our own (a ramp, a decay filter 1 predicts, a looping square wave). Tests: seven, each case worked by hand (the shift, the filters, the history, a wrap and a clip, the walk, the loop), and a golden |
| A3 the DSP's registers and the SPC700's I/O registers explained field by field | done, 26 September 2026: `explain::sound` gives all 128 DSP registers and `$F0–$FF` the 65816 registers' shape (`RegisterWrite`, its fields and short form), checked against fullsnes's text (`problemkaputt.de/fullsnes.txt`, its APU chapters), which also renamed the timer dividers `T0DIV`–`T2DIV` and key-off `KOFF`: volumes signed with their share of full and phase, KON/KOFF/ENDX/NON/EON as voice lists, PMON as who follows whom, DIR and ESA as the memory they point to, EDL as delay and buffer size, FLG's reset, mute, echo-write and noise clock, ADSR1/2 and GAIN with each rate's time worked out from fullsnes's step rules and rate table (attack 0 full in 4.0 s, decay 0 halving in 326 ms), SRCN as its directory entry, CONTROL's timers, port clears and boot ROM, TEST's settings, DSPADDR naming the DSP register it picks, the dividers as periods; `pitch_words` gives a pitch's sample rate and semitones. `explain::spc` follows a walk's code keeping A, X, Y and the DSP register picked between joins, so `MOV $F3,A`, `MOVW $F2,YA` and `INC $F2` read as the DSP write they make (`KON = $01: voice 0`), and names two waits: a timer's count (with its period when the divider was set) and a port, telling a loop that waits for a new byte from one that reads until two reads agree. `romlens registers --dsp|--spc [<reg>] [--value V]`; `spc disasm --walk` comments each write with its meaning and puts a `▸` note above each wait. Tests: five (names, global and voice writes, I/O writes, the pass) and two goldens |
| A4 the recorder's audio layer | done, 26 September 2026: on stock Mesen 2.2.1 (probed first: `spcRam`, `spcDspRegisters`, write callbacks on `snesRegister` `$2140–$2143` and on the SPC700's `spcMemory` `$F0–$FF` all work, and `emu.getCpuCycleCount(emu.cpuType.spc)` gives the SPC700's own clock at any callback). Stream version 3: a flags byte (the sound side), `spc.` fields but not `spc.dsp.`, an `A` record per frame of every port write both ways and every SPC700 I/O write, each on the SPC700's cycle count (and the master clock for the S-CPU's), and an `S` record of audio RAM's changed blocks and the 128 DSP registers. Format 1.2: regions `aram`, `dsp` and `spc` (`SpcState`, 32 bytes with the cycle count), and the `APUL` layer chunk (zstd) with `DSPADDR` carried in from the frame before, so `ApuEvents::dsp_writes` pairs each `DSPDATA` write with its register; `rec validate` checks the chunks, `rec info` names the layer, `rec pack` counts the events, and `rec extract` refuses audio RAM (rule 11). Measured on Super Mario World, 900 frames from power-on: about 1.1 ms a frame more in the recorder (6.75 s against 5.78 s headless), 176,359 events of which 14,666 are DSP writes, the recording 24% larger (11.9 MB against 9.6 MB); the voices' ENVX and OUTX rise and fall through the title music. A live session skips the sound records for now. Tests: the events' pairing and round trip, a stream with a sound side packed and read back (`encode::fixture_with_audio`, the sound fixture's directory and sample), goldens |
| A5 audio RAM, voices, notes and the ports over a recording, and the CLI | done, 26 September 2026: `audio::voices` reads the eight voices from the DSP's registers (volume, pitch, sample and its directory entry, envelope in words, ENVX and OUTX, echo, noise, modulation, KOFF, ENDX); `audio::directory` keeps the directory entries that start a sample ending within 4,096 blocks, from entry 0 on and those the voices name; `audio::aram_map` divides audio RAM by what the hardware points at: the direct page, I/O, stack, the code walked from the SPC700's program counter, the directory, each sample, the echo buffer (with writes on, even EDL 0 writes its 4 bytes) and the boot ROM, the rest the driver's data; `audio::timeline` gives each voice's notes keyed on, bent and keyed off from the DSP writes, pitch and sample carried from the frame before; `audio::port_messages` the ports both ways. A pitch is a rate, not a note, so `sample_tuning` estimates the frequency a sample makes at `$1000` from its loop with YIN's normalized difference (a plain difference or correlation took a smooth bass wave for a 16 kHz one), rounded to a whole division of the loop, and every note it names is marked as an estimate. `romlens apu voices|dsp|map|samples --rec R [--frame N]`, `apu timeline|ports --rec R [--frames A..B] [--limit N]`. On Super Mario World's title (frame 850): 20 samples behind the directory at `$8000`, a 32 ms echo buffer at `$6000` with writes off, six voices sounding, and the notes played come out as E6, B6, B3 and G4 within a few cents; the fixture recording's driver now sits in its audio RAM. Tests: five on the fixture recording and a golden |
| A6 the SPC700 execution log (MesenCE), imported | done, 26 September 2026: the fork's `SpcExecutionLogger` (MesenCE #7, PR #8) keeps the main CPU's MXLG format with CPU byte 1: the instructions run, what each read and wrote, and the DSP's own reads and writes, which `SpcDebugger` already tells apart (`MemoryAccessFlags::DspAccess`), so the fallback through the DSP's state was not needed. Lua's five log functions take an optional CPU type. The recorder starts it where it exists and writes `<name>.spc.mxlog`; `rec pack` checks it and puts it beside the recording; `model::spc_log` and `io::import::spc_log` read, write and merge it, and the main CPU's importer refuses it with where it goes. `aram_map` takes the log (docs/17): what ran is code, what the driver read or wrote is its data (the boot ROM's upload loop left out, since it touches every uploaded byte), what the DSP read is sample data, and each sample says whether it was played. On Super Mario World's first 900 frames: 1,252 instructions, a 2.3 MB log; the driver's code at `$0500`–`$133D`, its tables and song data beside it, and 7 of the 20 samples played. Tests: three on the format, merging and the map, and the golden with a log beside the recording |
| A7 the SPC700 and its timers and ports, emulated | done, 26 September 2026: `apu::Spc700` runs one bus call per cycle (`SpcBus`: read, write, idle), dummy reads included, and matches all 256,000 cases of Tom Harte's single-step suite (MIT, `ROMLENS_SPC_TESTS`, not committed) in registers, memory and every cycle's address and kind, the first time it ran. `apu::ApuBus` is audio RAM, `$F0–$FF` (CONTROL, the ports both ways, DSPADDR/DSPDATA with the read-only mirrors and ENDX cleared on write, the spare registers, the dividers, outputs cleared on read), writes reaching RAM under I/O and the boot ROM, and the three timers (a clock every 128, 128 and 16 cycles, a counter to the divider, a 4-bit output); the DSP is its register file until A8. S-CPU port writes queue on the cycle they land on. `apu::ipl` is our own 64-byte boot program, assembled from source with our assembler, written from fullsnes's description of the upload protocol (X keeps the last echo, so a byte is never taken for a command), and `boot_upload` boots straight into a driver. Format 1.3: the `spc` region grows to 48 bytes with each timer's phase and internal count (Mesen's `stage0`/`stage1`/`stage2`); `rec pack` halves Mesen's SPC700 clock (it counts halves), and times each S-CPU port write from its master clock, because Mesen calls the recorder before catching the SPC700 up and the cycle it reports can be a frame stale. `apu::replay` runs the SPC700 beside a recording: from a snapshot, the port writes on their cycles, events placed by cycle rather than by frame, compared at each frame's end with the registers, audio RAM, the DSP registers the SPC700 writes and every I/O write's register, value and cycle. Mesen stops the SPC700 inside an instruction at a frame's end and does not say where, so a replay starts from each instruction the snapshot could be inside (its clock wound back by what it could have run) and the end accepts either side of the last instruction. `romlens apu replay --rec R [--frames A..B] [--free]`. On Super Mario World's first 900 frames, run on from frame 20 (the driver just started, receiving its samples through the ports) to 899: 874 frames match exactly and 5 but for the registers at the frame's end, none differ, and all 69,403 I/O writes land on Mesen's cycle; each frame from its own snapshot, 855 of 880. Tests: the single-step suite, eleven on the timers, ports, DSP access, RAM under I/O and the boot ROM (a closed-loop upload of two blocks, one wrapping the index, then the jump, and the high-level boot leaving the same RAM), the 1.2 region size, a replay dev test (`ROMLENS_SPC_REC`), and a golden on a stream Romlens's own SPC700 made |
| A8 the S-DSP, emulated | done, 26 September 2026: `dsp::Dsp` holds what the chip keeps inside (each voice's place in its sample, decoder history, four-sample window, pitch fraction, key-on wait, envelope and its phase; the global counter; the noise; the echo position and the FIR history) beside the 128 registers the bus keeps. It runs on the SPC700's clock one step a cycle: voice *n* at step 3*n*+2, where fullsnes's timing chart has its ENVX written, and at step 31 the KON/KOFF poll (every other sample, KON latched on write), the global counter, the noise, echo and the mix. Voices: BRR streamed from the directory (ENDX set as an end block starts, a non-looping one releasing at level 0), the pitch counter with modulation from the voice before (`(OUTX >> 4) + $400`), 4-point Gaussian interpolation on 15-bit samples with the documented wrap and clamp, noise in place of the sample, the output at the envelope's level so far and the envelope stepping after it. Envelopes: ADSR and the four gain modes and direct gain, release by 8 a sample, each step when `(counter + offset) % period == 0` with the SNESdev wiki's period and offset tables and a counter counting down from `$7800`. Echo: the ESA/EDL ring, the 8-tap FIR (seven wrapping, the last clamped), EFB feedback, writes gated by FLG, and the mix with MVOL, EVOL and mute. The Gaussian table (`dsp/gauss.rs`) is checked value by value against both fullsnes and anomie. A replay places the DSP from the SPC700's cycle count since power on (its step, counter and KON phase); its inside is not in a snapshot, so it starts from rest. On Super Mario World, run on from frame 20: ENVX matches Mesen in 6,667 of 7,032 voice-frames (94.8%), OUTX 6,043 (85.9%), ENDX 874 of 879, and the SPC700 side stays exact. Found by the comparison: interpolation takes 15-bit samples, not doubled ones (OUTX 74% → 83%); a sample goes out before its envelope steps; voices stepping at their own points in the sample (→ 86%). Measured and left alone: the key-on wait (5 samples, fullsnes; 3–9 made no difference), the counter's offset, and the decay-to-sustain rule; what remains is timing inside the sample that the documents do not give. `romlens apu render --rec R [--frame N] [--seconds S] [--alone]` plays from a snapshot, following the recording's port writes, and prints only a SHA-256 of the samples, the levels and each voice's peak (rule 11); five seconds of Super Mario World's title take 50 ms. Tests: ten worked by hand (the counter, attack 15, direct gain and release, linear gain and rate 0, the pitch counter at three pitches, a flat sample's level, ENDX and the loop, the noise, the volumes and mute, echo through the FIR), and goldens on a fixture Romlens's SPC700 and DSP made, whose driver plays a note: render digests, and a replay matching ENVX and OUTX 32 of 32 |
| A9 the upload traced from the ROM, and the sound commands | done, 26 September 2026: `audio::upload` finds the routine that runs the upload protocol by what it does (reads APUIO0, holds `$BBAA` or `$CC`, reads a block list through `[dp],Y`), the constant pointers other code stores into that pointer (from the values before each store, 8- or 16-bit, direct page or absolute low RAM), and reads the block list at each: length and address, the bytes, a zero length and the entry. Each block says where its bytes are in the ROM. Constants stored to the ports are sound commands, and so are constants stored to a RAM byte that other code copies to a port (Super Mario World sets `$1DF9`–`$1DFC` and its NMI copies them). `from_ports` reads an upload back from a recording's port writes (a 16-bit store's two bytes, or byte then index), to check what the code says against what was sent. The analysis runs a fast path last (the routines the cross references show near the ports, and their callers three calls up, every routine that contains the call) and boots the driver on Romlens's SPC700 for a tenth of a second to learn the sample directory: uploaded bytes become samples where the directory names them, the directory by name, the rest the driver and its data, at 95% with a new evidence kind, `Uploaded` (FFI and inspector too), replacing the sweep's, the trace's and the heuristics' guesses but never code, a log's observation, an import or the user's word; the analysis still takes 31 ms on Super Mario World. The idiom `apu-upload` names the routine in the listing (docs/20). `romlens apu upload <rom> [--project P] [--rec R]` lists the routine, each upload's blocks with their ROM and audio RAM ranges, what the bytes are once the driver runs, the sound commands by port, and with a recording the uploads it saw sent, each matched to a traced list; `romlens apu render --rom R [--with LIST] [--port P=$xx]` boots the traced driver, lays more uploads over it, sends a command and describes the sound. On Super Mario World: the routine at `$00:8079`, five lists (the driver at `$0E:8000` with its entry `$0500`, the samples at `$0F:8000`, three song banks), all three uploads the recording sent matched byte for byte, 112 command values (sound effects on ports 0 and 3, music on port 2 through `$1DFB`); the song bank at `$03:E404` a heuristic had called a palette is now song data, and the 28 KB of samples one sample region; the title music plays from the ROM alone (`--with $0F:8000 --port 2=$01`, its voices' peaks those of the recorded title). The fixture `sound_upload_lorom` (`romlens testrom --fixture sound`) is a 65816 upload routine of our own with a driver, directory and sample; on Mesen, through Nintendo's boot program, it uploads and runs, and the recording's upload matches its traced list. Tests: four (the trace, the classification, booting and playing, reading the upload back in both write orders) and a golden |
| A10 the FFI and `ApuPlayer` | done, 26 September 2026: `romlens-ffi/src/audio.rs`, API 0.11.0. `RecordingSession` gains `has_sound`, `voices`, `dsp_registers` (all 128, each with its name, voice and the field-by-field explanation), `aram_map`, `samples`, `sample` (a directory entry decoded block by block, every value's steps), `aram`, `spc_listing` (walked from the program counter and the execution log, routines named, DSP and I/O writes decoded, waits named: `explain::spc::listing`), `spc_pc`, `note_timeline` and `port_events`; the map and samples read the `.spc.mxlog` beside the recording when there is one. `Workbench::sound_upload` (and `_blocking`) gives the traced routine, block lists and commands; `brr_at` decodes a sample in the ROM. `ApuPlayer` (`apu::player::Player` behind a lock, so `Send + Sync`) starts from a recording frame (following its port writes or not), from the traced upload with more lists laid over it, or from one sample, in the ROM or in a frame's audio RAM, alone on voice 0; it renders a batch of interleaved stereo at 32 kHz, keeps the last 2,048 samples of each voice and both sides for the scopes, takes port writes, mute and solo, key on and off and pitch for a keyboard, and answers the same questions as a frame (voices, registers, map, samples, listing) about itself now, with each part's ROM origin when an upload put it there. `envelope_curve` runs an ADSR or GAIN setting through the DSP itself for the Voices view to draw; `make_sound_test_rom` and `make_sound_test_recording` are the fixtures for shell tests. Measured on Super Mario World in a release build: booting the traced driver 2.6 ms, a second of music rendered in 9 ms, the voices 40 µs (12 µs with the tunings cached), the samples, map and a 200-line listing under 0.6 ms each. Tests: five FFI tests (a frame read every way, a player from a frame with its scopes and solo, the ROM's upload booted and answering a command with its ROM origins, a sample keyed on and off in the ROM and in a frame, an envelope curve) and three RomlensKit tests |
| A11 the Voices, Samples and Audio RAM views | done, 26 September 2026: an Audio menu in the toolbar beside Graphics, and View › Audio (`AudioModel`, `RomViewModel.audioTab`; a sound view and a graphics view share the editor, and a text tab closes either). The views read the attached recording at the Graphics views' frame (one frame for both, so the screen and the sound are the same moment), or the ROM: the driver its upload routine sends, booted by Romlens with the other uploads laid over it and run a second, each upload a toggle in the source bar (by default every one whose audio RAM does not overlap one before it, so the samples and the first song bank, not a second bank meant to replace it). A recording with sound opens on it unless the ROM was picked. **Voices**: eight strips, each with its sample and directory entry, the note estimated from the loop, the pitch in words, both volumes centred (a negative one marked as phase-inverted), the envelope its ADSR or GAIN makes drawn from the DSP itself (keyed on, off at 1.5 s) with ENVX now as a line, and the echo, noise, modulation, key-off and end flags; beside them the selected voice's registers and then the globals, each opening to its fields. **Samples**: the directory (start, loop, size, the note at `$1000`, whether the DSP read it while logged), and a sample opened: its waveform with the loop marked, its blocks' headers in a strip, and a block's sixteen values decoded step by step (nibble, shifted, the two before, the prediction, the clamped sum, the 15-bit value kept, the value played), clips and wraps marked, value 0 worked in words; Show in ROM for a sample the upload sent. **Audio RAM**: the 64 KB a byte a pixel coloured by part with the program counter marked, a legend with each kind's total, the parts listed, and a part opened: code as the SPC700 listing (routines named, DSP and I/O writes decoded, waits named, a line opening to its write field by field), anything else as bytes, with the upload blocks that filled it and Show in ROM. On Super Mario World from the ROM alone: the driver at `$0E:8000` with the samples at `$0F:8000` laid over it, the directory's 20 samples, the driver code at `$0500`, the 4 KB echo buffer. Tests: five app tests (the ROM's upload booted and shown with its origins and Show in ROM, a recording's sound at the Graphics frame and the source kept once picked, sound and graphics views sharing the editor, the default uploads, every view laid out in a window from both sources; `ROMLENS_SNAPSHOTS` saves each as a PNG); 114 pass |
| A12 playback, the transport, the port console and the Scope | done, 26 September 2026: `ApuAudio` plays through `AVAudioEngine`: an `AVAudioSourceNode` at the DSP's own 32 kHz (the engine resamples to the device) reads a lock-free ring (`SampleRing`, one writer and one reader on atomics, 0.25 s), and a queue renders the player 100 ms ahead of it every 10 ms. The audio thread never locks, allocates or calls into Rust; its block and the queue's handler are built outside the main actor so neither carries its isolation. The source bar gains Play and Pause (Space), the time heard, and for a recording Follow: its port writes arrive as they did and the frame moves with the sound (so the Frame view follows it too), a frame scrubbed by hand restarts it there, and it stops at the recording's end. The ROM's machine carries on from where it was. Each voice strip has Mute and Solo (the voice still runs, and its scope still shows it). Ports opens the console: a byte to write on each of the four ports as the S-CPU would, the values the game's own code sends on that port offered from the trace, and what the driver reads back. The Scope view draws each voice's wave after its envelope and the mix left and right at 30 frames a second. In Samples, two octaves of keys play the sample alone on voice 0, tuned from its loop's estimate (KON on press, KOFF on release). In the 65816 listing, a store of a known value to `$2140–$2143` gets Play This Command in the inspector: the ROM's driver booted and sent that byte. Tests: six playback tests (the ring, the ROM's driver answering a command with mute and solo, Play This Command, a recording played and stopped at its end, a sample from the keyboard, the real device pulling the ring at zero volume, skipped without a device) and the Scope in the layout test; 120 pass |
| A13 the Timeline, Ports and Echo views | done, 26 September 2026: **Timeline**: a piano roll, a lane a voice, each note from its key-on to its key-off (or the voice's next key-on) placed up its lane by pitch on a log scale and coloured by sample, around the frame (120 to 3,600 frames across) with the frame marked. The recording's notes are read once off the main thread; the ROM's machine logs its own as it plays (`Player::log_notes`: every DSP write the SPC700 makes goes through the same `audio::NoteTracker` the recording's do, so a run and a recording give the same events). A note opens its frame, its pitch and sample and estimated note, the SPC700 instruction whose KON write made it (logged by the player, or for a recording found by `apu::replay::frame_writers`, which runs the frame again from the snapshot before it and keeps the start whose writes agree with Mesen's longest), with Show in Audio RAM selecting that line in the listing, and the S-CPU's last request on each port before it, each linked to the 65816 code the trace found sending that value. On Super Mario World's recording every note sampled leads to the driver's KON write at `$069A`, and the title's notes to port 2 = `$01` at frame 241, which the trace finds set in `$1DFB`. **Ports**: the writes both ways around the frame, the S-CPU's linked to the code that sends them, and the uploads sent by that frame block by block (`sent_uploads`); from the ROM, the port console and every sound command the trace found, each with Send and Show in ROM. **Echo & Effects**: where the echo buffer is and how long its delay, EON, EFB, EVOL and whether echo writes are on, the eight FIR taps as bars and their frequency response from 0 to 16 kHz, and noise and pitch modulation, each register opening to its fields. The Audio menu and View › Audio list all seven views. FFI: `note_source`, `sent_uploads`, `ApuPlayer::log_notes`, `notes`, `note_count`; `NoteEventInfo` carries the instruction when known. Tests: an FFI test (a note to its instruction and command, from a recording and from the ROM), three app tests (a recorded note to its instruction, command and listing line with the ports and uploads; the ROM's notes logged as it plays; the FIR response), and the new views in the layout test; 123 pass |
| A14 N-SPC songs read as notes and commands | done, 26 September 2026: `audio::nspc` reads Nintendo's N-SPC driver in both versions, written from the Super Famicom Development Wiki's two format pages ("Nintendo Music Format (N-SPC)", the standard version, commands `$E0–$FA`; "Super Mario World Music Format", the older one, `$DA–$F2`; docs/05) and checked against Super Mario World's audio RAM. The driver is recognised by the table of its commands' parameter lengths, which it keeps to step over each command (Super Mario World's holds each length plus one at `$0FC2`, all 25 matching); the song table is the longest run of pointers to lists that read as songs (every block's eight tracks in RAM, each track parsing to its end byte), song *n* being the command *n* and entry *n* − 1; what is playing comes from the driver's direct page, the song list pointer at `$40` (the entry after the block playing) and a track pointer a voice at `$30–$3F`. A song is its list (blocks, repeats with their count, a jump back for ever, the end); a track its lengths in ticks (48 a quarter, named where they are a common beat) with the quantize and velocity indices, notes named from `$80` (C0 in the older version's description, C1 in the standard's), tie, rest, percussion, and each command with its parameters and what it does (a call with its target and count). `refresh` finds a known driver again without a search. `romlens apu song (--rec R [--frame N] | --rom R [--with L] [--port P=$xx] [--seconds S]) [--song n] [--limit N]` prints the driver, the song table, what is playing, the song's list and the block's tracks with the byte each voice reads next marked. In the app the Timeline gains a Song pane: the song (the one playing, or any in the table), its list with the block playing marked, and a voice's track (the voice of the note clicked) with its next byte marked and followed while playing. On Super Mario World, from the recording at frame 850 and from the ROM run by Romlens after `2=$01` alike: nine songs at `$1360`, song 1 (the title) at `$14AA` in eight blocks and a jump back to its second for ever, and each voice's place in its track. FFI: `nspc`, `nspc_song`, `nspc_track` on a recording frame and on a player, `make_nspc_test_player`. Tests: four on audio RAM laid out by hand (`fixtures::sound::nspc_aram`: recognition and refresh, a song's list with a repeat and a jump and a jump out of the list refused, a track's every kind of event in both versions, the direct page read as what is playing), an FFI test, an app test, and a golden |
| A15 measure and record | done, 26 September 2026: measured on five games recorded for 30 seconds from power-on by the fork's recorder (the table under *Measured*, below). What the measuring found and fixed: `rec pack` learnt Mesen's SPC700 rate from the first frame, where Super Metroid had hardly run the SPC700 (3,128 halves against 60 million master cycles), cached a ratio far from any real rate and mistimed every S-CPU port write it recorded; it now keeps a rate only once a frame confirms one, using Mesen's usual 32,040 Hz until then. A replay leaves out frames where the SPC700 is in the boot ROM at either end (Romlens's boot program is its own, so a snapshot's program counter there names Nintendo's instructions, not ours), and compares the echo buffer's bytes apart (the DSP writes them from a place in the buffer a snapshot does not hold), as it does ENVX and OUTX; a run from a frame starts again after any stretch in the boot ROM. `apu replay --free` also plays the notes (`replay::note_agreement`): each key-on the recording has, matched to Romlens's by voice, pitch and sample within a frame. The upload trace learnt Super Metroid's form: a routine that takes the list's offset into Y and its bank into the data bank from the pointer and calls the one that waits and sends (`LDA $0000,Y`); a pointer chosen from a table of three-byte pointers by index (`LDA table,X` into it, `table+1,X` into the byte after), read entry by entry; lists read on through the file so a block may run across a LoROM bank, as Super Metroid's loader follows it; and a pointer counted only where it is set by the upload routine, a caller of it or a caller's caller (Super Metroid fills `$00–$02` for its decompressor too). The manual pass has six new steps (docs/15, 45–50). Tests: a fixture of the banked form with a pointer table, a list across a bank and a decoy that is refused, the SPC rate kept only once confirmed, and the golden's note line |

Since, 27 September 2026, from using it: a live session carries the sound
side (`LiveSource` keeps each frame's audio RAM, DSP registers, SPC700 and
sound events, as the recorder already sent them), so File › Start Live
Session and the recorder script are all it takes to see and hear a game;
File › Open Recording… takes the recorder's `.rlstream` itself, starting
in Mesen's script data folder, and packs it into the app's own Recordings
folder before opening it (`pack_recorder_stream`); the Samples keyboard is
laid out as a piano's.

A song bank's own samples, 27 September 2026: laid over the driver, a
bank's samples were classified as the driver's data. Two causes, both in
reading the directory. Samples use shift 13 in quiet blocks (Super
Metroid's entry 4 has four such blocks in 142), and any such block
refused the entry, ending the run at entry 4; now an entry is refused
when more than one block in eight has a shift over 12, or a looping
entry's loop point is not one of its blocks. And a bank writes its
entries after one of the driver's that its own samples overwrite (Super
Metroid's banks write entries 24 on; entry 23 then points into a bank's
sample, looping to no block of it), so the run from entry 0 stops short;
now the entries an upload's block of whole entries writes are read too
(`audio::entries_written`), from the traced uploads when Romlens boots
the ROM's driver and from the uploads a recording sent through the ports
up to the frame (`upload::sent_spans`, the port writes kept per session
so a live one reads only its new frames). A block that merely runs
across the directory, like a driver's samples sent in one piece, says
nothing of it. Super Metroid's 24 banks each read as a directory and
10–19 KB of samples; Zelda's directory has 25 entries, not 3. An
uploaded entry counts only where its sample was uploaded too: Chrono
Trigger sends a table of `$E0FF`s to `$1F80`, inside its directory's
1 KB, and the RAM at `$E0FF` happens to read as a sample.

Square's uploads, 27 September 2026: Final Fantasy III's routine
(`$C5:0000`) and Chrono Trigger's (`$C7:0000`) send no block list. Each
fills its pointer from a table of two-byte pointers by index, the bank a
constant (`LDA $C5:0010,X` into `$10`, `LDA $C5:0011,X` into `$11`,
`LDA #$C5` into `$12`); each pointer is to a length and then the bytes;
a second table at the same index gives ports 2 and 3 each block's audio
RAM address; `CPX #$000C` ends the loop after six blocks; and the entry
is the constant on ports 2 and 3 when port 1 is given 0 (`LDY #$0200`,
`STY $2142`). `upload::paired_tables` reads that form
(`UploadForm::PairedTables`, "the tables at $C5:0010 and $C5:001C"), in
the analysis's fast path too, so the driver, its samples and its
directory are classified in the ROM and Romlens boots the driver from
the ROM alone. Both games' six blocks match what their recordings sent,
byte for byte. The table's earlier "0 / 4" for Chrono Trigger came from
reading the recording before `rec pack` learnt the SPC700's rate (A15),
which split the one upload into four.

## Measured

Five games, each recorded for 30 seconds (1,800 frames) from power-on by
the MesenCE recorder with zeroed RAM, 26 September 2026. *Per frame*: each
frame replayed from the snapshot before it; *run on*: from the first frame
after the boot ROM, never reset; *notes*: the recording's key-ons that
Romlens's machine played too, run on and following the recording's port
writes. Frames in the boot ROM are not compared, and neither are the echo
buffer's bytes and the DSP's own registers (see A15).

| | Super Mario World | Super Metroid | Final Fantasy III | Chrono Trigger | Zelda |
|---|---|---|---|---|---|
| Driver | N-SPC, older | N-SPC | Square's | Square's | N-SPC |
| Upload routine traced | `$00:8079` | `$80:8028` (the banked form) | `$C5:0000` | `$C7:0000` | `$00:8888` |
| Lists traced / uploads the recording saw | 5 / 3, every block matched byte for byte | 25 (the driver and 24 song banks) / 2, all 11 blocks matched | 1 (the driver, Square's paired tables) / 1, all 6 blocks matched | 1 (the same form) / 1, all 6 blocks matched |
| Sound command values traced | 112 | 11 | 49 | 38 | 6 |
| Directory entries read at the end (from entry 0 to the first that is not a sample, those the voices name, and those an upload's block of entries wrote) | 20 | 38 (7 before the fixes of 27 September) | 9 | 12 (4) | 23 (5) |
| N-SPC read | song 1 of 9 playing | song 5 of 6 | not N-SPC | not N-SPC | song 6 of 15 |
| Frames in the boot ROM | 18 | 254 | 21 | 22 | 80 |
| Per frame: match (and match but at the frame's end) | 1,700 (+20) of 1,781 | 1,453 (+21) of 1,545 | 1,529 (+104) of 1,778 | 1,427 (+86) of 1,777 | 1,629 (+21) of 1,719 |
| Run on: match (and at the end) | 1,759 (+21) of 1,780, none differ | 2 of 1,544; every write within 5 cycles of Mesen's | 254 of 1,777; drifts at frame 277 | none; lost at frame 24 | 1,697 (+21) of 1,718, none differ |
| Notes, run on | 545 of 545 | 74 of 74 | 9 of 9 | 0 of 202 | 463 of 463 |

What stays open:
- **The DSP's inside is not in a snapshot.** A replay starts it from rest:
  the envelopes until each voice is keyed again (ENVX matches Mesen's in
  43–69% of voice-frames per frame, 92–100% run on but for Chrono
  Trigger's 42%), and
  the echo buffer's position and length. The DSP takes a new EDL only when
  its echo index wraps, so a game that shortens the buffer (Chrono Trigger
  sets EDL 5 at `$C900` as its driver starts) goes on writing the longer
  one until then, over whatever is past its end: in Mesen that clears
  Chrono Trigger's command table at `$F172`, which Romlens's machine, its
  echo started from rest, leaves as it was. The driver then takes a
  different path at once. Recording Mesen's DSP state (its `spc.dsp.*`
  fields, left out since A4) and starting the DSP from it would close this.
- **Super Metroid run on** keeps every write within 5 cycles of Mesen's but
  rarely exact; its per-frame replay is exact in 95% of frames, so the
  offset comes from the run's start inside an instruction, not from its
  driver.

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
  - T0–2DIV, T0–2OUT.
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
  - KON, KOFF, ENDX (as voice lists);
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
  `SpcDebugger` tells the two apart already (`MemoryAccessFlags::DspAccess`).
- `io/import/spc_log.rs` reads CPU 1 into `model::spc_log::SpcLog`, kept
  per recording (`<recording>.spc.mxlog`), because audio RAM is not the ROM.
  The format and how the map uses it are in docs/17.

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
- Every KON, KOFF, pitch and sample change is an event for the timeline, so
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
  in A6: it can, through `MemoryAccessFlags::DspAccess`, so the fallback was
  not needed.
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
