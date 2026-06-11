# Almost-at-cap warning: per-device toggle + configurable threshold

## Context

The app already posts an "almost at usage cap" notification — the Phase D **low-headroom warning**
([Maintenance.kt](../../android/app/src/main/java/org/sunnypilot/dashdown/work/Maintenance.kt)):
when a device has a retention budget set, it warns once non-preserved local footage comes within
`WARN_HEADROOM_MIN = 10` minutes of the cap ("Storage almost full on …"). Today that warning is
**always on** (whenever a budget is set) and the **10-minute threshold is hardcoded**.

The request is to make that existing warning user-controllable: an **on/off toggle** and a
**configurable threshold in minutes** (retention is counted in minutes; each segment ≈ 1 minute, so
minutes are the natural unit — labeled plainly as "minutes" per the decision below). This is **one
feature extending the existing warning**, not a second notification (a separate notification firing
off the same budget math would double-notify).

**Decisions (user, 2026-06-11):**
- Threshold expressed in **minutes**, labeled just "minutes" (no "≈ segments" qualifier).
- Toggle defaults **on** and threshold defaults **10** so existing users with a budget keep today's
  exact behavior after upgrade.
- **Rate-limit: the warning fires at most once per day per device.** Today it re-posts every
  maintenance sweep (only `setOnlyAlertOnce` suppresses the buzz while it's still showing); a
  dismiss-then-re-post can re-alert. We add a hard 24h floor between alerts per device.

## Approach

Add two fields to the existing per-device settings seam (mirroring `retention_max_minutes` /
`auto_delete_min_age_min` exactly — same naming, same 5-site DB wiring, same UI/VM pattern). The Rust
core only stores them; the warning decision stays in `Maintenance.kt`, now reading the device's values
instead of constants.

New fields (Rust snake_case → Kotlin camelCase via UniFFI):
- `cap_warn_enabled: bool`  → `capWarnEnabled`  — column `INTEGER NOT NULL DEFAULT 1`
- `cap_warn_threshold_minutes: i64` → `capWarnThresholdMinutes` — column `INTEGER NOT NULL DEFAULT 10`

## Changes

### Rust core (`rust/core/`)
1. **`model/mod.rs`** (`Device` struct ~342-357): add the two fields. Update the in-file test fixture
   defaults (`cap_warn_enabled: true`, `cap_warn_threshold_minutes: 10`).
2. **`settings/mod.rs`** (`DeviceSettings` record + `settings()`/`apply_settings()`): add the two
   fields in all three places; extend the `settings_round_trip_*` test.
3. **DB schema migration v5** — new file `rust/core/src/db/schema_cap_warn.sql`:
   ```sql
   ALTER TABLE device ADD COLUMN cap_warn_enabled INTEGER NOT NULL DEFAULT 1;
   ALTER TABLE device ADD COLUMN cap_warn_threshold_minutes INTEGER NOT NULL DEFAULT 10;
   ```
   Register it as the 5th entry in `MIGRATIONS` in [migrations.rs](../../rust/core/src/db/migrations.rs)
   (`LATEST_VERSION` auto-derives to 5). Do **not** edit `schema.sql` (the v1 baseline). Existing
   on-device DBs run only v5, backfilling existing rows with enabled=1 / threshold=10.
4. **`db/mod.rs`** — add the two columns to the **five** hand-synced sites: `DEVICE_COLS` const
   (~30-32), `insert_device` INSERT + params (~85-104, new `?14`/`?15`), `update_device` UPDATE +
   params (~128-152), `RawDevice` struct (~708-722), `map_raw_device`/`raw_to_device` (~724-758,
   `r.get(13)`/`r.get(14)`).
5. **FFI**: no signature change — `get_settings`/`set_settings` already flow through
   `Device::settings()`/`apply_settings()` + `update_device`; the `DeviceSettings` record gains the
   fields automatically.

### Android (`android/app/`)
6. **UniFFI bindings** regenerate via the existing `:core:generateDebugUniFFIBindings` gradle task on
   build — no manual bindgen step.
7. **`Maintenance.kt`**: replace the always-on + hardcoded-threshold logic with the device's settings,
   and add the once-per-day rate limit.
   - Change `shouldWarn(s, threshold)` → `shouldWarn(s, enabled, threshold)`: `if (!enabled) return
     false` then the existing `budget - (local - preserved) < threshold`.
   - Add a pure helper `dueForNotification(lastMs: Long, nowMs: Long): Boolean = nowMs - lastMs >=
     MIN_NOTIFY_INTERVAL_MS` with `const val MIN_NOTIFY_INTERVAL_MS = 24*60*60*1000L`.
   - `sweep(...)`: if `shouldWarn(status, device.capWarnEnabled, device.capWarnThresholdMinutes)` →
     read the per-device last-notified stamp; if `dueForNotification(last, now)` then `warn(...)` and
     store `now`; otherwise leave the existing notification untouched (no re-alert). If `shouldWarn`
     is false → `cancel(...)` **and clear the stamp** so the next genuine crossing alerts immediately.
   - Persist the stamp in a small `SharedPreferences` (e.g. file `cap_warn`, key = device id) — no
     prefs/DataStore exists today; this is purely Android notification bookkeeping, kept out of the
     core domain DB. `now` = `System.currentTimeMillis()` (passed into the pure helper so it's
     testable). Keep `headroom()`; `warn()`/`cancel()`/channel/ids unchanged. `WARN_HEADROOM_MIN`
     becomes only the default reference (or is removed in favor of the DB/UI defaults).
8. **`DeviceSettingsViewModel.kt`**: add `capWarnEnabled: Boolean = true` and
   `capWarnThresholdMinutes: String = "10"` to `DeviceSettingsState`; read them in `load()`; add
   `onCapWarnEnabled`/`onCapWarnThreshold` (digit-filtered like `onMinAge`); pass them in the
   `DeviceSettings(...)` constructor in `save()`.
9. **`DeviceSettingsScreen.kt`**: in the retention section (after `settings_storage_usage`), add a
   `SwitchRow("Warn before older footage is auto-deleted", state.capWarnEnabled, onCapWarnEnabled,
   "settings_cap_warn")` and an `OutlinedTextField` labeled `"When within (minutes)"`, testTag
   `settings_cap_warn_threshold`, `enabled = state.capWarnEnabled` and only meaningful when a budget
   is set. Wire the two new callbacks through `DeviceSettingsRoute`.

## Tests
- **Rust unit**: `settings/mod.rs` round-trip extended with the new fields; a `db/mod.rs` device
  insert→read and update→read round-trip asserting the two columns persist; a `migrations.rs` test
  that a v4 DB upgrades to v5 and backfills enabled=1 / threshold=10 (follow existing migration-test
  style if present, else add a focused one).
- **Kotlin unit** (`MaintenanceTest.kt`): update existing `shouldWarn` calls to the new 3-arg form
  (enabled=true); add `disabledNeverWarns` (enabled=false ⇒ false even at the cap) and
  `customThresholdChangesBoundary` (e.g. threshold 30 warns where 10 wouldn't); keep the
  no-budget/preserved/headroom cases. Add `dueForNotification` cases: not due within 24h
  (`now - last < DAY`), due at/after 24h, and due when never notified (`last = 0`).
- **Settings VM** (if the existing test harness supports it): a load→edit→save round-trip carrying
  the two new fields through `repo.setSettings`.

## Verification
1. `cargo test` (workspace) — core settings/db/migration tests green.
2. `cd android && JAVA_HOME=…java-17 ANDROID_NDK_HOME=… ./gradlew :app:assembleDebug
   :app:testDebugUnitTest ktfmtCheck --no-daemon` — bindings regenerate, MaintenanceTest green,
   formatting clean.
3. On the api-35 emulator (boot per docs/TESTING.md §4 runbook): open a device's settings, set a
   small budget, toggle the new switch + threshold, Save; confirm it round-trips
   (`tools/dd-db.sh "SELECT cap_warn_enabled, cap_warn_threshold_minutes FROM device"`). Optionally
   add a Maestro assertion later (out of scope here).
4. Branch → PR → CI green (assemble + on-device + build·test + claude-review) → merge.

## Out of scope
A second/separate notification; byte- or segment-unit storage accounting; a unit picker; changing the
inert `auto_delete_*` settings; Maestro flow coverage for the new controls (existing instrumented +
unit tests cover the logic).
