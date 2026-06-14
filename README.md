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

### On an emulator

The same Gradle tasks target a running emulator (the Rust core is built for its ABI, usually
`x86_64`). Boot an AVD, then install to it by serial:

```sh
ANDROID_SERIAL=emulator-5554 ./gradlew :app:installDebug --no-daemon   # run from android/
```

Booting an AVD headless (no display) can die on a Qt platform-plugin error — force the offscreen
backend:

```sh
QT_QPA_PLATFORM=offscreen emulator -avd <name> -no-window -gpu swiftshader_indirect &
```

> An emulator can reach a comma on your LAN directly (it NATs out through the host), so you can point
> a device at the real copyparty (`<comma-ip>:8080`) and play real footage where `adb logcat` and the
> view hierarchy are easy to read.

### Inspecting a connected device

Two helpers wrap the common `adb`/`sqlite`/Maestro dance (set `ANDROID_SERIAL` to pick the device):

```sh
ANDROID_SERIAL=<serial> tools/dd-db.sh devices                        # pull + query the on-device index.sqlite
ANDROID_SERIAL=<serial> tools/dd-ui.sh add_device NAME=comma IP=<ip>  # run a parameterized Maestro flow
```

### Troubleshooting

- **`Dependency requires at least JVM runtime version 17 … This build uses a Java 11 JVM`** — a
  stale Gradle daemon is on an older JVM. Export `JAVA_HOME` to a JDK 17 (above) and pass
  `--no-daemon` so the build uses it.
- **`INSTALL_FAILED_… / No space left on device`** — the target is out of storage (footage downloads
  are large). Free space on the device, or on an emulator raise the AVD's disk / clear app data.

On-device instrumented tests and the Maestro UI suite (mock fixture + real hardware) are documented
in [docs/TESTING.md](docs/TESTING.md).

## Docs

- **Architecture & milestones:** [.claude/plans/sunnypilot-dashdown-master-plan.md](.claude/plans/sunnypilot-dashdown-master-plan.md)
- **Working conventions (incl. how `ref/` works):** [CLAUDE.md](CLAUDE.md)
- **Reference repos (pins, purpose, iOS-on-Linux recipe):** [docs/REFERENCES.md](docs/REFERENCES.md)
