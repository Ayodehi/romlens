#!/bin/bash
# Record a game from power-on in a headless Mesen and dump its screen at
# chosen frames, then compare each with the frame Romlens composes.
#
#   scripts/oracle/run.sh MESEN_APP ROM OUT_DIR FRAMES LIMIT
#
# MESEN_APP  a Mesen.app to run: use a copy in portable mode (an empty
#            settings.json beside Contents/MacOS/Mesen, then
#            `codesign --force --deep -s - Mesen.app`), so the installed
#            Mesen's home folder is never touched
# FRAMES     comma-separated frame numbers to dump and compare
# LIMIT      frames to record
#
# Development only (docs/22, P1): nothing here runs in CI, and the dumps
# stay in OUT_DIR.
set -euo pipefail
app=${1:?Mesen.app}; rom=${2:?ROM}; out=${3:?output folder}; frames=${4:?frames}; limit=${5:?frame limit}
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
romlens="$repo/target/release/romlens"
mkdir -p "$out"
cat "$repo/crates/romlens-core/src/recording/mesen/mesen_recorder.lua" "$here/shots.lua" > "$out/oracle.lua"
# Frame skipping would leave older pictures in the screen buffer; zeroed RAM
# makes two runs alike.
ROMLENS_REC_OUT="$out/rec.rlstream" ROMLENS_REC_FRAMES=$limit ROMLENS_SHOT_DIR="$out" ROMLENS_SHOT_FRAMES=$frames \
  perl -e 'alarm shift; exec @ARGV' 600 "$app/Contents/MacOS/Mesen" --testRunner --doNotSaveSettings \
  --debug.scriptWindow.allowIoOsAccess=true --snes.ramPowerOnState=AllZeros --snes.disableFrameSkipping=true \
  "$rom" "$out/oracle.lua" > "$out/mesen.log" 2>&1
"$romlens" rec pack "$out/rec.rlstream" --rom "$rom" --out "$out/rec.romrec" > "$out/pack.log"
for f in ${frames//,/ }; do
  printf '%s: ' "$f"
  "$romlens" render frame --rec "$out/rec.romrec" --frame "$f" --against "$out/shot-$f.bin" \
    | grep -E 'against|not drawn' | sed 's/against 256x239 dump //' | tr '\n' ' '
  echo
done
