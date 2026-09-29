# Diagrams: pictures Romlens draws for the tutor and its lessons

## Progress

Written 28 September 2026 and kept as written. This table is the only part
that tracks progress against it.

| Task | State |
|---|---|
| D0 this document, the pointers from `24` and `25`, the checklist rows | done, 28 September 2026 |
| D1 `romlens-draw`: fonts, the SVG writer, rendering, the validation pass | done, 28 September 2026: `crates/romlens-draw`. `fonts`: IBM Plex Sans (Regular, SemiBold) and Mono built in, every family an SVG names mapped to one of them and no fallback to the system's, `measure` for layout and `has_char`. `svg`: a writer (boxes, text, lines, dashed lines, arrows) in one palette whose contrasts a test measures. `check`: the five checks, each problem a line the model can act on. `render`: the checks, then `resvg` onto the paper at twice the display size (at most 720 points wide; answers and lessons scroll, so the height is not capped), as a PNG with real deflate. The new crates and the fonts are in `THIRD-PARTY-NOTICES.md`, all MIT, Apache-2.0, BSD, Zlib or OFL |
| D2 the diagram kinds, fed by the core | done, 28 September 2026: `romlens_draw::kinds::draw(kind, spec, source)`, where a `Source` gives the ROM, names, a recording frame's events and a pixel's chain. `fields`: the bits coloured by field, a bracket and name (or key letter) under each, a row per field with its bits, its name and what the value sets (or what it can be), unused bits said; a register or pair by name or address; one that is a single number, or data, is refused. `memory_map`: a bank as a column (not to scale) with each region's size and what it holds (a ROM region's file offsets), marks on the right with arrows, a vector marked says where it points; or the whole space, banks with the same layout as one column and `as $00–$3F` over a mirror. `blocks`: the machine (Romlens's own drawing, its arrows routed along its wires and buses and numbered, with a list under it; DMA from WRAM goes by the A bus), or free boxes laid out by `graph::layout`, turned to the other direction when too wide. `timeline`: a frame from the NMI through vertical blank and the picture, or a line's dots and horizontal blank, with marks, and a recording's events in lanes placed right to left so no line crosses a label. `chain`: up to eight steps in rows of four, each place checked (an address, a name, or a memory such as VRAM). In the core: `hardware_register_named` (names before hex, and the tutor's `reference` now finds `BBAD0`), `fields::layout_named`, `AddressMap::regions` and `timing`. Every kind's examples are drawn in the tests and must pass the checks; `ROMLENS_DRAW_OUT` writes them out to look at |
| D3 `draw_diagram` and `draw_svg`, the prompt | done, 28 September 2026: `tutor::draw` in the FFI. Each tool gives back the words (what the picture shows, and its id for a lesson step) and the PNG, whose id starts `draw-`; `lesson_step` takes one, and it is copied into the lesson like a tool's picture. A recording frame's timeline has its DMA starts (channel, source, destination by B-bus register, size) and the CPU's and HDMA's register writes, the DMA's own port writes left out, a register written four times or more as one stretch, at most twelve events with DMA kept first; a pixel's chain comes from `pixel_provenance`. A spec may be an object or its JSON as a string. A drawing of Romlens's own that fails the checks says it is Romlens's fault, not the spec's. Both are reads. The prompt says to draw with `draw_diagram` where a picture explains better than words, `draw_svg` only when no kind fits and `generate_image` only when asked, and a level 1–2 step about the machine's parts, memory or timing shows a diagram. Tests: every kind through `RomTools::run`, a bad spec and an unsafe SVG, a recorded frame and pixel, a lesson step keeping its diagram, and a session on the loopback server whose lesson carries one |
| D4 `romlens draw` and `romlens draw check` | planned |
| D5 app: diagrams in answers and lesson cards | planned |
| D6 live runs | planned |

## Context

Lessons (`25-lessons.md`) are all text. For beginner topics a picture teaches
more than text: how the CPU and the PPU share the work, a bank's memory map,
a register's bits, a frame's timeline, the path from ROM bytes to the
screen. The only picture tool so far, `generate_image`, sends words to an
outside image model. Its pictures are not checked against anything and it
needs an image provider.

So Romlens draws diagrams itself, from a small description the model
writes. Where a diagram shows the hardware, its facts come from the core's
own tables, as every other tutor tool's do: a drawing of INIDISP cannot put
forced blank on the wrong bit. Romlens does the layout, so text never
overflows its box and labels never collide.

For a picture no kind fits, the model can write SVG. Romlens parses it
strictly, refuses anything unsafe, checks the layout and draws it, and the
picture goes back to the model so it sees what it made before a lesson uses
it.

Mermaid was considered and left out. It needs a JavaScript engine in every
shell, it has none of the kinds SNES lessons need (memory maps, bit fields,
timelines), and checking it only proves the source parses.

## The tools

- `draw_diagram(kind, spec)`: one of the kinds below, drawn from the spec.
- `draw_svg(svg, title)`: the model's own SVG, checked and drawn.

Both return the picture (a PNG) and a text part saying what was drawn, with
addresses, so a model that cannot see pictures still knows what it shows.
Picture ids start `draw-`. A lesson step can carry one as its picture, and
it is kept with the lesson like any other.

