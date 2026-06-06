# Dashdown — iOS app (SwiftUI, built on Linux via xtool)

The iOS shell is an [xtool](https://xtool.sh) SwiftPM project. xtool is a
cross-platform Xcode replacement that builds, signs, and deploys iOS apps from
**Linux** using a Darwin Swift SDK extracted from an `Xcode.xip`.

The Rust core (`dashdown-core`) is consumed exactly like the Android side: the
in-workspace `dashdown-bindgen` generates UniFFI Swift bindings, and the core is
shipped as an `.xcframework` binary target. See [Package.swift](Package.swift).

## Layout

```
ios/
  Package.swift            # xtool project: app library "Dashdown" -> UniFFI -> DashdownCoreRS (xcframework)
  xtool.yml                # bundleID
  build-rust-ios.sh        # cross-compile core + gen Swift bindings + assemble xcframework
  Sources/Dashdown/        # SwiftUI app (DashdownApp, ContentView)
  Sources/UniFFI/          # generated bindings (gitignored) + C FFI module
  Frameworks/              # libdashdown_core-rs.xcframework (generated, gitignored)
  Tests/DashdownTests/     # binding-load smoke (runs on device/macOS, not Linux)
```

## Building on Linux (Docker — recommended)

This repo's host (Arch/EndeavourOS) has no native Swift, so B0 uses a Docker
image with Swift 6.3 + Rust + xtool: [tools/ios-build.Dockerfile](../tools/ios-build.Dockerfile),
driven by [tools/ios-build.sh](../tools/ios-build.sh).

```bash
# 1. Build the image
docker build -f tools/ios-build.Dockerfile -t dashdown-ios-build .

# 2. One-time: install the Darwin SDK from your Xcode.xip (Apple-licensed; you
#    supply the .xip). Cached under ~/.cache/dashdown-ios/swiftpm so it persists.
docker run --rm \
  -v /path/to/xip-dir:/xip:ro \
  -v "$HOME/.cache/dashdown-ios/swiftpm:/root/.swiftpm" \
  dashdown-ios-build xtool sdk install /xip/Xcode_26.5_Universal.xip

# 3. Build the app (cross-compile core + bindings + xcframework, then SwiftUI app)
tools/ios-build.sh        # -> ios/xtool/Dashdown.app
```

`build-rust-ios.sh` cross-compiles the core for `aarch64-apple-ios` using the
iOS sysroot from the installed Darwin SDK (the aws-lc-rs→ring crypto switch is
what lets the C deps compile), generates the Swift bindings, and assembles a
device-slice xcframework. `xtool dev build` then compiles + links the app.

## Running on a device

There is **no iOS Simulator on Linux**, so running needs a physical iPhone +
an Apple Developer account (xtool signs + installs over USB via usbmuxd):

```bash
xtool setup        # one-time Apple auth + SDK
xtool dev          # build + sign + install + launch on a connected iPhone
```

## Native toolchain (alternative to Docker)

On a distro with an official Swift toolchain you can skip Docker: install Swift
6.3 (<https://swift.org/install/linux>), `usbmuxd`, the `xtool` AppImage, then
`xtool sdk install <Xcode.xip>` and run `build-rust-ios.sh` + `xtool dev build`
directly.

## Status (B0)

**Build verified.** The Rust core cross-compiles to `aarch64-apple-ios`, the
UniFFI Swift bindings + xcframework build, and the SwiftUI app compiles and links
to `Dashdown.app` via `xtool dev build` (in the Docker image above). On-device
run is deferred until an iPhone + Apple Developer account are available (no
simulator on Linux). The `.xcframework` ships only the device slice; add the
simulator slice if/when a macOS simulator is used.
