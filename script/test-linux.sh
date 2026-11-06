#!/usr/bin/env bash
# Builds and runs the whole test suite inside a Linux container.
# Usage: script/test-linux.sh [extra cargo test args]
set -euo pipefail
cd "$(dirname "$0")/.."
DOCKER_BUILDKIT=0 docker build -f script/Dockerfile.linux -t signal-gpui-linux .
docker run --rm \
  -v "$PWD":/src \
  -v signal-gpui-cargo:/root/.cargo/registry \
  -v signal-gpui-git:/root/.cargo/git \
  -v signal-gpui-rustup:/root/.rustup \
  -e CARGO_TARGET_DIR=/src/target/linux \
  -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
  -e CARGO_PROFILE_DEV_DEBUG=line-tables-only \
  -e CARGO_PROFILE_TEST_DEBUG=line-tables-only \
  signal-gpui-linux \
  bash -c 'cargo test --workspace --locked "$@" && cargo build --release --locked -p signal-app' -- "$@"
