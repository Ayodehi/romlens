# SNES technical primer (what shapes the design)

This is the minimum a contributor needs to understand why the disassembler
and address model look the way they do. Sources are in `05-references.md`.

## The bundled ROM at a glance

Measured directly from `SuperMetroid.F8DF.sfc`:

| Field | Value | Meaning |
|---|---|---|
| File size | 3,145,728 bytes (3 MB) | No 512-byte copier header (size is a multiple of 1024) |
| Header location | `0x7FC0` | LoROM header (HiROM slot at `0xFFC0` is all `FF`) |
| Title | `Super Metroid` | 21 bytes, space padded |
| Map mode | `$30` | `$20` = LoROM, `+$10` = FastROM (3.58 MHz access in banks `$80–$FF`) |
| Cartridge type | `$02` | ROM + SRAM (battery) |
| ROM size code | `$0C` | 4 MB declared (codes are powers of two; 3 MB rounds up) |
| RAM size code | `$03` | 8 KB SRAM |
| Region | `$00` | Japan code, but this is the combined JP/US cart |
| Checksum / complement | `$F8DF` / `$0720` | XOR = `$FFFF`, valid |
| Computed checksum | `$F8DF` | Only when the last 1 MB is summed twice (3 MB = 2 MB + 1 MB mirrored) |

Vectors (native / emulation), all in bank `$80` after the boot jump:

| Vector | Native | Emulation |
|---|---|---|
| COP | `$8573` | `$8573` |
| BRK | `$8573` | — |
| ABORT | `$8573` | `$8573` |
| NMI | `$9583` | `$8573` |
| RESET | — | `$841C` |
| IRQ/BRK | `$986A` | `$8573` |

`$8573` is the "should never happen" handler: seven of the ten used vectors
point at it, and so do the two slots the CPU never fetches (native RESET,
emulation BRK), which is why `romlens xrefs … '$80:8573'` lists nine vector
references.

## Address translation

The CPU sees 24-bit addresses `$BB:AAAA`. The cartridge is mapped into that
space differently per mapping mode. The app's `AddressMap` protocol hides
this, but the rules are:

**LoROM** (this ROM): ROM appears in the upper half (`$8000–$FFFF`) of every
bank. 32 KB per bank.

```
fileOffset = (bank & 0x7F) * 0x8000 + (addr - 0x8000)      // addr >= 0x8000
bank $00–$7F  slow mirror of $80–$FF (banks $7E/$7F are WRAM, not ROM)
$80:841C  →  0x00 * 0x8000 + 0x041C  =  0x00041C
```

**HiROM**: ROM fills whole 64 KB banks starting at `$C0`, mirrored at `$40`
and, for the upper halves, at `$00`/`$80`.

```
fileOffset = (bank & 0x3F) * 0x10000 + addr
```

**ExHiROM**: like HiROM but banks `$C0–$FF` map the first 4 MB and
`$40–$7D` the next 4 MB (bit 23 inverted).

Special chips (SA-1, SuperFX, DSP, S-DD1, SPC7110) each change the map and
are out of scope until a ROM that needs them shows up. The header's map mode
and cartridge type bytes tell us which.

Canonical display address: the FastROM bank (`$80+`) for LoROM/HiROM when
the header says FastROM, else the slow bank. Mirrors are listed in the
inspector.

## The 65816: what makes disassembly hard

1. **Variable immediate widths.** `LDA #imm` is opcode `$A9` followed by
   *one* byte when the M flag is 1 (8-bit accumulator) and *two* bytes when
   M is 0. Same for X/Y with the X flag. A disassembler that gets the flag
   wrong desynchronizes and produces garbage from that point on. The flags are
   changed by `REP`/`SEP` (and `XCE`, `PLP`, `RTI`), so the analyzer must
   propagate flag state along every control-flow edge and let the user pin it.
2. **Emulation mode.** The CPU boots in 6502-emulation mode (E=1) with 8-bit
   registers. `CLC; XCE` switches to native. The boot code below does this in
   its first three bytes.
3. **Banks everywhere.** The program bank (PBR), data bank (DBR) and direct
   page (D) registers all affect what an operand means. `LDA $86` reads
   `D + $86`; `LDA $4212` reads `DBR:4212` unless DBR happens to make it
   hardware. Tracking `PHK; PLB` and `TCD` is necessary to resolve targets.
4. **Long jumps into mirrors.** Code frequently `JML`s from bank `$00` to
   bank `$80` for FastROM speed; the two addresses are the same bytes. The
   address model must treat them as one location.
