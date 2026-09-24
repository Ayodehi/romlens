# Pseudo-C for a routine (Phase 3)

## Progress

Written 23 September 2026 and kept as written; this section is the only part
that tracks against it.

| Task | State |
|---|---|
| T0 this document, the roadmap pointer, the checklist rows | done |
| T1 functions and control-flow graphs | done: `decompile::function` and `decompile::cfg`. On the development ROM (static analysis only) all 799 entries decompose, 28,500 instructions, 386 loops, 2 routines irreducible, in 5 ms. The snapshot now keeps `inline_args` |
| T2 lifting, `snes.h`, the `lift` level and `romlens decompile` | done: every opcode lifts in three width states and compiles clean under `-Wall -Wextra` with clang and gcc; the routines fixture (`fixtures::routines_lorom`, `testrom --fixture routines`) is compiled over an array-backed `snes.h` and run, and gives the 65816's results. On the development ROM all 799 routines are valid C at the `lift` level, 28,500 instructions in 26 ms, with 2,796 gotos and 55 asm comments |
| T3 data flow (the `clean` level) | done: stack slots, liveness with the accumulator's bytes apart, dead code, single-use propagation, simplification. On the development ROM the 799 routines go from 55,716 statements at `lift` to 16,739 at `clean` (−70%), all valid C. A differential test (`tests/decompile_diff.rs`) runs every routine at every level from the same pseudo-random states and compares what each leaves: 6,278 runs, no differences; it found four real bugs on the way (a value carried out of its register's width, reads a call makes through the convention, a table call's index, and a `PEA` return address taken for a slot). Not done: joining multi-byte `ADC` chains into one wider add; each byte's add stays exact but separate |
| T4 structuring (the `full` level) | done: `do … while` for a loop whose one latch branches back and is its only way out, `while` for a header that only tests, `for (;;)` otherwise, `if`/`else` (and `else if`) joined at the immediate post-dominator, `switch` from jump tables, `break`, `continue`, and a goto where a block is reached a second time. On the development ROM the gotos go from 2,796 at `clean` to 452 at `full`, all valid C; the differential test finds no difference in 3,139 runs at each level, and catches a deliberately swapped branch |
| T5 signatures | done, as summaries rather than typed parameters: `decompile::signature` works out, over the whole program, what each routine reads, writes, returns (what its callers read after it) and preserves (saves on entry and restores on every return), counting a table call's callers and a table slot's dispatcher as known. They replace the every-register assumption for calls, returns and tail calls, and each routine and declaration carries them as a comment (`/* Reads A and X; returns A; preserves Y. */`). On the development ROM 4 of 799 routines stay open (the vectors), gotos 439, all valid C, about a second for the whole ROM. The differential test's stubs now read and write only what each summary says, and two new checks keep the summaries honest: perturbing what a routine is said not to read changes nothing it returns, and what it is said not to write is unchanged. 9,408 runs, no differences; on the way they found five bugs (a preserved register that was also an input, a slot reused for a different value, a register read after its restore, a tail call's pass-through registers, and table stubs). Not done: `u16 f(u16 a)`-style parameters; registers stay the globals of `snes.h` |
| T6 FFI | done: `Workbench::decompile` (async, on its own thread, with its own cancel flag so dropping it never cancels an analysis) and `decompile_blocking`, `function_containing`, `snes_header()` and `make_routines_test_rom()`. Every routine's summary is built on the first decompile after an analysis and reused until the next. Tokens come back in UTF-16 units for the shell's string APIs. `API_VERSION` 0.5.0 (the bump planned after 2C had not been made, so it moves once). Tested from Rust and from RomlensKit |
| T7 macOS C tab | done: the C tab (View › C, ⌥⌘8) splits the disassembly and the routine at the selection as C; the header names the routine, carries the level picker and Export C… (the `.c` and `snes.h` beside it). Selecting an instruction highlights the C lines it made, including values carried into a later line, and clicking a C line selects its instructions; double-clicking a routine's or a label's name goes there. Decompile Routine in the context menu. Renames and variables show at once. Also View › Focus on Code (⌥⌘F, and a toolbar button), which hides the navigator, inspector and overview strip together and puts them back as they were. App tests: `DecompileTabTests` |
| T8 measure and record | done, 23 September 2026: see Measurements below. Running Super Mario World with its project through the differential test found four more problems, one in the output and three in the harness, all fixed (below) |

