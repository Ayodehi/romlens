# The SNES, briefly

Register bit layouts, opcodes, addressing modes, DSP and SPC700 registers and common idioms are in `reference`.

## The machine

- **S-CPU (5A22):** a 65C816 plus DMA/HDMA, a multiplier and divider, joypad auto-read, timers and interrupts. Master clock 21.477 MHz (NTSC); an access takes 6 master cycles (3.58 MHz) in fast areas, 8 (2.68 MHz) in slow ones, 12 for the joypad serial ports.
- **PPU:** 64 KB VRAM (tiles, tilemaps), 512 bytes CGRAM (256 colours), 544 bytes OAM (128 sprites), reached only through PPU registers and safely only in blanking or forced blank.
- **WRAM:** 128 KB at `$7E:0000–$7F:FFFF`; its first 8 KB is mirrored at `$0000–$1FFF` in banks `$00–$3F` and `$80–$BF`.
- **APU:** a separate computer: SPC700 at 1.024 MHz, the S-DSP, 64 KB audio RAM. Four ports are its only link to the S-CPU.

## The memory map

In the system banks (`$00–$3F`, `$80–$BF`): `$0000–$1FFF` WRAM, `$2100–$213F` PPU, `$2140–$2143` APU ports, `$2180–$2183` WRAM port (WMDATA/WMADD), `$4016–$4017` joypad serial, `$4200–$421F` S-CPU registers, `$4300–$437F` DMA channels 0–7, `$8000–$FFFF` ROM.

- **LoROM** (`$20`/`$30`): 32 KB of ROM at `$8000–$FFFF` per bank; file offset `(bank & $7F) × $8000 + (addr − $8000)`; header at file `$7FC0`.
- **HiROM** (`$21`/`$31`): 64 KB per bank in `$C0–$FF`, upper halves mirrored in the system banks; header at file `$FFC0`.
- **FastROM:** banks `$80+` run at 3.58 MHz once MEMSEL (`$420D`) bit 0 is set; `$00–$7F` stay slow, so FastROM games jump to a `$80+` bank early.
- **Vectors** at `$FFE0–$FFFF`: native NMI, IRQ, BRK, COP, then the emulation set with RESET at `$FFFC`. The CPU starts at RESET in emulation mode, bank `$00`.

## The 65816

- A, X, Y are 8 or 16 bits; S stack; D direct page; DBR data bank; PBR code bank; P = `N V M X D I Z C`; E emulation.
- M=1 makes A and memory 8-bit, X=1 makes X and Y 8-bit (`REP #$30` both 16, `SEP #$20` A 8). **Immediate operands follow the flags**: `LDA #` is one byte with M=1, two with M=0. A wrong flag guess shows as garbage after the point it went wrong. Routines expect flags on entry and may leave them changed.
- Reset is emulation mode (6502-like); `CLC; XCE` enters native mode, so RESET nearly always starts `SEI; CLC; XCE`.
- `abs` data uses DBR, jumps use PBR; `dp` is D + offset in bank 0; `long` is 24-bit. `(abs,X)` jumps are switch tables.
- `JSR`/`RTS` stay in bank, `JSL`/`RTL` carry the bank, `RTI` returns from interrupts; a mismatch is a classic trap. DBR has no load: `PHK; PLB` or `PEA; PLB; PLB`. `SED` makes ADC/SBC decimal (scores).

## A frame

- NTSC: 262 lines at 60 Hz, 224 visible (239 with overscan); PAL: 312 at 50 Hz. Vertical blank from line 225, about 37 lines on NTSC: the window for VRAM, CGRAM and OAM writes.
- NMITIMEN (`$4200`): bit 7 NMI, bits 4–5 H/V IRQ, bit 0 joypad auto-read. RDNMI/TIMEUP (`$4210/1`) acknowledge; HVBJOY (`$4212`) reports blanking and auto-read.
- Usual shape: the main loop runs logic once, then waits (`WAI` or a flag); the NMI DMAs prepared buffers (OAM shadow, tiles, palettes), sets scroll, reads pads.
- Forced blank (INIDISP `$2100` bit 7) allows VRAM writes any time, for loading; bits 0–3 are brightness.

## The PPU

