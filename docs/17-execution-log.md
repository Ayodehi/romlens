# Execution logs (.mxlog)

Added 23 September 2026. An execution log records what the SNES CPU did as
relationships: which instruction read or wrote which address, where each
branch, jump and call went, and what each DMA transfer moved where. A
code/data log (`.cdl`) keeps only flags per ROM byte, so it can say a byte was
read but not by what. A trace log has the relationships but writes every
step as text, which reaches gigabytes in minutes.

An execution log keeps each distinct relationship once, with a count. It grows
with how much of the game has been seen, not with how long it ran: the first
30 seconds of Super Metroid's attract sequence come to 770 KB, and 67 seconds
to 2.6 MB.

## Where logs come from

The logger is in the MesenCE fork (github.com/Ayodehi/MesenCE,
`Core/SNES/Debugger/SnesExecutionLogger.cpp`), exposed to Lua as
`emu.startExecutionLog()`, `emu.stopExecutionLog()`, `emu.clearExecutionLog()`,
`emu.getExecutionLog()` and `emu.takeExecutionLogDelta()`. Stock Mesen
releases do not have it.

`takeExecutionLogDelta()` returns what was recorded since the previous call,
in the same format: entries that are new or whose count or width states
changed, each count being the increase. Merging every delta in order gives the
whole log, and `getExecutionLog()` is unaffected. A live session uses it to
send the log as the game plays (`13-recording-format.md`, "Live sessions").

The recorder script (Help › Save Mesen Recorder Script…) feature-detects it.
On the fork it starts the log with the recording, writes `<name>.mxlog`
beside the `.rlstream` every minute and at the end, so a crash loses little,
and writes each copy to a `.part` file and renames it. On stock Mesen it
records the stream alone, as before.

Import a log with File › Import › Execution Trace… or
`romlens import trace <project> <file.mxlog>`. Imports merge: instructions
and transfers are unioned with their counts summed, and access and DMA runs
that overlap or touch are joined. A log from another ROM is refused, since the
header names the ROM by CRC32 and size.

## Format, version 1

All values are little-endian. This restates the fork's definition, which is
authoritative.

Header, 20 bytes:

| Offset | Type | Field |
|---|---|---|
| 0 | char[4] | `MXLG` |
| 4 | u16 | version, 1 |
| 6 | u8 | CPU: 0 for the SNES main CPU, 1 for the SPC700 (below) |
| 7 | u8 | reserved |
| 8 | u32 | CRC32 of the PRG ROM (the value the CDL file uses) |
| 12 | u32 | PRG ROM size |
| 16 | u32 | number of sections |

Each section is a 4-byte tag, a u32 record size and a u32 record count, then
the records. A reader skips a section it does not know by its record size,
and reads only the fields it knows from longer records. CPU addresses are
24-bit. `abs` is the offset in the memory `kind` names (0 unmapped, 1 PRG
ROM, 2 WRAM, 3 SRAM, 4 register, 5 other), or -1 when unmapped. For PRG ROM
it is a Romlens file offset. Counts saturate at 2³²−1.

| Tag | Size | Record |
|---|---|---|
| `INST` | 16 | u32 pc, i32 abs, u8 kind, u8 states, u16 reserved, u32 count. `states` has bit M + 2·X + 4·E set for each state the instruction ran in (M and X are 1 for 8-bit, E is 1 in emulation mode) |
| `ACCS` | 24 | u32 pc, u32 address, i32 abs, u32 length, u8 access (0 read, 1 write), u8 kind, u16 reserved, u32 count: a run of consecutive addresses one instruction touched |
| `FLOW` | 16 | u32 from, u32 to, u8 flow, u8[3] reserved, u32 count. Flow: 0 branch taken, 1 branch not taken, 2 jump, 3 indirect jump, 4 call, 5 indirect call, 6 return, 7 RTI, 8 NMI, 9 IRQ, 10 BRK/COP. For NMI/IRQ, `from` is the instruction that was about to run |
| `DMA ` | 24 | u32 pc, u32 address, i32 abs, u32 length, u8 B-bus address, u8 flags, u8 kind, u8 channel, u32 count: a run of A-bus addresses one channel moved. The B-bus address is the channel's `$43x1`, so `$18` for VRAM whether a byte went to `$2118` or `$2119`. Flags: bit 0 B-bus to A-bus, bit 1 HDMA, bits 2–4 the transfer mode. `pc` is the instruction that wrote `$420B` for a GP-DMA, 0 for HDMA |

