#!/usr/bin/env bash
# Build the Rust core for iOS and package it for the SwiftPM/xtool app:
#   1. cross-compile the staticlib for the device target (aarch64-apple-ios)
#   2. generate Swift bindings (+ FFI header/modulemap) via the in-workspace bindgen
#   3. hand-assemble an .xcframework (no `xcodebuild` on Linux)
#
# Run this once before `xtool dev` / `xtool dev build`. Re-run after any change
# to the Rust core.
#
# PREREQUISITES (Phase B / B0 toolchain — see ios/README.md):
#   - Swift 6.3 toolchain on PATH (`swift --version`)
#   - xtool + the Darwin Swift SDK built from an Xcode.xip (`xtool sdk status`)
#   - iOS Rust target installed: `rustup target add aarch64-apple-ios` (already in
#     rust-toolchain.toml)
#
# TOOLCHAIN ENV (must point at the xtool-provided Apple clang + iOS sysroot so the
# core's C deps — bundled SQLite, ring — cross-compile). These are confirmed and
# pinned against the actual installed SDK during B0 bring-up; defaults below are
# the expected shape:
#   export SDKROOT=...                       # iPhoneOS .sdk from the xtool SDK
#   export CC_aarch64_apple_ios=...clang
#   export AR_aarch64_apple_ios=...ar
#   export CFLAGS_aarch64_apple_ios="-target arm64-apple-ios17.0 -isysroot $SDKROOT"
set -euo pipefail

cd "$(dirname "$0")/.."                       # repo root (workspace Cargo.toml)
REPO_ROOT="$(pwd)"
CRATE=dashdown_core
DEVICE_TARGET=aarch64-apple-ios
STAGING="$REPO_ROOT/target/uniffi-ios-staging"
XCF="$REPO_ROOT/ios/Frameworks/libdashdown_core-rs.xcframework"

echo "==> 1/3 cross-compiling $CRATE for $DEVICE_TARGET (release)"
cargo build -p dashdown-core --lib --release --target "$DEVICE_TARGET"
DEVICE_LIB="$REPO_ROOT/target/$DEVICE_TARGET/release/lib${CRATE}.a"

echo "==> 2/3 generating Swift bindings"
rm -rf "$STAGING"; mkdir -p "$STAGING"
cargo run -p dashdown-bindgen --bin uniffi-bindgen -- generate \
    --library "$DEVICE_LIB" --language swift --out-dir "$STAGING" --no-format
mkdir -p "$REPO_ROOT/ios/Sources/UniFFI/include"
mv "$STAGING/${CRATE}.swift" "$REPO_ROOT/ios/Sources/UniFFI/${CRATE}.swift"
mv "$STAGING/${CRATE}FFI.h" "$REPO_ROOT/ios/Sources/UniFFI/include/${CRATE}FFI.h"
# xcframework Headers convention requires the modulemap be named module.modulemap.
mv "$STAGING/${CRATE}FFI.modulemap" "$STAGING/module.modulemap"

echo "==> 3/3 assembling device-slice xcframework"
rm -rf "$XCF"
SLICE="$XCF/ios-arm64"
mkdir -p "$SLICE/Headers"
cp "$DEVICE_LIB" "$SLICE/lib${CRATE}.a"
cp "$REPO_ROOT/ios/Sources/UniFFI/include/${CRATE}FFI.h" "$SLICE/Headers/${CRATE}FFI.h"
cp "$STAGING/module.modulemap" "$SLICE/Headers/module.modulemap"
cat > "$XCF/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>AvailableLibraries</key>
    <array>
        <dict>
            <key>LibraryIdentifier</key><string>ios-arm64</string>
            <key>LibraryPath</key><string>libdashdown_core.a</string>
            <key>HeadersPath</key><string>Headers</string>
            <key>SupportedArchitectures</key><array><string>arm64</string></array>
            <key>SupportedPlatform</key><string>ios</string>
        </dict>
    </array>
    <key>CFBundlePackageType</key><string>XFWK</string>
    <key>XCFrameworkFormatVersion</key><string>1.0</string>
</dict>
</plist>
PLIST

echo "OK: $XCF + ios/Sources/UniFFI/${CRATE}.swift"
echo "Next: xtool dev   (build + install + run on a connected iPhone)"
