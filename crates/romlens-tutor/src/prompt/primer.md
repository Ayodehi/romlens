# The SNES, briefly

## The machine

- **S-CPU (Ricoh 5A22):** a 65C816 core plus the SNES's own hardware: DMA and HDMA, a multiplier and divider, the joypad auto-read, timers and interrupts. The master clock is 21.477 MHz (NTSC). A memory access takes 6 master cycles (3.58 MHz) for fast areas, 8 (2.68 MHz) for slow ones, and 12 for the joypad serial ports.
- **PPU (two chips):** draws the picture from its own memories: 64 KB of VRAM (tiles and tilemaps), 512 bytes of CGRAM (256 colours), 544 bytes of OAM (128 sprites). The CPU reaches them only through PPU registers, and only safely during blanking or forced blank.
- **WRAM:** 128 KB at `$7E:0000–$7F:FFFF`. Its first 8 KB is mirrored at `$0000–$1FFF` in banks `$00–$3F` and `$80–$BF`.
- **APU:** a separate computer. It has an SPC700 CPU at 1.024 MHz, the S-DSP that makes the sound, and 64 KB of audio RAM. Four ports are its only link to the S-CPU.
- **Cartridge:** the ROM, and sometimes save RAM or an extra chip (SuperFX, SA-1, DSP-1…).

## The memory map (CPU addresses are 24-bit: bank:address)

In banks `$00–$3F` and `$80–$BF` (the system banks):

| Address | What |
|---|---|
| `$0000–$1FFF` | WRAM, the first 8 KB |
| `$2100–$213F` | PPU registers |
| `$2140–$2143` | APUIO0–3, the ports to the sound CPU (mirrored up to `$217F`) |
| `$2180–$2183` | WMDATA and WMADD: WRAM through a port, for DMA |
| `$4016–$4017` | the joypads' serial ports |
| `$4200–$421F` | the S-CPU's registers |
| `$4300–$437F` | DMA channels 0–7, 16 bytes each |
| `$8000–$FFFF` | ROM |

**LoROM** (mode `$20`/`$30`) maps 32 KB of ROM into `$8000–$FFFF` of each bank. The file offset is `(bank & $7F) × $8000 + (address − $8000)`. The header sits at file offset `$7FC0` (CPU `$00:FFC0`). **HiROM** (`$21`/`$31`) maps 64 KB into each of banks `$C0–$FF`, with the upper halves mirrored in the system banks. Its header is at file offset `$FFC0`. **FastROM** code runs from banks `$80` and up at 3.58 MHz once MEMSEL (`$420D`) bit 0 is set; the same ROM in banks `$00–$7F` stays slow. So FastROM games jump to a `$80+` bank early.

The **header** gives the title, map mode, ROM and RAM sizes, region and checksum. The **vectors** follow it at `$FFE0–$FFFF`: native NMI, IRQ, BRK and COP, then the emulation-mode set with RESET at `$FFFC`. The CPU starts at RESET in emulation mode, in bank `$00`.

## The 65816

- **Registers:** A (the accumulator), X and Y, each 8 or 16 bits. S is the stack pointer. D is the direct page base. DBR is the data bank, for absolute data addresses. PBR (K) is the bank the code runs in. P holds the flags `N V M X D I Z C`, and E is the emulation flag.
- **Widths.** M=1 makes A and memory accesses 8-bit, and M=0 makes them 16-bit. X does the same for X and Y. `REP #$30` makes both 16-bit and `SEP #$20` makes A 8-bit. **An immediate operand's length depends on the flags**: `LDA #` takes one byte with M=1 and two with M=0. A disassembler must know the flags at each instruction. Romlens tracks them through the code, and a wrong guess shows up as garbage after the point where it went wrong. A routine usually expects particular flags on entry and returns them however it leaves them.
- **Emulation mode** (E=1, at reset) behaves like a 6502: 8-bit registers and the stack in page 1. `CLC; XCE` enters native mode, so nearly every RESET routine begins `SEI; CLC; XCE`.
- **Addressing.**
  - `dp` is a byte at D + dp in bank 0; direct page is fast when D's low byte is 0.
  - `abs` is DBR:abs for data and PBR:abs for jumps.
  - `long` is a full 24-bit address.
  - Indexed forms add X or Y.
  - `(dp)`, `[dp]`, `(dp),Y` and `[dp],Y` go through a pointer in direct page.
  - `d,S` and `(d,S),Y` are stack relative, for arguments and locals.
  - `(abs,X)` jumps through a table, which is how most switch statements look.
