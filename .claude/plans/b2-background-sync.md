# Phase B2 — Background sync scheduler

> On approval this file is saved as `.claude/plans/b2-background-sync.md` (descriptive name per repo convention; the random name is just the plan-mode scratch file).

## Context

The user's core requirement: **footage must sync in the background, with no app open** — "the user is in the car / driving and must not have to open the app for footage to sync." Today there *is* a background path, but it's a coarse one-shot:

- `AutoSyncWorker` (`android/app/.../work/AutoSyncWorker.kt`) is a **periodic 6-hour** worker, constrained to **`UNMETERED` + battery-not-low**. On each run it does *one* pass: `syncNow` → `runMaintenance` → download every `NOT_DOWNLOADED`/`PARTIAL` drive → poll to completion (5-min cap), then exits. It promotes itself to a `dataSync` foreground service for the download.
- There is **no connectivity trigger** and **no in-session re-sync**, so a segment recorded *after* the pass started (an actively-recording drive) isn't picked up until the next 6-hour tick, and reaching the device never kicks a sync.
- Manual downloads already survive app close via the standalone `DownloadService` (`dataSync` FGS) — **left as-is** by B2.

B2 reworks the background engine into the roadmap's decided **hybrid**: a 15-min periodic backstop **plus** a connectivity-triggered session that, within seconds of reaching the device (while the process is alive), runs a **loop** of sync→download for new/partial drives until the device goes unreachable or work drains — picking up freshly-recorded segments in-session. Reachability-gated via B1's multi-IP resolver; long drives continue across ticks via the Rust core's `.part`-file resume.

**No Rust changes.** The FFI surface already supports everything (`sync_now`, `list_drives(offline)`, `start_drive_download` (resumes), `get_drive_status`, `check_connectivity`, `run_maintenance`). B2 is Android orchestration + instrumented tests only.

## Key findings from discovery

