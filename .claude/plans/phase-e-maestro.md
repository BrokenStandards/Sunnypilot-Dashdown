# Phase E — Maestro black-box suite + real-hardware harness

> On approval, saved as `.claude/plans/phase-e-maestro.md`. Research: `.claude/plans/phase-e-research.md`.

## Context

The final roadmap phase: a **black-box Maestro flow suite** for the app's user-facing features, runnable on
**both** the mock fixture and **real hardware** (read-only on the comma), driven by a one-command host
harness. It complements — doesn't replace — the instrumented tests: the Phase C live-refresh behavior is
already CI-guarded by `DotLiveRefreshTest`/`DriveListLiveRefreshTest`; the Maestro flows mirror those + the
read-only feature flows as **black-box UI** acceptance, run **locally and on the Pixel+comma**.

**Decisions (locked):** Maestro stays **local + real-hw only** (NOT in CI); a **scripted harness + runbook**
for real hardware (`MODE=real`, manual trigger); **full suite in one PR**; **no smallest-drive logic / no UI
change** — the download flow targets the **latest/first** drive, bounded the normal way (qcamera-only +
retention cap + autoSync off); **no new testTags** (existing tags + text selectors suffice). Verified: Maestro
**2.6.0** (runScript+`http` works); production polls **dot 8s / drives 20s** (flow timeouts budget ≥15s/30s);
`remove_device.yaml`'s `env:` block shadows `-e` (bug to fix). **No production app code changes** in this phase.

## Flow suite (`android/maestro/`)
Params via `-e`: `NAME`, `IP`, `PORT`, `MODE` (mock|real), `CONTROL` (control port). `*_mock` flows gate their
control-port steps `when: { true: "${MODE == 'mock'}" }` so a real run never POSTs to the comma. All
assertions are black-box (testTag id / text / contentDescription).

**Shared / both-targets:**
- `empty_state.yaml` (new) — `launchApp clearState:true` → assert "No devices yet. Tap + …".
- `add_device.yaml` (exists, keep) — FAB → `device_form_*` ← `${NAME}/${IP}/${PORT}/${WIFI_IP}` → save; optional "Use it".
- `drive_download.yaml` (**parameterize**) — replace hardcoded `127.0.0.1`/`8099`/`device_row_1`: add device via
  `${IP}/${PORT}/${NAME}`, open the device row **by name** (regex/text, not `device_row_1`), `extendedWaitUntil
  "Download"` (20s) → tap the first `drive_download_*` → `extendedWaitUntil "Complete"` (30s mock / 120s real).
- `play_drive.yaml` (new) — `runFlow: drive_download` → tap the drive row (`id: "drive_row_.*"`) → assert
  `drive_detail_player` visible → tap `drive_play_toggle`; tolerate "Preparing HD…" (emulator may not decode —
  assert the player UI renders, not actual frames).
- `star_drive.yaml` (new) — `runFlow: drive_download` → tap `drive_preserve_.*` → assert `preserve_on` (local-only).
- `manual_download_close.yaml` (new) — add device → open drives → tap "Download" → `pressKey: Home` (background;
  the dataSync FGS keeps running) → `launchApp` (no clearState) → reopen drives → `extendedWaitUntil "Complete"`.
- `remove_device.yaml` (exists, **de-shadow**) — drop the `env: { NAME }` block; rely on `-e NAME`.
- `clear_devices.yaml` (exists, keep).

**Mock-only (control port via `runScript` → `http`):**
- `connectivity_refresh_mock.yaml` (new) — add device at **`IP=10.0.2.2`** (host alias, no data-port reverse →
  a closed port is genuinely refused; control port stays reversed) → wait `conn_dot_green` → `runScript
  set_reachable {up:false}` → `extendedWaitUntil conn_dot_red` (15s ≥ 8s poll) → `{up:true}` → `conn_dot_green`.
- `drive_list_refresh_mock.yaml` (new) — add device at `IP=127.0.0.1` (membership refresh works over reverse) →
  wait baseline drive row → `runScript add_drive {route:"000009ee--maestroadd",segs:1}` → `extendedWaitUntil`
  the new `drive_row_000009ee--maestroadd--0` (30s ≥ 20s poll) → `runScript remove_drive` → wait it disappears.

**`android/maestro/scripts/{set_reachable,add_drive,remove_drive}.js`** — `http.post('http://127.0.0.1:'+CONTROL+
'/<ep>', { body: JSON.stringify({...}), headers:{'Content-Type':'application/json'} }); output.ok = r.ok`. Vars
passed per-`runScript` `env:` (never a top-level `env:` block → shadowing).

