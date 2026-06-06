#!/usr/bin/env bash
# Build the Rust core for iOS and package it for the SwiftPM/xtool app:
#   1. cross-compile the staticlib for the device target (aarch64-apple-ios)
#   2. generate Swift bindings (+ FFI header/modulemap) via the in-workspace bindgen
#   3. hand-assemble an .xcframework (no `xcodebuild` on Linux)
#
# Run inside the iOS build container (see tools/ios-build.sh), which provides
# Swift 6.3 + xtool + the Darwin SDK built from an Xcode.xip. Re-run after any
# change to the Rust core.
#
# The Rust C deps (bundled SQLite, ring) are cross-compiled with the Xcode iOS
# sysroot from the installed Darwin Swift SDK. We build ONLY the staticlib
# (`cargo rustc --crate-type staticlib`) so no Apple linker is needed.
set -euo pipefail

cd "$(dirname "$0")/.."                        # repo root (workspace Cargo.toml)
REPO_ROOT="$(pwd)"
CRATE=dashdown_core
DEVICE_TARGET=aarch64-apple-ios
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
STAGING="$TARGET_DIR/uniffi-ios-staging"
XCF="$REPO_ROOT/ios/Frameworks/libdashdown_core-rs.xcframework"

# iOS sysroot from the installed xtool Darwin SDK (override via DARWIN_SDK_BUNDLE).
SDK_BUNDLE="${DARWIN_SDK_BUNDLE:-$HOME/.swiftpm/swift-sdks/darwin.artifactbundle}"
SYSROOT="$SDK_BUNDLE/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk"
if [ ! -d "$SYSROOT" ]; then
  echo "error: iOS sysroot not found at $SYSROOT" >&2
  echo "       install the Darwin SDK first: xtool sdk install <Xcode.xip>" >&2
  exit 1
fi

# cc-crate cross-compile env for the C deps (sqlite, ring).
export SDKROOT="$SYSROOT"
export CC_aarch64_apple_ios=clang
export AR_aarch64_apple_ios=llvm-ar
export CFLAGS_aarch64_apple_ios="--target=arm64-apple-ios -isysroot $SYSROOT -mios-version-min=17.0"

echo "==> 1/3 cross-compiling $CRATE staticlib for $DEVICE_TARGET (release)"
cargo rustc -p dashdown-core --release --target "$DEVICE_TARGET" --crate-type staticlib
DEVICE_LIB="$TARGET_DIR/$DEVICE_TARGET/release/lib${CRATE}.a"

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
echo "Next: cd ios && xtool dev build   (cross-compile + link the app)"
