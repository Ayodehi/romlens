# The tutor: a chat over Romlens's data, on any provider

## Progress

Written 27 September 2026 and kept as written. This table is the only part
that tracks progress against it.

| Task | State |
|---|---|
| U0 this document, content-policy rule 8, the pointers from `06` and `22`, the checklist rows | done, 27 September 2026 |
| U1 `romlens-tutor`: the transcript, the three wire protocols, SSE, the model table, recorded-stream tests | done, 27 September 2026: `transcript` (turns of blocks, each assistant turn keeping what its provider sent as `Native`), `sse::Reader` (bytes in any chunks; a line cut off drops its event, so a short stream reads as short), `models::MODELS` (Claude Opus 5 by default, Opus 5.5, Fable 5.1, Sonnet 5, Haiku 4.5; GPT-6 Astra, Sol, Luna; a model not in the table is local, with nothing to pay), and `provider::{anthropic, responses, chat}`, each a request builder and a stream parser. Anthropic: adaptive thinking summarised, the effort, strict tools streamed eagerly, breakpoints after the last tool, the system prompt, the digest and the newest block, `fallbacks: "default"` where the model has it, its own turns sent back verbatim. Responses: `store: false` with the encrypted reasoning sent back, `prompt_cache_key`, pictures in call outputs. Chat: calls gathered by index, reasoning from `reasoning` or `reasoning_content`, LiteLLM's `thinking_blocks` back only to the endpoint that made them, pictures from tool results in a user message after them, no `tool_choice` for servers that refuse it. Tests: 10 unit and 9 on recorded streams, one conversation carried from Claude to GPT-6, back to Claude, and to a local model |
| U2 the HTTP transport, retries, cancelling, model lists, the loop, cost; `romlens tutor ask` and `models` | done, 27 September 2026: `http::UreqTransport` (ureq 3 over rustls with the platform's certificate verifier; a stream has no time limit, only its connection does), `FakeTransport` for tests, `list_models` (Anthropic's pages, OpenAI's and a local server's list), and the provider's own error message shown ("HTTP 401: x-api-key header is required"). `agent::Session::ask` runs the loop: a key per request from `Credentials` (none needed for an endpoint of the user's), reads in one round side by side, other calls one at a time, arguments that do not parse answered as an error and not run, retries on 408/409/429/5xx/529, on an overloaded stream and on a stream cut short, with `retry-after` or 1, 2, 4 s, a round cut short (Esc, the output limit, a refusal) answered with an error per call so the transcript stays valid, the cost of each reply from the model table, and a cap checked before each request. `romlens tutor ask <rom> "<question>" [--provider P --base-url U --responses] [--model M] [--effort E] [--cap $]` and `romlens tutor models`; with no tools yet (U3). Tests: 6 on the loop over recorded replies, 2 more on the transport |
| U3 the system prompt, the primer, the reference pages, the ROM digest, the cache layout; the read-only tools | done, 27 September 2026: `romlens_tutor::prompt` (the system prompt's rules on grounding, citing `$BB:AAAA` and `frame N`, answering and changing the project, then a primer of the SNES in our own words: the memory map, the 65816's widths and modes, a frame, the PPU, DMA and HDMA, the S-CPU's hardware, the APU, patterns, Romlens's terms; about 19 KB, fixed). The tools are in the FFI crate, which the CLI now links, so the app and the CLI run one set over the `Workbench`: code (`rom_info`, `resolve`, `read_bytes`, `listing`, `decode_as` for a what-if, `region_at`, `regions`, `labels`, `find_label`, `variables`, `xrefs`, `search`, `decompile`, `routine_graph`, `calls`, `explain_at`, `describe_register`, `screen_at`, `reference`), pictures and recordings (`preview_at`, `decode_tiles`, `recording_info`, `ppu_state`, `render_frame`, `render_layer`, `render_sprite`, `pixel_provenance`, `changes`, `history`, `who_writes` over the project's execution log) and sound (`sound_upload`, `voices`, `dsp_registers`, `aram_map`, `samples`, `spc_listing`, `note_timeline`, `port_events`, `brr_sample`); 41 in all, each schema strict with nullable values as `anyOf`, all declared whether or not a recording is open. Pictures are PNGs made in memory (`tutor::png`, written from the specification: a palette at 256 colours or fewer, stored deflate, small ones scaled up). `reference` pages come from Romlens's tables (the 65816 and SPC700 opcodes, the register layouts, the idioms' reasons, now `explain::idioms::WHYS`). The ROM digest is made once per conversation. `romlens tutor ask … [--project P] [--at A] [--rec R] [--attach PNG]`. Rule 8 now says tool pictures stay in the app's folder with the conversation, never exported. Tests: every tool on fixture ROMs and recordings (graphics and sound), strict schemas, the PNG's CRC and Adler checks and a decode by macOS's `sips`, and the real transport against a loopback server (a stream in pieces, a 429 with its message and `retry-after`, Esc part way) |
| U4 signatures, local names, structs, C comments and C versions in the project; the decompiler reads them | done, 27 September 2026, with structs left for later (below): `model::c_notes` and four commands with inverses, `SetLocalName` (a routine's local or parameter, `x`, `a8`, `x_out`…, by Romlens's name), `SetRoutineNote`, `SetCComment` and `SetCVersion` (a named text with its anchors, line ranges to address ranges, and who wrote it), and a new origin, `Origin::Tutor { conversation, turn }`, titled "Tutor: …" on the undo stack. `decompile::annotate` applies the first three to the finished C at every level: names only inside the routine's own definition and body, never in comments or callees' prototypes, with the tokens and each line's instructions kept; a C comment above the statement its instruction makes, selecting that instruction; the note above the routine's summary. Stored in `c_notes.json` and `c_versions.json`, only when there are any. `decompile::lex` colours C nobody generated (keywords, types, numbers, comments, registers by name, the project's labels, calls). FFI: the four commands, `execute_batch(commands, EditOrigin)`, `local_names`, `routine_note`, `c_comments`, `c_versions`, `lex_c`. CLI: `romlens project <p> local|note|ccomment|cversion …`. A routine's signature is its note and its parameters' names; changing a parameter's C type is not done. Tests: the C shaped and still compiling, undo back to the same text, tokens and lines, refused names changing nothing, the package round trip, the lexer, and the golden `decompile-annotated` |
| U5 the edit tools, permission modes, `Origin::Tutor`, rewinding edits, compaction | done, 27 September 2026: nine edit tools (`set_label`, `set_comment`, `define_variable`, `mark_region`, `set_flags`, `rename_local`, `set_routine_note`, `set_c_comment`, `write_c_version`), each with a reason for its card, and a read tool, `c_versions`. Read-only refuses with a line telling the model to suggest the change in words; ask-before-edits sends `EditProposed` (a line, the reason, what is there now and after) and waits on an `Approver` for Accept or Reject (with the student's words), as a permission prompt does; accept-edits makes it at once. What is made goes through `Workbench::apply_commands` with `Origin::Tutor { conversation, turn }`. `UndoStack::rewind` takes back a conversation's edits from a turn on: those on top are undone; older ones, with the student's edits after them, are reverted as one new entry ("Rewind the Tutor's Edits", itself undoable), except where a later edit of the student's changed the same thing (`Command::target`), which are kept and reported. `Session::rewind_to` stops sending a prompt and what followed and gives the prompt back; `Session::compact` has the model write a summary, sends nothing older after it, and runs by itself before a question past 80% of the context. `romlens tutor ask … --mode read-only\|ask\|accept` asks on the terminal in ask mode and saves the project after edits. Tests: the modes, a card declined with a reason, a bad name refused, rewinding by turn in the FFI and in the core (kept, reverted, undone, the rewind undone), rewinding the transcript, compaction |
| U6 the conversation store, resuming, prompt history; the FFI's `TutorSession` | done, 27 September 2026: `romlens_tutor::store` (a folder per conversation under the ROM's SHA-256, saved whole and atomically; the title from the first question's own words; `prompts.jsonl` for ↑, repeats in a row folded). The FFI's `TutorSession` (API 0.12.0): built from a `Workbench`, the conversations folder, a `CredentialStore` and a `TutorListener`, both Swift-side callbacks; `new_conversation` (fixes the system prompt and digest), `resume`, `conversations`, `delete_conversation`, `transcript` (every turn, sent or not, as display blocks), `picture`, `send` (text, pictures, the selection; a question is the context block then the student's own words), `cancel`, `answer` (an edit card, which blocks the turn's thread on a condition variable until then or Esc), `set_model` (a note in the transcript; the cache starts cold), `set_mode` (said in the next question), `set_cost_cap`, `set_recording`, `rewind` (the conversation, the edits, or both), `rewind_points`, `compact`, `prompt_history`, `cost`, `context_used`, `list_models`; `tutor_models`, `tutor_default_endpoints`, `tutor_default_model`. A turn runs on its own thread; `Ended` is told only once the session is back and saved. Tests: two end to end through the real transport against a loopback Chat Completions server (a tool round, saved, listed and resumed by a second session, the history, a rewind; an edit card answered, then rewound), the store, and three RomlensKit tests through the generated Swift (annotations and their rewind, `lexC` in UTF-16, a session made with Swift's key store and listener) |
| U7 the app's network entitlement, the Settings window, the Keychain | done, 27 September 2026: `com.apple.security.network.client` in `project.yml`. Settings… (⌘,) in the app menu opens one window with three tabs: Providers (Anthropic and OpenAI built in, endpoints of the student's added with a name, base URL, protocol, whether its models see pictures, take `tool_choice` or need a key; a key field saved to the Keychain and removed from it; Test, which lists the models through a new `tutor_list_models`), Tutor (the default provider, model with its prices from the table or typed for a local server, effort, edit mode, the cost cap, whether thinking is shown) and Privacy (what is sent and what stays, rule 8). `Keychain` keeps one generic password per endpoint (service `io.github.ayodehi.Romlens.tutor`); `TutorCredentials` is the core's `CredentialStore` over it; `TutorSettings` keeps the rest in the user defaults. Fixed on the way: a save now removes `variables.json`, `c_notes.json` or `c_versions.json` when the core stops writing it, so the last one removed no longer comes back on reopen. Tests: settings kept and the key never in the defaults, endpoint ids, the Keychain round trip, the window laid out, the menu item, the package losing a removed note; the app suite is 133 tests |
| U8 the Tutor window: transcript, citations, thinking, tool log, edit cards, composer, commands | done, 27 September 2026: View › Show Tutor (⌥⌘T) opens a second window of the project (`TutorWindowController`, which never closes the project and keeps its conversation when closed and shown again). `TutorModel` holds the core's `TutorSession` (made on first use, conversations under Application Support/Tutor) and turns its events, handed to the main actor in order, into the turn streaming in. The transcript shows each question (with its pictures and whether the selection went with it), then per answer the thinking collapsed as "How the tutor got here", the tool log collapsed to "N tool calls" with each call's arguments and first line of result, and the text: Markdown kept line by line, headings bold, lists as bullets, every `$BB:AAAA` and `frame N` a link that selects the address or frame in the main window and brings it forward, and code blocks coloured (C by the core's lexer with the C view's palette, assembly by a light pass) with a Copy button. Edit cards show the line, the reason, now and after, a box for why not, Decline, Accept and Accept All (the rest of that answer). The composer is an `NSTextView`: Return sends, ⇧Return a new line, ↑ on the first line and ↓ on the last walk the project's questions, ⇧⇥ cycles the mode, Tab completes a command, Esc stops a turn and twice opens rewind; pictures and photos pasted or dropped are attached (TIFF and HEIC as PNG; one over 2,576 pixels on its long side or 3.5 MB is scaled down and sent as JPEG, within what Claude and OpenAI take). Over it: `/` lists the commands as you type, a chip names the main window's selection (click to leave it out), and one attaches the recording's frame. The status line: provider and model (click for `/model`), the mode (click to cycle), how full the context is, the cost, Stop. Edits made by the tutor refresh the main window and re-run the analysis where a mark or flag changed (`WorkbenchSession.tutorEdited`). For the tests, the FFI exports a loopback model server with scripted replies. Tests: six shell tests (a question with a tool round and its citation followed, the history; an edit card accepted, on the undo stack as the tutor's, rewound; the commands and modes; citations and code found in text; the window laid out; one Tutor window per document); the app suite is 139 tests |
| U9 `/rewind`, `/resume`, `/model` mid-conversation, `/compact`, the cost cap, attaching a frame | done with U8, 27 September 2026: `/rewind` (and Esc Esc) lists the questions, latest first, and rewinds the conversation and the tutor's edits, either alone, putting the question back in the composer and saying which edits were kept because you changed them since; `/resume` lists the project's conversations with date, model, turns and cost, resumes one or deletes one not open; `/model` picks the provider (marked when it has no key), a model from the table or, for a local server, from its own list, and the effort, and the conversation goes on with it (a note says the cache starts cold); `/compact` summarises; `/new` and `/clear`; `/mode read-only\|ask\|accept`; `/cost`; `/attach frame`; `/selection`; `/help` with the keys. The cost cap comes from Settings |
| U10 image generation; C versions in the C tab; the annotation sheets | done, 27 September 2026: `romlens_tutor::images` asks OpenAI's Images API, or an endpoint that speaks it, for a picture from words only (`generate_image(prompt, size)`, declared always and refusing when Settings › Images names no endpoint); the picture comes back as a tool result, is kept with the conversation, and shows large in the answer, marked "Generated by an image model, not from the ROM", while pictures tools read from the ROM or a recording show small in the tool log. The C tab's header has a version picker (Generated, each version, New, Edit, Delete); a version shows coloured by the core's lexer, says who wrote it and that it is not checked, highlights the lines anchored to the selected instruction, and a click on an anchored line selects its instruction. Right-click in the C names a local ("Name “x”…"), comments the line's instruction, notes the routine, or writes a version, through one C sheet; an edited version keeps the anchors whose lines still exist. A conversation resumed under a Romlens that declares other tools drops Anthropic's old thinking blocks once (their summaries stay), since the prefix they were made under is gone (`Session::check_tools`, the digest kept in the conversation's meta). Tests: the Images request and answer, base64, the dropped thinking, a version in the C pane followed by its anchor, the sheet's subject, and a picture drawn end to end through two loopback servers; the app suite is 141 tests |
| U11 measure, record, the manual pass steps | in progress, 27 September 2026. Done: `crates/romlens-ffi/tests/tutor_live.rs`, five questions about the explain fixture ROM (RESET's start, the DMA, the multiplier, the C of the clear loop, and an edit in Accept edits), each graded by the citations and facts its answer must hold, printing time, tool calls and cost per question; it runs only with `ROMLENS_TUTOR_LIVE=1` and a key (`ROMLENS_TUTOR_PROVIDER` anthropic, openai or an endpoint URL; `ROMLENS_TUTOR_MODEL`, `ROMLENS_TUTOR_EFFORT`), never in CI. Manual pass steps 52–58 in `15`. Left, for the user (no key is in the sessions' environment and Ollama was not running): the live runs on Anthropic, OpenAI and Ollama, their table here, and the manual pass From the first live run (the model looked `ADDR_7F8000` up as a label, never saw the C, and found nothing): the C's made-up names (`ADDR_`, `L_`, `SUB_` and six hex digits) are read as their address by every tool that takes one, `find_label` says what such a name stands for, the system prompt explains them and says to `decompile` when a question names something in the C, and on the C tab the question carries the C the student is reading (the generated C around the selection, at most 160 lines, or the C version shown). `romlens tutor sessions` lists the app's saved conversations and `romlens tutor show [latest\|id] [--full] [--system]` prints one turn by turn (the question and its selection, thinking, each tool call and its result, answers, tokens and cost), for working out why the tutor answered as it did From the second run (it saw the C but told the student `ADDR_7F8000` "isn't really a variable", and answered with a table the window showed as pipes): the system prompt says a question about the C is answered in the C's terms, a variable first, with what the machine code shows beyond that as the next layer, and that the window shows Markdown; the window now draws pipe tables (a bold header, rules between rows, each cell inline Markdown with its citations as links) and `---` rules |

## Context

The sound track (`23-audio.md`) is finished, and the tutor is what is left
of Phase 3 (`22-phase3-finish.md`, T1–T4). `06-ai-tutor.md` designed it for
one provider, with proposals only, in a pane of the main window. On 27
September 2026 the user widened it:

- **Any provider.** Anthropic, OpenAI, and endpoints that speak OpenAI's
  protocol on the user's own machine or network (Ollama, LiteLLM, LM Studio,
  vLLM). The latest models, with thinking and prompt caching. The provider
  and model can change in the middle of a conversation.
- **Settings** where the user enters their keys, kept in the Keychain.
- **An agent with tools over everything Romlens computes**, primed on the
  SNES: the 5A22 and its 65C816, the PPU, DMA, the SPC700 and the DSP. It
  explains the assembly and the C, answers what-ifs, simplifies, reads the
  screenshots and photos the user gives it, and makes pictures with an image
  model.
- **Conversations kept**, so a student can come back to one later.
- **A chat modelled on Claude Code**: ↑ brings back earlier prompts, and
  commands such as `/rewind` and `/model`.
- **The AI edits the project**: names routines, defines variables, and
  shapes the C, so that when it works out what a routine does the C shows
  it.
- **Its own window**, beside the main one.

`06-ai-tutor.md` still holds for the principles (the tutor learns facts
only from tools, cites every claim, and shows its work), the tool ideas and
the evaluation. This document replaces its client, model, key and
edit-policy sections and T1–T4 in `22`.

## What already exists

- **Nothing of the tutor.** No HTTP or TLS crate in the workspace, no
  Keychain code, no Settings window. The sandboxed app has
  `network.server` for the live session and no outgoing network access.
- **The read side is broad.** The FFI's `Workbench`, `RecordingSession` and
  `ApuPlayer` answer almost every tool in `06`:
  - the listing and `disassemble(offset, count, flags)`, already called "the
    tutor's hypothesis tool";
  - regions, labels, cross-references, search;
  - the C at three levels, the routine graph, the call neighbourhood;
  - explanations of hardware writes and idioms, the screen setup;
  - tiles, palettes, OAM, tilemaps, frames, layers and pixel provenance from
    a recording;
  - the sound upload, voices, DSP registers, the audio RAM map, samples,
    notes, the ports.
- **Edits go through `Command`s** (`model/command.rs`): labels, comments,
  region marks and parameters, flag overrides, variables. `apply_batch`
  applies several as one undo step, and `Origin::Accepted` was left for the
  tutor. Undo lives in the core, and the app has no `NSUndoManager`.
- **The C is read-only.** It changes only through labels, variables, region
  marks and flag overrides. The project cannot hold a routine's signature,
  names for its locals, a struct, or a comment that shows in the C.
- **What the core cannot answer yet:** "who wrote this byte" outside a
  pixel's provenance (`provenance/source.rs code_writing` is not exported),
  a label by name, and a register's fields for an arbitrary value (the
  CLI's `registers --value` does it; the FFI does not).

## Scope decisions (27 September 2026)

1. **The loop is in Rust, in a new crate, `romlens-tutor`.** It holds the
   wire protocols, the transcript, the tools, the conversation store and
   the cost. romlens-core stays free of networking. The FFI and the CLI
   both use the crate; the app renders its events and never talks to a
   provider.
2. **Three wire protocols and one transcript.**
   - Anthropic's Messages API.
   - OpenAI's Responses API. OpenAI's current models need it for tools.
     Requests are stateless (`store: false`), with the encrypted reasoning
     returned and sent back.
   - Chat Completions, for compatible endpoints, which is what every local
     server speaks. An endpoint can be set to Responses instead where its
     server has it (Ollama 0.13.3 on, LM Studio, LiteLLM).

   The transcript is Romlens's own. Each protocol builds its request from it
   and parses its stream into it.
3. **The provider and model change between turns, never inside one.**
   - Reasoning is kept as the provider sent it, tagged with the provider and
     model, and sent back only to that provider. For any other it is left
     out, and its summary stays in the window.
   - Tool-call ids are Romlens's, mapped to each protocol's.
   - Chat Completions cannot carry an image in a tool's result, so the image
     follows in a user message.
   - The window notes the change and that the cache starts cold.
4. **Keys are the user's own.**
   - The app keeps them in the Keychain and hands one to the core per
     request, through a `CredentialStore` callback. The core keeps no key.
   - The CLI reads `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, or the variable an
     endpoint names.
   - Romlens ships no key and runs no proxy.
5. **Edits follow a permission mode, as in Claude Code.** Shift-Tab cycles
   through three:
   - **Read-only**: the edit tools refuse.
   - **Ask before edits** (the default): each edit is a card with Accept
     and Reject.
   - **Accept edits**: each edit is applied as it is made, and still shown
     as a card.

   Every edit applied goes through `apply_batch` with a new origin,
   `Origin::Tutor { conversation, turn }`. Undo, the evidence popover and
   `/rewind` all know which edits the tutor made.
6. **The C is shaped in two layers.**
   - **Annotations the decompiler reads**, made by the user and the tutor
     alike through the same commands:
     - a routine's signature: its parameters' names and types, its return;
     - names for its locals;
     - struct types for RAM tables and variables;
     - comments that show in the C.
   - **C versions**, a layer on top that never changes the disassembly or
     the generated C. A version is a named, hand-written C text for one
     routine, by the user or the tutor. It is anchored line by line to the
     routine's addresses, shown beside the generated C, and never compiled
     or checked. It is for explaining a routine another way, and later it is
     what the tools for making teaching media and videos start from.
7. **Conversations are the machine's, not the project's.**
   - Tool results hold ROM bytes and attachments can be screenshots, so
     rules 1 and 4 keep them out of the project package.
   - They live in the app's Application Support, under `Tutor/` and the
     project's identity, one folder each.
   - Annotations and C versions are the project's and go in the package;
     they hold no ROM bytes.
8. **Content policy rule 8 is rewritten** for any provider:
   - Only the selection, tool results and the user's own attachments are
     sent, and only when the user asks.
   - A local endpoint sends nothing off the machine.
   - Image generation is sent a text prompt only; no game asset goes to an
     image model.
   - Settings say this plainly.
9. **Facts come from tools, and the details come from the providers'
   current documentation.** Model ids, what each model can do, and prices
   live in one table, `models.rs`. Each is checked against the provider's
   documentation when it is written and again when U11 measures, not taken
   from this document.
10. **macOS first**, and every capability also in the CLI (`08`, rule 7).

## Design

### `romlens-tutor`

A new crate beside the core. It depends on romlens-core, serde, serde_json,
and ureq 3 with rustls and the platform's certificate verifier.

**The transcript** (`transcript.rs`):
- A `Conversation` has an id, a title, turns, and the models it has used.
- A `Turn` has its role and blocks, the provider and model that made it, its
  usage and cost, the selection it was asked about, and the edits it made.
- A block is one of:
  - text;
  - an image (an attachment, by id);
  - a tool call (our id, the name, the input);
  - a tool result (our id, text and image parts, whether it failed);
  - reasoning (provider, model, the summary shown, the payload sent back);
  - an edit (the command and whether it is proposed, applied, rejected or
    undone);
  - a note (a change of model, a compaction, a rewind).

**The protocols** (`provider/`): `anthropic.rs`, `responses.rs`, `chat.rs`
and a shared `sse.rs`. Each builds an HTTP request from the transcript and
parses the event stream into deltas: text, reasoning, a tool call's
arguments, usage, and the stop.
- **Anthropic:**
  - adaptive thinking with summaries shown, and the effort;
  - strict tool schemas;
  - cache breakpoints after the tools, the system prompt, the ROM digest
    and the newest message;
  - the stop reasons for a paused turn, a refusal and a full context.
- **Responses:**
  - reasoning effort and summaries;
  - function calls and their outputs by call id, with images in outputs;
  - the cache key and options for the model.
- **Chat Completions:**
  - tool calls gathered by index from the deltas;
  - reasoning read from whichever field the server uses (`reasoning`,
    `reasoning_content`, `thinking_blocks`);
  - `tool_choice` left out for servers that reject it;
  - images as data URLs.

**The model table** (`models.rs`): each model's id, context, output limit,
whether it sees images and thinks, its effort levels, and its prices. The
model picker merges the table with the provider's own model list. A local
model not in the table takes vision from its endpoint's settings, and its
cost shows as "local".

**The transport** (`http.rs`):
- A `Transport` trait, with two implementations: the real one on ureq,
  reading the stream on a worker thread, and a fake that replays recorded
  streams for tests.
- Retries on rate limits, overload and server errors, honouring
  `retry-after`.
- Cancelling (Esc) is a flag checked between chunks.

**The loop** (`agent.rs`):
- Send, stream, run the tool calls, return every result in one message, and
  repeat until the model stops.
- Read-only calls may run in parallel. Edits run one at a time, through the
  permission mode.
- A per-conversation cost cap stops the loop.
- It sends events to the shell:
  - text, reasoning and tool-call deltas;
  - a tool started and finished;
  - an edit proposed or applied;
  - usage and cost;
  - the turn ended, or failed.

**What the model is told** (`prompt/`):
- A fixed system prompt:
  - the tutor's role;
  - its rules: facts from tools only, every claim cited as `$BB:AAAA` or
    `frame N`;
  - how edits and modes work.
- A primer of about 8,000 tokens, written by us:
  - the memory map;
  - the 65816's modes and the M, X and E flags;
  - the PPU, DMA and HDMA;
  - the SPC700 and the DSP;
  - Romlens's own words for things.
- The ROM digest: the header, mapping, region counts, imported symbols and
  vectors.
- The selection is appended to each question, as `06` "Selection context"
  describes. Nothing that changes goes in the prefix, so the cache holds
  from turn to turn.

**Reference pages** (`reference/`), for the `reference(topic)` tool:
- Generated from Romlens's own tables:
  - the 65816 and SPC700 opcode tables;
  - the register layouts in `explain/fields.rs` and `explain/sound.rs`;
  - `model/hardware.rs`;
  - the "why games do this" texts in `explain/idioms.rs`.
- Overviews written by us.
- Nothing is copied from fullsnes or any GPL source.

**The tools** (`romlens-ffi/src/tutor/`, over the `Workbench`):
- Each tool has a name, a strict schema, a kind (read, edit or visual) and a
  handler.
- They are written once over the FFI's `Workbench` and `RecordingSession`,
  which the CLI links as a library, so the tutor reads what the app shows
  and the CLI and the app give the same answers. (This replaces a `Host`
  trait with two implementations.)
- Pictures (tiles, frames, layers, sprites) are PNGs made in memory, indexed
  where they have 256 colours or fewer, scaled up by whole pixels when
  small.

The tools, grouped:
- **Code:**
  - `rom_info`, `resolve`, `read_bytes` (4 KB at most);
  - `disassemble` (with flags for a what-if), `listing`, `region_at`,
    `regions`;
  - `labels`, `find_label`, `xrefs`, `search_bytes`, `search_text`;
  - `decompile`, `routine_graph`, `call_neighbourhood`;
  - `explain_at`, `screen_at`, `describe_register`;
  - `reference`.
- **Graphics**, returning pictures:
  - `decode_tiles`, `preview_at`;
  - `render_frame`, `render_layer`, `render_sprite`;
  - `ppu_state`, `pixel_provenance`.
- **Sound:**
  - `sound_upload`, `voices`, `dsp_registers`, `aram_map`;
  - `samples`, `brr_at`, `spc_listing`;
  - `note_timeline`, `port_events`, `nspc_song`.
- **Recordings:** `recording_info`, `changes`, `history`, `who_writes`.
- **Edits:**
  - `set_label`, `set_comment`, `define_variable`, `mark_region`,
    `set_flag_override`;
  - `set_signature`, `rename_local`, `define_struct`, `set_c_comment`;
  - `write_c_version`.
- **The app:** `show_in_romlens`, which moves the main window to an address
  or frame.
- **Pictures:** `generate_image`, only when an image provider is set:
  OpenAI's Images API or a compatible `/v1/images/generations`.

**The store** (`store.rs`):
- Per conversation, under the ROM's SHA-256: `transcript.json`,
  `pictures/`, and `meta.json` (title, endpoint, model, effort, mode, cost,
  and the fixed system prompt and digest, so a resumed conversation sends
  what it sent). Each save writes them whole, to a temporary name then
  renamed.
- An index of the folder for `/resume`.
- Per project, `prompts.jsonl` for ↑.
- Turns that were rewound or compacted stay in the file, marked as not
  sent.

**Compaction** (`compact.rs`):
- A meter shows how full the context is.
- `/compact`, and compaction near the limit, never in the middle of a tool
  round: the model writes a summary, and the next request starts from that
  summary and the new question, replaying nothing older.

**Everything sent is append-only.** Anthropic ties each thinking block to
the conversation before it, and any provider's cache is a prefix, so no
request rewrites what an earlier one sent:
- The system prompt, the digest and the full tool list are fixed for a
  conversation. In Read-only mode the edit tools are still declared and
  answer that the mode forbids them; `generate_image` answers that no image
  provider is set.
- A change of mode, selection or recording is said in the next user turn,
  not by editing the prompt.
- Each assistant turn keeps what its provider returned, byte for byte
  (thinking with its signature, OpenAI's encrypted reasoning items), and is
  replayed from that to the protocol that made it. Anthropic's turns go back
  to Anthropic unchanged even after a change of Claude model: the API drops
  what the new model cannot read.
- Images are sent as base64 of the same bytes every time.
- A turn cut off mid-round (Esc) gets an error result for each call left
  unanswered.

### The core

- **The project gains:**
  - a signature per routine: each parameter's slot, name and type, the
    return, and a note;
  - names for a routine's locals, by slot;
  - struct types, with fields at offsets; a variable's type may name one;
  - comments for the C, by address;
  - C versions per routine: a name, who wrote it (the user or the tutor),
    the text, and its anchors, each tying a range of lines to a range of
    addresses.

  Each goes in its own file in the package (`signatures.json`,
  `structs.json`, `c_versions.json` and so on) through
  `io/project_store.rs`. Projects without them still open.
- **New commands**, each with its inverse on the undo stack:
  `SetSignature`, `SetLocalName`, `SetStruct`, `SetCComment`, `SetCVersion`.
- **A new origin**, `Origin::Tutor { conversation, turn }`.
- **The decompiler reads them:**
  - a signature overrides what `signature.rs` infers;
  - `emit.rs`'s namer uses the local names and struct fields;
  - C comments are printed.
- **A small C lexer** (`decompile/lex.rs`) colours any C text, for versions
  and for code in the chat, with the existing `CTokenKind`s. A matching one
  covers 65816 text.
- **Small exports** the tools need: `code_writing` as "who wrote this", a
  register's fields for a given value, and labels by name.

### The FFI (API 0.12.0)

- **`TutorSession`**, built from a `Workbench`, an optional recording, a
  `CredentialStore` and a `TutorListener`:
  - conversations: start one, resume one, list them;
  - a turn: `send` a question with its attachments and selection, and
    `cancel` it;
  - edit cards: `accept`, `reject`;
  - `rewind` to a turn: the conversation, the edits, or both;
  - set the model, provider and effort; set the mode;
  - `compact`, the cost, the prompt history, and each provider's models.
- **Settings records:** a provider, an endpoint (name, base URL, protocol,
  where its key is, whether it sees images, whether it takes
  `tool_choice`), and the image provider.
- **Workbench commands** for the new annotations, so the app's own sheets
  can make them too.

### The app

- **The `network.client` entitlement**, in `project.yml`.
- **A Settings window** (⌘,):
  - **Providers:** the Anthropic key, the OpenAI key, and local endpoints
    (base URL, protocol, an optional key, vision). Each has a Test button
    that lists its models.
  - **Models:** the default provider, model and effort, and how thinking is
    shown.
  - **Tutor:** the default mode and the cost cap per conversation.
  - **Images:** the image provider and model.
  - **Privacy:** what is sent, and when.

  Keys are kept in the Keychain (`Keychain.swift` over SecItem, one item per
  provider). Everything else is kept in the user defaults.
- **The Tutor window:**
  - A second window controller on the same `ProjectDocument`: it closes with
    the project and shares the main window's model. View › Show Tutor
    (⌥⌘T) opens it, and so does a toolbar button.
  - **The transcript:**
    - Markdown text.
    - Code coloured by the core's lexers with the listing's palettes.
    - **Citations** (`$BB:AAAA`, `frame N`) as chips: a click moves the main
      window there, and hovering shows a peek.
    - Thinking in a collapsed block, and a collapsed tool log per turn.
    - Attached and generated pictures.
    - **Edit cards**, with before and after, Accept and Reject, and Accept
      All.
  - **The composer:**
    - Return sends and ⇧Return starts a new line.
    - ↑ and ↓ at the first and last line walk the prompt history.
    - Shift-Tab changes the mode.
    - Esc stops a turn, and Esc twice opens rewind.
    - A chip names the main window's selection; removing it sends the
      question without it.
    - Pictures and photos can be pasted or dropped. With a recording open,
      a chip attaches the current frame.
    - `/` opens a list of commands.
  - **A status line:** the provider and model, the mode, how full the
    context is, the conversation's cost, and whether the cache was used.
  - **Commands:**
    - `/rewind`: choose an earlier prompt, then whether to rewind the
      conversation, the tutor's edits since then, or both.
      - Edits still on top of the undo stack are undone. Otherwise their
        inverses are applied as one new undoable step, and a conflict with a
        later edit of the user's is reported, not forced.
      - The prompt comes back into the composer.
    - `/resume`, `/new` (also `/clear`), `/model` (provider, model and
      effort), `/mode`, `/compact`, `/cost`, `/attach frame`, `/selection`,
      `/help`.
- **The C tab** gains a strip of versions: Generated, then each version by
  name. A version shows with its anchors, and selecting one of its lines
  selects the addresses it is anchored to. New sheets set a signature, name
  a local, and define a struct.

### The CLI

- `romlens tutor ask <rom> "<question>" [--at <a>] [--rec R] [--provider P] [--model M] [--mode read-only|accept] [--attach <png>] [--resume <id>] [--project P]`
  streams the answer, its tool log and its cost.
- `romlens tutor models [--provider P]`.
- `romlens tutor conversations`.
- `romlens project <p> signature|local|struct|ccomment|cversion …`.
- `romlens decompile` honours the new annotations.

## Ordered tasks

| # | Task | Size (days) |
|---|---|---|
| U0 | This document, rule 8, the pointers, the checklist rows | 0.5 |
| U1 | The crate: transcript, three protocols, SSE, the model table; recorded-stream tests, including a change of provider mid-conversation | 3 |
| U2 | The transport, retries, cancelling, model lists, the loop, cost; `romlens tutor ask` and `models` | 2 |
| U3 | The system prompt, primer, reference pages, ROM digest and cache layout; the read-only tools, each tested on fixture ROMs | 4 |
| U4 | Signatures, locals, structs, C comments, C versions: commands, undo, package files, the decompiler, the C lexer, CLI and goldens | 4 |
| U5 | Edit tools, modes, `Origin::Tutor`, rewinding edits, compaction | 2 |
| U6 | The store, resuming, prompt history; the FFI's `TutorSession`; RomlensKit tests | 2 |
| U7 | The entitlement, the Settings window, the Keychain | 1.5 |
| U8 | The Tutor window | 4 |
| U9 | `/rewind`, `/resume`, `/model`, `/compact`, the cap, attaching a frame | 2 |
| U10 | Image generation; C versions in the C tab; the annotation sheets | 2 |
| U11 | A golden set of questions, live runs on Anthropic, OpenAI and Ollama, the manual pass, and the numbers here | 1.5 |

Each task is one or more commits, pushed.

## What is cut for now

- The Explain and Quiz modes and scene drafts (Phase 4).
- Making media or videos from C versions. The versions are kept for it.
- Sending game assets to an image model, and editing pictures.
- Diagrams drawn from text (Mermaid, SVG) in the chat.
- Branching conversations. A rewind cuts the conversation; the cut turns
  stay on disk but are not shown.
- The Windows and Linux credential stores. The trait is ready for them.
- Conversations kept on a provider's servers.
- **Struct types** (U4). SNES games mostly keep their objects as parallel
  arrays indexed by slot (`$0F00,X` for one field, `$0F20,X` for the next),
  which variables already name; a struct read through a pointer needs the
  decompiler to follow typed pointers first.
- Changing a routine's parameter types in the C: the note and the names
  are the signature for now.

## Risks

- **Three protocols drift.** Each has one request builder and one parser,
  recorded streams test each, and the model table is checked against the
  documentation.
- **Local models are weaker at tools.** The tools are few, with small strict
  schemas. The loop hands errors back to the model. Settings mark models
  without tools or vision, and the tutor says what it cannot do.
- **Confident nonsense.** Facts come from tools, claims are cited, uncited
  ones are shown muted, and the golden set checks both.
- **Cost.** A prefix that stays the same keeps the cache, the status line
  shows the cost, and a cap stops a conversation.
- **Policy.** Rule 8 says what is sent, transcripts stay out of packages,
  and Settings say it plainly.
- **Rewinding edits the user built on.** The inverses are applied as a new
  step, and conflicts are reported.

## Verification

- `make test`:
  - each protocol's parser on recorded streams;
  - the transcript sent to a second provider after a change;
  - the loop on the fake transport: tool rounds, cancelling, the cap, the
    modes;
  - each tool on fixture ROMs and recordings;
  - the new commands, their undo, and a package round trip;
  - `decompile` goldens with a signature, locals and a struct.
- `make swift` and `make app-test`, in the scratch DerivedData while the
  user's app runs:
  - the composer's history and commands;
  - rewinding;
  - the Tutor window with each kind of block;
  - Settings with a stand-in Keychain.
- **Live runs**, only with `ROMLENS_TUTOR_LIVE=1` and keys in the
  environment, never in CI:
  - one question per provider;
  - a change of model mid-conversation;
  - the cache used on the second turn;
  - one picture generated.
- **By hand**, in the Super Mario World project:
  - Ask what RESET does, and follow the citations.
  - Attach a screenshot and ask which layer draws the status bar.
  - In Accept edits mode, have it name a routine and write a C version.
  - `/rewind` those edits.
  - Change from Claude to a local Ollama model and go on.
  - Quit, reopen, `/resume`.
