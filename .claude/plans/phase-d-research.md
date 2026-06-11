# Phase D Research — Retention / Local Clear-down

> Multi-agent sweep, file:line-verified. Feeds the Phase D plan.
> **Scope:** clear oldest local downloads over the retention budget; never evict starred (preserved)
> drives. LOCAL mirror only — the real Comma is read-only (`auto_delete_from_comma` stays a stub).

## Executive summary — Phase D is small

The Rust retention engine (planner + enforcement + FFI), the Android **settings budget field**, the
**star/preserve control**, DB persistence, and the **B2 background trigger** are **all already wired and
live**. Both user scenarios work end-to-end today:
- **Over-threshold clear-down** — set budget in settings → `setSettings` → next reachable session
  `SyncSessionWorker.kt:116` → `runMaintenance` → `enforce_retention` → `plan_prune` deletes oldest
  non-preserved mirrors + reconciles to `NotDownloaded`.
- **Starred retention** — star a drive → `togglePreserve` → `setPreserved` → `plan_prune` never returns
  a preserved drive (`retention.rs:46-54`); flag survives index refreshes (`replace_drives`).

The pure policy is exhaustively unit-tested (`retention.rs` 8 cases) + a full Rust integration test
(`it_retention.rs:82-145`). **No core logic needs to change.**

## Current-state map (key refs)
- **Policy:** `sync_engine/retention.rs:23-58` `plan_prune(drives, budget_minutes) -> Vec<key>`. `None`
  budget = never prune. Only `Complete|Partial` count (in-flight never evicted). Order newest-first by
  `end_ms↓, start_ms↓, last_seg↓, drive_key↓`. Budget consumed by `segment_count` (≈1 min/seg). Drive
  exactly at budget kept; first strictly-over prunes the older tail. **Preserved consume budget but are
  never pruned.**
- **Enforcement:** `sync_engine/mod.rs:259-279` `enforce_retention` (plan → `mirror.remove_dir` each seg
  → `reconcile` → NotDownloaded). Wrapper `run_maintenance` `mod.rs:290-293`. Deletion idempotent +
  crash-safe (`storage/mod.rs:105-113`).
- **Model/DB:** `Device.retention_max_minutes: Option<i64>` (`model/mod.rs:354`), `Drive.preserved`
  (`:335`), `set_drive_preserved` (`db/mod.rs:413-425`); `replace_drives` preserves star + sync_state.
- **FFI/Android:** `run_maintenance`/`set_preserved` (`ffi/mod.rs:235-243,361-364`); repo
  `runMaintenance`/`setPreserved`/`getSettings`/`setSettings`. **Settings field already present** —
  `DeviceSettingsScreen.kt:122-130` "Keep local footage up to (minutes)", blank=Unlimited,
  `testTag("settings_retention")`. **Star already present** — `DriveRow` star (`drive_preserve_<key>`),
  `togglePreserve`. Both `DrivesListScreen` + `DriveDetailScreen`.
- **Background:** `SyncSessionWorker.kt:116` `runMaintenance(d.id)` per device, after the loop —
  **only for `autoSync` devices reachable at session start** (`:67`). `SyncBackstopWorker` only enqueues
  the session; **no offline maintenance path**. `DownloadService` does NOT call `runMaintenance`.

## What's missing (the actual backlog)
| Piece | Status |
|---|---|
| **Instrumented eviction self-test** (planned deliverable) | MISSING |
| **Deterministic fixture drive-age control** (blocks the test) | MISSING — `/add_drive` has no mtime; `full_segment` writes wall-clock mtime (`fixtures.rs:33-41`); drive `end_ms` = file mtime (`drive_grouping/mod.rs:128-129`) |
| **Maintenance when offline / decoupled from reachability** | MISSING by design (only runs in the reachability-gated session) |
| **Maintenance after a foreground manual download** | MISSING (`DownloadService`) |
| **Prune→re-download loop guard / budget warning** | MISSING (budget < a drive ⇒ download-prune-repeat; no data loss) |
| **Storage-usage readout** | MISSING (optional) |
| **Dead `auto_delete_from_comma` toggle** | PRESENT but inert/misleading (`DeviceSettingsScreen.kt:133-147`) |
| **Maintenance silent-failure logging** | `tryIo` swallows errors at `:116` |