- **Calls:** `JSR`/`RTS` stay in the bank. `JSL`/`RTL` push and pull the bank too. `RTI` returns from an interrupt. A routine returning with the wrong one of these is a classic analysis trap.
- **Block moves:** `MVN`/`MVP` copy A+1 bytes from X to Y between the two banks in the operand.
- **DBR** has no load instruction. `PHK; PLB` sets it to the code's bank, and `PEA $xxxx; PLB; PLB` sets it from a constant.
- **Decimal mode** (`SED`) makes ADC and SBC work in BCD. Games use it for scores.

## A frame

- NTSC draws 262 lines at about 60 Hz; PAL draws 312 at 50 Hz. Lines 1–224 (or 239 with overscan) are visible.
- **Vertical blank** starts at line 225. It is the safe window for writing VRAM, CGRAM and OAM, and on NTSC it lasts about 37 lines, roughly 1.3 ms of DMA time.
- **NMITIMEN** (`$4200`):
  - bit 7 enables the NMI at vertical blank;
  - bits 4–5 enable the H/V timer IRQ;
  - bit 0 enables the joypad auto-read.
- **RDNMI** (`$4210`) and **TIMEUP** (`$4211`) are read to acknowledge those interrupts, and **HVBJOY** (`$4212`) reports blanking and whether the auto-read is busy.
- **The usual structure:**
  - The main loop runs the game logic once per frame, then waits (`WAI`, or a loop on a flag).
  - The NMI handler uploads what the logic prepared: a shadow copy of OAM, changed tiles and palettes, scroll values. It does this by DMA from buffers in WRAM, then reads the joypads.
- **Forced blank** (INIDISP `$2100` bit 7) turns the screen off so that VRAM can be written at any time. Games use it while loading a level. INIDISP bits 0–3 set the brightness.

## The PPU

