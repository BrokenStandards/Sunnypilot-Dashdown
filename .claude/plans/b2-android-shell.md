# B2 — Android App Shell (full)

## Context

Phase B builds the native shells over the shared Rust core. **B0** stood up the Android
toolchain and a minimal Compose app that loads the cross-compiled `.so` and calls
`ping()/version()/pingAsync()`. **B1** built `mock-comma-mcp` + the hermetic test harness
(Maestro, mobile-mcp). **B2** turns the B0 skeleton into the *full Android app*: the five-screen
UI, real background downloads (Foreground Service + WorkManager), and the hard-case acceptance
tests on a **physical Pixel 10 Pro XL** (connected over USB, arm64 — already covered by the B0
cargo-ndk `arm64`/`x86_64` targets).

The Rust core already owns the entire transfer engine (resumable, byte-range, SQLite index,
mirror store). The Android layer is a **driver + presenter + background host** — it never
re-implements transfer logic. "Done" for B2 means built, wired end-to-end, and verified by
automated tests including: **background→complete**, **kill/restart→resume-missing-only**,
**unreachable→red**, **interrupt→resume**.

### Verified FFI surface (from the generated `dashdown_core.kt`)
- Constructor is a **plain Kotlin constructor**: `AppCore(dbPath: String, mirrorRoot: String)`,
  `@Throws(CoreException)`. There is no `.new()`. `AppCore` is `AutoCloseable`; the last ref must
  only be released after downloads are **drained** (drop abandons in-flight tasks with no terminal
  callback — the Foreground Service exists to prevent that mid-download).
- Error type in Kotlin is **`CoreException`** (flat → single message string).
- All device/drive/download methods are **`suspend`**; `setProgressSink`/`setLogSink` are sync.
- `ProgressSink`/`LogSink` are Kotlin foreign-callback `interface`s. Records are `data class`;
  several fields are unsigned (`Device.port: UShort`, `*Size: ULong`, `files*: UInt`) → convert
  at the UI boundary.
- Package: `uniffi.dashdown_core`. New app code package: `org.sunnypilot.dashdown`.

### Locked decisions (from user)
1. **Storage:** external app-specific — `getExternalFilesDir(null)` for the mirror **and** the
   SQLite index, with a fallback to `filesDir` if external is unavailable (null). Capacity for
   GB-scale footage; permissionless; app-private.
2. **Password:** core-DB-only (app-private SQLite where the core needs it anyway) +
   `android:allowBackup="false"`. **No** `androidx.security-crypto`. Edit form is write-mostly
   (blank = keep existing).
3. **Delivery:** staged sub-PRs (8 gated steps), each `ktfmtCheck` + `cargo fmt/clippy` + tests
   green and independently reviewable; a final integration PR enables the physical-device flows.
4. **Auto-sync:** auto-download on Wi-Fi — the worker refreshes the index then escalates to the
   Foreground Service to download `NotDownloaded`/`Partial` drives (constraints: unmetered +
   battery-not-low).

## Architecture

**No Hilt.** A hand-rolled service locator on a custom `Application`, plus one thin repository.

- **`DashdownApp : Application`** — holds the single `AppCore` (constructed off the main thread on
  first use), the singleton `ProgressBus`, and a `LogcatLogSink`; installs both sinks on the core
  exactly once. Resolves storage paths (external-with-fallback), creates parent dirs before
  constructing `AppCore`. Schedules the periodic `AutoSyncWorker`. Registered via
  `android:name=".DashdownApp"`.
- **`DashdownRepository`** — 1:1 wrapper over `AppCore` that (a) maps `CoreException` → a sealed
  `UiError`, (b) converts unsigned types, (c) merges live progress (`ProgressBus`) with
  `getDriveStatus`/`listDrives` snapshots, (d) exposes `downloads: StateFlow<Map<driveKey, …>>`.
  Single instance; never builds a second `AppCore`. Seam for fakes in tests.
