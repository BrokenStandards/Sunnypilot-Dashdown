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

## 4. Maestro UI flows (black-box, mock + real hardware)

Black-box Compose-UI acceptance flows under `android/maestro/`. They complement — don't replace —
the instrumented tests in §3: the Phase C live-refresh behavior stays CI-guarded by
`DotLiveRefreshTest` / `DriveListLiveRefreshTest`; these flows mirror that plus the read-only feature
flows as black-box UI, run **locally and on the Pixel + comma**. **Not in CI** (Maestro can't
`adb reverse` to a host control port from Maestro Cloud; a self-hosted-emulator step is a later option).

### One command — `tools/run-maestro-e2e.sh`

```bash
# Full mock suite on the emulator (builds the APK + mock, reverses ports, installs, runs every flow):
ANDROID_SERIAL=emulator-5554 tools/run-maestro-e2e.sh

# A single flow (name, file, "all", or a directory all resolve):
ANDROID_SERIAL=emulator-5554 tools/run-maestro-e2e.sh connectivity_refresh_mock

# Real hardware against the comma (read-only) — explicit ANDROID_SERIAL is REQUIRED:
ANDROID_SERIAL=192.168.1.210:5555 tools/run-maestro-e2e.sh drive_download IP=192.168.1.100 PORT=8080 MODE=real
ANDROID_SERIAL=192.168.1.210:5555 tools/run-maestro-e2e.sh all IP=192.168.1.100 PORT=8080 MODE=real
```

Args: `[TARGET] [MODE=mock|real] [IP=…] [PORT=…] [NAME=…]`. Env knobs: `MOCK_PORT` (8099),
`CONTROL_PORT` (8098), `FIXTURE` (single_drive).

### Emulator runbook — boot → run → monitor → cleanup

The full chain against the `dashdown-b0` (api-35) AVD, headless. The harness builds the APK + mock,
starts `mock-copyparty`, `adb reverse`s both ports, installs, runs the flows, and **on exit kills the
mock + removes the reverses** (trap cleanup) — so the only manual teardown is the emulator itself.

```bash
# 1. Boot the emulator headless. QT_QPA_PLATFORM=offscreen is MANDATORY — the shell inherits
#    =wayland with no real display, so even -no-window aborts on a Qt plugin error without it.
nohup env -u WAYLAND_DISPLAY QT_QPA_PLATFORM=offscreen ANDROID_SDK_ROOT=/opt/android-sdk \
  /opt/android-sdk/emulator/emulator -avd dashdown-b0 -no-window -no-audio \
  -no-snapshot-save -no-boot-anim -gpu swiftshader_indirect -netdelay none -netspeed full \
  >/tmp/emu.log 2>&1 &

# 2. Wait for boot to finish (it comes up as emulator-5554).
adb -s emulator-5554 wait-for-device
until [ "$(adb -s emulator-5554 shell getprop sys.boot_completed | tr -d '\r')" = 1 ]; do sleep 2; done

# 3. Run the suite, targeting the emulator EXPLICITLY (a real Pixel may also be on adb — never let the
#    harness pick "first device"). `tee` keeps a log for monitoring/inspection.
ANDROID_SERIAL=emulator-5554 tools/run-maestro-e2e.sh all 2>&1 | tee /tmp/maestro.log
#    …or one flow:  ANDROID_SERIAL=emulator-5554 tools/run-maestro-e2e.sh connectivity_refresh_mock

# 4a. MONITOR per step (live) — Maestro streams every command; this trims the gradle noise to just
#     flow boundaries + per-step verdicts. Run in a second shell, or after backgrounding step 3.
tail -f /tmp/maestro.log | grep --line-buffered -E '==> flow:|\.\.\. (COMPLETED|FAILED|WARNED)|^    (PASS|FAIL)'

# 4b. MONITOR just-on-end — the per-flow verdicts + final tally (the harness exits non-zero on any fail):
grep -E '^==> flow:|^    (PASS|FAIL)|^==> all|flow\(s\) FAILED' /tmp/maestro.log

# 5. Cleanup — kill the emulator (the harness already killed the mock + removed the adb reverses).
adb -s emulator-5554 emu kill
```

To watch per-step while the run proceeds unattended, background step 3 (`… >/tmp/maestro.log 2>&1 &`)
and use the step-**4a** `tail -f` in another shell; the step-**4b** grep then gives the summary once
`==> all N flow(s) passed` (or `flow(s) FAILED`) lands.

