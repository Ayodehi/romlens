# The workspace window: views in a sidebar, documents in a grid

## Progress

Written 1 October 2026 and kept as written. This table is the only part
that tracks progress against it.

| Task | State |
|---|---|
| W0 this document, the README row, the pointer from `02`, the checklist rows | done, 1 October 2026 |
| W1 `EditorItem`, `EditorLayout`, `dropZone`; unit tests | done, 1 October 2026: `Model/Workspace/EditorLayout.swift`. `EditorContent` (code in one of five representations, the atlas, compare, source, a graphics or sound view, the tutor; all but code one tab per window, by `singletonKey`), `EditorItem` (id, content, `followsSelection`), `TabGroup`, `LayoutSplit` (axis, children, fractions) and `EditorLayout` with `open` (after the shown tab, or showing the existing one), `select`, `move` (reordering within a group), `split` (a sibling sharing the target's fraction when the parent runs the same way, else a new split; nothing when the target's only tab is dragged onto it), `close` (the right-hand neighbour shown), `equalize`, `setFractions` and `apply` for the four presets, every change leaving no empty group but the last, no split of one and no split inside one along its axis. `dropZone` gives the outer third of each side, the nearer edge in a corner. All `Codable`. Tests: 14 in `EditorLayoutTests`; the app suite is 175 tests |
| W2 the workspace on `RomViewModel`, targeted scrolling, per-tab models | done, 1 October 2026: `Workspace` (`Model/Workspace/Workspace.swift`) holds the layout, the focused group and each code tab's `DecompileModel` and `GraphModel`, and is the only way the window changes the layout, so the focus always names a group. On `RomViewModel`, `editorTab`, `graphicsTab` and `audioTab` are now the focused tab: a code representation changes the focused code tab in place (or the group's, or opens one), a graphics, sound or other view opens or shows its one tab, and clearing a graphics or sound view shows the group's text tab. `decompiler` and `graph` are the focused code tab's. A second code tab with a representation already open does not follow the selection. `ScrollRequest.targets` names the focused tab and every follower; panes learn their tab from the `editorItem` environment value and act only on requests for it, except a new pane, which opens on the selection. C and Graph follow for every shown tab that has focus or follows. History entries remember their tab, and Back and Forward bring it forward. `GraphicsViewState` was not needed: with one tab per graphics view, the viewers' settings stay shared as before. Tests: 5 in `WorkspaceModelTests`; the app suite is 180 tests |
| W3 the grid, tab groups, the representation strip, menus | done, 1 October 2026: `Views/Workspace/EditorGridView.swift` builds the layout as nested `GridSplitView`s (an `NSSplitView` that writes divider drags back as fractions, equalizes on a double-click, and keeps each pane at least 160 points), rebuilt only when the tree's shape changes; each leaf is a `TabGroupView`, a `TabBarView` (`Views/Workspace/TabBarView.swift`: icon, title, close on hover or when shown, a middle click closes, the shown tab joined to the view and edged in the accent colour while its group has focus, Split Right and Split Down at the end, a menu with Close, Close Other Tabs, the splits and Follow Selection) over the tab's view. A tab's view is an `NSHostingView` of `EditorItemBody` made once and kept, hidden, while the tab exists, so scroll and zoom survive switching. A code tab shows the representation strip (Assembly, C, Graph, Hex, Both, and the follow toggle) and is named by the label at or before its place in the bank. A click anywhere in a group gives it focus. `EditorView`, `GraphicsEditorView` and `AudioEditorView` show a given tab's content. The toolbar keeps the navigator button, Back and Forward and gains the inspector button; the editor picker, the Graphics, Audio and Tutor buttons, the address menu and Focus are gone, with `GraphicsMenu` and `AudioMenu`. File gains Close Tab (⌘W; the window when nothing is left) and Close Window (⇧⌘W); View gains Split Right (⌘\\), Split Down (⌥⌘\\), Editor Layout, Next and Previous Tab (⇧⌘] and ⇧⌘[) and Focus Group (⌃1 to ⌃4). Split Right on code opens the tab again beside, as Visual Studio Code does; another view moves. Tests: 5 in `WorkspaceShellTests`, whose snapshots go to the log as base64 with `ROMLENS_SNAPSHOTS=log` because the sandbox's folder cannot be read from outside; the app suite is 185 tests |
| W4 dragging tabs | not started |
| W5 the sidebar | not started |
| W6 the jump bar and Open Quickly | not started |
| W7 the status bar | not started |
| W8 the inspector's Tutor tab; the Tutor window removed | not started |
| W9 the tutor as a tab; citations into tabs | not started |
| W10 the layout kept in `local.json`; narrow windows | not started |
| W11 the shim removed; snapshots; the manual pass | not started |

