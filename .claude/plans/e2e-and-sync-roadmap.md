# Roadmap — background sync, live UI refresh, and an E2E test suite (real-hw + mock)

> **This is a roadmap, not an implementation spec.** Per the repo convention ("Plan per phase"),
> **each phase below enters plan mode on its own, does its own research + code discovery, writes its
> own plan under `.claude/plans/`, builds, tests itself, and commits before the next phase begins.**
> This file fixes the architecture and the phase order; the detail lives in each phase's own plan.
> On the first phase this file is renamed to a descriptive name (e.g. `e2e-and-sync-roadmap.md`).

## Context

The Android shell has good unit/integration coverage but only **one** black-box Maestro flow
(`drive_download.yaml`, happy path vs a mock fixture). The user wants (1) a Maestro suite that also
runs on **real Comma hardware** for existing read-only features, and (2) **mock-server tests** for
live-refresh behaviour (connectivity red/green on server stop/start; drive list updating on
add/remove via polling without manual refresh; **segments added to an active drive being synced**).

A 7-agent research sweep (all key files read) established the ground truth and surfaced decisions,
now resolved with the user:

- **Downloads must happen in the BACKGROUND.** The user is in the car / driving and must **not**
  have to open the app for footage to sync. So "automatic downloads", "manual download continues
  when the app is closed", and "a newly-recorded segment on an active drive gets pulled" are all
  jobs of a **background sync engine**, not of any screen. *(This corrects an earlier draft that
  escalated downloads from the foreground drives screen — removed.)*
- **The app has no live foreground polling today.** The connectivity dot re-probes only on screen
  *resume*; the drives screen only re-syncs from the network on empty-load or pull-to-refresh.
  Foreground polling will be added **purely to refresh live UI status** (the dot, the drive list)
  while the app is open — it never drives downloads.
- **The mock harness can't mutate state at runtime** (immutable fixtures; `set_state` swaps a whole
  fixture). Runtime injection (add/remove drive, add segment, toggle reachability) must be added so
  the mock-server tests can exist. The mock server already serves its temp tree **live per request**,
  so file-tree mutation needs no restart; only reachability needs the listener dropped/rebound.
- **Real hardware is strictly read-only** (never delete from the device, never change its settings;
  only our local mirror is ours to clear). The real Comma4 auto-prunes its *own* old drives when low
  on space. → real hardware exercises existing read-only features; the mock covers polling,
  local-delete/retention, and state injection.

## Architecture principle (applies to every phase)

```
BACKGROUND (no app open needed): the sync engine downloads — automatic schedule, responsive
  pickup of new drives/segments while connected, manual downloads that survive app close,
  retention/local clear-down.   ← WorkManager / foreground service domain
FOREGROUND (only while a screen is open): lightweight polling that refreshes displayed STATUS —
  connectivity red/green, drive list add/remove. Never initiates downloads; just reflects what
  the background engine and the device already did.
```

---

## Phases

Each phase: **EnterPlanMode → research/discovery → own plan file → build → test → commit** (branch
→ PR → CI green → merge, per CLAUDE.md). Listed in dependency order.

### Phase A — Runtime-mutable mock + control plane (Rust)
**Goal:** let tests inject server state changes at runtime. **Scope:** a mutation core in
`rust/mock-copyparty` (add/remove drive, add segment to a route, toggle reachability — leveraging
live temp-tree serving), exposed via an HTTP **control port** (so on-device tests and Maestro
`runScript` can drive it over a second `adb reverse`) **and** matching `mock-comma` MCP tools (for
interactive runs). **Self-tests:** extend `it_mock_harness.rs` / `it_mcp.rs`. Prerequisite for B–E.
**To research in its plan:** exact `MockServer`/supervisor wiring, control-endpoint shape, DRY
between the HTTP and MCP adapters.

### Phase B — Background sync engine (the corrected core)
**Goal:** automatic and manual downloads run in the background with no app open; new drives/segments
on a connected device are picked up promptly. **Scope:** evolve `AutoSyncWorker` / `DownloadService`
so manual downloads survive app close and auto-sync is responsive (not a flat 6-hour timer).
**Key decision to research in its plan:** what makes background pickup of a new segment timely and
battery-sane — periodic + expedited work, a connectivity/network trigger that fires when the phone
joins the Comma's hotspot, and/or a foreground service that syncs while connected — within Android
background-execution limits. **Self-tests:** instrumented worker/service tests driving Phase A's
`add_segment`/`add_drive` (covers "automatic downloads" and "segment added → synced", app closed).

### Phase C — Foreground live UI refresh (status only)
**Goal:** while a screen is open, the connectivity dot and the drive list update on their own.
**Scope:** lifecycle-scoped (`repeatOnLifecycle(RESUMED)`) polling that calls existing
`checkConnectivity` (device list) and a **silent** network re-sync (drives list) on an interval, with
silent variants so poll ticks don't flash the load/pull-to-refresh spinners. **No downloads.**
**Self-tests:** instrumented + Compose live tests via Phase A's control plane (assert red↔green and
drive add/remove with **no** manual refresh).

### Phase D — Retention / local clear-down
**Goal:** "clearing older downloads over the threshold" + "retention of starred." **Scope:** confirm
the Android wiring of the existing Rust retention engine (`run_maintenance`, `set_preserved`) and run
it where appropriate in the background path (ties into Phase B). **Self-tests:** instrumented (small
budget, multi-drive fixture from Phase A, star one → assert oldest non-starred local mirror cleared,
starred survives); Rust `it_retention.rs` already covers the policy.

### Phase E — Maestro suite + real hardware + host harness
**Goal:** black-box flows for existing read-only features that run on **mock and real hardware**,
plus mock-only live-refresh flows and a one-command harness. **Scope:** parameterize flows by
`${DEVICE_IP}`/`${DEVICE_PORT}` (default mock over `adb reverse`; real run points at the device IP,
read-only — download one smallest drive, play, star, never delete/reconfigure); mock flows use
`runScript` against Phase A's control port for red/green and drive add/remove; `tools/run-android-e2e.sh`
+ README runbook (build APK, launch mock with control port, reverse both ports, run
`connectedDebugAndroidTest` or Maestro). **Self-tests:** the flows themselves; CI keeps the existing
`assumeTrue(mockPort)` gating so live tests self-skip without a fixture.

## Scenario → phase coverage

| Requested scenario | Phase(s) |
|---|---|
| Empty state | E (Maestro, mock + real-hw) |
| Manual download (survives app close) | B (background) + E (Maestro) |
| Automatic downloads (background, no app open) | B |
| Segment added to active drive → synced (background) | A + B |
| Clearing older downloads over threshold | D |
| Retention of starred downloads | D |
| Auto-refresh red/green on server stop/start | A + C |
| Drive list add/remove on load + polling (no manual refresh) | A + C |

## Out of scope / risks
- No remote deletion from the Comma (read-only mount; `auto_delete_from_comma` stays a stub).
- Background-execution limits (Doze, background-start restrictions, foreground-service types) are the
  central risk for Phase B and will be researched there before committing to a mechanism.
- Foreground polling is foreground-only and lifecycle-cancelled; network-poll cadence tuned in C.
