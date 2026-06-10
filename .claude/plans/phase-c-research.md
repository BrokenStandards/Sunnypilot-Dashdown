# Phase C Research — Foreground Live UI Refresh (Status Only)

> Research/discovery notes (multi-agent sweep, all file:line-verified). Feeds the Phase C plan.
> **Scope:** while a screen is OPEN, the connectivity dot and the drive list update on their own —
> no manual refresh, no spinner flash. Phase C **never initiates downloads**; it reflects status the
> B2 background engine / device already produced.

## 1. Current-state map

### 1.1 Device list + connectivity dot
- `ui/devices/DeviceListScreen.kt:44-69` — only re-probe trigger today is one-shot on RESUME:
  `LifecycleResumeEffect(Unit){ vm.refresh(); onPauseOrDispose{} }` (`:55-60`), state via `collectAsStateWithLifecycle`.
- `DeviceListViewModel.kt:29-69` — `refresh()` sets `loading=true` (`:40`), then per-device
  `repo.checkConnectivity(d.id).dot` + `summarize(listDrives(offline=true))` concurrently, failure-tolerant
  (`.getOrNull()` → gray dot). State: `DeviceListUiState(rows, loading, error)`; `DeviceRow(device, dot: ConnDot?, summary)`. No ticker.
- `ui/components/Dots.kt:16-45` — `ConnDotIndicator`, `contentDescription = "conn_dot_{green|blue|red|unknown}"` (test-addressable; no change needed).
- `rust/core/src/model/mod.rs:42-49` — `ConnDot { Green=reachable+idle, Blue=reachable+downloading, Red=unreachable }`.

### 1.2 Drives list + refresh state machine
- `ui/drives/DrivesListScreen.kt:69-95` — on RESUME runs only `vm.loadOffline()` (no network) (`:79-82`).
- Spinners (`:121-129`): pull-to-refresh `PullToRefreshBox(isRefreshing = state.refreshing)` (`:122`);
  initial `CircularProgressIndicator` when `state.loading && state.drives.isEmpty()` (`:126`).
- `DrivesListViewModel.kt:16-102` — `DrivesUiState(drives, loading=true, refreshing=false, error)`.
  `init` offline-loads then auto-`refreshOnline()` if empty; also `repo.terminalEvents.collect { loadOffline() }` (`:52`).
  `loadOffline()` = `listDrives(offline=true)` (network-free); `refreshOnline()` sets `refreshing=true` → `listDrives(offline=false)` → false (the loud sync).
- **Already live:** per-drive progress bars + SyncStatus badges via `ProgressBus` (`core/ProgressBus.kt`, `DriveRow` from `live: DriveProgress?`).
  **NOT live:** drive-list **membership** (add/remove) and the **connectivity dot** — the precise hole Phase C fills.

### 1.3 Lifecycle / Compose plumbing
- `collectAsStateWithLifecycle` (lifecycle **2.9.4**, `android/app/build.gradle:58`) used on every screen.
- `LifecycleResumeEffect` used for one-shot resume refreshes. **`repeatOnLifecycle` is NOT used anywhere — must be introduced.**
- ViewModels created per `*Route` via `viewModel(factory=…)`; repo from `rememberRepository()` (`ui/Locator.kt:9-13`), process-wide singleton.
- No foreground interval poller exists today (background `SyncSessionWorker` uses `RECORDING_POLL_MS=30s`/`POLL_MS=500ms`; `MultiCamPlayer` has a 100ms clock).

### 1.4 Repository / FFI surface
- `data/DashdownRepository.kt`: `checkConnectivity` (`:122-124`), `listDrives(offline)` (`:62-64`), `syncNow` (`:74`),
  `progress: StateFlow` (`:29`), `terminalEvents: SharedFlow`. All suspend on `Dispatchers.IO`.
- `rust/core/src/ffi/mod.rs:177-184` — `list_drives(offline)`: `true`→`reconcile_device` (local, no net); `false`→`sync_now` (network index refresh).
- `sync_engine/mod.rs:189-204` — `sync_now` is **index-only, download-free** (list→upsert→group→replace→reconcile). No spinner-suppression flag in core (Kotlin-side concern).

### 1.5 Background / connectivity model (B2)
- `sync_engine::check_connectivity` (`sync_engine/mod.rs:295-336`): TCP probe candidate IPs (2s timeout); unreachable→Red; else `has_active_job`→Blue/Green.
  **The dot live-reflects B2 downloads for free** — Phase C just calls it periodically.
- `SyncSessionWorker.kt`: only download initiator is `repo.startDriveDownload(...)` (`:103`), background-only.
- DB: rusqlite + r2d2, `WAL`, `busy_timeout=5000`, transactional mutations, read-only `has_active_job`, all via `spawn_blocking` → concurrent foreground+background `sync_now` serializes safely (no deadlock/corruption).

## 2. Gaps Phase C must fill
1. Lifecycle-scoped interval re-probe of the dot while RESUMED (device list).
2. Lifecycle-scoped interval re-sync of drive-list **membership** while RESUMED (`listDrives(offline=false)`).
3. **Silent-variant state handling (no spinner flash)** — the core correctness gap: a poll tick must NOT set
   `refreshing`/`loading` (drives) or `loading` (device list; today `refresh()` always sets `loading=true` at `DeviceListViewModel.kt:40`).
4. Cadence constant(s), test-injectable.
5. Minor: stale-dot UX on transient probe failure (avoid green↔gray flicker).