## Context

The window chooses what it shows from the toolbar: eight segments (Hex,
Disassembly, Both, C, Graph, Source, Atlas, Compare), then Graphics, Audio
and Tutor, then the address columns. Under macOS 26 each is a glass
capsule, four or five of them doing one job, and two kinds of control
doing it. Source and Compare come and go, and below 1180 points the
segments fold into a menu. The navigator puts its four lists behind
another segmented control that shrinks to icons and then a pop-up. The
band with the strip and the percentages sits over every view, the sound
views included, and the tutor is a second window that competes with the
first for room.

`02-ux-proposal.md` chose the three-pane workbench for Phases 0 and 1. With
every view now built, the layout was redrawn on 1 October 2026 as four
options on a design canvas ("Romlens Layout Options",
https://claude.ai/artifact/7njUgwCMW17hRgEFy2eumD), and the one chosen
takes something from three of them:

- The sidebar and toolbar of the first: every view listed in the sidebar,
  grouped by the chip that owns it, and a toolbar of commands only.
- The document tabs of the second: a tab is a thing (a routine, a tilemap,
  a song) and offers only the representations that fit it.
- Those tabs in groups laid out in a grid, arranged by dragging as in
  Visual Studio Code.
- The first's inspector on the right, with the tutor as its second tab,
  and the tutor also openable as a tab in the grid. Both show one
  conversation.

## What already exists

- One `RomViewModel` per document (`Model/RomViewModel.swift`) holds which
  view shows (`editorTab`, `graphicsTab` and `audioTab`, which clear each
  other), one `scrollRequest`, one history, and one each of the decompiler,
  graph, atlas, compare, source, graphics and audio models.
  `refreshDecompile` and `refreshGraph` run only while `editorTab` is C or
  Graph. Nothing about the window is kept between launches.
- Every editor view takes the whole model. The tables keep their scroll in
  their `NSScrollView` and act on the one `scrollRequest`, so two
  disassembly views would both jump; views are rebuilt when the editor's
  switch changes, losing their scroll and zoom.
- `GraphicsModel` holds both the recording (the session, the frame, a live
  session) and each viewer's settings (offset, format, palette, scale).
  `AudioModel` holds the player.
- `WorkbenchSession` is the document's core wrapper and already broadcasts
  every change; the selection and what is worked out from it, the
  navigator's filtering and the inspector's sections need no change.
- `TutorView` needs only a `TutorModel`, but has a minimum width of 380
  points. `TutorWindowController` makes the model; `ProjectDocument` keeps
  the controller. Citations go through `TutorModel.follow` and raise the
  main window.
- `local.json` in the package is machine-local and written only as part of
  a save.

## Scope decisions (1 October 2026)

- The Tutor window goes. ⌥⌘T shows the inspector on its Tutor tab.
- Each project's layout is kept in the package's `local.json`. Changing the
  layout never marks the project edited.
- Code tabs may be opened more than once. The atlas, compare, source, each
  graphics view, each sound view and the tutor have at most one tab in a
  window; opening one again brings its tab forward. This keeps the
  recording and the player shared, as they are now.
- Back and Forward stay one history for the window, as in Visual Studio
  Code; each entry remembers its tab.
- All groups share one selection. A tab follows the selection unless the
  student turns that off for it.

## Design

### The models

An `EditorItem` is one tab: an id, its content, whether it follows the
selection, and the models only it needs, made when first wanted. Content
is one of: code (a representation, Assembly, C, Graph, Hex or Both, and
an anchor), the atlas, compare, a source file, a graphics view, a sound
view, or the tutor. A code tab is titled by the routine or label at its
anchor, the rest by the view's name. A code tab has its own decompiler and
graph models; a graphics tab its own viewer settings, moved out of
`GraphicsModel` into `GraphicsViewState`.

An `EditorLayout` is a tree whose leaves are tab groups (their items and
which is shown) and whose nodes are splits (an axis, children and their
fractions). Every change is a function on the tree: open, move (to a group
at an index), split (a group on an edge, with an item), close (removing a
group left empty and a split left with one child) and equalize.
`dropZone(point, rect)` says where a tab dropped at a point goes: the
middle, or the outer third of a side; in a corner the nearer edge wins. It
is all `Codable`.

`RomViewModel` gains a `Workspace`: the layout and the focused group and
item. While the views move over, `editorTab`, `graphicsTab` and `audioTab`
stay as computed properties over the focused item. A scroll request names
the tab it is for (the focused one unless said), and tabs that follow the
selection also scroll when it moves. The decompiler and graph refresh for
every visible code tab showing C or Graph.

### The window

The grid is AppKit: nested `NSSplitView`s built from the tree, each leaf a
tab group with an AppKit tab bar over a SwiftUI body. Dragging the divider
writes the fractions back; double-clicking it makes the children equal.
AppKit because a drag needs its dragging session, the location as it
moves and Esc to cancel, and because SwiftUI's split views have already
cost a day here (`61a0b72`). Each tab keeps its view alive while it
exists, so scroll and zoom survive switching tabs.

Under the tab bar, the representation strip shows only what fits the tab,
then the view itself, given the model and its item.

Dragging a tab works as in Visual Studio Code. The pasteboard carries a tab
id, or something to open (a row of the sidebar). Over a group, a
translucent overlay in the accent colour shows where the tab will go: the
whole group to move it there, or the half on that side to split. Over a
tab bar, a bar between two tabs shows where it will be inserted, and
within one bar this reorders. The tab being dragged is dimmed where it was.
Esc cancels; nothing changes until the drop.

The sidebar replaces the navigator: Cartridge (Atlas, Header and Vectors,
Compare), CPU (Disassembly, Pseudo-C, Graph, Hex, Source), PPU (the six
graphics views), APU (the seven sound views), Learn (Tutor, Lessons and
Quizzes) and Symbols (labels, variables, regions and banks as outlines).
A view that cannot open yet stays, dimmed, saying what it needs ("needs a
recording", "no folder"). One field at the bottom filters views and
symbols. A row opens in the focused group, or brings its tab forward, and
can be dragged into the grid.

The toolbar keeps the sidebar button, Back and Forward and the inspector
button, and gains a jump bar (chip, view, bank, routine, instruction, each
a menu) and Open Quickly (⇧⌘O), which finds labels, variables,
`$BB:AAAA` addresses, views, lessons and conversations. The address
columns move to the View menu and the column header's menu.

The band becomes a status bar under the grid: the analysis and its
percentages, Cancel and Retry, and the strip as a slim map of the ROM.
The results panel stays under the grid.

The inspector gains a Tutor tab, with a button that opens the tutor as a
tab. "What this does" moves up, under the selection. `TutorModel` moves to
the document, so both places show one conversation; `TutorView` gains a
narrow layout from about 300 points. A citation brings forward, or opens,
the tab it names, and hovering one outlines its lines in every group.

The View menu's items keep their keys and act on the focused group; for
code they change the focused tab's representation. New: Split Right (⌘\),
Split Down (⌥⌘\), Close Tab (⌘W), Close Window (⇧⌘W), Next and Previous
Tab (⇧⌘] and ⇧⌘[), Focus Group 1 to 4 (⌃1 to ⌃4), and Editor Layout:
Single, Two Columns, Two Rows, Three. Closing the last tab leaves an empty
group pointing at the sidebar. Below about 1100 points only the focused
group shows; the layout comes back when the window widens.

### Keeping the layout

`local.json` gains the layout, the focus, which panels show and the
inspector's tab. It is written with every save, and on closing the window
by a coordinated write into the package that leaves the document clean.
Content that no longer exists is dropped on reading. A ROM opened on its
own uses one group until it is saved as a project.

## Ordered tasks

| # | Task | Size (days) |
|---|---|---|
| W0 | This document, the README row, the pointer from `02`, the checklist rows | 0.5 |
| W1 | `EditorItem`, `EditorLayout` and its operations, `dropZone`, all `Codable`; unit tests, nothing on screen changes | 1.5 |
| W2 | The workspace on `RomViewModel` with the computed shim; scroll requests for a tab; per-tab decompiler and graph; `GraphicsViewState`; one tab per view where decided; history entries name their tab | 3 |
| W3 | The grid and tab groups (tab bar, representation strip, focus, the empty group); the editor keyed on its item; the toolbar's pickers removed; the menus on the focused group; split, close and next tab | 3 |
| W4 | Dragging: the pasteboard type, the overlay, inserting and reordering in a tab bar, the dimmed source, empty groups closing, equalizing, dragging sidebar rows | 2.5 |
| W5 | The sidebar; the navigator removed | 2 |
| W6 | The jump bar and Open Quickly; the address columns in the View menu and the column header | 2 |
| W7 | The status bar in place of the band; the results panel under the grid; Focus hides the sidebar, inspector and status bar | 1 |
| W8 | `TutorModel` on the document; the inspector's two tabs; the narrow tutor; the Tutor window removed; ⌥⌘T | 1.5 |
| W9 | The tutor as a tab; citations into tabs; the hover outline in Assembly and Hex | 1.5 |
| W10 | The layout in `local.json`, saved and restored; narrow windows | 1 |
| W11 | Tests and callers off the shim, the shim removed; snapshots of each layout; the manual pass | 1.5 |

Each task is one or more commits, pushed, and leaves `make app-test` green.

## What is cut for now

- More than one tab of a graphics, sound, atlas or compare view.
- Dropping a tab outside the window to open a second window.
- A history per group.
- Saved layouts beyond the four in Editor Layout.

## Risks

- The shim can send a change to the focused tab when another was meant.
  W11 removes it, and no setter of `editorTab` may remain.
- SwiftUI bodies in `NSSplitView` panes: sizes and minimum widths. Read the
  frames from the view tree, as the toolbar's blur was found, not by eye.
- Writing `local.json` outside a save while the document autosaves: use a
  file coordinator and the document's own file access.
- The tutor's sheets (Lessons is 720 points wide) now attach to the main
  window; check them at its minimum width.

## Verification

- `make app-test` after every task.
- Unit tests for every layout operation (the empty group, the split with
  one child, one tab per view), for `dropZone` in each zone, at the corners
  and at the thresholds, and a `Codable` round trip.
- Shell tests on an off-screen window, through the model, not real drags:
  two code tabs at different routines, where a jump scrolls only its tab
  and a following hex tab follows; every drop through the entry point the
  drop handler calls; ⌥⌘T showing the inspector's tutor, and a citation
  bringing the right tab forward; a layout saved, reopened and restored,
  with the document clean after a layout change.
- Snapshots with `ROMLENS_SNAPSHOTS=1`: one group, two columns with the
  tutor as a tab, three groups with the inspector's tutor, and a 1000-point
  window, compared with the canvas.
- The manual pass, steps 74 to 79 in `15-conformance-checklist.md`.