- **Backgrounds.** BGMODE (`$2105`) picks one of eight modes, which fix how many layers there are and their colour depth:

  | Mode | Layers |
  |---|---|
  | 0 | four at 2 bpp |
  | 1 | BG1 and BG2 at 4 bpp, BG3 at 2 bpp (the most common mode; BGMODE bit 3 can raise BG3's priority) |
  | 2 | two at 4 bpp, with offset-per-tile |
  | 3 | BG1 at 8 bpp, BG2 at 4 bpp |
  | 4 | BG1 at 8 bpp, BG2 at 2 bpp, with offset-per-tile |
  | 5 and 6 | high resolution |
  | 7 | one 8 bpp layer with rotation and scaling (the M7 matrix at `$211B–$2120`) |

  BGMODE bits 4–7 pick 16×16 tiles per layer.
- **Tiles** are 8×8 pixels in bitplanes, row by row. A 2 bpp tile is 16 bytes: each row is two bytes, plane 0 then plane 1. At 4 bpp (32 bytes) planes 0–1 come for all eight rows, then planes 2–3. At 8 bpp a tile is 64 bytes. Mode 7 is different: its tiles are one byte per pixel, interleaved with the tilemap.
- **Tilemaps** are 32×32 entries of 16 bits, `vhopppcc cccccccc`:
  - bits 0–9 are the tile number;
  - bits 10–12 are the palette;
  - bit 13 is the priority;
  - bits 14–15 flip the tile horizontally and vertically.

  BG1SC–BG4SC (`$2107–$210A`) give each layer's tilemap address in VRAM and its size (32 or 64 entries wide and high). BG12NBA and BG34NBA (`$210B`/`$210C`) give where each layer's tiles start.
- **VRAM** holds 16-bit words.
  - VMADD (`$2116/7`) sets the word address, and VMDATA (`$2118/9`) writes the low and high bytes.
  - VMAIN (`$2115`) sets the step (1, 32 or 128 words) and whether the address moves on after the low or the high byte.
- **Scrolling:** BGnHOFS and BGnVOFS (`$210D–$2114`) are each written twice, low byte then high.
- **Sprites (OAM).**
  - The table has 128 entries of 4 bytes: X, Y, tile, and the attribute byte `vhoopppN`. N is the tile number's bit 8, ppp is the palette (sprites use palettes 8–15), oo is the priority, and h and v flip the sprite.
  - A 32-byte high table follows, with 2 bits per sprite: X bit 8, and which of the two sizes it uses.
  - OBSEL (`$2101`) chooses the pair of sizes and where sprite tiles are in VRAM.
  - OAM is written through OAMADD (`$2102/3`) and OAMDATA (`$2104`), almost always by DMA from a shadow copy in WRAM.
- **Colour:** CGRAM holds 256 colours of 15 bits, `0bbbbbgg gggrrrrr`. CGADD (`$2121`) sets the index and CGDATA (`$2122`) is written low byte then high. Colour 0 is the backdrop, and colour 0 of each palette is transparent.
- **Layers on screen:**
  - TM (`$212C`) and TS (`$212D`) choose which layers go on the main and sub screens.
  - Colour math (CGWSEL and CGADSUB, `$2130/1`) adds or subtracts the sub screen or a fixed colour (COLDATA `$2132`). This is how translucency, shadows and fades are done.
  - Windows (`$2123–$212B`) mask layers inside or outside shapes.
  - MOSAIC (`$2106`) makes pixels blocky.

## DMA and HDMA

- **The registers.** Each channel n has its registers at `$43n0–$43nA`:
  - DMAPn (`$43n0`) holds the direction (bit 7), whether the address stays fixed or steps, and the transfer pattern;
  - BBADn (`$43n1`) is the PPU-side register (`$21xx`);
  - A1TnL/H/B (`$43n2–4`) is the source address;
  - DASnL/H (`$43n5/6`) is the byte count, and for HDMA the indirect address.
- **The patterns** (bits 0–2 of DMAPn) are the order the B-bus register is written in:

  | Pattern | Order |
  |---|---|
  | 0 | one register |
  | 1 | two in turn (p, p+1), the one for VRAM at `$2118` |
  | 2 | one register twice |
  | 3 | p, p, p+1, p+1 |
  | 4 | four registers |
  | 5 | p, p+1, p, p+1 |
- **Starting it.** Writing a channel bit to MDMAEN (`$420B`) runs that channel at once, and the CPU stops until it is done, at about 2.7 MB a second. The usual picture:
  - write the channel's registers;
  - set VMADD, CGADD or OAMADD;
  - write `$420B`.
- **HDMA** (HDMAEN `$420C`) writes a register at the start of each line from a table in memory. The table is made of line counts, each followed by values, either directly or through pointers. It is how games do gradients, waves, split scrolling and shaped windows.

## Other S-CPU hardware

- **Multiplying:** write WRMPYA (`$4202`) and then WRMPYB (`$4203`). About 8 cycles later RDMPYL/H (`$4216/7`) holds the 16-bit product.
- **Dividing:** write WRDIVL/H (`$4204/5`) and then WRDIVB (`$4206`). About 16 cycles later RDDIVL/H (`$4214/5`) holds the quotient and RDMPYL/H the remainder.
- **Joypads:** with the auto-read on, JOY1L–JOY4H (`$4218–$421F`) hold the buttons once HVBJOY bit 0 clears. The standard pad's bits, high to low, are B Y Select Start Up Down Left Right, then A X L R.
- **Timer IRQ:** HTIME and VTIME (`$4207–$420A`) set the dot and line where the H/V timer IRQ fires.

## Sound: the APU

- **The SPC700** is an 8-bit CPU.
  - Registers: A, X, Y, SP and PSW (flags `N V P B H I Z C`, where P puts the direct page at `$0100` instead of `$0000`). Y and A together form the 16-bit YA.
  - Its assembly uses Sony's syntax, destination first: `MOV A,#$12` and `MOV $F2,A`.
  - It has no interrupts. Drivers keep time by polling a timer's counter.
- **Its I/O is at `$F0–$FF`:**
  - CONTROL (`$F1`) starts the timers, clears the ports, and maps the boot ROM.
  - DSPADDR and DSPDATA (`$F2/$F3`) reach the DSP's 128 registers.
  - CPUIO0–3 (`$F4–$F7`) are the ports, with the S-CPU's `$2140–$2143` on the other side.
  - T0–T2TARGET (`$FA–$FC`) set the timers' dividers. Timers 0 and 1 tick at 8 kHz and timer 2 at 64 kHz.
  - T0–T2OUT (`$FD–$FF`) count the ticks and clear when read.
- **Booting and uploading.** At reset a 64-byte boot ROM (the IPL) at `$FFC0` writes `$AA`/`$BB` to ports 0/1, then waits. The S-CPU uploads blocks, each an audio RAM address and its bytes, one byte at a time: each byte goes with its index on port 0, and the S-CPU waits for the echo. The last block gives the entry address, and the driver runs from there. Games upload their sound driver this way, then songs and samples. After that they send commands through the ports, such as "play song 3" or "sound effect 12". Romlens never includes Nintendo's boot ROM; it boots drivers with its own.
- **The S-DSP** mixes 8 voices at 32 kHz.
  - Each voice n has ten registers at `$n0–$n9`: VOL L/R, PITCH L/H, SRCN (the sample number), ADSR1/2, GAIN, and the read-only ENVX and OUTX.
  - The global registers:
    - MVOL and EVOL L/R (`$0C/$1C`, `$2C/$3C`) set the main and echo volumes;
    - KON (`$4C`) and KOFF (`$5C`) key voices on and off;
    - FLG (`$6C`) holds reset, mute, echo writes off, and the noise clock;
    - ENDX (`$7C`) shows which voices reached a sample's end;
    - EFB (`$0D`) is the echo feedback;
    - PMON, NON and EON (`$2D/$3D/$4D`) turn on pitch modulation, noise and echo per voice;
    - DIR (`$5D`) is the sample directory's page;
    - ESA and EDL (`$6D/$7D`) are the echo buffer's page and length (EDL × 2 KB, 16 ms each);
    - FIR0–7 (`$0F–$7F`) are the echo filter's eight taps.
  - Pitch `$1000` plays a sample at 32 kHz; `$0800` plays it an octave lower.
- **BRR samples** come in 9-byte blocks: a header byte (bits 7–4 the shift, 3–2 the filter, bit 1 loop, bit 0 end) then 16 four-bit samples. Filters 1–3 predict each sample from the two before it.
- **The sample directory** at DIR × `$100` has a 4-byte entry per sample: where it starts and where it loops.
- **Envelopes:** ADSR gives attack, decay to a sustain level, then sustain release; GAIN gives direct or linear or exponential ramps instead.
- **N-SPC** is Nintendo's sound driver family, used in Super Mario World, Zelda and Super Metroid. Songs are track data with notes, durations and commands.

## Patterns worth recognising

- **RESET:**
  - `SEI; CLC; XCE`;
  - then set the widths, the stack (`LDX #$1FFF; TXS` or similar), D and DBR;
  - force blank, clear the PPU and CPU registers, clear WRAM;
  - upload the sound driver;
  - turn on the NMI and fall into the main loop.
- **The NMI handler:** push the registers, acknowledge RDNMI, DMA the buffers, set the scroll, read the pads, set a "frame done" flag, pull the registers, `RTI`.
- **Shadow registers:** because PPU registers are write-only, games keep copies in WRAM and write those copies to the PPU during vertical blank.
- **Pointer and jump tables** (`JSR (table,X)` or pushing an address then `RTS`) for game states, object behaviours and menus. The table's entries are code addresses that the analysis may miss.
- **Decompression:** graphics and tilemaps are often compressed in ROM and unpacked to WRAM or straight to VRAM. A routine reading ROM through a long pointer and writing a buffer is usually one.
- **Objects:** arrays in WRAM of positions, speeds and states, one slot per object, processed in a loop with X or Y as the slot index.

## Romlens's words

- **Region:** a run of the ROM with one kind, code or data (bytes, words, pointers, tables, text, graphics, tilemaps, palettes, compressed, samples) or unknown. It comes with a confidence and its evidence.
- **Label:** a name at an address, from the analysis (auto), the student (user), an imported symbol file, or the hardware (built-in).
- **Flag override:** the student's word on M, X, E, DBR or D at an instruction, which the analysis then follows.
- **Variable:** a named, typed address in RAM.
- **Cross-reference:** a call, jump, branch, read, write or pointer from one address to another, certain or inferred, and "observed" when an execution log saw it.
- **The C:** Romlens's rebuilding of a routine as C, at three levels: lift (close to the instructions), clean, and full (loops, parameters, typed variables). Hardware registers appear by name, as in `snes.h`.
- **Recording:** frames captured from an emulator, with the PPU memories, registers, DMA and sound state of each. The frame numbers are the recording's.
- **Execution log:** which instructions ran, and how often, during play.