## Gaps Phase D must fill (ordered)
1. **Extend the mock** — add optional `mtime_s` to `/add_drive` → `set_file_times` so drive age is
   deterministic (prerequisite for a non-flaky eviction test). Unit-test in `mutate.rs`.
2. **`RetentionLiveTest.kt`** — multi-drive fixture w/ explicit ages, small budget via `setSettings`,
   star one via `setPreserved`, `runMaintenance`, assert oldest non-starred mirror gone + starred +
   newest survive. Wire into `run-android-e2e.sh`.
3. **Maintenance scheduling** — run clear-down regardless of reachability (local-only needs no network).
4. **Prune→re-download loop guard** — warn when budget < largest local drive (no core change).
5. **(Optional)** storage-usage readout; log pruned keys; fix the dead `auto_delete_from_comma` controls.

## Self-test plan (deterministic eviction)
Mirror `it_retention.rs`: 3 drives A(2 seg, oldest, starred) / B(3 seg, mid) / C(2 seg, newest), budget 4
→ newest-first keeps C(2), B pushes to 5>4 ⇒ B is the eviction target; A starred survives. Stage ages via
the new `mtime_s` (A=1000,B=2000,C=3000). Download all via `TestListenableWorkerBuilder<SyncSessionWorker>`,
`setPreserved(A)`, `runMaintenance`, assert B→`NOT_DOWNLOADED` + `driveLocalPaths(B).size==0`, A/C `COMPLETE`+nonempty.
Reuse `tools/run-android-e2e.sh` + `MockControl` + `autoSyncDevice`. Keep mock reachable throughout. `finally` cleanup.

## Decisions — RESOLVED (user, 2026-06-10) — this REPLACES the "small phase" framing

The user corrected the retention MODEL, making Phase D a real core change:
1. **Budget = segments = minutes** (1 segment = 1 minute regardless of how many camera streams it holds). ✅
2. **Preserved (starred) drives are EXCLUDED from the budget** — today `plan_prune` counts them (`retention.rs:43`); that's a bug to fix. ✅
3. **Keep the newest N non-preserved SEGMENTS — segment-level retention** (a long drive can be partly local → renders `Partial`). NOT drive-level. ✅
4. **The prune↔re-download loop must be structurally IMPOSSIBLE** → the download window MUST equal the retention window: auto-sync only fetches what retention keeps. ✅
5. **Offline/periodic maintenance** — clear-down runs even when the comma is unreachable. ✅
6. **Low-headroom phone notification** (NEW) — warn when < ~N (10) minutes of headroom remain before auto-prune deletes older segments, so the user can star what to keep. ✅
7. **Storage-usage readout** in settings. ✅
8. **Default budget stays unlimited / opt-in** (no change). ✅
9. **Explicit user-delete + "don't re-download" tombstone → DEFERRED to a follow-up PR** (needs a new `deleted` column/migration + FFI + UI). ✅

This is no longer "confirm wiring + a test" — it is a segment-level retention engine change. The drive-granular
`plan_prune` + drive-granular auto-download are replaced by a shared segment-level window.

## Risks
Test flakiness from non-deterministic drive age (TOP — fix via `mtime_s`); prune→re-download loop (no data
loss); pruning a downloading drive (IMPOSSIBLE — excluded); losing starred footage (IMPOSSIBLE — protected +
tested); budget-unit confusion (mitigate w/ "≈ X h"); silent data-loss UX (log pruned keys); maintenance never
running offline (scheduling decision); misleading dead toggle.