5. **Code and data interleave freely.** Jump tables follow `JMP (abs,X)`,
   inline arguments follow some `JSL`s, and graphics sit between routines.
   Nothing in the byte stream says which is which.

## Worked example: the first 65 bytes of Super Metroid's boot

File offset `0x41C`, SNES `$80:841C` (emulation RESET vector). Decoded by
hand; this is the demo the app should reproduce on day one.

```
$80:841C  78            SEI            ; disable IRQ
$80:841D  18            CLC
$80:841E  FB            XCE            ; E=0: native mode
$80:841F  5C 23 84 80   JML $808423    ; jump to the NEXT byte, but in bank $80 (FastROM)
$80:8423  E2 20         SEP #$20       ; M=1: A is 8-bit
$80:8425  A9 01         LDA #$01       ; 2-byte immediate because M=1
$80:8427  8D 0D 42      STA $420D      ; MEMSEL: enable FastROM
$80:842A  85 86         STA $86        ; direct page variable
$80:842C  C2 30         REP #$30       ; M=0, X=0: 16-bit A, X, Y
$80:842E  A2 FF 1F      LDX #$1FFF
$80:8431  9A            TXS            ; stack at $1FFF
$80:8432  A9 00 00      LDA #$0000     ; 3-byte immediate because M=0
$80:8435  5B            TCD            ; direct page = $0000
$80:8436  4B            PHK
$80:8437  AB            PLB            ; data bank = program bank ($80)
$80:8438  E2 30         SEP #$30       ; back to 8-bit
$80:843A  A2 04         LDX #$04
$80:843C  AD 12 42      LDA $4212      ; HVBJOY
$80:843F  10 FB         BPL $843C      ; wait for V-blank start
$80:8441  AD 12 42      LDA $4212
$80:8444  30 FB         BMI $8441      ; wait for V-blank end
$80:8446  CA            DEX
$80:8447  D0 F3         BNE $843C      ; ... four frames
$80:8449  C2 30         REP #$30
$80:844B  A2 FE 1F      LDX #$1FFE
$80:844E  9E 00 00      STZ $0000,X    ; clear WRAM $0000–$1FFF
$80:8451  CA            DEX
$80:8452  CA            DEX
$80:8453  10 F9         BPL $844E
$80:8455  22 46 91 8B   JSL $8B9146
$80:8459  22 0A 80 80   JSL $80800A
```

Everything the app must get right is already in these bytes: emulation to
native switch, the mirror jump, `SEP`/`REP` changing immediate widths twice,
hardware register targets, a direct-page store, `PHK/PLB` bank setup,
backward branches with negative displacements, and long calls into other
banks. Note the same two-byte instruction `LDA #` appears once as `A9 01`
and once as `A9 00 00`.

## Code vs data discovery strategy

Ordered by trustworthiness:

1. **Execution evidence.** A CDL (code/data log) from Mesen2 or a usage map
   from bsnes-plus records which bytes were fetched as opcodes, read as data,
   or never touched during real play. This is ground truth for what it covers.
2. **Reachability.** Recursive descent from the twelve vectors, following
   every static branch and call. High confidence, but misses anything reached
   through computed jumps and pointer tables.
3. **Structural heuristics.** Pointer tables (runs of values inside the ROM's
   address range with a consistent bank), `JMP (abs,X)` followed by a table,
   routines that end in `RTS`/`RTL`, ASCII runs, 2bpp/4bpp bitplane
   signatures, 15-bit palette signatures, per-window entropy (compressed data
   is near 8 bits per byte; code is around 5–6; tables are lower).
4. **Imported knowledge.** Public symbol lists and disassemblies for known
   games. For Super Metroid, PJBoy's bank logs cover essentially the whole
   ROM and let us score everything above.

Measured on this ROM, per-32 KB bank Shannon entropy already separates
obvious classes: bank `$85` is 1.5 bits per byte (sparse tables), bank `$80`
is 4.6 (code with tables), bank `$96` is 7.0 (compressed graphics).

## Hardware registers worth naming from day one

The disassembler should render well-known hardware addresses symbolically:
`$2100–$213F` PPU, `$2140–$2143` APU ports, `$4200–$421F` CPU/joypad, `$4300–
$437F` DMA channels, `$420D` MEMSEL. A built-in symbol table for these makes
even unlabeled code readable.

Phase 1 ships this table in `model/hardware.rs` (`romlens registers`), with
two additions beyond the list above: the WRAM port `$2180–$2183`
(`WMDATA`, `WMADDL/M/H`) and the joypad serial ports `$4016/$4017`. The
operand stays numeric (`STA $420D`) and the register name is an automatic
comment (`; MEMSEL`), so exports have no define-table dependency.