- BGMODE (`$2105`): mode 0 four 2 bpp layers; 1 two at 4 bpp plus BG3 at 2 bpp (most common); 2 two at 4 bpp with offset-per-tile; 3 8 bpp + 4 bpp; 4 8 bpp + 2 bpp with offset-per-tile; 5–6 hi-res; 7 one rotating/scaling 8 bpp layer.
- Tiles are 8×8 bitplanes: 16 bytes at 2 bpp, 32 at 4 bpp (planes 0–1 for all rows, then 2–3), 64 at 8 bpp; mode 7 is a byte per pixel, interleaved with its map.
- Tilemap entries are 16 bits, `vhopppcc cccccccc`: tile, palette, priority, flips. BG1SC–BG4SC give each map's address and size, BG12NBA/BG34NBA the tile bases; VRAM is words, through VMADD/VMDATA with VMAIN setting the step.
- OAM: 128 × 4 bytes (X, Y, tile, `vhoopppN`) plus a 32-byte high table (X bit 8, size); OBSEL picks sizes and tile base; written by DMA from a WRAM shadow. Games hide a sprite by moving it below the screen, usually Y = `$F0`.
- CGRAM colours are 15-bit BGR; colour 0 is the backdrop and index 0 of each palette transparent. TM/TS choose main and sub screen layers; colour math adds or subtracts them for translucency and fades; windows mask; MOSAIC blocks.

## DMA and HDMA

Channel n's registers are at `$43n0–$43nA` (mode, B-bus register, source, count). Writing MDMAEN (`$420B`) runs channels at once, halting the CPU, about 2.7 MB/s; the usual sequence sets the channel, then VMADD/CGADD/OAMADD, then `$420B`. HDMA (HDMAEN `$420C`) writes registers each line from a table, for gradients, waves and split scrolls.

## Other S-CPU hardware

Multiply: WRMPYA/B (`$4202/3`), product in RDMPYL/H (`$4216/7`) about 8 cycles later. Divide: WRDIVL/H/B (`$4204–6`), quotient RDDIVL/H (`$4214/5`), remainder RDMPYL/H, about 16 cycles later. Joypads: JOY1L–JOY4H (`$4218–$421F`) after auto-read. HTIME/VTIME (`$4207–$420A`) place the timer IRQ.

## Sound

- The SPC700 is 8-bit with Sony syntax, destination first (`MOV A,#$12`). It has no interrupts; drivers poll timers. Its I/O is at `$F0–$FF`; the DSP's 128 registers are reached through `$F2/$F3`, and `$F4–$F7` are the ports.
- At reset a 64-byte boot ROM (IPL) signals `$AA`/`$BB` on ports 0/1; the S-CPU then uploads blocks byte by byte, each echoed on port 0, and the last block gives the entry point. Games upload a driver, then songs and samples, then send commands through the ports. Romlens never includes Nintendo's boot ROM.
- The S-DSP mixes 8 voices at 32 kHz, each with volume, pitch (`$1000` = 32 kHz), sample number and envelope (ADSR or GAIN); KON/KOFF start and stop voices; echo has its own buffer and FIR filter.
- Samples are BRR: 9-byte blocks, a header (shift, filter, loop, end) then 16 four-bit samples. The directory at DIR × `$100` gives each sample's start and loop.
- N-SPC is Nintendo's driver family (Super Mario World, Zelda, Super Metroid).

## Patterns

- **RESET:** `SEI; CLC; XCE`, widths, stack, D and DBR; forced blank; clear registers and WRAM; upload the sound driver; enable NMI; main loop.
- **NMI:** save registers, acknowledge RDNMI, DMA buffers, scroll, pads, set a frame flag, restore, `RTI`.
- **Shadow registers** in WRAM, since PPU registers are write-only.
- **Jump tables** (`JSR (table,X)`, or push an address and `RTS`) for states and objects; their targets are code the analysis may miss.
- **Decompression** reads ROM through a long pointer and writes a WRAM buffer or VRAM.
- **Objects:** WRAM arrays of positions, speeds and states, looped over with X or Y as the slot.

## Romlens's words

- **Region:** a run of the ROM classed as code, data (bytes, words, pointers, tables, text, graphics, tilemaps, palettes, compressed, samples) or unknown, with confidence and evidence.
- **Label:** a name at an address, from the analysis (auto), the student (user), an imported symbol file, or the hardware (built-in).
- **Flag override:** the student's word on M, X, E, DBR or D at an instruction, which the analysis follows.
- **Variable:** a named, typed address in RAM.
- **Cross-reference:** a call, jump, branch, read, write or pointer between addresses, certain or inferred, "observed" when an execution log saw it.
- **The C:** a routine rebuilt as C at lift, clean or full (loops, parameters, typed variables), hardware registers named as in `snes.h`.
- **Recording:** frames captured from an emulator with each frame's PPU memories, registers, DMA and sound state.
- **Execution log:** which instructions ran, and how often, during play.
