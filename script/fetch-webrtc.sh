#!/usr/bin/env bash
# Downloads Signal's prebuilt WebRTC for RingRTC (voice calls) and checks its SHA-256.
#
#   script/fetch-webrtc.sh [platform]   # mac-arm64, mac-x64, linux-x64 or linux-arm64
#
# Output: .webrtc/<platform>/{release,debug}/obj/libwebrtc.a (outside target/, so that
# `cargo clean` and CI build caches leave it alone)
# With no argument, it fetches the host platform and links .webrtc/host to it.
# .cargo/config.toml sets OUTPUT_DIR=.webrtc/host for RingRTC's build script.
# For other targets, set OUTPUT_DIR=.webrtc/<platform>.
# The version and checksums come from RingRTC v2.75.0 (config/webrtc_artifact_checksums.json).
set -euo pipefail

WEBRTC_VERSION=8037e
STAMP="$WEBRTC_VERSION-1" # bump when the local processing changes
platform="${1:-}"
host=0
if [[ -z "$platform" ]]; then
  case "$(uname -s)" in Darwin) os=mac ;; Linux) os=linux ;; *) echo "unsupported OS" >&2; exit 1 ;; esac
  case "$(uname -m)" in x86_64|amd64) arch=x64 ;; arm64|aarch64) arch=arm64 ;; *) echo "unsupported CPU" >&2; exit 1 ;; esac
  platform="$os-$arch"
  host=1
fi
case "$platform" in
  mac-arm64) sum=d1f4bdc1decea09fe1f64222e77ea71e6024541039323d6cf1e05a5635a7679c ;;
  mac-x64) sum=184f1e3fcb73d9862994265dcacf98684867e155ef504c06dd9501fd49ec487b ;;
  linux-x64) sum=6c2a8e1d86c86968dcb3f64b32a554711e1f716fd18f02f5af655f741c5735da ;;
  linux-arm64) sum=02d5b6476cbd3277c196e64e03ac0705e421299da77caccddf77ee85b7115a54 ;;
  *) echo "unsupported platform: $platform" >&2; exit 1 ;;
esac

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/.webrtc/$platform"
stamp="$out/.version"
link_host() { if [[ $host == 1 ]]; then ln -sfn "$platform" "$root/.webrtc/host"; fi; }
if [[ -f "$stamp" && "$(cat "$stamp")" == "$STAMP" && -f "$out/release/obj/libwebrtc.a" ]]; then
  link_host
  echo "$out"
  exit 0
fi

archive="$root/.webrtc/webrtc-$WEBRTC_VERSION-$platform-release.tar.bz2"
mkdir -p "$out"
if [[ ! -f "$archive" ]] || ! echo "$sum  $archive" | shasum -a 256 -c - >/dev/null 2>&1; then
  echo "Downloading WebRTC $WEBRTC_VERSION for $platform..." >&2
  curl -fL --retry 3 -o "$archive.tmp" \
    "https://build-artifacts.signal.org/libraries/webrtc-$WEBRTC_VERSION-$platform-release.tar.bz2"
  echo "$sum  $archive.tmp" | shasum -a 256 -c - >&2
  mv "$archive.tmp" "$archive"
fi
rm -rf "$out/release" "$out/debug"
tar -xjf "$archive" -C "$out"
lib="$out/release/obj/libwebrtc.a"
[[ -f "$lib" ]] || { echo "libwebrtc.a missing in archive" >&2; find "$out" -maxdepth 3 >&2; exit 1; }
if [[ "$platform" == linux-* ]]; then
  # libwebrtc.a contains BoringSSL, whose C functions have the same names as
  # OpenSSL's (which SQLCipher uses on Linux). Give WebRTC's own C symbols a
  # prefix so each library keeps its own copy. RingRTC calls only Rust_*
  # functions; C++ (_Z*) and compiler runtime (__*) names stay as they are.
  echo "Renaming WebRTC's C symbols..." >&2
  map="$out/symbols.map"
  nm -g --defined-only -P "$lib" 2>/dev/null |
    awk 'NF >= 2 && $2 ~ /^[A-Za-z]$/ { print $1 }' |
    grep -vE '^(_Z|Rust_|__)' | sort -u | awk '{ print $1 " halyard_webrtc_" $1 }' >"$map"
  objcopy --redefine-syms="$map" "$lib"
fi
ln -s release "$out/debug"   # RingRTC looks in debug/ for debug builds; only release prebuilts exist
echo "$STAMP" > "$stamp"
link_host
echo "$out"