## Context

The roadmap's Phase 3 ends with "a first pseudo-C rendering of a chosen
routine" (`01-vision-and-roadmap.md`). This is that rendering: pick a routine,
and Romlens shows it as C beside its disassembly, the way Ghidra and IDA do for
other processors. It is a reading aid. Nothing here tries to decompile a whole
game into a source tree.

Super Mario World's reset handler shows why it is worth having. Its first
screen of assembly clears the hardware registers, sets up the stack, then runs
a loop that stores `$008D` and `Y` through `$7F:8002,X`. Written as C, the loop
reads as what it is:

```c
    *(u16 *)0x7F8000 = 0xF0A9;
    u16 x = 0x017D, y = 0x03FD;
    do {
        *(u16 *)(0x7F8002 + x) = 0x008D;
        *(u16 *)(0x7F8003 + x) = y;
        y -= 4;
        x -= 3;
    } while ((s16)x >= 0);
    *(u8 *)0x7F8182 = 0x6B;
```

`A9 F0` is `LDA #$F0`, `8D lo hi` is `STA abs`, and `6B` is `RTL`: the game
writes 128 unrolled stores into RAM, a routine that hides every sprite by
setting each OAM entry's Y to `$F0`. The assembly hides that; the C makes it a
question a reader can ask.

## Scope decisions (agreed 23 September 2026)

1. **The output is valid C.** Every result is a translation unit that passes
   `cc -std=c11 -fsyntax-only` against the `snes.h` Romlens generates. It is not
   expected to compile back into the original bytes, and it is not expected to
   run.
2. **What cannot be translated stays as a comment.** A stack trick, a computed
   jump nothing resolved, a direct-page access with D unknown and no better
   form: the instruction is kept as `/* asm: PLA */` with a note saying what
   could not be recovered, and the C around it stays valid. Nothing is guessed
   silently.
3. **One routine at a time, on demand.** No whole-program pass; a routine is
   decompiled when it is asked for, and the result is cached until the analysis
   or the project changes.
4. **The macOS view is a split tab.** A fourth editor tab, **C**, shows the
   disassembly and the C side by side for the routine at the cursor. Selecting
   a line on either side highlights its counterpart on the other.
5. **Every stage stays visible.** Three levels, `lift`, `clean` and `full`,
   each valid C, so a wrong result can be traced to the stage that made it.

## What already exists

- **Decoded instructions with state.** `AnalysisSnapshot::decode_at`
  (`analysis/snapshot.rs`) rebuilds each `Instruction` with its M, X and E, DBR
  and D where known, and the assumption bits saying which of those were
  assumed. `Instruction.target` gives the unindexed effective address.
- **Control transfers.** `Mnemonic::is_branch`, `is_call`, `is_jump`,
  `is_block_end`; resolved jump tables (`snapshot.jump_tables`), inline
  arguments after calls (`analysis/inline.rs`), and cross-references in both
  directions (`xrefs_to`, `xrefs_from`), including those an execution log saw.
- **Names.** Labels and variables through `model/symbols.rs`
  (`Symbols::name_for`, which also names addresses inside a variable as
  `Name+N`), variable types from `Project.variables`, hardware register names
  from `model/hardware.rs`, and `Project::canonical` to fold mirrors.
- **Recorded widths.** `ExecInsn::mixed_widths` in `model/exec_log.rs` says
  when an instruction ran in more than one width state.

What does not exist: any notion of a function or a basic block, and any table
of what each opcode reads and writes. The decoder tracks M, X, E, C, DBR and D;
N, Z and V are not tracked at all.

## Pipeline

A new module, `crates/romlens-core/src/decompile/`, with one entry point:

```rust
pub fn decompile(
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    at: SnesAddress,
    opts: &DecompileOptions,
) -> Result<Decompiled, DecompileError>
```

`Decompiled` holds the routine's name and entry, the C text, token ranges (for
colouring and for links to labels and variables), a line map from each C line
to the instructions it came from, warnings, and counts (instructions, gotos,
asm comments) for measurement.