- **`ProgressBus : ProgressSink`** — the only sink the core sees. Holds
  `MutableStateFlow<Map<driveKey, DriveProgress>>` (atomic `update {}`; UniFFI calls arrive on
  tokio threads) plus a `SharedFlow<TerminalEvent>` (replay 0) for one-shot completed/failed so the
  Service/Worker can react without polling. Hot, app-lifetime. Screen flows are cold, VM-scoped,
  `combine`d with the bus and `stateIn(WhileSubscribed(5000))`.

ViewModels are `AndroidViewModel` subclasses (reach the locator via the application) using a shared
`DashdownViewModelFactory(repo)`; they expose `StateFlow<…UiState>` and do work in
`viewModelScope`. The **Foreground Service** and **Worker** read the same locator — never a second core.

## Background execution

- **`DownloadService : Service`** (type `dataSync`) — the **keep-alive + notification + cancel host**,
  not the downloader (the core's owned runtime runs the detached task). `onStartCommand` →
  `startForeground(NOTIF_ID, notif, FOREGROUND_SERVICE_TYPE_DATA_SYNC)` within 5s; **the service
  owns `startDriveDownload`** so the `SyncHandle` and keep-alive share a lifetime (UI "Download"
  just `startForegroundService`s with `deviceId`+`driveKey`). Tracks active keys in a
  `ConcurrentHashMap<String, SyncHandle>`; rebuilds the notification from `ProgressBus` (throttled
  ~1/s, determinate from `bytesDone/bytesTotal`); a notification **Cancel** action → `handle.cancel()`.
  On terminal events for all keys → `stopForeground(REMOVE)` + `stopSelf`, then posts a separate
  completed/failed notification. Implements `onTimeout` (Android 14+ `dataSync` cap) with a graceful
  stop (mock fixtures never hit the ~6h/24h cap, but the handler must exist).
- **`AutoSyncWorker : CoroutineWorker`** — constraints `UNMETERED` + `setRequiresBatteryNotLow`;
  for each `autoSync` device: `syncNow` + `runMaintenance`, then if unmetered and drives are
  `NotDownloaded`/`Partial`, **start `DownloadService`** for them (worker stays short, one
  downloader/notification owner). Periodic `KEEP` (~6h), re-ensured when `autoSync` toggles.
- **Manifest additions:** `INTERNET`, `ACCESS_NETWORK_STATE`, `FOREGROUND_SERVICE`,
  `FOREGROUND_SERVICE_DATA_SYNC`, `POST_NOTIFICATIONS`; `<service .DownloadService
  foregroundServiceType="dataSync" exported="false"/>`; `android:name=".DashdownApp"`,
  `allowBackup="false"`. `POST_NOTIFICATIONS` requested at runtime (API 33+); downloads still run
  if denied (silent notification).

## Screens, ViewModels, and core calls

Single-activity Navigation-Compose graph in `AppNavHost.kt`; `MainActivity` rewritten to
`DashdownTheme { AppNavHost() }` + the notification-permission launcher.

| Screen | Route | Core calls |
|---|---|---|
| **Device list** | `devices` | `listDevices()`; per-device `checkConnectivity(id)` → dot; `listDrives(id, offline=true)` for sync summary; `removeDevice(id)`; merge `ProgressBus` for Downloading badge |
| **Add/Edit device** | `device/edit?deviceId={id}` | load via `listDevices()` find-by-id (no `getDevice` in FFI); `addDevice(Device)` / `updateDevice` / `setActiveMode`; `Device.port` parsed to `UShort` |
| **Drives list** | `device/{id}/drives` | `listDrives(id, offline=true)` initial; PullToRefresh → `listDrives(id, offline=false)`; `setPreserved`; Download → `startForegroundService` |
| **Drive detail** | `device/{id}/drive/{driveKey}` | `getDrive` + `getDriveStatus`; Download/Resume → `startForegroundService` (resume is automatic); Cancel → `DownloadService` `ACTION_CANCEL`; `setPreserved`; Export → SAF (below); inline playback (below) |
| **Per-device settings** | `device/{id}/settings` | `getSettings(id)` / `setSettings(id, DeviceSettings)`; 8-bool `FileSelection`; toggling `autoSync` re-ensures the worker |

- **Media3 playback (Drive detail):** plays a **complete** `qcamera.ts` (MPEG-TS; only realistically
  playable stream). Resolve `File(mirrorRoot, "$deviceId/realdata/${seg dirName}/qcamera.ts")` →
  `MediaItem.fromUri(Uri.fromFile(...))` (in-process, no FileProvider). `ExoPlayer` in
  `remember`/`DisposableEffect`, hosted via `AndroidView { PlayerView }`. Only offered when downloaded.
- **SAF zip export:** `ActivityResultContracts.CreateDocument("application/zip")` →
  `exportDriveZip(id, driveKey, tempInCacheDir)` → stream temp → `contentResolver.openOutputStream(uri)`
  → delete temp. (Reconciles the path-based core API with SAF content URIs.)

## Dependencies (catalog + `:app/build.gradle`)

Add to `gradle/libs.versions.toml` and `:app` (pin resolved latest-stable at implementation time —
**version-check** each): `androidx.navigation:navigation-compose`,
`androidx.lifecycle:lifecycle-viewmodel-compose` + `lifecycle-runtime-compose`
(`collectAsStateWithLifecycle`), `androidx.work:work-runtime-ktx`,
`androidx.media3:media3-exoplayer` + `media3-ui`. Tests: `androidx.work:work-testing`,
`androidx.test.uiautomator:uiautomator`. Compose BOM `2025.01.01` stays (covers `PullToRefreshBox`).
**No** `security-crypto`, **no** DataStore (a one-flag `SharedPreferences` covers "notif asked").

## File / package layout (`org.sunnypilot.dashdown`)
```
app/.../DashdownApp.kt              ServiceLocator, AppCore, ProgressBus, sinks, worker scheduling
app/.../core/ProgressBus.kt         ProgressSink impl → StateFlow + terminal SharedFlow
app/.../core/LogcatLogSink.kt       LogSink → Logcat
app/.../data/DashdownRepository.kt  thin wrapper, UiError mapping, progress merge
app/.../data/UiModels.kt            DriveProgress, UiError, *UiState, DeviceForm
app/.../service/DownloadService.kt  foreground dataSync host + notification + cancel
app/.../work/AutoSyncWorker.kt      CoroutineWorker auto-sync + escalate
app/.../ui/AppNavHost.kt            nav graph + testTagsAsResourceId root
app/.../ui/devices/{DeviceListScreen,DeviceListViewModel}.kt
app/.../ui/edit/{DeviceEditScreen,DeviceEditViewModel}.kt
app/.../ui/drives/{DrivesListScreen,DrivesListViewModel}.kt
app/.../ui/detail/{DriveDetailScreen,DriveDetailViewModel}.kt
app/.../ui/settings/{DeviceSettingsScreen,DeviceSettingsViewModel}.kt
app/.../ui/ViewModelFactory.kt
app/src/main/AndroidManifest.xml    (perms, service, app name, allowBackup=false)
```
`MainActivity.kt`, `ui/theme/*` already exist (theme reused; MainActivity rewritten).

## Build order — 8 gated sub-PRs

Each ends green on `./gradlew :app:assembleDebug :core:assembleDebug ktfmtCheck` + workspace
`cargo fmt --all --check` / `cargo clippy --all-targets -- -D warnings` (Rust untouched but the gate
stays) + its instrumented test.

1. **DI/wiring skeleton** — `DashdownApp` + locator + repository + `ProgressBus` + `LogcatLogSink`;
   manifest perms + `allowBackup=false`; external-with-fallback paths. Test: instrumented build of
   `AppCore`, `listDevices()` empty, sink installed.
2. **Nav shell + Device list (read-only)** — graph + list with connectivity dots. Test: Compose +
   fake repo.
3. **Add/Edit device + settings** — add/update/remove, get/set settings. Test: form round-trip vs
   real core + mock-provisioned device.
4. **Drives list (offline + PullToRefresh online)** — grouping/badges/preserve. Test: `single_drive`/
   `gap_split` via `adb reverse` + mock.
5. **Foreground Service + download from UI** — service, POST_NOTIFICATIONS, progress→notification,
   cancel. Test: **background→complete** on emulator.
6. **Drive detail + Media3 + SAF export** — getDrive/getDriveStatus, qcamera.ts playback, zip export.
   Test: valid zip produced; player loads a complete segment.
7. **WorkManager auto-sync + escalation** — worker + scheduling + start-service. Test:
   `WorkManagerTestInitHelper` drives the worker vs mock.
8. **Acceptance hardening (physical device)** — kill/restart→resume, interrupt→resume,
   unreachable→red as Maestro + mobile-mcp flows; finalize all `testTag`s; add
   `:app:connectedDebugAndroidTest` to CI.

## Verification — hard cases against mock-comma-mcp

**Wiring:** mock runs on the host at `127.0.0.1:<stable port>`. Physical Pixel (USB):
`adb reverse tcp:<port> tcp:<port>` and configure the in-app device as `hotspotIp=127.0.0.1`. CI
emulator: `hotspotIp=10.0.2.2`. Provision via `provision_device(device_id, fixture)`, read back the
port, set up the reverse, then drive the UI.

- **(a) background→complete** *(emulator + device):* provision `single_drive`, add device, Download,
  assert FGS foreground (`dumpsys activity services`), HOME to background, poll mock `status` /
  `getDriveStatus` to complete, reopen → **Complete** badge, mirror files present
  (`run-as … ls`).
- **(b) kill/restart→resume-missing-only** *(device):* multi-segment `single_drive`, start, interrupt
  early, `adb shell am force-stop`, relaunch (core recovers killed job → resumes `.part` via Range),
  Resume → assert already-complete files are **not** refetched (mock served-bytes/request count if
  exposed; else `.part` offset / `bytesDone > 0`) and drive ends Complete.
- **(c) unreachable→red** *(device + emulator socket case):* Green when idle → `set_reachable(false)`
  → `checkConnectivity`/pull-refresh → dot **Red**; **Blue** while a download is active; flip mid-
  download → Red + job fails; `set_reachable(true)` → Green.
- **(d) interrupt→resume** *(device):* start, `set_reachable(false)` mid-transfer → Failed/Partial,
  `set_reachable(true)` → Resume completes from `.part` (no full refetch). Variant: `size_mismatch`
  fixture → core classifies `SizeMismatch` → drive `Partial`/resumable → UI shows resumable badge.

**Test layering:** Espresso/Compose instrumented (logic, classification vs fixtures, SAF, nav,
ProgressBus, background→complete logic) run on the **emulator** in CI (extend the existing
`connectedDebugAndroidTest` job to include `:app`); Maestro YAML + mobile-mcp drive the
**physical-device** acceptance cases (b)(c)(d) that need real `am force-stop`/timing. Compose nodes
carry `testTag`s with `semantics { testTagsAsResourceId = true }` on the nav root:
`device_row_{id}`, `device_conn_dot` (contentDescription `conn_dot_{green|blue|red}`),
`add_device_fab`, `device_form_{name,hotspot_ip,wifi_ip,port,password,mode_toggle,save}`,
`drive_row_{key}`, `drive_sync_badge` (`sync_{complete|partial|downloading|not_downloaded|failed}`),
`drive_preserve_star`, `drives_pull_refresh`, `drive_detail_{download_btn,cancel_btn,export_btn,player}`,
`drive_progress`, `settings_{autosync,file_*,retention,autodelete,min_age,save}`.

## Boundaries (NOT in B2)
- **B3** (iOS shell), **B4** (shared cross-platform Maestro flows + agentic screenshot checks),
  **B5** (native CI incl. iOS xtool job). B2 only extends `android-ci.yml` to add `:app`
  instrumented tests on the emulator; the physical Pixel cases run locally (CI can't host it).
- Retention/auto-delete *logic* is core (M6, done); B2 only surfaces the settings and tests against
  the mock.
- `getDevice` is absent from the FFI — editing uses `listDevices()` find-by-id; adding it to the core
  is out of B2 scope (flagged).

## Workflow
Per CLAUDE.md: branch → PR → CI green → squash-merge per sub-PR; `ktfmt` + `cargo fmt`/`clippy -D
warnings` + tests gate every step. Don't commit build outputs, generated bindings, `.so`, secrets,
`local.properties`. Rename this plan file to `b2-android-shell.md` on approval (matching the B0/B1
naming).
