#!/usr/bin/env bash
# Builds the Linux release packages in an Ubuntu 22.04 container, for the
# Docker host's architecture, into dist/. Uses a snapshot of HEAD.
set -euo pipefail
cd "$(dirname "$0")/.."
if ! docker info >/dev/null 2>&1 && command -v colima >/dev/null; then
  colima start
fi
DOCKER_BUILDKIT=0 docker build -f script/Dockerfile.release -t halyard-release .
mkdir -p dist
git archive --format=tar HEAD | docker run --rm -i \
  -v halyard-release-cargo:/root/.cargo/registry \
  -v halyard-release-git:/root/.cargo/git \
  -v halyard-release-rustup:/root/.rustup \
  -v halyard-release-target:/target \
  -v "$PWD/dist:/out" \
  -e CARGO_TARGET_DIR=/target \
  -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
  halyard-release \
  bash -c 'mkdir -p /work && tar -x -C /work && cd /work && script/build-linux-packages.sh && cp dist/* /out/'
