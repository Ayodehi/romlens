#!/usr/bin/env bash
# Local stand-in for the CI matrix: format, lint, test on the host, then
# `cargo check` the whole workspace for the Linux and Windows targets.
# `check` does not link, so no cross linkers are needed; `make docker-test`
# is the real Linux run.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
[ -x "$CARGO" ] || CARGO=cargo
# cargo invokes `rustc` from PATH; with a Homebrew Rust also installed, that
# one lacks the rustup targets, so rustup's bin directory must come first.
[ -d "$HOME/.cargo/bin" ] && export PATH="$HOME/.cargo/bin:$PATH"
TARGETS=(x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-pc-windows-msvc)

cd "$ROOT"
if command -v rustup >/dev/null; then
  rustup target add "${TARGETS[@]}"
fi

echo "==> fmt / clippy / test (host)"
"$CARGO" fmt --all -- --check
"$CARGO" clippy --workspace --all-targets -- -D warnings
"$CARGO" test --workspace --quiet

for target in "${TARGETS[@]}"; do
  echo "==> cargo check --target $target"
  "$CARGO" check --workspace --all-targets --target "$target" --quiet
done
echo "==> cross-check OK"
