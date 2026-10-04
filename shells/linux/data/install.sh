#!/bin/sh
# Installs Romlens from this tarball: ./install.sh [PREFIX]   (default /usr/local)
# Needs GTK 4.14, libadwaita 1.6, libsecret and ALSA installed. Uninstall by
# deleting what this lists.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
prefix="${1:-/usr/local}"
cd "$here/usr"
find . -type f | while read -r f; do
  install -Dm"$([ -x "$f" ] && echo 755 || echo 644)" "$f" "$prefix/$f"
  echo "$prefix/$f"
done
command -v update-desktop-database >/dev/null && update-desktop-database "$prefix/share/applications" || true
command -v update-mime-database >/dev/null && update-mime-database "$prefix/share/mime" || true
command -v gtk4-update-icon-cache >/dev/null && gtk4-update-icon-cache -qtf "$prefix/share/icons/hicolor" || true
