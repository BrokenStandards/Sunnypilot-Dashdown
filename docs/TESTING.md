# Testing — Sunnypilot Dashdown

How the project is tested and how to run every layer. Three layers: the **Rust core** (`cargo test`),
the **Android JVM unit tests** (no device), and the **Android instrumented tests** (real
device/emulator) — the last split into hermetic UI tests and **live** tests that run against the
`mock-copyparty` fixture.

CI runs all of it on every PR (`.github/workflows/android-ci.yml`): an `assemble` job (Rust build +
test + cross-compile + bindgen, Android assemble + unit tests + `ktfmtCheck`) and an
`on-device tests (emulator)` job that now runs the connected suite **with the mock wired in**, so
the live + background-sync tests actually execute (they previously self-skipped).

---

## 1. Rust core — `cargo test`

```bash
cargo test                      # whole workspace (unit + integration)
cargo test -p dashdown-core     # the core crate only
cargo test -p mock-copyparty    # the test fixture server
cargo test -p mock-comma-mcp    # the MCP wrapper
```

**Unit tests** live in `#[cfg(test)]` modules next to the code (e.g. `model/`, `drive_grouping/`,
`sync_engine/resume.rs`, `sync_engine/retention.rs`, `identity.rs`, `connectivity/`,
`copyparty_client/{listing,pinning}.rs`, `video/remux.rs`, `mock-copyparty/src/mutate.rs`).

**Integration tests** (`rust/core/tests/`, one binary per file):

| File | Covers |
|---|---|
| `it_appcore.rs` | `AppCore` FFI surface end-to-end (in-process) |
| `it_drive_grouping.rs` / `it_offline_grouping.rs` | grouping segments into drives (network + on-disk) |
| `it_download.rs` / `it_resume.rs` | download, cancel mid-download, `.part`-file resume |
| `it_listing.rs` / `it_listing_auth.rs` | copyparty listing parse (+ password) |
| `it_connectivity.rs` | multi-IP reachability / dot |
| `it_identity.rs` / `it_tls.rs` | hostname identity, cert TOFU-pinning, HTTPS verifier |
| `it_db.rs` | schema + migrations (current `schema_version`) |
| `it_retention.rs` | retention/prune policy (preserve pins) |
| `it_remux_local.rs` | HEVC→MP4 remux |
| `it_mock_harness.rs` | the fixture server itself |

**Gated real-server tests** (run locally, skip without the env var):
- `it_real_copyparty.rs` — runs against a locally-launched real `copyparty` (see
  [REFERENCES.md](REFERENCES.md) / the `copyparty-http-api` memory).
- `it_live_device.rs` — runs against a real Comma on the LAN.

`mock-copyparty/tests/it_control.rs` and `mock-comma-mcp/tests/it_mcp.rs` cover the runtime-mutation
control plane (add/remove drive, add segment, toggle reachability) used by the Android live tests.

---

## 2. Android JVM unit tests (no device)

```bash
cd android
JAVA_HOME=/usr/lib/jvm/java-17-openjdk ./gradlew :app:testDebugUnitTest :core:testDebugUnitTest --no-daemon
```

| Class | Methods |
|---|---|
| `ui.detail.RouteTimelineTest` | timeline span / gap / multi-segment math (8 tests) |
| `ui.detail.RouteClockTest` | route wall-clock formatting |

