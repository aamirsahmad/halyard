#!/usr/bin/env bash
# Builds and runs the whole test suite inside a Linux container, against a
# snapshot of the current commit (HEAD), so edits in progress don't interfere.
# Usage: script/test-linux.sh [extra cargo test args]
set -euo pipefail
cd "$(dirname "$0")/.."
DOCKER_BUILDKIT=0 docker build -f script/Dockerfile.linux -t signal-gpui-linux .
git archive --format=tar HEAD | docker run --rm -i \
  -v signal-gpui-cargo:/root/.cargo/registry \
  -v signal-gpui-git:/root/.cargo/git \
  -v signal-gpui-rustup:/root/.rustup \
  -v signal-gpui-target:/target \
  -e CARGO_TARGET_DIR=/target \
  -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
  -e CARGO_PROFILE_DEV_DEBUG=line-tables-only \
  -e CARGO_PROFILE_TEST_DEBUG=line-tables-only \
  signal-gpui-linux \
  bash -c 'mkdir -p /work && tar -x -C /work && cd /work && cargo test --workspace --locked "$@" && cargo build --release --locked -p signal-app' -- "$@"
