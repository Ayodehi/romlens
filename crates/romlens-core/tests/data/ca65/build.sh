#!/bin/sh
# Rebuild the fixture with cc65 (https://cc65.github.io). The outputs are
# committed so the tests need no toolchain.
set -e
cd "$(dirname "$0")"
ca65 -g -o main.o main.s
ca65 -g -o palette.o palette.s
ld65 -C lorom.cfg --dbgfile fixture.dbg -o fixture.sfc main.o palette.o
rm -f main.o palette.o