The stages, in order:

### 1. Functions — `decompile/function.rs`

From an entry, follow the snapshot's instructions. Conditional branches,
`BRA`, `BRL` and in-bank `JMP abs` stay inside the function. `JSR` and `JSL`
become call sites, skipping any inline arguments. `RTS`, `RTL` and `RTI` exit.
A `JMP` or `JML` to another entry (a call target, a vector, a user label) is a
tail call. `JMP (abs,X)` with a resolved table becomes a switch; unresolved, an
asm comment. `function_containing(offset)` finds the routine at the cursor by
trying entry candidates backwards from it.

### 2. Control-flow graph — `decompile/cfg.rs`

Basic blocks, predecessors, dominators and post-dominators, back edges and
natural loops.

### 3. Lifting — `decompile/ir.rs`, `decompile/lift.rs`

Each instruction becomes a few IR statements that say everything it does:

- **State:** A (as `a` and the hidden high byte `b` when M is 8-bit), X, Y, S,
  D, DBR, and the flags N, Z, C and V as separate values.
- **Memory:** `Mem { addr, width }` with a width of 1, 2 or 3 bytes.
- **Operations:** add and subtract with carry, and, or, xor, shifts and rotates
  through carry, compare, and decimal add and subtract.

`lift.rs` is one match over all 256 opcodes, and a test checks that each one
lifts. Addresses are resolved as far as the state allows: with DBR known, an
absolute operand becomes a 24-bit address; with D known, a direct-page operand
does too. Otherwise the address stays symbolic (`MEM8(DBR, 0x1234)`,
`DP8(0x94)`) with a warning. Indexed modes add X or Y; indirect modes load the
pointer from its slot first. Every resolved address goes through
`Project::canonical`.

This table is also the "what does this opcode read and write" information the
later stages need.

### 4. Data flow — `decompile/dataflow.rs`

- **Liveness and dead code.** Liveness over registers and each flag, then
  statements whose results are never read are dropped. Most N, Z and V
  updates go here.
- **Expression propagation.** A value used once is substituted into its use,
  so `LDA #$80` / `STA $2100` is `INIDISP = 0x80`.
- **Conditions from the flag's producer.** A branch tests the expression that
  last set its flag: `CMP` then `BCC` is `<` (unsigned), a load then `BEQ` is
  `== 0`, `DEX` then `BPL` is `(s16)x >= 0`, `BIT` is a mask test.
- **Arithmetic chains.** `CLC; ADC` is `+`; an `ADC` pair carrying across bytes
  is one 16- or 24-bit add.
- **Width.** With M 8-bit, `a` is a `u8` where the hidden high byte is dead,
  and an explicit merge, `a = (a & 0xFF00) | …`, where it is not.
- **Stack.** Balanced `PHA`/`PLA` (and X, Y) become saved temporaries;
  `PHP`/`PLP` restores widths; `PHB`/`PLB` and `PHK`/`PLB` save and restore
  DBR. Anything unbalanced is an asm comment.

### 5. Structuring — `decompile/structure.rs`

Structural analysis over the graph: `if`/`else`, `while`, `do … while`,
`break` and `continue`, and `switch` from jump tables. Where the graph does not
reduce, `goto L_xxxx;` to a label. Written iteratively: the CLI runs on an
8 MB stack, and deep recursion over a large routine should not depend on it.

### 6. Signatures — `decompile/signature.rs`

A summary per function: which registers and flags it reads before writing (its
parameters) and which it writes (its possible results). A result is kept only
when some caller reads it after the call. Summaries are memoised, with a depth
cap and a cycle cut. The output reads `u16 SUB_0080E8(u16 a, u8 x)` and
`a = SUB_0080E8(a, x);`. Where the summary is not sure, the function is
`void f(void)` over the global `A`, `X` and `Y`.

### 7. Printing — `decompile/emit.rs`, `decompile/header.rs`

C11 using `u8`, `u16`, `u32` and `s16`. Names are chosen in this order: a
variable, typed from its `VarType` (arrays for a count above one, `u32` for a
long), a hardware register, a label, then a plain `*(u8 *)0x7F8000`. Locals are
`a`, `x`, `y` and temporaries `tN`. The output is deterministic, so goldens are
stable.

