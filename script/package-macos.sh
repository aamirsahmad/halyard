#!/usr/bin/env bash
# Builds a universal (Apple silicon + Intel) Halyard.app and packages it as
# dist/Halyard-<version>-macos-universal.{dmg,zip}.
#
# Signing (optional, recommended for distribution):
#   MACOS_SIGN_IDENTITY="Developer ID Application: Name (TEAMID)"
# Notarization (optional, needs signing):
#   APPLE_ID=you@example.com APPLE_TEAM_ID=TEAMID APPLE_APP_PASSWORD=app-specific-password
# Without an identity the app is ad-hoc signed: it runs, but Gatekeeper asks the
# user to confirm on first launch (see README).
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(cargo metadata --no-deps --format-version 1 |
  python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="signal-app"))')
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

for t in "${TARGETS[@]}"; do
  rustup target add "$t" >/dev/null
  case "$t" in aarch64-*) platform=mac-arm64 ;; x86_64-*) platform=mac-x64 ;; esac
  # prebuilt WebRTC (voice calls) for this architecture
  OUTPUT_DIR="$(script/fetch-webrtc.sh "$platform")" \
    cargo build --release --locked -p signal-app --target "$t"
done

DIST=dist
APP="$DIST/Halyard.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/halyard" \
  target/aarch64-apple-darwin/release/halyard target/x86_64-apple-darwin/release/halyard
sed "s/__VERSION__/$VERSION/g" packaging/macos/Info.plist >"$APP/Contents/Info.plist"
cp packaging/macos/Halyard.icns "$APP/Contents/Resources/Halyard.icns"
cp LICENSE "$APP/Contents/Resources/LICENSE"

if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
  codesign --force --options runtime --timestamp \
    --entitlements packaging/macos/entitlements.plist \
    --sign "$MACOS_SIGN_IDENTITY" "$APP"
else
  echo "warning: no MACOS_SIGN_IDENTITY, ad-hoc signing (users will need to approve the app)"
  codesign --force --sign - --entitlements packaging/macos/entitlements.plist "$APP"
fi
codesign --verify --strict "$APP"

BASE="$DIST/Halyard-$VERSION-macos-universal"
rm -f "$BASE.zip" "$BASE.dmg"
ditto -c -k --keepParent "$APP" "$BASE.zip"

STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -quiet -volname "Halyard $VERSION" -srcfolder "$STAGE" -fs HFS+ -format UDZO "$BASE.dmg"
rm -rf "$STAGE"
if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
  codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" "$BASE.dmg"
fi

if [[ -n "${MACOS_SIGN_IDENTITY:-}" && -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_PASSWORD:-}" ]]; then
  xcrun notarytool submit "$BASE.dmg" --wait \
    --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD"
  xcrun stapler staple "$BASE.dmg"
  xcrun stapler staple "$APP"
  rm -f "$BASE.zip"
  ditto -c -k --keepParent "$APP" "$BASE.zip"
fi

(cd "$DIST" && shasum -a 256 "$(basename "$BASE").dmg" "$(basename "$BASE").zip" >"$(basename "$BASE").sha256")
echo "Built $BASE.dmg and $BASE.zip"
