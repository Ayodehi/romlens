#!/usr/bin/env bash
# Builds the Flatpak bundle for this architecture: dist/romlens-<arch>.flatpak.
#
#   scripts/build-flatpak.sh [--install]
#
# Needs flatpak, flatpak-builder, python3 (3.11 or newer, for the crate list)
# and the Flathub remote, which the runtime and the Rust SDK extension are
# installed from. --install also installs the result for this user.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dir="$root/shells/linux/flatpak"
arch="$(uname -m)"
app=io.github.ayodehi.Romlens
echo "==> crate sources"
python3 "$root/scripts/cargo-sources.py" "$root/shells/linux/Cargo.lock" -o "$dir/cargo-sources.json"

echo "==> flatpak-builder ($arch)"
mkdir -p "$root/dist"
cd "$dir"
flatpak-builder --user --force-clean --install-deps-from=flathub \
  --state-dir=.flatpak-builder --repo=repo build-dir "$app.yaml"
flatpak build-bundle repo "$root/dist/romlens-$arch.flatpak" "$app"
if [ "${1:-}" = "--install" ]; then
  flatpak install --user -y "$root/dist/romlens-$arch.flatpak"
fi
echo "==> $root/dist/romlens-$arch.flatpak"
