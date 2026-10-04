#!/usr/bin/env bash
# Builds the Linux packages for the architecture this runs on.
#
#   scripts/package-linux.sh [deb] [rpm] [tarball] [all]
#
# Output goes to dist/ with a SHA256SUMS file. GTK is built where it runs, so
# run this on each architecture rather than cross-compiling. Needs the build
# dependencies in the README, plus `cargo install cargo-deb cargo-generate-rpm`.
# The Flatpak and the snap have their own recipes (shells/linux/flatpak,
# snap/snapcraft.yaml).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
shell="$root/shells/linux"
dist="$root/dist"
arch="$(uname -m)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$shell/Cargo.toml" | head -1)"
cargo="${CARGO:-cargo}"

targets=("$@")
[ ${#targets[@]} -eq 0 ] && targets=(all)
want() {
  local t
  for t in "${targets[@]}"; do
    [ "$t" = all ] || [ "$t" = "$1" ] && return 0
  done
  return 1
}

mkdir -p "$dist"
echo "==> release build ($arch)"
(cd "$shell" && "$cargo" build --release --locked)

if want deb; then
  echo "==> .deb"
  (cd "$shell" && "$cargo" deb --no-build --output "$dist/")
fi

if want rpm; then
  echo "==> .rpm"
  (cd "$shell" && "$cargo" generate-rpm --output "$dist/")
fi

if want tarball; then
  echo "==> tarball"
  name="romlens-$version-linux-$arch"
  stage="$(mktemp -d)"
  trap 'rm -rf "$stage"' EXIT
  d="$stage/$name"
  install -Dm755 "$shell/target/release/romlens" "$d/usr/bin/romlens"
  install -Dm644 "$shell/data/io.github.ayodehi.Romlens.desktop" "$d/usr/share/applications/io.github.ayodehi.Romlens.desktop"
  install -Dm644 "$shell/data/io.github.ayodehi.Romlens.metainfo.xml" "$d/usr/share/metainfo/io.github.ayodehi.Romlens.metainfo.xml"
  install -Dm644 "$shell/data/io.github.ayodehi.Romlens.xml" "$d/usr/share/mime/packages/io.github.ayodehi.Romlens.xml"
  install -Dm644 "$shell/data/icons/hicolor/scalable/apps/io.github.ayodehi.Romlens.svg" "$d/usr/share/icons/hicolor/scalable/apps/io.github.ayodehi.Romlens.svg"
  install -Dm644 "$shell/data/icons/hicolor/symbolic/apps/io.github.ayodehi.Romlens-symbolic.svg" "$d/usr/share/icons/hicolor/symbolic/apps/io.github.ayodehi.Romlens-symbolic.svg"
  install -Dm644 "$root/LICENSE" "$d/usr/share/doc/romlens/LICENSE"
  install -Dm644 "$root/THIRD-PARTY-NOTICES.md" "$d/usr/share/doc/romlens/THIRD-PARTY-NOTICES.md"
  install -Dm755 "$shell/data/install.sh" "$d/install.sh"
  tar -C "$stage" --owner=0 --group=0 -czf "$dist/$name.tar.gz" "$name"
fi

(cd "$dist" && shopt -s nullglob && sha256sum *.deb *.rpm *.tar.gz *.flatpak > SHA256SUMS)
echo "==> done"
ls -l "$dist"
