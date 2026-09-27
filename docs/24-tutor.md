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
| U4 signatures, local names, structs, C comments and C versions in the project; the decompiler reads them | to do |
| U5 the edit tools, permission modes, `Origin::Tutor`, rewinding edits, compaction | to do |
| U6 the conversation store, resuming, prompt history; the FFI's `TutorSession` | to do |
| U7 the app's network entitlement, the Settings window, the Keychain | to do |
| U8 the Tutor window: transcript, citations, thinking, tool log, edit cards, composer, commands | to do |
| U9 `/rewind`, `/resume`, `/model` mid-conversation, `/compact`, the cost cap, attaching a frame | to do |
| U10 image generation; C versions in the C tab; the annotation sheets | to do |
| U11 measure, record, the manual pass steps | to do |

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
- Per conversation: an append-only `transcript.jsonl`, `attachments/`, and
  `meta.json` (title, models, cost, last change).
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
