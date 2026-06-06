# Phase B — Native Shells (Android Compose + iOS SwiftUI) on the Rust UniFFI core

## Context

Phase A is complete and merged: the shared Rust core (`dashdown-core`) exposes the full
`AppCore` UniFFI surface (M8), verified end-to-end on host against `mock-copyparty` and real
copyparty. Phase B builds the two **native apps** that consume that surface via generated
Swift + Kotlin bindings, and — per the master plan's operating principle — builds and **fully
tests each platform including the hard background cases** (app backgrounded/killed mid-download
→ completes; interrupted transfer → resumes only missing files; device unreachable → red dot).

Why now / what's new: no `android/` or `ios/` project exists yet. The binding-*generation*
pipeline works ([tools/gen-bindings.sh](tools/gen-bindings.sh)) but bindings have never been
**loaded and called on a device** — M8's CI smoke only greps the generated source. Phase B turns
that into real on-device load + FFI calls, then layers the product UI and background execution.

Two structural facts drive the plan, both verified this session:
1. **Crypto backend blocks iOS.** [rust/core/Cargo.toml:23](rust/core/Cargo.toml) pulls `reqwest`'s
   `rustls` feature → the **aws-lc-rs** provider (CMake + C, lockfile-confirmed `aws-lc-sys`).
   That is exactly what the CI comment flags as un-cross-compilable to `aarch64-apple-ios` on
   Linux. Fix: switch rustls to the **ring** provider (cross-compiles cleanly to all iOS +
   Android targets, no CMake/Go).
2. **The pipeline is fully mirrorable** from `ref/uniffi-starter` (read-only): Android `:core`
   (cargo-ndk + build-time Kotlin bindgen + JNA) + `:app` Compose; iOS SwiftPM `Package.swift`
   + `.xcframework` binary target + a `build-ios.sh`. Our iOS build replaces `xcodebuild` with
   **xtool** on Linux.

**Decisions locked with the user (this session):**
- **iOS-on-Linux is set up now.** The user provides an `Xcode.xip`; B0 bootstraps a Linux Swift
  toolchain + `xtool` (`xtool setup` extracts the iOS SDK once) so **both** platforms build and
  run on Linux. iOS sim verification in B0; on-device via libimobiledevice as needed.
- **Android test target: emulator now, physical device for B2.** Install emulator + system image
  + AVD (KVM is available → fast x86_64). A physical device is attached for the B2 background/
  kill/resume acceptance run (most faithful to foreground-service/Doze lifecycle).
- Carried constraints: per-milestone plan → TDD build → gates → branch→PR→CI→squash-merge;
  `ref/` is read-only (never import/build/copy; search via `tools/refgrep`); `cargo add` for
  latest deps; never commit `target/`, generated bindings, `.so`/`.xcframework`, or secrets.

---

## Phase B milestone breakdown

Each milestone gets its own detailed plan when reached (this file details **B0**). Android is
low-risk/ready; iOS carries the toolchain bootstrap. B0 de-risks the whole pipeline on both
platforms with a trivial app before any product UI is built.

- **B0 — Toolchain + binding-integration smoke (both platforms).** *(detailed below)* Switch
  crypto aws-lc-rs→ring (keep all host tests + Android cross-compile green); bootstrap the iOS
  toolchain (Swift + xtool + `Xcode.xip` SDK) and the Android emulator; create `android/` and
  `ios/` skeletons that build the lib, generate bindings, **load the lib and call
  `version()`/`pingAsync()` on emulator + simulator**. This is the milestone that converts the
  M8-deferred "binding smoke" into a real on-device load. Extend CI.
- **B1 — `mock-comma-mcp` + hermetic test harness (shared).** New workspace crate wrapping the
  existing `mock-copyparty` lib; exposes MCP tools to provision device fixtures, inject states
  (single-drive / 1-min-gap split / partial / **size-mismatch**, the new one), and **toggle
  reachability** for green/blue/red. Finish test tooling (Maestro, mobile-mcp). Built before the
  UI phases because both consume it.
