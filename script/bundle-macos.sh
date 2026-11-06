#!/usr/bin/env bash
# Builds a release `Signal GPUI.app` bundle in target/release/bundle.
# Sign/notarize afterwards with your Developer ID, e.g.:
#   codesign --deep --options runtime -s "Developer ID Application: …" "target/release/bundle/Signal GPUI.app"
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --locked -p signal-app
VERSION=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="signal-app"))')
APP="target/release/bundle/Signal GPUI.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/signal-gpui "$APP/Contents/MacOS/signal-gpui"
sed "s/__VERSION__/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"
echo "Built $APP"
