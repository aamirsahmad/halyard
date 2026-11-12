#!/usr/bin/env bash
# Installs Halyard for the current user (~/.local), from an extracted tarball.
# Uninstall: rm ~/.local/bin/halyard ~/.local/share/applications/halyard.desktop
#            and the halyard icons under ~/.local/share/icons/hicolor
set -euo pipefail
cd "$(dirname "$0")"
PREFIX=${PREFIX:-$HOME/.local}
install -Dm755 halyard "$PREFIX/bin/halyard"
install -Dm644 halyard.desktop "$PREFIX/share/applications/halyard.desktop"
mkdir -p "$PREFIX/share/icons"
cp -R icons/. "$PREFIX/share/icons/hicolor/"
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$PREFIX/share/icons/hicolor" || true
echo "Installed to $PREFIX/bin/halyard"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "Add $PREFIX/bin to your PATH to run 'halyard' from a terminal." ;; esac