- **B2 — Android shell (full).** All 5 Compose screens + ViewModels/`StateFlow`,
  `ProgressSink`/`LogSink` Kotlin impls, navigation, Keystore for the password, Media3 playback,
  SAF zip export. **Background (required): Foreground Service** hosting the download coroutine +
  **WorkManager** for auto-sync/maintenance. Espresso + Maestro + mobile-mcp vs mock-comma-mcp,
  **including background→complete, kill/restart→resume-missing-only, unreachable→red** (physical
  device for these).
- **B3 — iOS shell (full).** Same IA in SwiftUI + `@Observable` VMs, `ProgressSink`/`LogSink`
  Swift impls, Keychain, AVPlayer, share-sheet export. **Background (required): BGProcessingTask
  + background `URLSession`** handing finished files to the core for commit (core byte-range
  resume makes interrupts safe). XCUITest/Swift Testing + Maestro + xtool/libimobiledevice vs
  mock server, same hard cases. B2/B3 run in parallel.
- **B4 — Cross-platform Maestro flows + agentic verification (shared).** Shared YAML flows keyed
  on accessibility IDs; mobile-mcp (Android) + xtool/libimobiledevice (iOS) screenshot checks of
  dots, grouping, partial/resume badge, settings.
- **B5 — CI for native.** `android-ci.yml` (Gradle `assemble` + emulator `connectedCheck`) and an
  iOS xtool build job on Linux (feasibility proven in B0); else document the Phase-C macOS-runner
  fallback (already an open question in the master plan).
- **B6 — Phase close.** Merge Phase B branch(es) to `main`; update [CLAUDE.md](CLAUDE.md) +
  [docs/REFERENCES.md](docs/REFERENCES.md) with the realized iOS-on-Linux recipe + crypto note.

---

## B0 — detailed plan

### 1. Crypto backend: aws-lc-rs → ring

Edit [rust/core/Cargo.toml](rust/core/Cargo.toml) (via `cargo add`, latest stable):
```
cargo add reqwest@0.13 --no-default-features --features rustls-no-provider,stream -p dashdown-core
cargo add rustls --no-default-features --features ring,tls12,logging,std -p dashdown-core
```
Intended result:
```toml
reqwest = { version = "0.13.4", default-features = false, features = ["rustls-no-provider", "stream"] }
rustls  = { version = "0.23",   default-features = false, features = ["ring", "tls12", "logging", "std"] }
```
`rustls-platform-verifier` still arrives transitively; no extra dep needed.

