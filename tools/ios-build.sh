#!/usr/bin/env bash
# Build the iOS app on a Linux host via Docker (Swift 6.3 + Rust + xtool).
#
# One-time setup:
#   1. docker build -f tools/ios-build.Dockerfile -t dashdown-ios-build .
#   2. Install the Darwin SDK from your Xcode.xip into the cached swiftpm dir
#      (Apple-licensed; you supply the .xip):
#        docker run --rm \
#          -v /path/to/xip-dir:/xip:ro \
#          -v "$HOME/.cache/dashdown-ios/swiftpm:/root/.swiftpm" \
#          dashdown-ios-build xtool sdk install /xip/Xcode_26.5_Universal.xip
#
# Then build (cross-compile core + bindings + xcframework, then the SwiftUI app):
#   tools/ios-build.sh
#
# This is BUILD-only. Running on a device additionally needs an Apple Developer
# account + a connected iPhone (no iOS Simulator exists on Linux): `xtool dev`.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE=dashdown-ios-build
SWIFTPM_CACHE="${DASHDOWN_IOS_SWIFTPM_CACHE:-$HOME/.cache/dashdown-ios/swiftpm}"
CARGO_CACHE="${DASHDOWN_IOS_CARGO_CACHE:-$HOME/.cache/dashdown-ios/cargo-registry}"

docker image inspect "$IMAGE" >/dev/null 2>&1 \
  || docker build -f "$REPO_ROOT/tools/ios-build.Dockerfile" -t "$IMAGE" "$REPO_ROOT"

if [ ! -d "$SWIFTPM_CACHE/swift-sdks/darwin.artifactbundle" ]; then
  echo "error: Darwin SDK not installed in $SWIFTPM_CACHE" >&2
  echo "       run 'xtool sdk install <Xcode.xip>' first (see header)" >&2
  exit 1
fi
mkdir -p "$CARGO_CACHE"

docker run --rm \
  -v "$REPO_ROOT":/work \
  -v "$SWIFTPM_CACHE":/root/.swiftpm \
  -v "$CARGO_CACHE":/root/.cargo/registry \
  -e CARGO_TARGET_DIR=/work/target-ios \
  "$IMAGE" \
  bash -lc 'set -e; cd /work && bash ios/build-rust-ios.sh && cd ios && xtool dev build'
