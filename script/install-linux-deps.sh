#!/usr/bin/env bash
# System packages needed to build and test Halyard on Debian/Ubuntu.
set -euo pipefail
SUDO=$(command -v sudo || true)
$SUDO apt-get update
$SUDO apt-get install -y --no-install-recommends \
  build-essential clang cmake pkg-config protobuf-compiler git curl ca-certificates \
  libfontconfig-dev libfreetype-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libx11-xcb-dev libxcb1-dev libvulkan1 libssl-dev libzstd-dev libdbus-1-dev \
  libasound2-dev libpulse-dev mesa-vulkan-drivers
