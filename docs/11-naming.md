# Naming

**Decided 21 September 2026: the project is Romlens.** "Cartograph" was the
working name in revisions 1 to 3 of the proposal and appears below only as
the name that was researched and rejected. Identifiers: `io.github.<user>.Romlens`,
crates `romlens-core`, `romlens-ffi`, `romlens-cli`, CLI `romlens`, project
package `.romlens`, recording `.romrec`. The GitHub account is not chosen
yet, so `<user>` is the literal `placeholder` in code until it is; it is
replaced once before the first public release.

Researched 21 September 2026. The project is free, open source, has no
domain, no store listing and no support channel. That changes what a name
must do: it does not need to be a trademark, it needs to be findable on
GitHub and unambiguous next to what already exists.

## "Cartograph" is taken in the ways that matter

- GitHub name search returns about 4,700 repositories matching
  `cartograph`, including a 7,973-star SLAM project (Cartographer), a
  7,320-star Swift layout library (Cartography), an organisation named
  CartographAI, a task runner, an HTTP proxy and a map tool all named
  exactly Cartograph.
- **Cartograph CF** is a commercial monospaced coding font sold through
  Adobe Fonts. A code-viewing application called Cartograph, whose main
  screen is monospaced code, would be confused with it in search and in
  conversation. This is the collision that matters.
- USPTO records were not checked from here; for a free non-commercial tool
  the trademark exposure is low, but the confusion is real regardless.

Recommendation: rename before any bundle identifier, file extension or
Flatpak ID exists.

## Identifiers without a domain

- Flathub accepts code-hosting IDs: `io.github.<user>.<App>` with the
  repository URL reachable; underscores in the ID map to dashes in the
  GitHub path. No domain needed.
- The macOS bundle identifier and the Windows package identity can use the
  same reverse-DNS string.
- The project file extension and recording extension should derive from
  the name and be short.

## Candidates checked

GitHub name-search counts on 21 September 2026 (`<name> in:name`):

| Name | Repos | Notes |
|---|---|---|
| romlens | 1 (unrelated shop) | Descriptive across consoles, pronounceable, reads as a tool for looking into ROMs. Extension `.romlens`, ID `io.github.<user>.Romlens`. |
| sfcope | 0 | Clever (SFC + scope) but hard to say and SNES-only. |
| snestudy | 0 | Clear, but SNES-only and the plan says other consoles later. |
| bankwalk | 2 (unrelated) | Evocative of walking banks; slightly opaque to newcomers. |
| hexplain | 10 (small) | Nice pun, but says hex, not ROMs, and is taken a few times. |
| tilewalk / bytewalk | 4 / 11 | Taken, including a 152-star ByteWalk. |
| romatlas / lorom | no result returned | LoROM is a technical term and would confuse; romatlas unverified. |

## Recommendation

**Romlens.** Descriptive, console-neutral, essentially uncollided, works as
an identifier everywhere (`io.github.<user>.Romlens`, `romlens-core`,
`romlens` CLI, `.romlens` project package, `.romrec` recording). Verify
`romlens` is free on crates.io before the first push, and do a two-minute
USPTO TESS search for completeness.

If the user prefers the map metaphor, "Atlas" variants are all heavily
taken; "Cartograph" should not be kept.
