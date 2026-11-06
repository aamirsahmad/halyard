#!/usr/bin/env bash
# Builds and runs the whole test suite inside a Linux container.
# Usage: script/test-linux.sh [extra cargo test args]
set -euo pipefail
cd "$(dirname "$0")/.."
docker build -f script/Dockerfile.linux -t signal-gpui-linux .
docker run --rm \
  -v "$PWD":/src \
  -v signal-gpui-cargo:/root/.cargo/registry \
  -v signal-gpui-git:/root/.cargo/git \
  -v signal-gpui-rustup:/root/.rustup \
  -e CARGO_TARGET_DIR=/src/target/linux \
  signal-gpui-linux \
  bash -c 'cargo test --workspace --locked "$@" && cargo build --release --locked -p signal-app' -- "$@"