Both tools are local and deterministic, so they are reads: they run
alongside the other reads, in every permission mode.

## The kinds

A spec is JSON. Every property is present, and optional ones are `null`.

| Kind | Spec | What it draws | Facts from |
|---|---|---|---|
| `fields` | `register` (a name or `$21xx`), `value`, `highlight` (field names) | The register's bits, 7 to 0 (15 to 0 for a pair), each field spanning its bits with its name; with a value, its bits filled in and each field's meaning underneath | `explain::fields`, `model::hardware` |
| `memory_map` | `view` (`bank` or `banks`), `bank`, `marks` (address and label), `highlight` (ranges) | One bank as a column (low RAM, the B-bus ports, the CPU and DMA registers, ROM or SRAM), or the whole 24-bit space for this ROM's mapping; a mark on a vector says where it points | `memory::map`, the header's vectors, the project's labels |
| `blocks` | `preset` (`machine`), `nodes`, `edges`, `direction`, `highlight` | Boxes and arrows. `machine` is a fixed drawing of the SNES: the S-CPU, the A and B buses, the PPUs with VRAM, OAM and CGRAM, WRAM, the cartridge, the S-SMP and DSP with ARAM behind the four ports | The preset is Romlens's; free boxes are the model's, laid out by `graph::layout` |
| `timeline` | `span` (`frame` or `line`), `frame`, `marks` | A frame's 262 lines: the visible ones, vblank and the NMI at its start; or a line's 341 dots and hblank. With a recording's frame, its DMA and register writes placed on their lines | `timing`; the recording's `dma_records` and `line_writes` |
| `chain` | `from` (frame, x, y), `steps` (label and place) | How bytes reach the screen, left to right: ROM, the code or DMA that moved them, VRAM, OAM or CGRAM, the pixel | `provenance::chain` with a recording; the model's steps otherwise, each place checked as a lesson focus is |

## The checks

Every picture, Romlens's own and the model's, passes the same checks before
it is drawn. A test draws every kind's examples through them, so Romlens's
layout is held to the same standard as the model's SVG. A failed check is an
error listing each problem, which the model fixes and sends again.

1. Size and structure: at most 64 KB of well-formed XML, with a `viewBox`
   of at most 1,600 × 1,200 and an aspect between 1:3 and 3:1.
2. Nothing unsafe: no `script`, `foreignObject`, `image` or `iframe`, no
   animation, no `on…` attributes, no `@import`, and no `href` but a local
   `#fragment`.
3. On the canvas: every element's box inside the `viewBox`.
4. Text: laid out with Romlens's fonts (any family becomes sans or mono);
   no two texts overlapping; at least 11 px at display size; at least 3:1
   contrast against what it sits on.
5. Not empty: something visible, and some text.

## The style

- Drawn on its own paper: a light card with a thin border and rounded
  corners, like a figure in a book. It reads the same in a light or a dark
  window.
- A small palette: ink, muted, an accent for what is highlighted, and a few
  fills for regions, all chosen for contrast on the paper.
- Fonts shipped with Romlens, IBM Plex Sans and IBM Plex Mono (SIL Open
  Font License 1.1), so the layout and the pixels are the same on every
  machine. Addresses and values are in mono.
- SVG is the one format. It is drawn with `resvg` (Apache-2.0 or MIT, pure
  Rust) at a long side of about 1,200 pixels, and shown at half that in
  points so it is sharp on Retina screens.

## Content policy

A diagram is Romlens's own drawing of the machine, with names and addresses
and no game graphics or ROM bytes. So, unlike the tutor's pictures of game
graphics (content policy rules 5 and 8), `romlens draw` may write one to a
file.

## Ordered tasks (one commit each, pushed to main)

| # | Task | Days |
|---|---|---|
| D0 | This document; pointers from `24` and `25`; checklist rows in `15` | 0.25 |
| D1 | `crates/romlens-draw`: the fonts, measuring text, the SVG writer, rendering, the checks. Tests: each check refuses a bad SVG and passes a good one; a render is a PNG of the expected size | 1.5 |
| D2 | The kinds, and in the core `hardware_register_named` (names before hex, which also fixes `reference` reading `BBAD0` as an address), `memory::map::regions` and `timing`. Tests: every kind's examples draw, pass the checks and describe the right facts | 2 |
| D3 | The tools, a `draw-` picture in a lesson step, the prompt's lines. Tests through `RomTools::run` and a session on the loopback server | 1 |
| D4 | `romlens draw <rom> <kind> <spec>` and `romlens draw check <svg>`; a golden of each kind's SVG | 0.5 |
| D5 | App: diagrams large in answers with their label, smoothly scaled in answers and lesson cards; tests and snapshots | 0.75 |
| D6 | Live: in SMW with Explain on, "how do the CPU and the PPU work together?" (the machine), "where does the game start?" (a memory map with the reset vector), "what does forced blank do?" (INIDISP's fields), and with a recording "when does the game copy sprites to the PPU?" (a timeline). Record what went wrong here | 0.5 |

## What is cut for now

- Drawings in the window's dark colours (the paper reads in both).
- Animated diagrams and scenes (`01`, Phase 4).
- Exporting diagrams from the app.
- Mermaid.
- Editing a drawn diagram by hand: the model sends a new spec.