- **FFI surface (no gaps):** `AppCore` exposes async `sync_now`, `list_drives(offline)`, `get_drive(_status)`, `start_drive_download → SyncHandle{cancel,is_cancelled}`, `check_connectivity → {dot,reachable,downloading}`, `run_maintenance`, `set_preserved`. `start_drive_download` runs on the core's **owned tokio runtime** (outlives the call) and **auto-resumes** from `.part` files; progress flows to the `ProgressSink` (`ProgressBus` → `repo.progress`/`terminalEvents`). Drives carry `syncState` and `recording`.
- **All real networking is in Rust (reqwest/rustls)**, *not* Android's HTTP stack — which is why cleartext HTTP to `127.0.0.1`/the real comma works with no `networkSecurityConfig`. Android's cleartext policy only bites *Kotlin-side* sockets → relevant only to the test harness POSTing to the mock control port (handled with a raw-socket helper, no app config change).
- **Manifest already correct:** `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_DATA_SYNC`, `POST_NOTIFICATIONS`, `ACCESS_NETWORK_STATE`, `INTERNET` all present; WorkManager's own `SystemForegroundService` handles the `dataSync` FGS (the current worker already promotes successfully on the API-35 CI emulator). `targetSdk=36`, `minSdk=29`.
- **Phase A control plane exists:** `mock-copyparty --control-port N` serves `POST /add_segment`, `/add_drive`, `/remove_drive`, `/reachable` (+ `GET /status`) — reachable from a test via a second `adb reverse`. `androidx.work:work-testing` is already a dep; `TestListenableWorkerBuilder<W>(app).build().doWork()` is the established pattern (`AutoSyncWorkerLiveTest`). Live tests gate on `assumeTrue(mockPort != null)` and self-skip in CI (CI doesn't start the mock — Phase E wires that).

## Platform constraints (disclosed, not knobs)

1. **Cold-app trigger floor ≈ 15 min.** 15 min is the platform periodic minimum. A `ConnectivityManager.NetworkCallback` gives the "within seconds" reaction but **cannot survive process death** — so when the app has been killed, the durable trigger is the 15-min periodic (JobScheduler restarts the process). There is no durable sub-15-min event trigger on Android. The callback covers the "process currently/recently alive" case (and instant sync when the user opens the app).
2. **Android 15+ `dataSync` FGS budget = 6h per rolling 24h**, shared across our FGS, **resets only when the user foregrounds the app** (`targetSdk=36` ⇒ this applies). We self-cap each session well under this and resume next trigger; a >6h/day *active-transfer* backlog would pause until the app is opened. Typical LAN footage transfer is far below this.
3. **Local Network Protection** (Android 17 / `targetSdk 37`) will later require a runtime LAN permission (affects even Rust sockets) — out of scope now (`targetSdk 36` = opt-in).

## Design

Three components, all funnelling through one serialized, FGS-promoted **session**:

**1. `SyncSessionWorker` (new) — one-time, the workhorse.**
Unique work `"sync-session"`, `ExistingWorkPolicy.KEEP` (concurrent triggers coalesce), constraint `NetworkType.CONNECTED` (drop `UNMETERED`). `getForegroundInfo()` = `dataSync` (moved from `AutoSyncWorker`). `doWork()`:

```
repo = (app as DashdownApp).locator.repository
devices = repo.listDevices().filter { it.autoSync }
reachable = devices.filter { runCatching { repo.checkConnectivity(it.id).reachable }.getOrDefault(false) }
if (reachable.isEmpty()) return Result.success()        // no FGS, no notification when comma absent
runCatching { setForeground(getForegroundInfo()) }
val deadline = elapsedRealtime() + SESSION_MAX_MS       // self-cap (≈25 min) — bounds battery + FGS budget
for (d in reachable) {
  while (elapsedRealtime() < deadline && !isStopped) {
    if (!repo.checkConnectivity(d.id).reachable) break                 // device left mid-session
    val drives = runCatching { repo.syncNow(d.id) }.getOrElse { break } // network refresh picks up new drives/segments
    val pending = drives
        .filter { it.syncState == NOT_DOWNLOADED || it.syncState == PARTIAL }
        .filter { repo.getDriveStatus(d.id, it.driveKey).status != DOWNLOADING } // skip manual-in-flight (DownloadService priority)
    if (pending.isEmpty()) {
      if (drives.any { it.recording }) { delay(RECORDING_POLL_MS); continue } // wait for next segment on an active drive
      else break                                                              // work drained for this device
    }
    for (drive in pending) {
      if (isStopped || elapsedRealtime() >= deadline) break
      current = repo.startDriveDownload(d.id, drive.driveKey)  // resumes from .part automatically
      awaitTerminal(d.id, drive.driveKey, deadline)            // poll getDriveStatus → COMPLETE/FAILED (existing 500ms pattern, bounded)
      current = null
    }
    // re-loop: re-sync to catch segments recorded *during* these downloads
  }
  runCatching { repo.runMaintenance(d.id) }   // retention (Phase D refines the policy)
}
return Result.success()
```
- On `isStopped` (WorkManager cancellation), cancel the held `current` `SyncHandle` → `.part` left for resume next session.
- `SESSION_MAX_MS ≈ 25 min`, `RECORDING_POLL_MS ≈ 30 s` (constants, tunable).
- Reachability triage **before** promotion ⇒ no pointless FGS notification and no `dataSync`-budget burn when the comma isn't around.

**2. `SyncBackstopWorker` (reworked from `AutoSyncWorker`) — periodic 15-min heartbeat.**
`PeriodicWorkRequestBuilder<SyncBackstopWorker>(15, MINUTES)`, constraint `CONNECTED` + `setRequiresBatteryNotLow(true)`, `enqueueUniquePeriodicWork("sync-backstop", KEEP, …)`. `doWork()` just calls `SyncSessionWorker.enqueue(context)` and returns `success`. WorkManager **persists this across reboot** (its own boot receiver) — no `BOOT_COMPLETED` receiver needed (and API-35 forbids starting a `dataSync` FGS directly from boot anyway; we don't).

**3. Connectivity fast-path — `ConnectivityManager.NetworkCallback`.**
Registered in `DashdownApp.onCreate()` (process-lifetime); on `onAvailable`/`onCapabilitiesChanged(WIFI)` → `SyncSessionWorker.enqueue(this)` (KEEP). Instant while alive; honest floor is the 15-min backstop when dead (constraint #1).

**`DashdownApp.onCreate()`** becomes: `SyncBackstopWorker.ensureScheduled(this)` + `SyncSessionWorker.enqueue(this)` (immediate attempt) + register the network callback.

**Minor UX:** request `POST_NOTIFICATIONS` at runtime from `MainActivity` (API 33+) so the FGS notification is visible (FGS still runs if denied).

## Files

**Android (app module):**
- `work/SyncSessionWorker.kt` — **new** one-time session worker (loop above; `getForegroundInfo` moved here; `enqueue()` helper, unique KEEP).
- `work/AutoSyncWorker.kt` → **rename/rewrite** as `work/SyncBackstopWorker.kt` — periodic 15-min, `CONNECTED`+battery, `ensureScheduled()` enqueues the session.
- `work/SyncTriggers.kt` (or inline in `DashdownApp`) — the `NetworkCallback` registration.
- `DashdownApp.kt` — onCreate wiring (backstop + immediate session + callback). Previously only called `AutoSyncWorker.ensureScheduled`; the callback is registered inline (no separate `SyncTriggers.kt`).
- `MainActivity` — runtime `POST_NOTIFICATIONS` request: **already implemented** (no change).
- `AndroidManifest.xml` — **change required after all** (a real on-device run disproved the plan's
  original "no manifest change"): a worker promoted via `setForeground(dataSync)` is hosted by
  WorkManager's *own* `SystemForegroundService`, whose library manifest entry declares no service
  type, so the OS throws `IllegalArgumentException: foregroundServiceType 0x1 is not a subset of
  0x0` and the app crash-loops on launch. Fix = a `tools:node="merge"` overlay adding
  `android:foregroundServiceType="dataSync"` to `androidx.work.impl.foreground.SystemForegroundService`.
  (Latent before B2: the old 6h worker never fired the real FGS path, and `TestListenableWorkerBuilder`
  no-ops `setForeground`.)
- **No Rust change** (verified).

**Tests (app `androidTest`):**
- `SyncSessionWorkerLiveTest.kt` — port of `AutoSyncWorkerLiveTest`: run session via `TestListenableWorkerBuilder`, assert drive `COMPLETE` (gate `mockPort`).
- `SyncSessionSegmentPickupTest.kt` — session→`COMPLETE`; `POST /add_segment` to control port; session again; assert the new segment is mirrored (`drive_local_paths` count grows / still `COMPLETE`). Covers **"segment added to active drive → synced, app closed."** (gate `mockPort`+`controlPort`).
- `SyncSessionDriveAddTest.kt` — session; `POST /add_drive`; session; assert new drive downloaded. Covers **"automatic download of a newly-appeared drive."**
- `MockControl.kt` (test helper) — minimal raw-`Socket` HTTP POST to `127.0.0.1:$controlPort` (avoids any app `networkSecurityConfig` change; Kotlin-side cleartext otherwise blocked at `targetSdk≥28`).
- Runbook comment in each test: `mock-copyparty --fixture single_drive --port 8099 --control-port 8098` + `adb reverse tcp:8099 tcp:8099` + `adb reverse tcp:8098 tcp:8098` + `-Pandroid.testInstrumentationRunnerArguments.mockPort=8099 -Pandroid.testInstrumentationRunnerArguments.controlPort=8098`. CI continues to self-skip (Phase E wires the mock into CI).

## Verification

1. **Build + unit + format:** `cd android && JAVA_HOME=/usr/lib/jvm/java-17-openjdk ./gradlew :app:assembleDebug :app:testDebugUnitTest ktfmtCheck --no-daemon`. Run `ktfmtFormat` before commit (CI `assemble` runs `ktfmtCheck`).
2. **Instrumented (local, against emulator or the real Pixel `192.168.1.210:5555`):** launch the mock with a control port, two `adb reverse`s, run the three new live tests with `mockPort`+`controlPort`. Assert: initial drive `COMPLETE`; after `/add_segment` the extra segment mirrors; after `/add_drive` the new drive downloads — **all with no UI, worker-driven.**
3. **Real scheduled-path validation (api-35 emulator):** done on the clean `dashdown-b0` AVD rather than against `escape2020` (which would pull the comma's 33 real drives). Add an `autoSync` device pointing at the mock, relaunch so `DashdownApp.onCreate` enqueues a *real* `SyncSessionWorker` (not `TestListenableWorkerBuilder`), and confirm via logcat/`dd-db` that WorkManager promotes the `dataSync` FGS and the drives reach `complete`. **This caught the `SystemForegroundService` manifest crash the harness tests missed.**
4. **Branch → PR → CI green (assemble + on-device + claude-review + build/test) → squash-merge** (per CLAUDE.md). Exclude the unrelated `.claude/plans/b2-android-shell.md` working-tree change from all commits.

## Out of scope / deferred
- Foreground live-UI polling (the dot / drive list while a screen is open) → **Phase C**.
- Retention threshold/clear-down policy depth (starred preservation) → **Phase D** (B2 only preserves the existing `runMaintenance` call).
- CI wiring of the mock + `adb reverse` + the Maestro/host harness → **Phase E**.
- Remote deletion from the comma stays a stub (read-only mount).