## Harness — `tools/run-maestro-e2e.sh` (new)
`tools/run-maestro-e2e.sh <flow|dir> [MODE=mock|real] [IP=…] [PORT=…] [NAME=…]`
- **mock** (default): build the debug APK + the mock binary, start `mock-copyparty --fixture single_drive
  --port $MOCK_PORT --control-port $CONTROL_PORT`, poll `/status`, `adb reverse` both ports, `adb install -r`
  the APK, then `maestro --device $SERIAL test -e MODE=mock -e IP=127.0.0.1 -e PORT=$MOCK_PORT -e
  CONTROL=$CONTROL_PORT <target>`; trap-cleanup (kill mock + remove reverses). Reuses the mock-lifecycle from
  `tools/run-android-e2e.sh` — **duplicated** (~12 lines) rather than refactoring the CI-critical runner (zero
  CI risk; the mock CLI/endpoints are stable).
- **real** (`MODE=real`, `IP=<comma>`): **require explicit `ANDROID_SERIAL`** (the Pixel) — never "first adb
  device"; no mock/reverse; install the APK; run only the bounded read-only flows (`drive_download`,
  `play_drive`, `star_drive`, `empty_state`, `add_device`, `remove_device`) with `MODE=real` (control-port
  steps self-gate off), longer timeouts. Bounded: qcamera-only selection + a small `settings_retention` cap +
  `autoSync` off + manual single-drive download → never fills the Pixel, never touches the comma.
- JDK 17 + `$HOME/.maestro/bin` on PATH (like `dd-ui.sh`).

`tools/dd-ui.sh` (keep as the thin single-flow launcher) — no breaking change; existing `NAME/IP/PORT/WIFI_IP`
usage stays.

## Files
- **New:** `android/maestro/{empty_state,play_drive,star_drive,manual_download_close,connectivity_refresh_mock,
  drive_list_refresh_mock}.yaml`; `android/maestro/scripts/{set_reachable,add_drive,remove_drive}.js`;
  `tools/run-maestro-e2e.sh`.
- **Edit:** `android/maestro/drive_download.yaml` (parameterize), `android/maestro/remove_device.yaml`
  (de-shadow), `docs/TESTING.md` (§4 Maestro: flow list, mock-vs-real matrix, `MODE`/`IP`/`PORT`/`CONTROL`
  contract, env-shadowing rule, runScript note, the one-command runbook for both modes).
- **No production Kotlin/Rust changes.**

## Verification
1. **Smoke-test `runScript`+`http` first** (it's unproven in-repo): run `connectivity_refresh_mock` alone on the
   emulator; confirm the POST reaches the control port and the dot flips. **Fallback** if it fails (CLI/engine
   issue): split the control-port steps out and `curl` them host-side from the harness between flows.
2. **Mock suite on the api-35 emulator:** boot `dashdown-b0`, `ANDROID_SERIAL=emulator-5554
   tools/run-maestro-e2e.sh android/maestro MODE=mock` → all flows pass (each flow self-skips/should-run per
   `MODE`). Verify dot red↔green and drive add/remove happen with **no manual refresh**.
3. **Real-hw (manual, optional):** `ANDROID_SERIAL=192.168.1.210:5555 tools/run-maestro-e2e.sh drive_download
   IP=192.168.1.100 PORT=8080 MODE=real` against the comma — add device, download one drive, play, star; confirm
   read-only (the comma is untouched; only the Pixel's local mirror + app DB change). `tools/dd-db.sh drives` to inspect.
4. **CI:** unchanged — Maestro isn't in CI; the existing `assemble` + `connected-check` (instrumented) + `build·
   test` + `claude-review` jobs pass (no compiled code changed). Branch → PR → CI green → squash-merge. Exclude
   `.claude/plans/b2-android-shell.md`.

## Risks / mitigations
- **runScript+http unproven** → smoke-test first; host-side `curl` fallback ready.
- **Env shadowing** → fix `remove_device`; never a top-level `env:` for overridable vars; per-`runScript` env.
- **Red/green masking over adb reverse** → connectivity flow uses `IP=10.0.2.2` for the data port (proven by `DotLiveRefreshTest`).
- **Black-box poll timing** → `extendedWaitUntil` timeouts ≥ production interval (15s dot / 30s drives) + margin.
- **Filling the Pixel** → qcamera-only + retention cap + autoSync off + one manual drive.
- **Wrong adb device on real run** → `MODE=real` requires explicit `ANDROID_SERIAL`.
- **`device_row_1` fragility** → flows select the device by **name**, not id.

## Out of scope
Maestro-in-CI (a later, optional self-hosted-emulator step); smallest-drive sort/UI; new testTags; any comma
write path. Background auto-download + segment-pickup stay as the existing instrumented live tests (no UI surface).
