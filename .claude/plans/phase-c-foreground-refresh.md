# Phase C — Foreground live UI refresh (status only)

> On approval, saved as `.claude/plans/phase-c-foreground-refresh.md`. Full research:
> `.claude/plans/phase-c-research.md`.

## Context

The app's per-drive download **progress/badges** already update live (via `ProgressBus`), but two things
only refresh on screen *resume* or pull-to-refresh: the **connectivity dot** (device list) and **drive-list
membership** (drives appearing/removed). The user wants both to update **on their own while a screen is open**,
with **no manual refresh and no spinner flash** — purely reflecting status the B2 background engine / device
already produced. Phase C **initiates no downloads**.

Verified in research: **no Rust changes** — `checkConnectivity` and `listDrives(offline=false)` already provide
everything, the dot's Blue/Green/Red (incl. "downloading") is computed in-core, and `sync_now` is download-free.
A foreground `syncNow` poll racing B2's background `syncNow` is **DB-safe** (WAL + `busy_timeout`).

**Decisions (locked with user):** scope = device list + drives list only (drive *detail* excluded); cadence =
**dot 8s / drives 20s**, reachability-gated + defer-to-B2; stale dot keeps last-known color (hysteresis);
autoSync default/label stays deferred; **all testing on the emulator** (Pixel is off-network).

## Design

**1. A lifecycle-scoped poll helper** — new `ui/components/PollWhileResumed.kt`:
```kotlin
@Composable
fun PollWhileResumed(intervalMs: Long, key: Any? = Unit, tick: suspend () -> Unit) {
  val owner = LocalLifecycleOwner.current
  LaunchedEffect(owner, key, intervalMs) {
    owner.lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
      while (true) { delay(intervalMs); tick() }   // delay-first: resume already did the loud load
    }
  }
}
```
Auto-cancels below RESUMED — polling runs **only while the screen is open** (the Phase C requirement) and
never from a bare `viewModelScope` (which would leak into the background). `repeatOnLifecycle`/`LocalLifecycleOwner`
are already on the classpath (lifecycle 2.9.4), unused so far.

**2. Silent ViewModel refreshes** (never toggle spinners/error):
- `DeviceListViewModel.silentRefresh()` — same per-device fan-out as `refresh()` but updates only `rows`
  (never `loading`/`error`). **Hysteresis:** a device whose `checkConnectivity` probe fails keeps its
  previous dot (`?: prevDot[d.id]`) instead of flashing to gray.
- `DrivesListViewModel.silentRefresh()` — reachability-gated, defer-to-B2:
  ```kotlin
  fun silentRefresh() = viewModelScope.launch {
    val conn = runCatching { repo.checkConnectivity(deviceId) }.getOrNull()
    if (conn?.reachable != true) return@launch                 // don't network-poll a dead device
    val offline = conn.downloading                              // B2 active → just reclassify from disk
    val drives = runCatching { repo.listDrives(deviceId, offline = offline) }.getOrNull() ?: return@launch
    _state.update { it.copy(drives = drives) }                 // never touch refreshing/loading/error
  }
  ```
  The loud paths (`refresh()`, `refreshOnline()`, `loadOffline()`) are unchanged; `PullToRefreshBox` stays
  bound to `state.refreshing`, so a poll tick can't flash the spinner.

**3. Wire the polls into the Routes** (keep the existing one-shot resume load, add the loop):
- `DeviceListRoute(..., dotPollMs: Long = DOT_POLL_MS)` → keep `LifecycleResumeEffect { vm.refresh() }`, add
  `PollWhileResumed(dotPollMs) { vm.silentRefresh() }`.
- `DrivesListRoute(..., drivesPollMs: Long = DRIVES_POLL_MS)` → keep `LifecycleResumeEffect { vm.loadOffline() }`,
  add `PollWhileResumed(drivesPollMs) { vm.silentRefresh() }`.
- Defaults `DOT_POLL_MS = 8_000L`, `DRIVES_POLL_MS = 20_000L`. The interval is a **Route parameter** so tests
  render the Route with a short interval (~150 ms) — no instrumentation-arg plumbing needed for cadence.
  `AppNavHost` callers use the defaults (no change to call sites needed).

