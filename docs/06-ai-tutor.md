# The tutor: an AI component that works through the tools

## What it is for

A student selects a routine, a byte range, a sprite on screen or a moment in a
recording and asks a question. The tutor answers with evidence, offers to fix
what the analyzer got wrong, and can carry out multi-step investigations over
execution recordings. It is a teaching assistant sitting on top of the
deterministic core, never a replacement for it.

The five things the user asked for map to five modes:

| Mode | Student intent | Typical output |
|---|---|---|
| **Ask** | "What does this do?" "Why is A 8-bit here?" | A short, cited explanation anchored to the selection |
| **Explain** | "Walk me through this routine" | A narrated walkthrough that can be promoted to a scene draft (Phase 4) |
| **Fix** | "This looks like data, not code" "The flags are wrong after this call" | A proposal card: reclassify, override M/X, retype data, rename, with the evidence that supports it |
| **Investigate** | "Which code moves Samus?" "What is written to VRAM during the door transition?" | A multi-step run over a recording with a visible plan, tool log and a findings summary with addresses |
| **Quiz** | "Test me on this" | Socratic questions checked against the core or the emulator, not against the model's memory |

## Principles

1. **The tutor never reads bytes from memory.** Every fact about the ROM
   comes from a tool call into `SNESCore` or the recording index. If it cannot
   get a fact from a tool, it says so.
2. **Every claim carries an address.** Answers cite `$80:843C`-style
   locations, recording frame numbers, or hardware register names. The UI
   turns each citation into a link that navigates the Workbench. An uncited
   claim is rendered in a muted style so students learn to distrust it.
3. **Edits are proposals.** The tutor cannot write to the project. It emits
   proposals; the student accepts or rejects each one. Accepted proposals go
   on the undo stack with source `ai (accepted by user)`, so the evidence
   popover always shows who decided.
4. **Show the work.** Tool calls are visible in a collapsible log with their
   arguments and a one-line result. This is an educational tool; the method
   is part of the lesson.
5. **Offline is fine.** Without an API key the app works fully; the Tutor pane
   shows how to add one. No feature outside the pane depends on the model.

## Architecture

```
 student question + current selection
            │
            ▼
   ┌────────────────────┐   POST /v1/messages (streaming, tool use)   ┌──────────────┐
   │ Tutor module (core) │ ───────────────────────────────────────────▶│ Claude Opus 5 │
   │ transcript, cache   │ ◀─────────────────────────────────────────── │ adaptive      │
   │ tool loop, proposals│      tool_use blocks / text with citations   │ thinking      │
   └───────┬────────────┘                                              └──────────────┘
           │ executes tool calls locally
           ▼
   ┌──────────────────────────────────────────────┐
   │ SNESCore snapshot · Recording index · Project │  (read-only tools)
   └──────────────────────────────────────────────┘
           │ proposals only
           ▼
   Proposal cards → user Accept → Project overlay → re-analysis
```

- **Client.** The tool loop lives in the Rust core so that the macOS,
  Windows and Linux shells share one implementation and one transcript
  format. It speaks the Messages API over HTTPS and implements the loop
  itself (send, receive `tool_use` blocks, run them, return all
  `tool_result` blocks in one user message, repeat until `end_turn`). This
  is a few hundred lines. Shells receive a stream of tutor events and render
  them; they never hold the API key or talk to the API.
- **Model.** `claude-opus-5` with adaptive thinking on (the default), effort
  `medium` for Ask and Quiz, `high` for Fix and Investigate. Streaming always.
  Thinking display `summarized`, surfaced as a collapsible "how the tutor got
  here" block, which is itself educational. Server-side refusal fallbacks
  enabled by default so a rare classifier refusal does not dead-end the
  student. Tool definitions use `strict: true` so arguments always validate.
- **Context and caching.** The request prefix is, in order: the tool list,
  a frozen system prompt, and a per-project "ROM digest" (header facts,
  mapping, hardware register table, imported symbol names, region
  statistics), with a cache breakpoint after each. Only the selection
  context and the question vary per turn. The digest is regenerated when the
  project changes, which invalidates the cache once, not per message.
- **Selection context.** Each turn appends the current selection as text:
  the bytes, the disassembly plus a window of lines around it, region kind and
  evidence, labels and cross-references in range, and, if a recording is
  loaded, execution counts for the range. Typically one to four thousand
  tokens. The tutor is told this is a window and must call tools to see more.
- **Keys and cost.** The API key is always the user's own (decided): entered
  in settings, stored in the platform credential store (Keychain, Windows
  Credential Manager, Secret Service) through the core's one
  `CredentialStore`. Romlens ships no key and runs no proxy. The pane shows tokens and estimated cost per conversation and a
  running total per project. A per-conversation cap stops runaway
  investigations.
- **Privacy and copyright.** Only what a question needs leaves the machine:
  the selection window and tool results. The app never bulk-uploads the ROM
  or a recording. Settings state this plainly.

## Tool surface

Dedicated tools rather than a general "run code" tool, because every action
must be gateable, renderable and auditable in the UI. Read-only tools are
parallel-safe; proposals are serialized and always rendered as cards.

### Read-only (run immediately, shown in the log)