Each result starts with `#include "snes.h"` and `extern` declarations for every
function, variable and data label it uses. `header.rs` generates `snes.h`:

- the typedefs;
- every register from `all_hardware_registers()`, as
  `#define INIDISP (*(volatile u8 *)0x2100)`;
- the CPU state as `extern` globals (`A`, `X`, `Y`, `D`, `DBR`, the flags);
- helpers: `SEI()`, `CLI()`, `native_mode()`, `wai()`, `stp()`,
  `bcd_add16()`, `mvn()`, `mvp()`;
- the memory macros `MEM8`, `MEM16`, `MEM24`, `DP8` and `FAR8`.

### Levels and options

`DecompileOptions.level`:

- `lift`: the raw IR as C over the global CPU state, one block per label, gotos
  between them.
- `clean`: after data flow.
- `full` (the default): after structuring and signatures.

`assume_dp` sets D for a routine whose D the analysis does not know. Where an
execution log saw an instruction run with more than one width, the result warns
("ran with 8- and 16-bit A; showing the recorded state").

## Surfaces

- **CLI** (`crates/romlens-cli/src/commands/decompile.rs`, after the pattern of
  `commands/xrefs.rs`):
  `romlens decompile <rom> [--project P] <address> [--level lift|clean|full] [--json]`,
  `romlens decompile --header <out>` to write `snes.h`, and
  `romlens decompile <rom> --all --check` to decompile every function, run each
  through `cc -fsyntax-only` when a compiler is present, and print the counts.
- **FFI** (`crates/romlens-ffi`): `DecompiledInfo` in `records.rs`;
  `Workbench::decompile(snes_address)` as a `ThreadFuture` like `analyze`,
  `function_containing(file_offset)` and `snes_header()` in `workbench.rs`.
  Results are cached by entry and the analysis and view generations.
- **macOS**: `EditorTab.c`, beside Hex, Disassembly and Both, and View › C
  (⌥⌘8; ⌥⌘4–7 are the graphics views). `Views/CSplitView.swift`, after
  `LockstepEditorView`: the disassembly table on the left, the C read-only and
  coloured on the right, the cursor kept in step through the line map. A name in
  the C navigates to its label or variable. Decompile Routine in the context
  menu; Export C… writes the `.c` and `snes.h`. The C is rebuilt when the
  analysis or the project changes, so renames and new variables show at once.

## Ordered tasks

| # | Task | Size (days) | Depends on |
|---|---|---|---|
| T0 | This document, the roadmap pointer, the checklist rows | 0.5 | — |
| T1 | `function.rs` and `cfg.rs`, tested on fixtures (`fixtures::build_with_code`, `dispatch_lorom`) | 2 | — |
| T2 | `ir.rs` and `lift.rs` for all 256 opcodes, `header.rs`, `emit.rs` at the `lift` level, `romlens decompile`: the first valid C end to end | 3 | T1 |
| T3 | `dataflow.rs`: liveness, dead flags, propagation, conditions, carry chains, 8-bit A, stack pairs (`clean`) | 4 | T2 |
| T4 | `structure.rs`: if/else, loops, switch, the goto fallback (`full`) | 3 | T3 |
| T5 | `signature.rs`: parameters, results, call sites | 2 | T4 |
| T6 | FFI, with a RomlensKit test | 1 | T5 |
| T7 | The macOS C tab: split view, cursor sync, navigation, export, menus, app tests | 3 | T6 |
| T8 | Measure on Super Metroid (and Super Mario World locally) and record it here | 1 | T7 |

T8 is not optional. The numbers it records are the syntax-valid rate (the
target is 100%), gotos per function, and asm comments per function, so later
work has something to beat.

## What is deliberately cut

- Struct and pointer types beyond what variables declare.
- Naming locals from how they are used.
- Recognising idioms such as code generated into RAM, the RESET example above.
- SA-1 and Super FX code.
- A width state per call site. One state is shown, with the warning.
- Output that compiles back into the ROM.

## Risks and mitigations