Formatting (CI's `assemble` runs `ktfmtCheck`): run `./gradlew ktfmtFormat` on changed Kotlin before
committing.

---

## 3. Android instrumented tests (device / emulator)

Run on a connected device or emulator. Build with cargo-ndk (the `:core` Rust `.so`), JDK 17.

### One command (recommended) — `tools/run-android-e2e.sh`

Builds + starts `mock-copyparty` (data **and** control port), sets up `adb reverse`, and runs the
connected suite with `mockPort`/`controlPort` so the **live** tests execute; tears the mock down on
exit.

```bash
# all connected tests, both modules:
tools/run-android-e2e.sh

# pick a device when several are attached (e.g. a phone + an emulator):
ANDROID_SERIAL=emulator-5554 tools/run-android-e2e.sh

# scope to one class:
ANDROID_SERIAL=emulator-5554 tools/run-android-e2e.sh \
  -Pandroid.testInstrumentationRunnerArguments.class=org.sunnypilot.dashdown.SyncSessionWorkerLiveTest
```

Env knobs: `MOCK_PORT` (8099), `CONTROL_PORT` (8098), `FIXTURE` (single_drive),
`GRADLE_TASKS` (both modules' `connectedDebugAndroidTest`).

This is exactly what CI's `on-device tests (emulator)` job runs.

### Hermetic tests (no mock needed)

| Class | Methods |
|---|---|
| `core.CoreLoadTest` (`:core`) | `syncFfiWorks`, `asyncFfiWorks` |
| `WiringTest` | `locatorSingletonsAreStable`, `coreReachableThroughRepository` |
| `DeviceCrudTest` | `addUpdateSettingsRemoveRoundTrip` |
| `ui.edit.DeviceEditScreenTest` | `fieldsPresent`, `saveDisabledWhenNameBlank`, `saveDisabledWhenPortInvalid`, `saveEnabledWhenValid` |
| `ui.devices.DeviceListScreenTest` | `showsRowsAndConnectivityDots`, `showsEmptyState` |
| `ui.drives.DrivesListScreenTest` | `showsDrivesWithBadges`, `showsEmptyState` |
| `ui.detail.DriveDetailScreenTest` | `rendersHeaderActionsAndFiles` |
| `ConnectivityLiveTest` | `unreachableDeviceShowsRed` (points at a closed port — needs no fixture) |

### Live tests (need the mock fixture + `adb reverse`; gated by `mockPort`)

| Class | Method | Asserts |
|---|---|---|
| `DrivesSyncLiveTest` | `syncGroupsSingleDriveFixture` | sync groups the 3-segment fixture into one drive |
| `ConnectivityLiveTest` | `reachableDeviceShowsGreen` | reachable device → green dot |
| `DownloadServiceLiveTest` | `serviceDownloadRunsToComplete` | the manual `DownloadService` (real FGS) downloads to COMPLETE |
| `DriveExportLiveTest` | `downloadThenExportProducesZip` | download then export a valid zip |
| `ResumeAfterInterruptLiveTest` | `resumeAfterFileLossReachesComplete` | interrupted transfer resumes to COMPLETE |
| `SyncSessionWorkerLiveTest` | `sessionSyncsAndDownloads` | **background session** auto-downloads (no UI) |
| `SyncSessionSegmentPickupTest` | `segmentAddedToActiveDriveGetsSynced` | **segment appended to an active drive → synced** |
| `SyncSessionDriveAddTest` | `newDriveGetsDownloaded` | **new drive appears → auto-downloaded** |
| `SyncSessionScheduledFgsTest` | `scheduledWorkerPromotesDataSyncFgsAndDownloads` | **real WorkManager `dataSync` FGS path** downloads to COMPLETE (regression guard for the `SystemForegroundService` manifest merge) |

The `SyncSession*` tests drive runtime state changes through the mock's **control port** (raw-socket
`MockControl` helper) and isolate their mutations on dedicated routes (cleaned up in `finally`) so
they never pollute the shared fixture.

### Media-decode tests (self-skip on the AOSP emulator)

These need a real HEVC decoder / a staged file and `assumeTrue`-skip otherwise (so they're no-ops in
CI). Run on real hardware:

| Class | Method |
|---|---|
| `MultiCamHevcPlaybackLiveTest` | `remuxedRoadCameraDecodesAndSeeksOnDevice` |
| `DeviceRemuxDecodeTest` | `platformDecoderPlaysAndSeeksRemuxedMp4` (stage a remuxed MP4 first) |

### Manual fallback (no helper)

```bash
cargo run -q -p mock-copyparty -- --fixture single_drive --port 8099 --control-port 8098 &
adb reverse tcp:8099 tcp:8099 && adb reverse tcp:8098 tcp:8098
cd android && ./gradlew :app:connectedDebugAndroidTest \
  -Pandroid.testInstrumentationRunnerArguments.mockPort=8099 \
  -Pandroid.testInstrumentationRunnerArguments.controlPort=8098
```

---

## 4. Maestro UI flows (real-device black-box)

Parameterized flows under `android/maestro/`, driven by `tools/dd-ui.sh` (sets JDK 17):

```bash
tools/dd-ui.sh add_device NAME=escape2020 IP=192.168.1.100 [PORT=8080] [WIFI_IP=…]
tools/dd-ui.sh remove_device NAME=escape2020
tools/dd-ui.sh clear_devices
# drive_download.yaml — happy-path add-device + download against the mock fixture
```

After a flow, dump the screen once via mobile-mcp `list-elements` to read the end state.

Inspect the on-device DB with `tools/dd-db.sh [devices|identity|drives|segments|schema|"<SQL>"]`.

---

## 5. CI summary

| Job | Runs |
|---|---|
| `assemble (cargo-ndk + bindgen + APK)` | `:app:assembleDebug :core:assembleDebug :app:testDebugUnitTest ktfmtCheck` |
| `build · test · cross-compile · bindgen` | `cargo test` + cross-compile + UniFFI bindgen |
| `on-device tests (emulator)` | api-35 AOSP emulator → `tools/run-android-e2e.sh` (connected suite **with the mock**; media-decode tests self-skip) |
| `claude-review` | automated review |

iOS is not built in GitHub Actions (needs an Apple-licensed SDK); it's verified locally via
`tools/ios-build.sh` — see [REFERENCES.md](REFERENCES.md).