**Mandatory runtime fix.** `rustls-no-provider` means no compiled-in default provider, so
`reqwest::Client::builder().build()` fails at runtime ("no process-level CryptoProvider") unless
one is installed as the process default. The client is built at
[rust/core/src/copyparty_client/mod.rs:27](rust/core/src/copyparty_client/mod.rs#L27). Add a
one-time installer and call it before any client build:
```rust
// rust/core/src/tls.rs  (+ `pub mod tls;` in lib.rs)
use std::sync::OnceLock;
static TLS_INIT: OnceLock<()> = OnceLock::new();
pub fn ensure_crypto_provider() {
    TLS_INIT.get_or_init(|| { let _ = rustls::crypto::ring::default_provider().install_default(); });
}
```
Call sites: top of `CopypartyClient::new` and `with_client` (so `cargo test`, which builds
clients without `AppCore`, also passes) and in `AppCore::new`
([rust/core/src/ffi/mod.rs](rust/core/src/ffi/mod.rs)) next to `logging::install()`.

**Acceptance for the crypto change:**
- `cargo fmt --all --check` + `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `cargo test --workspace` green (all M0–M8, incl. `it_real_copyparty`).
- `cargo tree -p dashdown-core -i aws-lc-rs` → **empty**; `cargo tree -i ring` shows the chain.
- Android still cross-compiles: `cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -t x86 -o target/jniLibs build -p dashdown-core --release`.
- **Close the runtime-coverage gap:** mock-copyparty is plain HTTP, so no test exercises TLS
  today. Add one TLS-exercising test (a wiremock/served HTTPS endpoint, or a real HTTPS GET) so
  the ring provider is genuinely run, not just compiled.

### 2. Toolchain bootstrap

**Android emulator (KVM present):**
```
yes | sdkmanager --sdk_root=/opt/android-sdk "emulator" "system-images;android-34;google_atd;x86_64"
avdmanager create avd -n dashdown-b0 -k "system-images;android-34;google_atd;x86_64" --device pixel_6
/opt/android-sdk/emulator/emulator -avd dashdown-b0 -no-window -no-audio -gpu swiftshader_indirect &
adb wait-for-device
```

**iOS (Swift + xtool), user supplies `Xcode.xip`:**
```
# 1. Linux Swift toolchain (swiftly or swift.org tarball) → confirm `swift --version`
# 2. tools/fetch-refs.sh phaseb   # pins ref/xtool, ref/libimobiledevice, ref/osxcross (read-only)
# 3. build/install xtool; `xtool setup`  # one-time iOS SDK extraction from the provided Xcode.xip
# 4. iOS Rust std targets already installed; libimobiledevice for on-device (sim is enough in B0)
```
Record the exact realized recipe in [docs/REFERENCES.md](docs/REFERENCES.md) at B6.

### 3. Android skeleton — `android/`

Mirror `ref/uniffi-starter/android` (do not copy). Namespaces `org.sunnypilot.dashdown[.core]`.
Pin a **Gradle wrapper** and target **JDK 17** (host default is Gradle 9 / JDK 26, unverified
with AGP/cargo-ndk plugin). Bindgen must call the **in-workspace** `dashdown-bindgen`.

```
android/
  settings.gradle.kts            # include(":core",":app")
  build.gradle.kts               # plugins apply false: AGP, Kotlin, compose-compiler, cargo-ndk, ktfmt
  gradle.properties              # useAndroidX, nonTransitiveRClass
  gradle/libs.versions.toml      # AGP 8.13.x, Kotlin 2.2.x, compose-bom, cargo-ndk 0.3.4, JNA 5.18.x  [VERIFY]
  gradle/wrapper/…  gradlew      # pinned Gradle wrapper
  local.properties               # sdk.dir=/opt/android-sdk (gitignored)
  core/
    build.gradle.kts             # androidLibrary; JNA `net.java.dev.jna:jna:5.18.1@aar`; cargoNdk{}; bindgen Exec task
    src/main/AndroidManifest.xml
    src/main/jniLibs/<abi>/       # cargo-ndk output (gitignored)
    src/androidTest/java/org/sunnypilot/dashdown/core/CoreLoadTest.kt
  app/
    build.gradle.kts             # androidApplication + compose; implementation(project(":core"))
    src/main/java/org/sunnypilot/dashdown/MainActivity.kt
    src/main/res/…
```

`core/build.gradle.kts` load-bearing blocks: `cargoNdk { module="../.."; librariesNames=
listOf("libdashdown_core.so"); extraCargoBuildArguments=listOf("-p","dashdown-core") }`, and a
per-variant `Exec` task running `cargo run -p dashdown-bindgen --bin uniffi-bindgen -- generate
--library android/core/src/main/jniLibs/arm64-v8a/libdashdown_core.so --language kotlin --out-dir
<build>/generated/source/uniffi/<variant>/java --no-format`, wired as a `dependsOn` of the
Kotlin compile and added to the variant source set. Generated package: `uniffi.dashdown_core`.

`MainActivity.kt` (trivial real-FFI proof): `Text("dashdown ${version()}")` plus
`LaunchedEffect { pong = pingAsync() }` (suspend → exercises JNA + async path).
`CoreLoadTest.kt` (runs under `connectedCheck`): assert `version()`==crate version, `ping()=="pong"`,
`runBlocking { pingAsync() }=="pong"` — the real on-device binding load.

### 4. iOS skeleton — `ios/`

Mirror `ref/uniffi-starter` (Package.swift + UniFFI target + app target + build script), but
assemble the `.xcframework` via **xtool**, not `xcodebuild`, and generate bindings with the
in-workspace `dashdown-bindgen`.

```
ios/
  Package.swift                  # .binaryTarget → ios/Frameworks/libdashdown_core-rs.xcframework; UniFFI + app targets
  Sources/Dashdown/DashdownApp.swift     # Text(version()) + .task { pong = try await pingAsync() }
  Sources/UniFFI/dashdown_core.swift     # generated (gitignored)
  Sources/UniFFI/include/                # dashdown_coreFFI.h + module.modulemap (generated)
  Tests/DashdownTests/CoreLoadTests.swift
  build-ios.sh                   # see below
```

`ios/build-ios.sh`: (1) `cargo build -p dashdown-core --lib --release --target {aarch64-apple-ios,
aarch64-apple-ios-sim,x86_64-apple-ios}` **under xtool's Swift SDK env** (provides Apple clang +
SDK so bundled SQLite + ring compile); (2) `cargo run -p dashdown-bindgen --bin uniffi-bindgen --
generate --library target/aarch64-apple-ios/release/libdashdown_core.a --language swift --out-dir
ios/Sources/UniFFI --no-format` (move the `.h` + `.modulemap` into `include/`); (3) lipo the two
sim slices into a fat sim lib; (4) assemble `ios/Frameworks/libdashdown_core-rs.xcframework` via
**xtool**'s create-xcframework equivalent (fallback: hand-assemble the dir + `Info.plist`). The
swift bindings generate `version()`, `ping()`, `pingAsync() async throws`.

### 5. CI

Add `.github/workflows/android-ci.yml` (mirror `ref/uniffi-starter/.github/workflows/android.yml`):
JDK 17, install cargo-ndk, `touch android/local.properties`, `./gradlew build`, plus a
`connectedCheck` job via `reactivecircus/android-emulator-runner@v2`. Keep the existing
[.github/workflows/rust-ci.yml](.github/workflows/rust-ci.yml) grep smoke and add the rustls→ring
`cargo tree -i aws-lc-rs` empty-check there. Add an iOS xtool build job once B0 proves it on
Linux (else document the Phase-C macOS-runner fallback).

### Files B0 creates / edits
- Edit [rust/core/Cargo.toml](rust/core/Cargo.toml) (reqwest features; add rustls/ring). New
  `rust/core/src/tls.rs` + `pub mod tls;`; call sites in
  [rust/core/src/copyparty_client/mod.rs:27](rust/core/src/copyparty_client/mod.rs#L27) and
  [rust/core/src/ffi/mod.rs](rust/core/src/ffi/mod.rs).
- New trees `android/` and `ios/` (§3, §4) + `ios/build-ios.sh`.
- New `.github/workflows/android-ci.yml` (+ optional iOS job); minor edit to `rust-ci.yml`.
- Run `tools/fetch-refs.sh phaseb` to populate read-only `ref/{xtool,libimobiledevice,osxcross}`.

---

## Verification (B0 acceptance gates)

1. **Crypto:** fmt/clippy/`cargo test --workspace` green; `cargo tree -i aws-lc-rs` empty;
   Android 4-ABI `cargo ndk` build succeeds; the new TLS-exercising test passes.
2. **Android:** `./gradlew :app:assembleDebug` builds (4 `.so` via cargo-ndk; bindgen emits
   `uniffi/dashdown_core/dashdown_core.kt`; JNA links); app launches on the AVD showing
   `version()` + the `pingAsync()` pong; `./gradlew :core:connectedDebugAndroidTest` runs
   `CoreLoadTest` green — **real on-device binding load**.
3. **iOS:** `bash ios/build-ios.sh` assembles the xcframework + Swift bindings; the app builds via
   xtool and runs in a simulator (or paired device) showing `version()` + `pingAsync()`;
   `CoreLoadTests` pass — **real binding load on Apple**.
4. **CI:** `android-ci.yml` green (assemble + emulator connectedCheck); `rust-ci.yml` extended
   check green; iOS xtool job green (or documented fallback).
5. Commit B0 on a branch → PR → CI → squash-merge to `main` (per workflow).

## Risks / open items (re-verify before/while coding B0)
- **`Xcode.xip` provisioning + license** — the gating dependency for the iOS toolchain; user-supplied.
- **rustls ring API** — confirm `rustls::crypto::ring::default_provider().install_default()` against
  the resolved `rustls 0.23.x` minor; prefer wiring the provider via the builder if cleaner.
- **Gradle/JDK/AGP/cargo-ndk-plugin matrix** — pin a Gradle wrapper + JDK 17; if the willir
  cargo-ndk plugin is stale on modern Gradle, drive `cargo ndk` from a plain `Exec` task instead.
- **xtool specifics** — exact create-xcframework invocation, lipo/cctools availability on Linux,
  and whether xtool needs its own project manifest; hand-assemble the xcframework as fallback.
- **JNA aar version** — confirm `net.java.dev.jna:jna:<latest>@aar` is compatible with the
  uniffi-0.31-generated Kotlin.

---

## B0 — outcome (delivered)

- **Crypto:** reqwest `rustls-no-provider` + direct `rustls`/ring; `tls::ensure_crypto_provider()`
  installs the ring provider at `CopypartyClient::new` + `AppCore::new`. New `it_tls` runs a real
  ring handshake. 140 host tests pass; `cargo tree -i aws-lc-rs` empty; 4-ABI `cargo ndk` green.
- **Android (fully verified on emulator):** `android/` Gradle project (Groovy, Gradle 8.13 wrapper,
  JDK 17, AGP 8.13, compileSdk 36); `:core` uses the willir cargo-ndk plugin (`module=".."`,
  targets arm64+x86_64) + a build-time `dashdown-bindgen` Exec task + JNA + kotlinx-coroutines;
  `:app` Compose calls `version()`/`ping()`/`pingAsync()`. `:app:assembleDebug` builds; the app
  renders `dashdown core 0.0.0 / sync: pong / async: pong`; `:core:connectedDebugAndroidTest`
  (2 tests) passes on the `dashdown-b0` AVD — the real on-device binding load.
- **iOS (build-only, per user decision; no device on hand):** host is Arch/EndeavourOS with no
  native Swift → built via Docker (`tools/ios-build.Dockerfile`: swift:6.3-jammy + Rust + xtool;
  `tools/ios-build.sh`). Darwin SDK built once from the user's `Xcode_26.5_Universal.xip`
  (`xtool sdk install`). `ios/build-rust-ios.sh` cross-compiles the core staticlib for
  aarch64-apple-ios (Xcode sysroot for the C deps), generates Swift bindings, assembles a
  device-slice xcframework; UniFFI target pinned to Swift 5 language mode. `xtool dev build`
  compiles + links the SwiftUI app → `Dashdown.app`, clean. On-device run deferred (no iOS
  Simulator on Linux; needs an iPhone + Apple Developer account).
- **CI:** `rust-ci.yml` gains an aws-lc-rs-absent guard; new `android-ci.yml` (assembleDebug +
  ktfmtCheck, and an emulator `:core:connectedDebugAndroidTest` job). iOS build stays local
  (Xcode.xip is Apple-licensed) — hosted iOS CI is a Phase C decision.
- **Deltas from the pre-coding plan:** `module=".."` (plugin resolves relative to the root gradle
  project, not `:core`); compileSdk 36 (androidx requires it); Swift 5 language mode for the UniFFI
  target; device-only xcframework (no simulator on Linux); iOS built in Docker (no native Swift on
  the host). Deferred to later milestones: Android physical-device background tests (B2) and the
  iOS on-device run.
