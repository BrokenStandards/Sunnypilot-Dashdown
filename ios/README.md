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

## Toolchain prerequisites (B0)

1. **Swift 6.3** toolchain on `PATH` — <https://swift.org/install/linux> (`swift --version`).
2. **usbmuxd** (+ optionally `libimobiledevice-utils`) to talk to a device over USB.
3. **xtool** — the `xtool.AppImage` release on `PATH`, or built from source.
4. **`Xcode.xip`** (Xcode 26, Apple-ID-gated download) → build the Darwin SDK:
   `xtool sdk build --xip /path/to/Xcode.xip` (`xtool sdk status` to verify).
5. **Apple Developer account** + a **physical iPhone** — required to *run*: there
   is **no iOS Simulator on Linux**, so `xtool dev` signs and installs to a
   connected device. `xtool setup` performs the Apple auth.
6. iOS Rust target (already in `rust-toolchain.toml`): `aarch64-apple-ios`.

## Build & run

```bash
./build-rust-ios.sh     # cross-compile core, generate bindings, assemble xcframework
xtool dev               # build + sign + install + launch on a connected iPhone
# or just compile:
xtool dev build
```

## Status

B0 establishes this skeleton and the crypto change (aws-lc-rs→ring) that lets the
core cross-compile to iOS. Build/run verification on Apple hardware is gated on
the toolchain inputs above (Xcode.xip + device + Apple account) being provided.