**4. Stable drive-row test selector** — add `testTag("drive_row_${drive.driveKey}")` to `DriveRow`'s `ListItem`
(`DrivesListScreen.kt:188`); device rows already have `device_row_${id}`. Tiny, enables the membership test.

## Files
- **New** `ui/components/PollWhileResumed.kt` — the helper above.
- `ui/devices/DeviceListViewModel.kt` — add `silentRefresh()` (+ `DOT_POLL_MS` companion); hysteresis.
- `ui/devices/DeviceListScreen.kt` — `DeviceListRoute` gains `dotPollMs` param + `PollWhileResumed`.
- `ui/drives/DrivesListViewModel.kt` — add `silentRefresh()` (+ `DRIVES_POLL_MS` companion).
- `ui/drives/DrivesListScreen.kt` — `DrivesListRoute` gains `drivesPollMs` param + `PollWhileResumed`; add the
  drive-row `testTag`.
- `ui/AppNavHost.kt` — unaffected (defaults), unless param wiring needs a touch; verify.
- **No Rust, no manifest change.**

## Tests (instrumented, emulator; gated `mockPort`(/`controlPort`), self-skip in CI without them — but the
mock-backed CI job now supplies them, so these run in CI via `tools/run-android-e2e.sh`)
- `DeviceDotLiveRefreshTest` — render `DeviceListRoute(dotPollMs=150ms)` against a device on `127.0.0.1:mockPort`;
  `waitUntil { onAllNodesWithContentDescription("conn_dot_green")… }`; `MockControl.post(controlPort,"/reachable","{\"up\":false}")`;
  `waitUntil { …"conn_dot_red"… }`; toggle back → green. **No manual refresh.** Uses `createComposeRule()`
  rendering the Route (real RESUMED lifecycle → the poll fires); fall back to `createAndroidComposeRule<ComponentActivity>()`
  if the lifecycle doesn't reach RESUMED.
- `DriveListLiveRefreshTest` — render `DrivesListRoute(drivesPollMs=150ms)`; `/add_drive` a **dedicated route** →
  `waitUntil { onAllNodesWithTag("drive_row_<key>")… }`; `/remove_drive` → `waitUntil` it disappears. Cleanup in `finally`.
- `DrivesSilentRefreshTest` (VM-level, no Compose) — construct `DrivesListViewModel(repo, deviceId)`, call
  `silentRefresh()`, `waitUntil` drives populate, assert `state.refreshing == false && state.loading == false`
  (the no-spinner-flash guarantee).

Anti-flake: short injected interval + `rule.waitUntil(timeoutMillis=…)` (never `sleep`); per-test device +
dedicated route + `finally` cleanup.

## Verification
1. `cd android && JAVA_HOME=/usr/lib/jvm/java-17-openjdk ./gradlew :app:assembleDebug :app:testDebugUnitTest ktfmtCheck --no-daemon` (run `ktfmtFormat` first).
2. Boot the `dashdown-b0` emulator; `ANDROID_SERIAL=emulator-5554 tools/run-android-e2e.sh` — full connected suite
   incl. the 3 new live tests; assert 0 failures.
3. Branch → PR → CI green (the `on-device tests (emulator)` job runs the new tests against the mock) → squash-merge.
   Exclude the unrelated `.claude/plans/b2-android-shell.md` from all commits.

## Risks / mitigations
- **Spinner flash** (top trap) → dedicated `silentRefresh()` never sets `loading`/`refreshing`; VM-level test guards it.
- **Lifecycle leak** → poll lives under `repeatOnLifecycle(RESUMED)`, not `viewModelScope`.
- **Battery/network churn** → reachability-gate + defer-to-B2 + 8s/20s cadence.
- **Dot flicker on jitter** → last-known-color hysteresis.
- **Scroll jump** → `LazyColumn` keyed by `driveKey` (stable); new drives sort by existing order.
- **Test flakiness** → injected short interval + `waitUntil` + isolation.

## Out of scope
Drive **detail** live-refresh; autoSync default/label; any cadence-tuning UI. (Per locked decisions.)