- **A routine runs in more than one width state.** The snapshot keeps one; the
  execution log says when there were others, and the result warns. Choosing the
  state per call site is the next step, not this one.
- **Unknown D or DBR.** Explicit `DP8` and `MEM` forms and a warning, never a
  guess; `assume_dp` when the reader knows better.
- **Structuring hides a control-flow mistake.** The `lift` and `clean` levels
  stay available, and the semantic tests below run at every level.
- **Large routines are slow.** Per-function work with a block cap, memoised
  summaries, and the app decompiles off the main thread.

## Verification (end to end)

- **Goldens.** Hand-assembled routines through `fixtures::build_with_code`: a
  copy loop, a 16-bit add chain, a jump-table switch, `PHA`/`PLA` pairs, mixed
  8- and 16-bit code. The expected C at each level lives in
  `crates/romlens-cli/tests/golden/decompile-*.txt` (`UPDATE_GOLDEN=1`
  rewrites them).
- **Validity.** Every golden, and every function in the fixtures, compiled with
  `cc -std=c11 -fsyntax-only -Wall` against the generated `snes.h`. Skipped
  where no C compiler is installed.
- **Meaning.** For the small fixture routines, the output is compiled with a
  test `snes.h` that backs memory with arrays and implements the helpers, then
  run on inputs, and the memory and registers it leaves are compared with the
  expected ones. This is what checks that data flow and structuring kept the
  meaning, not just the syntax.
- **The development ROM** (`make test-rom`): every function in Super Metroid
  decompiles without a panic and all of it is syntax-valid; a golden pins
  `$80:841C`.
- **Gates:** `make test`, `make swift`, `make app-test`.
- **By hand** (manual-pass steps 34–36 in `15-conformance-checklist.md`).

## Measurements (23 September 2026)

Every routine the analysis found, `romlens decompile <rom> --all --check`,
compiled with `cc -std=c11 -fsyntax-only -Wall`. Super Metroid is the
development ROM with static analysis only; Super Mario World is a user's
project, with an execution log from play.

| | Routines | Instructions | Level | Statements | Gotos | Asm comments | Valid C | Time |
|---|---|---|---|---|---|---|---|---|
| Super Metroid | 799 | 28,500 | lift | 55,716 | 2,796 | 55 | 799 / 799 | 1.1 s |
| | | | clean | 16,940 | 2,796 | 55 | 799 / 799 | 1.2 s |
| | | | full | 16,940 | 439 | 55 | 799 / 799 | 1.2 s |
| Super Mario World | 795 | 35,337 | lift | 77,564 | 4,303 | 22 | 795 / 795 | 0.8 s |
| | | | clean | 18,198 | 4,303 | 22 | 795 / 795 | 0.9 s |
| | | | full | 18,198 | 759 | 22 | 795 / 795 | 0.9 s |

Data flow removes 70% (Super Metroid) and 77% (Super Mario World) of the
statements the lift writes; structuring removes 84% and 82% of the gotos.
The time covers every routine's summary as well as its C; the app pays it
once per analysis and then decompiles a routine in milliseconds.

**Meaning.** The differential test (`tests/decompile_diff.rs`) runs every
routine at every level from four pseudo-random states and compares what it
leaves with the `lift` level, and checks every summary against the `lift`
code. Super Metroid, in CI with `ROMLENS_ROM_DIR`: 9,408 runs, no
differences. Super Mario World with its project (`ROMLENS_DIFF_ROM`,
`ROMLENS_DIFF_PROJECT`): 9,357 runs, no differences. Getting there found:

- in the output, a value carried out of a `u32` temporary into a
  comparison, where C's int arithmetic let it go negative; it now stays
  `(u32)`;
- in the harness, stubs for calls into unanalysed code that read N, V and
  Z (the analysis assumes they do not), stubs leaving a 16-bit value in X
  where the hardware keeps an 8-bit index register's high byte zero, and a
  stack in memory the routines read.

**Syntax.** Super Mario World also showed routines keeping a read through a
pointer (which may be a hardware register) for its effect alone. Those now
print as `(void)MEM8(…);` rather than as a temporary nothing reads, and a
goto label followed only by a comment gets an empty statement.