HDMA table reads that transfer nothing (line counts, indirect addresses) are
not recorded. DMA reads and writes are paired per channel, so HDMA that
interrupts a GP-DMA stays separate. An SA-1's CPU is not logged.

### The SPC700's log (CPU 1)

Added 26 September 2026 (docs/23, A6). The fork logs the sound CPU too,
with `emu.startExecutionLog(emu.cpuType.spc)`; the five log functions take
an optional CPU type, the SNES CPU by default. The format is the same with
these differences:
- Addresses are the SPC700's 16-bit ones. `kind` is 6 for audio RAM and 7
  for the boot ROM at `$FFC0`; `abs` is the offset in that memory.
- `ACCS` has two more access kinds: 2, the DSP read the bytes by itself
  (the sample directory, BRR sample data, the echo buffer), and 3, the DSP
  wrote them (the echo buffer). Their `pc` is 0.
- `FLOW` adds `CBNE`, `DBNZ`, `BBS` and `BBC` to the branches, `PCALL` and
  `TCALL` to the calls (`TCALL` as an indirect call, through its vector),
  and `RETI` as RTI. There is no `DMA ` section.
- `states` is always 0.

Audio RAM is rewritten as the game runs (a new song, new samples), so this
log belongs with the recording it was made beside, not with the ROM, and is
not imported into a project: the main CPU's importer refuses it and says
where it goes. The recorder writes it beside the stream as
`<name>.spc.mxlog`; `romlens rec pack` checks it against the ROM and puts
it beside the recording; `romlens apu map` and `apu samples` read it from
there (or `--log FILE`).

In the map, what the log saw run is code, what the driver's code read or
wrote is the driver's data, and what the DSP read is sample data; each
sample says whether it was played. Two things are left out on purpose:
- The boot ROM's upload loop reads and writes every byte it uploads, code
  and samples alike, so its accesses say nothing of what the bytes are.
- Below `$0200` the DSP reads only through a directory entry not yet set
  up (a sample at `$0000`).

The walk from the logged code still finds the code of branches the log
never saw taken, but it can wander into data from there, so it only fills
bytes nothing else claims. Short gaps (16 bytes or fewer) between the
driver's data are taken as one table read in part.

## What Romlens does with a log

Romlens stores the merged log in the project package as
`traces/execution.mxlog`, in the same format.

- **Coverage.** The log becomes coverage like a CDL's
  (`ExecLog::to_coverage`): each instruction start with its widths, the bytes
  it spans, ROM read and written as data (DMA sources included), and
  routine entries. Unlike a CDL, which marks an instruction start only at
  jump targets and subroutine entries, the log gives every start, so the
  descent decodes every instruction that ran with the widths it ran in. Where
  an instruction ran in more than one width state, the first (16-bit before
  8-bit) is kept, since coverage holds one width per opcode.
- **References the game made.** Every call, jump and taken branch between
  ROM instructions becomes a cross-reference, indirect ones included, and so
  does every instruction's first read or write of each run of ROM, and each
  GP-DMA's ROM source. A reference the disassembler also found keeps its
  record, marked as seen. The app shows "seen" in Find References and the
  inspector, and `romlens xrefs` prints `(seen)`. References to RAM and
  registers are left out for now: nothing in the ROM view can select them.
- **Data typed by where it went.** ROM a DMA channel sent to CGRAM is a
  palette (95%). ROM sent to VRAM is graphics (80%); VRAM holds tiles and
  tilemaps alike, so the evidence says the 4bpp depth was assumed. ROM sent
  to OAM is a struct (a sprite table). Other destinations make the bytes data
  without a type. This outranks the sweep, operand hints and heuristics, but
  not code, resolved tables, the header or the user.
- **Dispatch tables read.** The ROM a `JMP (abs,X)` or `JSR (abs,X)` was seen
  to read its target from is painted as that dispatcher's table.

On Super Metroid, a 67-second log took cross-references from 14,722 to
59,653, and code seen by the descent includes routines no static walk
reached. For example, `$8B:93D2` dispatches to four routines through a table,
and had no outgoing references before.

## Limits and next steps

- Super Metroid decompresses most graphics into `$7E`/`$7F` and DMAs them from
  there, so DMA typing reaches little of its ROM. The log already holds the
  chain: the decompressor's reads of ROM, its writes to the RAM buffer, and
  the DMA from that buffer. Following it would type the compressed source as
  compressed graphics. That is the next use to build.
- The stack writes made on entry to an interrupt are attributed to the
  instruction before it.
- Code copied to RAM is logged with its RAM address. Mapping it back to the
  ROM it was copied from also needs the copy chain above.
