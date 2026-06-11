# Phase E Research — Maestro suite + real hardware + host harness

> Multi-agent sweep (4 code readers + a Maestro web briefing), file:line / URL-verified. Feeds the plan.
> **Goal:** black-box Maestro flows for read-only features that run on **both** the mock and real hardware
> (parameterized by `${DEVICE_IP}`/`${DEVICE_PORT}`), **mock-only** live-refresh flows that drive the control
> port from inside Maestro, a one-command host harness + runbook. CI gating preserved.

## Verified environment facts
- **Maestro 2.6.0** on the rig → GraalJS engine, `runScript` exposes a synchronous global `http`
  (`http.post(url,{body,headers})`, body must be `JSON.stringify(...)`, `response.ok/.status/.body`). ✓
- **Production poll intervals** (black-box flows wait these, can't inject 150ms): `DOT_POLL_MS = 8_000`
  (`DeviceListViewModel.kt:100`), `DRIVES_POLL_MS = 20_000` (`DrivesListViewModel.kt:123`). So
  `extendedWaitUntil` timeouts ≈ 15s (dot) / 30s (drives) + margin.
- **`remove_device.yaml` has `env: {NAME: escape2020}`** (lines 3-4) → **shadows** `-e NAME=…` (Maestro
  precedence YAML `env:` > CLI `-e` > OS). Latent bug; drop the block.

## Current-state map
- **Flows** (`android/maestro/`): `add_device` (parameterized correctly, NO `env:` block — `-e` wins),
  `remove_device` (de-shadow needed), `clear_devices` (`repeat while visible "More"`), `drive_download`
  (**hardcoded** `127.0.0.1`/`8099`/`device_row_1` via `clearState`; waits text "Download" then "Complete").
  `tools/dd-ui.sh` runs one flow (`maestro --device $SERIAL test -e … flow.yaml`, JDK 17, first-adb-device if
  `ANDROID_SERIAL` unset, injects PORT/WIFI_IP defaults after user args).
- **testTags available** (black-box selectors), by screen:
  - Devices: `add_device_fab`, `device_row_{id}`; dot `contentDescription conn_dot_{green|blue|red|unknown}`;
    overflow icon `"More"`, menu items **text-only** "Edit/Settings/Remove"; empty "No devices yet. Tap + …".
  - Edit form: `device_form_{name,dongle,hotspot_ip,wifi_ip,port,password,save}`; dialog "Device detected" +
    text-only "Use it"/"Skip".
  - Drives: `drives_pull_refresh`, `drive_row_{key}`, `drive_preserve_{key}` (cd `preserve_on/off`),
    `drive_download_{key}`, `drive_cancel_{key}`, `drive_progress`, `drive_sync_badge` (cd `sync_{state}`);
    empty "No drives yet. Pull down to sync…".
  - Detail: `drive_detail_{preserve,download_btn,cancel_btn,export_btn,player}`.
  - Player (`MultiCamPlayer`): `drive_detail_player`, `camera_toggles`, `camera_toggle_{road|wide|driver}`,
    `drive_play_toggle`, `drive_audio_toggle`, `drive_scrubber`, `drive_filmstrip`; "Preparing HD…" text.
  - Settings: `settings_{autosync,retention,storage_usage,autodelete,min_age,save}`, `settings_file_{kind}`.
  - **Gaps** (text-only today): overflow menu items, dongle dialog buttons, player time label.
- **Harness/control plane**: `tools/run-android-e2e.sh` (builds+starts mock with data+control port, `adb
  reverse` both, runs `connectedDebugAndroidTest`; passes `$@` through). Control endpoints
  (`rust/mock-copyparty/src/control.rs`): `GET /status`, `POST /reachable {up}`, `/add_drive {route,segs,mtime_s}`,
  `/add_segment {route,n}`, `/remove_drive {route}`. `MockControl.kt` (raw-socket) is the instrumented analog.
- **CI** (`android-ci.yml`): `connected-check` runs `run-android-e2e.sh` on an api-35 emulator; live tests
  self-skip without `mockPort`. **No Maestro in CI today.** The Phase C behavior is already CI-guarded by the
  instrumented `DotLiveRefreshTest` + `DriveListLiveRefreshTest` (which Phase E mirrors as black-box flows).
- **Real-hw**: Pixel `192.168.1.210:5555` + comma `escape2020 192.168.1.100:8080` (read-only `/routes`, ~33
  drives). **Read-only is structural** — `enforce_retention` prunes only the Pixel's local mirror;
  `auto_delete_from_comma` is an inert stub; star writes a local SQLite flag; no code path DELETEs/reconfigures
  the comma. Real risk = filling the Pixel.

## runScript → control port (feasible, Maestro 2.6)
```js
// scripts/set_reachable.js
const r = http.post('http://127.0.0.1:' + CONTROL + '/reachable',
  { body: JSON.stringify({ up: UP === 'true' }), headers: { 'Content-Type': 'application/json' } })
output.ok = r.ok
```
```yaml
- runScript: { file: scripts/set_reachable.js, env: { CONTROL: "${CONTROL}", UP: "false" }, when: { true: "${MODE == 'mock'}" } }
- extendedWaitUntil: { visible: { id: "conn_dot_red" }, timeout: 15000 }   # ≥ 8s dot poll
```
- Control port reached at `127.0.0.1:$CONTROL` via the harness `adb reverse`. Pass vars per-`runScript`
  `env:` (NOT a top-level `env:` block → shadowing). Gate control-port steps `when: MODE == 'mock'` so a real
  run never POSTs to the comma.
- **Red/green caveat (from Phase C):** over `adb reverse` a closed host data-port still accepts the device
  connect → dot never goes red. So the connectivity flow's device must use the emulator host alias
  **`10.0.2.2`** for the DATA port (no reverse) while the control port stays reversed — **emulator-only**
  (fine; it's mock-only). Exactly what `DotLiveRefreshTest` does.
- **Fallback** if runScript-http is ever unavailable: split into up/down flows and `curl` the control port
  host-side from the harness between them.

## Flow suite (proposed)
`*_mock.yaml` = mock-only (control port, gated `MODE==mock`); shared flows parameterized to run on both.
1. `empty_state` (both) — clearState → assert "No devices yet…".
2. `add_device` (both, exists) — keep; ensure `${DEVICE_IP}/${DEVICE_PORT}` params.
3. `drive_download` (both, **parameterize**) — select row by **name** (not `device_row_1`); `${DEVICE_IP}`
   (def 127.0.0.1)/`${DEVICE_PORT}` (def 8099 mock / 8080 real); wait "Complete" (real: longer timeout).
4. `play_drive` (both) — open completed drive → `drive_detail_player` → `drive_play_toggle`; "Preparing HD…" clears.
5. `star_drive` (both) — tap `drive_preserve_{key}`; assert `preserve_off`→`preserve_on` (local only).
6. `manual_download_survives_close` (both) — start download → stop/relaunch → still Complete/resumes.
7. `connectivity_refresh_mock` (**mock-only**, 10.0.2.2 data) — `/reachable false→true`, assert dot red↔green, no refresh tap.
8. `drive_list_refresh_mock` (**mock-only**) — `/add_drive` then `/remove_drive` (dedicated route), assert row appears/disappears, no refresh.
9. `remove_device` (both, **de-shadow**) — drop `env:` block.
10. `retention_star_mock` (optional, mock-only) — small budget + staged drives + star → starred survives, unstarred pruned.

Roadmap coverage: empty(1), add(2), manual download survives close(6), red/green(7), drive add/remove(8),
retention/star(5,10). Background auto-download + segment-pickup stay as instrumented tests (no UI surface).

## Harness + CI
- **New `tools/run-maestro-e2e.sh`** (sibling, don't overload the gradle runner). Mock mode (`DEVICE_IP=127.0.0.1`
  or `10.0.2.2`): reuse run-android-e2e.sh's mock launch + `/status` wait + `adb reverse` both ports + install
  APK, then `maestro --device $SERIAL test -e MODE=mock -e DEVICE_IP=… -e DEVICE_PORT=$MOCK_PORT -e CONTROL=$CONTROL_PORT <flow|dir>`,
  trap cleanup. Real mode (`DEVICE_IP=<comma>`): no mock/reverse; **require explicit `ANDROID_SERIAL`** (never
  "first device"); `MODE=real` (control steps self-gate off); run only the bounded read-only flow; longer timeouts.
- **CI: keep Maestro local** (recommended). Maestro-Cloud can't `adb reverse` to a host control port; a headless
  GH-Actions emulator + CLI install is possible but the action is early-stage. Instrumented Dot/DriveList live
  tests remain the CI guardians. Maestro-in-CI = a later, optional step.
- **docs/TESTING.md §4**: add the mock-vs-real matrix, the new flows, the `${DEVICE_IP}/${DEVICE_PORT}/MODE`
  contract, the env-shadowing rule, and the one-command runbook for both modes.

## Bounded real-hardware safety
Comma is structurally read-only; guard the Pixel's storage: pre-cap `settings_retention` (e.g. 30-60 min) so
maintenance bounds the mirror; keep `autoSync=false` (manual `DownloadService` only — no 33-drive fan-out);
download exactly the **first** `drive_download_*` (no size sort exists — `ORDER BY route_id,first_seg`); prefer
**qcamera-only** file selection (tiny). Star + remove_device are app/local-only.

## Decisions — RESOLVED (user, 2026-06-11)
1. **Maestro stays LOCAL + real-hw only** — no Maestro in CI; the instrumented Dot/DriveList live tests keep
   guarding Phase C behavior in CI. ✅
2. **Scripted harness + runbook** for real hardware (`run-maestro-e2e.sh … MODE=real`, manual trigger). ✅
3. **Full suite in one PR** (all ~9 flows + harness + remove_device de-shadow + docs). ✅
4. **No smallest-drive logic / no UI change** — the download flow just targets the **latest/first** drive (the
   mock fixture's drive in tests; the latest on real hardware), bounded the normal way (qcamera-only +
   retention cap + autoSync off). ✅
5. **No new testTags** — existing testTags + text selectors (which the current flows already use successfully)
   are sufficient; the app is English-only. Keep Phase E to flows + harness + the two fixes + docs.

Verified: Maestro **2.6.0** (runScript+http OK); poll intervals **dot 8s / drives 20s**; `remove_device.yaml`
`env:` shadow confirmed.

## Risks
Env shadowing (live in remove_device); runScript-http unproven in-repo (smoke-test first; curl fallback);
red/green masking (use 10.0.2.2 data port); black-box poll timing (budget ≥8s/20s, real-hw longer); filling
the Pixel (qcamera + retention cap + manual one-drive + autoSync off); wrong-adb-device on real run (require
explicit ANDROID_SERIAL); adb-over-wifi flakiness (keep real-hw semi-manual).
