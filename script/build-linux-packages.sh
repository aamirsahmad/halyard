#!/usr/bin/env bash
# Builds the Linux release packages for this machine's architecture into dist/:
#   Halyard-<version>-<arch>.AppImage   download, chmod +x, run
#   halyard_<version>_<debarch>.deb      Debian/Ubuntu package
#   halyard-<version>-linux-<arch>.tar.gz  binary, desktop file, icons, install.sh
# Needs the packages from script/install-linux-deps.sh plus `file` and `curl`.
# Build on an old distribution (Ubuntu 22.04) so the result runs on newer ones.
set -euo pipefail
cd "$(dirname "$0")/.."

ARCH=$(uname -m) # x86_64 | aarch64
case "$ARCH" in
  x86_64) DEBARCH=amd64 ;;
  aarch64) DEBARCH=arm64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 1 ;;
esac
PROTOC_VERSION=29.3
APPIMAGETOOL_VERSION=1.9.0

# Ubuntu 22.04's protoc is too old for libsignal-service's protos.
if ! protoc --version 2>/dev/null | grep -qE 'libprotoc (2[3-9]|[3-9][0-9])'; then
  PROTOC_DIR=$(mktemp -d)
  case "$ARCH" in x86_64) P=x86_64 ;; aarch64) P=aarch_64 ;; esac
  curl -sSfL -o "$PROTOC_DIR/protoc.zip" \
    "https://github.com/protocolbuffers/protobuf/releases/download/v$PROTOC_VERSION/protoc-$PROTOC_VERSION-linux-$P.zip"
  unzip -q "$PROTOC_DIR/protoc.zip" -d "$PROTOC_DIR"
  export PROTOC="$PROTOC_DIR/bin/protoc"
fi

cargo build --release --locked -p signal-app
BIN=${CARGO_TARGET_DIR:-target}/release/halyard
VERSION=$(cargo metadata --no-deps --format-version 1 |
  python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="signal-app"))')
DIST=dist
mkdir -p "$DIST"
WORK=$(mktemp -d)

# Installs the app into a FHS-style root ($1/usr/...).
install_tree() {
  local root=$1
  install -Dm755 "$BIN" "$root/usr/bin/halyard"
  install -Dm644 packaging/linux/halyard.desktop "$root/usr/share/applications/halyard.desktop"
  mkdir -p "$root/usr/share/icons"
  cp -R packaging/icons/hicolor "$root/usr/share/icons/"
  install -Dm644 LICENSE "$root/usr/share/doc/halyard/copyright"
}

# --- tarball -------------------------------------------------------------
TARDIR="$WORK/halyard-$VERSION-linux-$ARCH"
mkdir -p "$TARDIR"
install -m755 "$BIN" "$TARDIR/halyard"
install -m644 packaging/linux/halyard.desktop LICENSE README.md "$TARDIR/"
cp -R packaging/icons/hicolor "$TARDIR/icons"
install -m755 packaging/linux/install.sh "$TARDIR/install.sh"
tar -C "$WORK" -czf "$DIST/halyard-$VERSION-linux-$ARCH.tar.gz" "halyard-$VERSION-linux-$ARCH"

# --- .deb ----------------------------------------------------------------
DEBROOT="$WORK/deb"
install_tree "$DEBROOT"
mkdir -p "$DEBROOT/DEBIAN"
SIZE=$(du -sk "$DEBROOT/usr" | cut -f1)
cat >"$DEBROOT/DEBIAN/control" <<CONTROL
Package: halyard
Version: $VERSION
Architecture: $DEBARCH
Maintainer: Halyard developers
Installed-Size: $SIZE
Depends: libc6 (>= 2.35), libasound2t64 | libasound2 (>= 1.2.6), libxkbcommon0, libxkbcommon-x11-0, libxcb1, libvulkan1, libwayland-client0
Recommends: mesa-vulkan-drivers
Section: net
Priority: optional
Homepage: https://github.com/aamirsahmad/halyard
Description: Unofficial desktop client for the Signal messenger
 Halyard is a native Signal client for Linux built with GPUI. It links to
 your phone as a secondary device. Not affiliated with Signal.
CONTROL
dpkg-deb --build --root-owner-group "$DEBROOT" "$DIST/halyard_${VERSION}_${DEBARCH}.deb" >/dev/null

# --- AppImage ------------------------------------------------------------
APPDIR="$WORK/Halyard.AppDir"
install_tree "$APPDIR"
ln -s usr/bin/halyard "$APPDIR/AppRun"
cp packaging/linux/halyard.desktop "$APPDIR/halyard.desktop"
cp packaging/icons/hicolor/256x256/apps/halyard.png "$APPDIR/halyard.png"
ln -s halyard.png "$APPDIR/.DirIcon"
TOOL="$WORK/appimagetool"
curl -sSfL -o "$TOOL" \
  "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-$ARCH.AppImage"
chmod +x "$TOOL"
ARCH=$ARCH APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream "$APPDIR" \
  "$DIST/Halyard-$VERSION-$ARCH.AppImage"

(cd "$DIST" && sha256sum "Halyard-$VERSION-$ARCH.AppImage" "halyard_${VERSION}_${DEBARCH}.deb" \
  "halyard-$VERSION-linux-$ARCH.tar.gz" >"halyard-$VERSION-linux-$ARCH.sha256")
rm -rf "$WORK"
ls -la "$DIST"
