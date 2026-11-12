#!/usr/bin/env bash
# Renders packaging/icons/halyard.svg into the PNG sizes Linux desktops use and,
# on macOS, the Halyard.icns app icon. The outputs are committed, so this only
# needs to run when the SVG changes. Requires `resvg` (cargo install resvg).
set -euo pipefail
cd "$(dirname "$0")/.."
SVG=packaging/icons/halyard.svg
OUT=packaging/icons/hicolor
for size in 16 24 32 48 64 128 256 512; do
  mkdir -p "$OUT/${size}x${size}/apps"
  resvg -w "$size" -h "$size" "$SVG" "$OUT/${size}x${size}/apps/halyard.png"
done
mkdir -p "$OUT/scalable/apps"
cp "$SVG" "$OUT/scalable/apps/halyard.svg"

if command -v iconutil >/dev/null; then
  SET=$(mktemp -d)/Halyard.iconset
  mkdir -p "$SET"
  for size in 16 32 128 256 512; do
    resvg -w "$size" -h "$size" "$SVG" "$SET/icon_${size}x${size}.png"
    resvg -w $((size * 2)) -h $((size * 2)) "$SVG" "$SET/icon_${size}x${size}@2x.png"
  done
  iconutil -c icns -o packaging/macos/Halyard.icns "$SET"
fi
echo "icons rendered"
