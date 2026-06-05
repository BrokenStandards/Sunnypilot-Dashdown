#!/usr/bin/env bash
# Build the host cdylib and generate Kotlin + Swift bindings from it (library mode).
# Output: target/bindings/{kotlin,swift}/. Used as the M0 bindgen smoke test and
# reusable locally. iOS .xcframework assembly happens later via xtool (Phase B).
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build -p dashdown-core
LIB="target/debug/libdashdown_core.so"   # host cdylib (Linux)

for lang in kotlin swift; do
  cargo run -p dashdown-bindgen --bin uniffi-bindgen -- generate \
    --library "$LIB" --language "$lang" --out-dir "target/bindings/$lang" --no-format
done

echo "bindings generated under target/bindings/"