## 3. Design options + recommendation
- **(a) Polling primitive → `repeatOnLifecycle(RESUMED)` in each `*Route`** calling a new `vm.silentRefresh()`.
  Auto-cancels on pause (matches "only while OPEN"); ViewModel-ticker rejected (`viewModelScope` not lifecycle-scoped).
  Optional `@Composable PollWhileResumed(intervalMs, key, block)` helper to avoid duplication.
- **(b) Silent refresh → a dedicated `silentRefresh()`** that updates `rows`/`drives` only, never toggling
  `loading`/`refreshing` and not clearing/setting `error` on a tick (so a transient poll error doesn't blank a working screen).
  Loud paths (`refresh()`, `refreshOnline()`) untouched; PullToRefreshBox stays bound to `state.refreshing`.
- **(c) Cadence → dot ≈ 8s, drives ≈ 20s** defaults (B2's 30s is the anchor), **injectable** via a ViewModel default
  param overridable from an instrumentation arg (`testPollIntervalMs`, wired through `tools/run-android-e2e.sh`).
  Add error backoff (~30s after a failed tick).

## 4. Concurrency & correctness
- Foreground-poll vs B2 `syncNow` overlap is **safe** (WAL + busy_timeout serialize writers; `sync_now` idempotent + download-free;
  a poll's `sync_now` even helps heal stale `running` jobs via `reconcile`). Worst case = duplicated network/index cost.
- **Guardrails (recommended):** skip the drive network tick when `checkConnectivity().downloading` (defer to B2);
  reachability-gate the drive poll; `withTimeoutOrNull` around poll calls.
- **No downloads from Phase C:** only initiator is `repo.startDriveDownload` (background worker); manual via `DownloadService`
  (button callbacks). Phase C touches only `checkConnectivity` + `listDrives(offline=false)`. Airtight.

## 5. Self-test strategy
- Infra: `MockControl.post(port,path,json)` (raw-socket to mock control port); control endpoints `/reachable {up}`,
  `/add_drive {route,segs}`, `/remove_drive {route}`, `/add_segment`, `GET /status` (`rust/mock-copyparty/src/control.rs:97-179`);
  `tools/run-android-e2e.sh` starts mock + adb reverse + instrumentation args; precedent in `SyncSessionDriveAddTest`.
- **Use `createAndroidComposeRule<MainActivity>()`** (real Lifecycle) + real `locator.repository` + `rule.waitUntil` — the
  stateless `createComposeRule` used by existing screen tests cannot drive `repeatOnLifecycle`.
- Test 1: dot red↔green via `/reachable {up:false/true}`, assert `conn_dot_red`/`conn_dot_green` flips with NO manual refresh.
- Test 2: drive list add/remove via `/add_drive` + `/remove_drive` on a **dedicated route** (cleaned up in `finally`), assert row appears/disappears.
- No-spinner-flash: assert at the VM level that the silent path leaves `refreshing==false` (UI "spinner absent" is flaky).
- Anti-flake: inject short interval (`testPollIntervalMs=100`) + `waitUntil(timeout)`, per-test isolation/cleanup, never `sleep`.

## 6. Open questions — RESOLVED (user, 2026-06-10)
1. **Cadence:** dot **8s** / drives **20s**; reachability-gate + defer-to-B2 the drive poll. ✅
2. **Scope:** **device list + drives list ONLY** (roadmap scope). Drive DETAIL screen is NOT in Phase C. ✅
3. **Stale-dot:** keep last-known color (hysteresis), don't flicker to gray on a transient probe failure. ✅ (recommended)
4. **autoSync default/label:** **keep deferred** — separate change, not Phase C. ✅
5. **Drive poll uses `offline=false`** (network membership refresh) per "silent network re-sync." ✅
6. **Testing on the EMULATOR** (user's physical Pixel is off-network; the comma is present but irrelevant — use the clean emulator).

## 7. Risks
Battery/network churn (mitigate: cadence, backoff, reachability-gate, defer-to-B2); lifecycle leaks (must be `repeatOnLifecycle`,
not bare `viewModelScope`); spinner flash (dedicated `silentRefresh()` + VM test); scroll-jump (LazyColumn keyed by `driveKey`;
prefer stable sort); dot flicker (last-known hysteresis); test flakiness (injectable interval + `waitUntil` + `createAndroidComposeRule`).

## Anchor points
| Concern | File:line | Change |
|---|---|---|
| Dot poll loop | `DeviceListScreen.kt:57-60` | `repeatOnLifecycle(RESUMED){ while(true){ delay(dotMs); vm.silentRefresh() } }` |
| Silent dot refresh | `DeviceListViewModel.kt:38-61` | add `silentRefresh()` (no `loading`/`error` toggle) |
| Drive poll loop | `DrivesListScreen.kt:79-82` | `repeatOnLifecycle(RESUMED)` loop → `vm.silentRefresh()` |
| Silent drive refresh | `DrivesListViewModel.kt:68-78` | add `silentRefresh()` (network `listDrives(offline=false)`, no `refreshing`/`error` toggle) |
| Cadence + injection | new const + instrumentation arg | wire `testPollIntervalMs` in `tools/run-android-e2e.sh` |
| Live tests | new `androidTest` files | `createAndroidComposeRule<MainActivity>()` + `MockControl` `/reachable`,`/add_drive`,`/remove_drive` + `waitUntil` |

**No Rust changes required** — `checkConnectivity` + `list_drives(offline=false)` already provide everything; `sync_now` is verified download-free.