### Flows (`android/maestro/`)

| Flow | Targets | Asserts |
|---|---|---|
| `empty_state` | both | fresh install shows "No devices yet…" |
| `add_device` | both | add a device through the form (`-e NAME/IP/PORT/WIFI_IP`) |
| `drive_download` | both | add → sync → download the first drive → "Complete" (device selected **by name**) |
| `play_drive` | both | open a completed drive → the multi-cam player mounts + the play toggle responds |
| `star_drive` | both | star a drive from the list → preserve icon flips `preserve_off`→`preserve_on` (local-only) |
| `manual_download_close` | both | start a download, `Home` (background), relaunch → still reaches "Complete" (dataSync FGS) |
| `remove_device` | both | remove one device by `-e NAME` (overflow → Remove) |
| `clear_devices` | both | remove every device |
| `connectivity_refresh_mock` | **mock** | dot flips green→red→green on the 8s poll as the server drops/restores — no refresh tap |
| `drive_list_refresh_mock` | **mock** | a route added/removed on the server appears/disappears on the 20s poll — no refresh tap |

### mock vs real

| | mock (default) | real (`MODE=real`) |
|---|---|---|
| Device under test | host `mock-copyparty` fixture | the comma at `IP:PORT` (read-only) |
| Data port | `127.0.0.1:$MOCK_PORT` over `adb reverse` (or `10.0.2.2` for `connectivity_refresh_mock`) | the comma's LAN IP, direct |
| Control port | host `CONTROL_PORT`, driven by `runScript` → `http` (host-side) | not used — `*_mock` flows are excluded |
| `ANDROID_SERIAL` | optional (first device) | **required** (never guesses) |
| Suite run | all flows | bounded read-only set: `empty_state add_device drive_download play_drive star_drive remove_device` |

**Real-hardware safety.** The comma serves a **read-only** copyparty volume — nothing in the app can
delete or reconfigure it (retention prunes only the Pixel's local mirror; star writes a local SQLite
flag). The only real risk is filling the Pixel, so the real suite downloads exactly **one** drive,
previews-only (qcamera, the add-device default), with autoSync **off** (the default) — no 33-drive
fan-out. Inspect the result with `tools/dd-db.sh drives`.

### The `-e` contract + gotchas

- Params: `NAME`, `IP`, `PORT`, `MODE` (mock|real), `CONTROL` (control port), `WIFI_IP`. The harness
  passes all of them to every flow; unused ones are harmless.
- **Never use a top-level flow `env:` block for an overridable var** — in Maestro a flow `env:`
  **shadows** CLI `-e` (precedence: flow `env:` > `-e` > OS). `remove_device.yaml` had a
  `env: { NAME: escape2020 }` block that silently ignored `-e NAME=…`; it was removed. Pass per-step
  vars via a `runScript:` `env:` block instead (scoped, no shadowing).
- `*_mock` flows gate their control-port steps `when: { true: "${MODE == 'mock'}" }`, so a stray real
  run never POSTs to the comma (and the harness doesn't schedule them in real mode anyway).
- **Red/green over `adb reverse` is masked** — a reverse tunnel accepts the device-side connect even
  when the host listener is closed. `connectivity_refresh_mock` therefore points the **data** port at
  the emulator host alias `10.0.2.2` (no reverse → a closed port is genuinely refused) while the
  control port stays reversed. Emulator-only; same trick as `DotLiveRefreshTest`.
- `runScript` runs **host-side** in Maestro's GraalJS engine; `scripts/{set_reachable,add_drive,
  remove_drive}.js` POST to the control port over plain host loopback (`http.post(url, {body, headers})`,
  body `JSON.stringify(...)`).

### Single-flow launcher — `tools/dd-ui.sh`

For one-off device setup (unchanged): `tools/dd-ui.sh <flow> [KEY=VAL …]` runs a single flow against
the connected device (injects `PORT`/`WIFI_IP` defaults).

```bash
tools/dd-ui.sh add_device NAME=escape2020 IP=192.168.1.100 [PORT=8080] [WIFI_IP=…]
tools/dd-ui.sh remove_device NAME=escape2020
tools/dd-ui.sh clear_devices
```

After a flow, dump the screen once via mobile-mcp `list-elements` to read the end state. Inspect the
on-device DB with `tools/dd-db.sh [devices|identity|drives|segments|schema|"<SQL>"]`.

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
