# Sunnypilot Dashdown

A mobile app (iOS + Android) for downloading dashcam footage from Comma devices running the
[Sunnypilot](https://github.com/sunnypilot/sunnypilot) openpilot fork, where footage is served
over a [copyparty](https://github.com/9001/copyparty) file server. It groups 1-minute segments
into gap-split **drives**, mirrors files locally for offline browsing, downloads whole drives in
the background with file-granular resume, and manages retention/auto-delete per device.

A shared **Rust core** (via **UniFFI**) holds all logic; **SwiftUI** and **Jetpack Compose**
provide native UIs.

## Status

**Active development.** The shared Rust core and the **Android** app are built and run on real
hardware — device management, drive grouping, background download with file-granular resume,
retention/auto-delete, and a multi-camera drive player. The iOS shell is in progress. Milestone
history is in the master plan.

## Getting started

```sh
# 1. Reference source (gitignored, read-only) used while developing:
tools/fetch-refs.sh                 # clones pinned copyparty / sunnypilot / uniffi-rs / uniffi-starter into ref/

# 2. Toolchain (Phase 0 already installed these locally):
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
cargo install cargo-ndk             # Android .so builds; needs an Android NDK (r27.x)
```

## Build & run on an Android phone

The app builds from `android/` with Gradle. The shared Rust core is cross-compiled to the phone's
ABI automatically (via `cargo-ndk`) and the UniFFI bindings are generated as part of the build — so
a single Gradle task builds everything and installs it.

**Prerequisites**

- **JDK 17** (Gradle requires 17+).
- **Android SDK** (with `adb` on your `PATH`) and the **NDK r27.x** (e.g. `/opt/android-sdk/ndk/27.3.13750724`).
- A phone with **USB debugging** enabled (Settings → Developer options).

```sh
# Point the build at JDK 17 and the NDK (paths are environment-specific):
export JAVA_HOME=/usr/lib/jvm/java-17-openjdk
export ANDROID_NDK_HOME=/opt/android-sdk/ndk/27.3.13750724

# Connect the phone — over USB, or over Wi-Fi (wireless debugging paired, same LAN):
adb devices                          # USB: the phone shows up as "device"
# adb connect 192.168.1.210:5555     # Wi-Fi: connect by the phone's IP:port

# Build the Rust core + app and install the debug build straight to the phone:
cd android && ./gradlew :app:installDebug --no-daemon

# Launch it:
adb shell am start -n org.sunnypilot.dashdown/.MainActivity
```

**Multiple devices attached?** Set `ANDROID_SERIAL` so the build targets one device (not all of
them, and not an emulator) — list serials with `adb devices`:

```sh
ANDROID_SERIAL=192.168.1.210:5555 ./gradlew :app:installDebug --no-daemon   # run from android/
```

Build the APK **without** installing (e.g. to sideload it elsewhere):

```sh
cd android && ./gradlew :app:assembleDebug --no-daemon
# → android/app/build/outputs/apk/debug/app-debug.apk
ANDROID_SERIAL=<serial> adb install -r app/build/outputs/apk/debug/app-debug.apk
```

On-device instrumented tests and the Maestro UI suite (mock fixture + real hardware) are documented
in [docs/TESTING.md](docs/TESTING.md).

## Docs

- **Architecture & milestones:** [.claude/plans/sunnypilot-dashdown-master-plan.md](.claude/plans/sunnypilot-dashdown-master-plan.md)
- **Working conventions (incl. how `ref/` works):** [CLAUDE.md](CLAUDE.md)
- **Reference repos (pins, purpose, iOS-on-Linux recipe):** [docs/REFERENCES.md](docs/REFERENCES.md)
