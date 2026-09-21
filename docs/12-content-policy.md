# Content policy: ROMs, third-party work, Nintendo, and distribution logistics

Researched 21 September 2026. This is a policy grounded in what we found,
not legal advice.

## Framing

Romlens is a study and visualization tool first. Through Phase 4 it runs no
game code; the Phase 5 debugger track adds an embedded core for stepping a
ROM the user supplies. It never ships or fetches a ROM, and does not need a
commercial ROM to be useful: its natural purpose includes helping people develop their own
SNES ROMs for real hardware or an emulator, and everything in it works on a
homebrew build. Recordings come from outside in a published format. The
developer's own test ROMs are dumps of cartridges they own; the app does not
ask users where theirs came from.

## What we found

- **Nintendo's position.** Its Intellectual Property and Piracy FAQ says
  unauthorised emulators "encourage the use of unauthorized (i.e. pirate)
  copies of games" and that downloading ROMs is illegal regardless of
  ownership; it does not address backups or fan projects. Its Game Content
  Guidelines for video and image sharing (updated 2 September 2024) permit
  gameplay videos that include the creator's own creative input and
  commentary, forbid mere copies of game content, and reserve the right to
  revoke permission. They do not mention code or asset display.
- **Enforcement pattern.** The large GitHub takedowns of 2023 to 2024
  targeted Switch emulators and key tools under DMCA section 1201
  (circumvention of encryption), for example the yuzu notice that removed a
  network of 8,535 repositories. The SNES has no encryption, so that legal
  theory does not apply to us. Decompilation projects that host no assets
  and require the user's own ROM (sm64, the N64 decomp organisation, a
  Donkey Kong Country SNES disassembly) have persisted for years on GitHub.
  Fan games that ship assets have not.
- **Third-party work we rely on.** PJBoy's Super Metroid bank logs at
  patrickjohnston.org carry **no license statement** on the index or bank
  pages we scanned; treat them as publicly viewable, all rights reserved.
  A relocatable disassembly derived from them (InsaneFirebat/sm_disassembly)
  is published under 0BSD and requires the user's own ROM, but its
  derivative status is unclear, so we do not lean on its license. Mesen2 and
  bsnes are GPLv3 and we use them out of process. Ares, LakeSnes and
  super-sabicom are permissive (see `09-emulator-core-licensing.md`).

## Policy

**ROM images**

1. The repository, releases, project files, recordings we publish and CI
   never contain a ROM or any byte range copied from one. ROMs live in the
   git-ignored `roms/` folder, and every ROM file extension is ignored
   repository-wide as a second line of defence. Tests that need a
   commercial ROM are opt-in, keyed by SHA-256, and read from
   `ROMLENS_ROM_DIR`; CI uses a small homebrew test ROM the test suite
   builds itself, under 0BSD.
2. The app never downloads a ROM, never suggests where to get one, and never
   copies a ROM into a project package. Project files store a hash, a
   remembered path and annotations.
3. Exported disassembly listings contain the ROM's bytes by nature. The
   export dialog says so and defaults to a label-and-comment export without
   raw bytes for sharing; a full listing is for local use.

**Recordings and graphics**

4. Recordings contain VRAM, CGRAM and OAM snapshots and framebuffers, which
   are copyrighted assets. Recordings are local by default, are excluded
   from project packages meant for sharing, and the export path warns.
5. Decoded tiles and frames are displayed in the app for study. The app has
   no "export sprite sheet" feature; that is what ROM hacking tools are for.

**Third-party disassemblies and symbols**

6. PJBoy's logs are used locally by the developer to measure analyzer
   accuracy; the ground-truth test downloads nothing and reads a local copy
   the developer obtained themselves. We do not redistribute the logs or
   symbols derived from them without written permission from the author,
   which we should ask for; a symbol pack under an explicit license would be
   a large win for first-run readability.
7. Imported symbol files keep their own license notices in the project.

**The tutor**

8. The Anthropic API key is always the user's own, entered in settings and
   kept in the platform credential store; Romlens ships no key and runs no
   proxy. Only the selection window and tool results leave the machine,
   only on the student's request, and never a bulk upload of the ROM or a
   recording. Settings say this plainly.

**Published educational content**

9. Videos and documents made with the app lead with commentary and
   explanation; code excerpts are shown as the subject of teaching, asset
   display is incidental and minimal, and nothing offers a playable or
   extractable copy of the game. Content states it is unaffiliated with and
   not endorsed by Nintendo. This follows the spirit of Nintendo's creator
   guidelines and the fair-use factors, and it is still a risk we accept
   knowingly rather than a permission we hold.

**If a takedown arrives**

10. GitHub notifies the owner before disabling and offers the counter-notice
    process. Keep the repository mirror-able (no GitHub-only state), keep a
    copy of every release, and keep this policy in the repository so the
    project's posture is documented in advance.

## Distribution logistics for a free, open-source project

| Platform | Finding | Plan |
|---|---|---|
| macOS | Notarization requires the paid Apple Developer Program; there is no free path. | The developer already has an account. Sign, notarize, ship a .dmg and a Homebrew cask. No Mac App Store. |
| Windows | SignPath Foundation signs open-source releases for free (OV certificate held on their HSM; publisher shows "SignPath Foundation"; requires an OSS license, active maintenance, a released build and documented functionality). | Apply once the first Windows release exists. MSIX via winget; unsigned zip as the stopgap with a SmartScreen warning. |
| Linux | Flathub accepts `io.github.<user>.<App>` IDs verified through the repository URL; no domain required. | Flathub first, then .deb and .rpm from CI. |
| Everywhere | Free and open source removes the sales, support and domain questions. | Releases from CI, signed on macOS and Windows, Flatpak on Linux. |
