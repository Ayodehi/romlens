#!/usr/bin/env bash
# Build the Rust FFI library, generate the Swift bindings, and assemble the
# XCFramework the RomlensKit Swift package wraps.
#
# Output (all git-ignored):
#   bindings/swift/RomlensKit/RomlensFFI.xcframework
#   bindings/swift/RomlensKit/Sources/RomlensKit/RomlensKit.swift
#
# macOS 27 runs on Apple silicon only, so the framework is arm64 only.
# rustup's cargo owns the cross targets, so it is called explicitly.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
[ -x "$CARGO" ] || CARGO=cargo
# cargo invokes `rustc` from PATH; with a Homebrew Rust also installed, that
# one lacks the rustup targets, so rustup's bin directory must come first.
[ -d "$HOME/.cargo/bin" ] && export PATH="$HOME/.cargo/bin:$PATH"
TARGET=aarch64-apple-darwin
PROFILE="${PROFILE:-release}"
PKG="$ROOT/bindings/swift/RomlensKit"
GEN="$PKG/.gen"
LIB="$ROOT/target/$TARGET/$PROFILE/libromlens_ffi.a"
DYLIB="$ROOT/target/$TARGET/$PROFILE/libromlens_ffi.dylib"

cd "$ROOT"
echo "==> cargo build (-p romlens-ffi, $TARGET, $PROFILE)"
"$CARGO" build --quiet -p romlens-ffi --target "$TARGET" --profile "$([ "$PROFILE" = release ] && echo release || echo dev)"

echo "==> uniffi-bindgen-swift"
rm -rf "$GEN"
mkdir -p "$GEN/headers" "$PKG/Sources/RomlensKit"
"$CARGO" run --quiet -p romlens-ffi --bin uniffi-bindgen-swift -- \
  "$DYLIB" "$GEN" \
  --swift-sources --headers --modulemap \
  --module-name RomlensFFI --modulemap-filename module.modulemap
# The plain `module` flavour (not `framework module`) is what a static
# library inside an XCFramework needs, so --xcframework is deliberately absent.
mv "$GEN"/*.h "$GEN/module.modulemap" "$GEN/headers/"
cp "$GEN/RomlensKit.swift" "$PKG/Sources/RomlensKit/RomlensKit.swift"

echo "==> xcodebuild -create-xcframework"
rm -rf "$PKG/RomlensFFI.xcframework"
xcodebuild -quiet -create-xcframework \
  -library "$LIB" -headers "$GEN/headers" \
  -output "$PKG/RomlensFFI.xcframework"

echo "==> done: $PKG/RomlensFFI.xcframework"
echo "    If Xcode keeps an old copy, run: xcodebuild -resolvePackageDependencies, or clear DerivedData."
