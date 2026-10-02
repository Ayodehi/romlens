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

# The tutor's HTTPS (ureq → rustls → ring) compiles C in its build script,
# even for `check`, so the crates above it need a C compiler for the
# target. Set CC_<target> (for example CC_x86_64_unknown_linux_gnu) to one
# to check everything; without one, the crates that need no C are checked
# and CI's native Linux and Windows jobs cover the rest.
NO_C=(-p romlens-core -p romlens-draw)
for target in "${TARGETS[@]}"; do
  cc_var="CC_${target//-/_}"
  if [ -n "${!cc_var:-}" ] || command -v "${target/-unknown/}-gcc" >/dev/null; then
    echo "==> cargo check --target $target"
    "$CARGO" check --workspace --all-targets --target "$target" --quiet
  else
    echo "==> cargo check --target $target (romlens-core, romlens-draw: no C compiler for the target, set $cc_var to check all)"
    "$CARGO" check "${NO_C[@]}" --all-targets --target "$target" --quiet
  fi
done
echo "==> cross-check OK"
