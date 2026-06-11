# Phase D — Segment-level retention + clear-down (no prune↔redownload loop)

> On approval, saved as `.claude/plans/phase-d-retention.md`. Research: `.claude/plans/phase-d-research.md`.

## Context

Retention exists but is **drive-granular** and counts preserved drives toward the budget. The user
corrected the model: budget is in **segments = minutes** (1 seg = 1 min, any number of camera streams);
**keep the newest N non-preserved SEGMENTS** (a long drive may be kept partially → `Partial`); **preserved
(starred) drives are excluded from the budget and never pruned**; and the **prune↔re-download loop must be
structurally impossible**. The clean way to guarantee that: make the **download window equal the retention
window** — auto-sync only fetches segments retention would keep, so nothing is downloaded just to be pruned
next tick. Plus: run clear-down **offline/periodically**, a **low-headroom notification**, and a **storage
readout**. Default budget stays **unlimited/opt-in**. Explicit user-delete + tombstone is **deferred**.

This is a core change. With `None` budget everything is in-window (today's behaviour — backward compatible).

## The single shared concept: the segment window

`retention_window(drives, budget) → Set<(route_id, segment_num)>` = **all segments of preserved drives**
(never counted) **plus the newest `budget` non-preserved segments** (ordered by segment time, newest-first;
`None` budget ⇒ every segment). Both sides use it:
- **Download** fetches only in-window segments → never downloads what it would prune.
- **Prune** deletes only out-of-window, non-preserved local segments.
The two sets are disjoint by construction ⇒ **no loop**.

## Rust core (`rust/core`)

1. **`sync_engine/retention.rs` — replace drive-granular `plan_prune` with the segment window.**
   - New pure `retention_window(drives, budget) -> HashSet<(String,u32)>` (preserved always in; newest-N
     non-preserved by segment time). Segment time from `mtime_s` (reuse the segment's approx-time helper used
     in `drive_grouping` for `end_ms`; verify the method name in impl).
   - Rewrite the unit tests for: preserved excluded from budget; newest-N non-preserved kept; a drive kept
     **partially** (boundary inside a multi-segment drive); equal-time tie-break; `None` ⇒ all.
2. **`sync_engine/mod.rs`:**
   - **`download_drive`** (`:381-395`): compute the window from the already-loaded `all_drives` and **skip
     out-of-window segments** when building `items` (loop guard).
   - **`enforce_retention`** (`:259-279`): prune **out-of-window segments** of drives that have local data
     (segment-granular `mirror.remove_dir` per `seg.name`), then `reconcile` (drives reclassify to
     `Partial`/`NotDownloaded` from disk — `drive_status` already does this).
   - **New `pending_download_keys(device) -> Vec<String>`**: drive_keys that have ≥1 in-window segment whose
     selected files aren't all `Complete` on disk (so B2 never "downloads" a fully-out-of-window drive).
   - **New `retention_status(device) -> RetentionStatus`** (record `{ local_minutes, preserved_minutes,
     budget_minutes }`): counts locally-complete segments (reuse `classify_file`) for the readout + warning.
3. **`ffi/mod.rs`:** export `pending_download_keys(device_id) -> Vec<String>` and
   `retention_status(device_id) -> RetentionStatus`. `run_maintenance`/`set_preserved` unchanged.
4. **Regenerate UniFFI bindings** (`cargo run -p bindgen …`, per CLAUDE.md) for the new exports.
5. **`tests/it_retention.rs`:** rewrite to segment-level — a drive kept partially (old segments pruned,
   newest kept), a preserved drive fully kept and excluded from budget, and a **loop-guard** assertion:
   after a prune, a second `sync_now` + download (driven by `pending_download_keys`) does **not** re-fetch
   the pruned segments.

## Android (`android/app`)

6. **B2 download planning — `work/SyncSessionWorker.kt`:** replace the `pending = drives.filter{NotDownloaded
   ||Partial}` block with `repo.pendingDownloadKeys(d.id)` (then keep the existing `giveUp` + skip-`DOWNLOADING`
   filters and the recording-drive wait). This makes auto-download obey the window.
7. **Offline/periodic maintenance — `work/SyncBackstopWorker.kt`:** `doWork()` sweeps maintenance for **every**
   device (no reachability gate — local clear-down needs no network) via the new `Maintenance.sweep`, then
   enqueues the session. `SyncSessionWorker`'s post-loop `runMaintenance(d.id)` becomes `Maintenance.sweep`.
8. **`work/Maintenance.kt` (new):** `sweep(context, repo, device)` = `runMaintenance` + low-headroom warning.
   `shouldWarn(status, threshold)` is pure (warn when `budget != null && budget - (local - preserved) <
   WARN_HEADROOM_MIN`, default 10). Notification: per-device id, `setOnlyAlertOnce`, channel "Storage almost
   full", tap → MainActivity, cancel when headroom recovers. Uses `repo.retentionStatus(deviceId)`.
9. **Storage readout — `ui/settings/DeviceSettingsViewModel.kt` + `DeviceSettingsScreen.kt`:** load
   `repo.retentionStatus(deviceId)`; show `"Using ~X min locally"` (+ `" of Y"` when a budget is set) under
   the retention field (`testTag("settings_storage_usage")`).
10. No manifest change (POST_NOTIFICATIONS already declared/requested).

## Mock + tests

11. **Deterministic drive age — `rust/mock-copyparty`:** add `mtime_s: Option<i64>` to `/add_drive`
    (`control.rs:135`), thread into `mutate::add_drive(root, route, segs, mtime_s)` which sets each written
    file's mtime (`filetime::set_file_mtime`; add the dep). Update callers to pass `None`; add a `mutate.rs`
    unit test.
12. **Android instrumented `RetentionLiveTest.kt`** (gated `mockPort`+`controlPort`, emulator): stage drives
    with explicit `mtime_s`; `autoSync` device, budget `null`; download via `TestListenableWorkerBuilder
    <SyncSessionWorker>` → assert all `COMPLETE`. Then `setPreserved(oldest)`, `setSettings(retention=N)`,
    point at a **dead port** (prove offline), `Maintenance.sweep` → assert the oldest **non-preserved**
    segments pruned (`driveLocalPaths` shrinks / drive → `Partial`), the **preserved** drive intact, and
    newest kept. Then run another `SyncSessionWorker` and assert the pruned segments are **not re-downloaded**
    (loop impossible). Dedicated routes, `finally` cleanup.
13. **Android JVM unit `MaintenanceTest.kt`:** `shouldWarn` truth table.

## Verification
1. `cargo test -p dashdown-core -p mock-copyparty` (retention window + segment-level it_retention + mtime).
2. `cd android && JAVA_HOME=/usr/lib/jvm/java-17-openjdk ./gradlew :app:assembleDebug :app:testDebugUnitTest ktfmtCheck --no-daemon` (run `ktfmtFormat`; bindings regenerated).
3. Boot `dashdown-b0`; `ANDROID_SERIAL=emulator-5554 tools/run-android-e2e.sh` — full suite incl. `RetentionLiveTest`; 0 failures.
4. Branch → PR → CI green → squash-merge. Exclude `.claude/plans/b2-android-shell.md`. (May land in two
   commits on the branch: core retention+loop-guard+tests, then maintenance/notification/storage.)

## Risks / mitigations
- **Loop** → impossible by design (download set ⊆ window; prune set = local ∖ window; disjoint) + an explicit
  "not re-downloaded after prune" test.
- **`download_drive` on a fully-out-of-window drive marking it Complete** → avoided: B2 only starts drives from
  `pending_download_keys` (which requires an in-window non-complete segment).
- **Segment time ordering / equal mtimes** → total order (time, then route, then segment_num); the mock sets
  explicit `mtime_s` so tests are deterministic (no sleeps).
- **Losing starred footage / evicting an in-flight segment** → preserved segments are always in-window;
  `Downloading` excluded from local-data accounting (unchanged).
- **Notification spam** → per-device id + `setOnlyAlertOnce`; cancel on recovery.
- **FFI/bindgen drift** → regenerate bindings; CI `assemble` + connected tests catch mismatches.

## Out of scope (per locked decisions)
Explicit user-delete + tombstone (next PR); soft default budget; fixing the inert `auto_delete_from_comma`
toggle; GB budgets; the deferred B2 autoSync default/label.