| Tool | Purpose |
|---|---|
| `rom_info()` | Header facts, mapping, size, checksum, vectors |
| `read_bytes(address, length)` | Raw bytes at a SNES address or file offset (max 4 KB per call) |
| `disassemble(address, count, flags?)` | Instructions from the snapshot, or re-decoded with an M/X/E override for hypothesis testing |
| `region_at(address)` / `regions(range, kind?)` | Region kind, confidence and evidence |
| `labels(range)` / `xrefs(address)` | Names and references in and out |
| `resolve(address)` | File offset, mirrors, bank speed, hardware register name if any |
| `search_bytes(pattern, range?)` | Byte pattern search with wildcards |
| `hardware_register(address)` | Bundled description of PPU/CPU/DMA/APU registers |
| `reference(topic)` | Bundled notes: 65816 opcode semantics, addressing modes, flag rules, SNES graphics formats |
| `decoder_sanity(address, flags)` | Re-decode a range under given flags and report invalid opcodes, dangling ends, plausibility score |
| `decode_tile(address, bpp, palette?)` | Pixel indices and a PNG preview of a tile at an address |
| `trace_query(recording, kind, range, frames?)` | Execution counts, reads, writes or DMA transfers touching a range |
| `who_writes(recording, address, frames?)` | Instructions that wrote a RAM or PPU address, with counts |
| `call_tree(recording, frame)` | The JSR/JSL tree executed during one frame |
| `ppu_state(recording, frame)` | Registers, OAM, CGRAM summary at a frame |
| `pixel_provenance(recording, frame, x, y)` | The layer, object, VRAM, transfer and ROM chain behind a pixel |

### Proposals (rendered as cards, never applied automatically)

| Tool | Card |
|---|---|
| `propose_region(range, kind, confidence, reason)` | Reclassify code/data with before/after preview |
| `propose_flag_override(address, m, x, e, reason)` | Shows the disassembly diff the override produces |
| `propose_data_type(range, type, stride?, reason)` | Table, pointer list, string, graphics with a preview |
| `propose_label(address, name, reason)` | Rename with all xrefs listed |
| `propose_comment(address, text)` | Line or block comment |
| `propose_scene(steps)` | Phase 4: a scene draft opened in the scene editor |

### The Fix loop, concretely

Student: "The disassembly after `$80:9C41` looks like garbage."

1. Tutor calls `disassemble($80:9C30, 24)` and `region_at`, sees a run of
   improbable opcodes after a `JSL`.
2. Calls `xrefs` on the JSL target, then `disassemble` on the callee's tail,
   finds it ends with `SEP #$20` before `RTL`.
3. Calls `decoder_sanity($80:9C41, m=1)` and gets a clean decode ending at
   `RTS`.
4. Emits `propose_flag_override($80:9C41, m=1, reason: "callee $8B:A1F0 returns with M=1")`.
5. The card shows the old and new disassembly side by side. The student
   accepts; the analyzer re-runs; the evidence popover now reads
   "flag override, proposed by tutor, accepted by user, reason: …".

### Investigate over recordings, concretely

Student: "Which code moves Samus horizontally?"

1. Tutor asks `reference("super metroid ram map")` if symbols are imported,
   or asks the student to point at the RAM address (the app already shows
   which RAM bytes change when the student moves; see the recording UI).
2. Calls `who_writes(recording, $7E:0AF6)` and gets three writer PCs with
   counts.
3. Calls `disassemble` and `xrefs` on each, `call_tree` on a frame where the
   value changed.
4. Reports: the writer that runs every frame during movement, its caller
   chain, and what it adds to the position, all cited. Offers a
   `propose_label` for the routine and a `propose_scene` for a walkthrough.

## UX

- **Tutor pane** replaces the Inspector via a segmented control (Inspector |
  Tutor), or opens as a fourth pane when the window is wide enough.
- A **selection chip** at the top of the composer shows what the question is
  anchored to. Changing selection updates the chip; the transcript notes the
  change so the model sees it.
- **Citations** render as monospace chips. Click to navigate; hover to peek.
- **Proposal cards** have Accept, Reject and "Show evidence". Multiple cards
  from one answer can be accepted together.
- **Tool log** is a collapsed row per turn: "4 tool calls" expanding to the
  list. Errors are shown, not hidden.
- **Modes** are a menu in the composer; Ask is default. Investigate shows a
  plan first and a stop button while running.
- **Inline entry points:** every disassembly line has a `?` on hover
  ("Explain this instruction"), every region chip has "Ask why", every pixel
  in the frame view has "Trace this pixel".

## Quality and evaluation

- **Golden set.** Fifty questions about Super Metroid routines with answers
  derived from PJBoy's annotations, plus twenty deliberate analyzer errors
  with known fixes. Run on every change to the system prompt or tools; graded
  by exact-address checks where possible and by a model judge otherwise.
- **Citation rate.** Percentage of factual sentences with a resolvable
  citation. Target above ninety percent; below that the prompt is wrong.
- **Proposal precision.** Fraction of accepted proposals that survive later
  evidence (a trace contradicting a reclassification counts against it).
- **Cost per answered question,** tracked per mode.

## Phasing

| Phase | Tutor capability |
|---|---|
| 1 | Ask mode over read-only ROM tools. Citations, selection chip, tool log |
| 2 | Fix mode with proposal cards. `decoder_sanity`, data-type proposals, tile decode |
| 3 | Investigate mode over recordings. Plans, stop button, `who_writes`, `pixel_provenance` |
| 4 | Explain mode producing scene drafts. Quiz mode with emulator-checked answers |

## Risks

- **Confident nonsense.** Mitigated by tools-only grounding, citation
  rendering and the golden set. The UI never shows an uncited claim as fact.
- **Cost surprises.** Mitigated by per-conversation caps, cache-friendly
  prefixes and visible totals.
- **Copyright sensitivity.** Only excerpts leave the machine, and only on the
  student's request. Documented in settings.
- **API drift.** The client is a thin layer with one request builder and one
  response parser; model and parameter names live in one place.
