# M8 — UniFFI surface (`AppCore` facade) — final Phase-A milestone

## Context

Phase A milestones M0–M7 built the entire Rust core (copyparty client, drive grouping, mirror
storage, sync/download/resume engine, retention + remote delete, connectivity) — all as **plain
Rust**, verified by `cargo test`. Nothing crosses the FFI boundary yet: `model/mod.rs:1` literally
says *"no UniFFI derives yet — the boundary is added in M8."* M8 is the milestone that turns the
library into the native-facing API the SwiftUI/Compose shells (Phase B) consume: the **`AppCore`**
facade object, the boundary types (Records/Enums), the `ProgressSink`/`LogSink` callback traits, a
`tracing`→`LogSink` bridge, `export_drive_zip`, and the generated Swift + Kotlin bindings.
Completing M8 closes Phase A and unblocks the native shells.

This plan was researched via multi-agent workflows (Understand + Design). The **runtime
architecture is the decisive finding** and is settled below.

## Decisions (settled)

- **Runtime (the crux):** uniffi 0.31 keeps **no persistent runtime** — it wraps each exported
  future in `async_compat::Compat` and the *foreign* side drives one poll loop per call
  (`ref/uniffi-rs/uniffi_macros/src/export/scaffolding.rs:273`, `uniffi_core/.../rustfuture/`). A
  detached `tokio::spawn` from inside an exported method is therefore **orphaned** once the call
  returns. **Verdict:** `AppCore` owns one long-lived **multi-thread `tokio::runtime::Runtime`
  (`enable_all`)**. All data methods are `#[uniffi::export(async_runtime = "tokio")]` async; the
  owned runtime is used **only** for `runtime.handle().spawn(...)` to launch the detached drive
  download. **No `block_on` anywhere** (panics under `Compat`), **no bare `tokio::spawn`**. So
  `start_drive_download` is async: it `.await`s the device lookup, then `handle().spawn(...)`s
  `engine.download_drive(...)` — it does **not** call the engine's existing bare-`tokio::spawn`
  `start_drive_download` (which would orphan the task).
- **Derives in place, no parallel boundary types.** Add `#[derive(uniffi::Record/Enum)]` directly
  to the existing model/engine types; make `SegmentName` a `Record` so `Segment` and the whole
  engine/`paths.rs` stay untouched (zero `From` shims). `port: u16` is **FFI-native** (no
  `custom_type!`). `dest_path` crosses as plain `String`. `CoreError` gets `#[derive(uniffi::Error)]`
  directly (all variants are `String`/unit → FFI-legal; it already says so at `error.rs:5`).
- **Callbacks:** `#[uniffi::export(rust, foreign)]` foreign traits → `Arc<dyn Trait>` (modern; not
  the deprecated `callback_interface`/`Box<dyn>`). Foreign-trait method args must be **owned**
  (`String`, not `&str`) → `ProgressSink::on_completed/on_failed` change from `&str` to `String`
  (ripples to the engine's call sites in `download_drive`/`finish`).
- **User scope calls:** `export_drive_zip` is **built now** (zips the local mirror; adds a `zip`
  crate). Binding smoke stays **generate + grep symbols** (no Swift/Kotlin compile — deferred to
  Phase B; the Rust `it_appcore` e2e is the functional proof). `drive_id` in the plan == the
  existing **`drive_key: String`** (no numeric drive id exists). `DeviceSettings` is a real Record
  with `get_settings`/`set_settings` (connection edits stay on `update_device`).

## Design

### Module layout
- **`lib.rs`** (edit): add `pub mod ffi; pub mod settings; pub mod logging;`. Keep `version`/`ping`/
  `ping_async` for M8 (zero-cost async-FFI canary; retire in a follow-up).
- **`ffi/mod.rs`** (new): `AppCore` `#[derive(uniffi::Object)]` + two impl blocks — a sync
  `#[uniffi::export]` (constructor + sink setters) and an async
  `#[uniffi::export(async_runtime="tokio")]` (all data methods); the `SyncHandle` Object; `NoopSink`;
  the `DriveSyncStatus` Record.
- **`ffi/callbacks.rs`** (new): `LogSink` foreign trait + `pub use crate::sync_engine::ProgressSink`
  (the engine owns `ProgressSink`; the FFI attribute is applied there so the engine layer doesn't
  depend on `ffi`).
- **`error.rs`** (edit): add `#[derive(uniffi::Error)]` to `CoreError` (skip the planned
  `ffi/errors.rs` — annotate in place; the `From` impls stay).
- **`settings/mod.rs`** (new): `DeviceSettings` Record + `impl Device { fn settings()/apply_settings() }`.
- **`logging/mod.rs`** (new): `LogLevel` Enum, `LogEvent` Record, `LogSink`-forwarding `tracing`
  Layer + global `install()`/`set_sink(sink, level)` with **password redaction**.

### Type derives (in place)
- **`#[derive(uniffi::Enum)]`:** `ConnMode`, `ConnDot`, `DownloadState`, `SyncStatus`, `JobState`,
  `FileKind`, + new `LogLevel`. (Inherent `as_str`/`parse` are unaffected.)
- **`#[derive(uniffi::Record)]`:** `SegmentName`, `SegmentFile`, `Segment`, `Drive`, `Device`,
  `FileSelection`, `DownloadProgress`, `DeviceConnectivity`, + new `DeviceSettings`, `LogEvent`,
  `DriveSyncStatus`. (`SegmentName` keeps `#[derive(Hash)]` alongside.)

### `AppCore` (`ffi/mod.rs`)
```rust
#[derive(uniffi::Object)]
pub struct AppCore {
    engine: SyncEngine,                                   // Clone; holds Arc<Repo> + mirror_root
    repo: Arc<Repo>,                                      // cheap Arc clone for device CRUD
    runtime: Arc<tokio::runtime::Runtime>,                // owned multi-thread; spawn target only
    progress_sink: std::sync::RwLock<Option<Arc<dyn ProgressSink>>>,
}
```
**Sync block** `#[uniffi::export]`:
- `#[uniffi::constructor] fn new(db_path: String, mirror_root: String) -> Result<Arc<Self>, CoreError>`
  — `Repo::open`, `SyncEngine::new`, `Builder::new_multi_thread().enable_all().build()` (map
  `io::Error`→`CoreError::Io`), `logging::install()` (idempotent), `progress_sink = RwLock(None)`.
- `fn set_progress_sink(&self, sink: Option<Arc<dyn ProgressSink>>)`
- `fn set_log_sink(&self, sink: Option<Arc<dyn LogSink>>, level: LogLevel)` → `logging::set_sink(..)`
  (two-arg form; a level with no sink is a no-op).

**Async block** `#[uniffi::export(async_runtime = "tokio")]`, every method `-> Result<_, CoreError>`.
A private `load_device(id)` (`spawn_blocking repo.get_device` → `NotFound` if `None`) and a
`spawn_blocking` join-error shim mirror the engine's `db()`.

| Method | Backing |
|---|---|
| `list_devices() -> Vec<Device>` | `spawn_blocking repo.list_devices` |
| `add_device(device: Device) -> Device` | `repo.insert_device`, echo back assigned `id` |
| `update_device(device: Device)` | **new** `repo.update_device` |
| `remove_device(device_id: i64)` | **new** `repo.delete_device` (FK-cascade) + best-effort `tokio::fs::remove_dir_all(engine.mirror_root()/<id>)` |
| `set_active_mode(device_id, mode: ConnMode)` | load → set `active_mode` → `update_device` |
| `get_settings(device_id) -> DeviceSettings` | `load_device().settings()` |
| `set_settings(device_id, settings: DeviceSettings)` | load → `apply_settings` → `update_device` |
| `list_drives(device_id, offline: bool) -> Vec<Drive>` | online → `engine.sync_now(&dev)`; **offline → `engine.reconcile_device(&dev)`** (see fix #1) |
| `get_drive(device_id, drive_key: String) -> Drive` | **new** `repo.get_drive` (filter `get_drives`); `NotFound` |
| `get_drive_status(device_id, drive_key: String) -> DriveSyncStatus` | `repo.get_job` + drive row → projection |
| `sync_now(device_id) -> Vec<Drive>` | `engine.sync_now(&dev)` |
| `set_preserved(device_id, drive_key: String, preserved: bool)` | `spawn_blocking repo.set_drive_preserved` |
| `start_drive_download(device_id, drive_key: String) -> Arc<SyncHandle>` | load dev; `token=CancellationToken::new()`; `sink = progress_sink.read().clone().unwrap_or_else(NoopSink)`; `self.runtime.handle().spawn(engine.download_drive(dev, drive_key, sink, token.clone()))`; return `Arc::new(SyncHandle::new(token))` |
| `export_drive_zip(device_id, drive_key: String, dest_path: String)` | see below |
| `run_maintenance(device_id)` | `engine.run_maintenance(&dev, time::now_ms())` |
| `check_connectivity(device_id) -> DeviceConnectivity` | `engine.check_connectivity(&dev)` |

### `SyncHandle` (FFI Object)
Add `#[derive(uniffi::Object)]` to the existing `SyncHandle` + `#[uniffi::export] impl { pub fn cancel(&self) }` (and `is_cancelled`). Add a `pub(crate) fn new(token)` so `AppCore` constructs it (token field is private).

### Callbacks
- `ProgressSink` (in `sync_engine/download_job.rs`): add `#[uniffi::export(rust, foreign)]`; change
  `on_completed(&self, drive_key: String)` / `on_failed(&self, drive_key: String, error: String)`
  (from `&str`). Update the two call sites in `sync_engine/mod.rs` (`finish`/loop) to pass owned `String`.
- `LogSink` (in `logging/mod.rs`): `#[uniffi::export(rust, foreign)] pub trait LogSink: Send + Sync { fn on_log(&self, event: LogEvent); }`.

### `export_drive_zip` (zip the local mirror — offline)
In `spawn_blocking`: `let drive = repo.get_drive(device_id, &drive_key)?`; for each `seg`/`f` where
`device.file_selection.includes(f.kind)`, compute `rel = file_rel(REALDATA_REL, &seg.name, &f.name)`
and, when `mirror.is_complete(rel)`, add `mirror.final_path(rel)` to a `zip::ZipWriter` at
`PathBuf::from(dest_path)` under a drive-relative entry name (`<seg dir>/<file>`). Reuses
`MirrorStore` (`engine.mirror_root().join(device_id)`), `FileSelection::includes` (`model/mod.rs:114`),
`file_rel` (`storage/paths.rs:12`). `zip` crate (sync API) → that's why it runs in `spawn_blocking`.

### `tracing` → `LogSink` (`logging/mod.rs`)
- `LogLevel{Error,Warn,Info,Debug,Trace}` (Enum); `LogEvent{level, target: String, message: String, timestamp_ms: i64}` (Record, ms via `time::now_ms()`).
- A `tracing_subscriber::Layer` whose `on_event` formats the event, **redacts any field named
  `password`/`pw`** (and the inherent pw-redaction the core already practices), filters by the
  configured level, and forwards to the global sink. `install()` registers the layer once
  (`std::sync::OnceLock`); `set_sink(sink, level)` swaps a global `RwLock<Option<Arc<dyn LogSink>>>` +
  an `AtomicU8` level. Add `tracing-subscriber` (`cargo add`, `registry` feature).

### Repo gaps (`db/mod.rs`)
- `update_device(&self, d: &Device) -> Result<()>` — `UPDATE device SET <DEVICE_COLS-without-id> WHERE id=?`.
- `delete_device(&self, id: i64) -> Result<()>` — `DELETE FROM device WHERE id=?1` (children cascade; `foreign_keys=ON` at `db/mod.rs:41`, FKs in `schema*.sql`).
- `get_drive(&self, device_id, drive_key: &str) -> Result<Option<Drive>>` — filter `get_drives`.

### Other core additions
- `SyncEngine::mirror_root(&self) -> &Path` accessor (fix #3 — private field today; needed by
  `export_drive_zip` + `remove_device` cleanup).
- `model::time::now_ms() -> i64` (fix — only `secs_to_ms` exists today; mirror `db::now_s`).
- `DeviceSettings` = `{auto_sync, file_selection, retention_max_minutes, auto_delete_from_comma, auto_delete_min_age_min}`; `Device::settings()`/`apply_settings()`.
- `DriveSyncStatus` = `{drive_key: String, status: SyncStatus, files_done: u32, files_total: u32, bytes_done: u64, bytes_total: u64, error: Option<String>}`.

### Cargo (`rust/core/Cargo.toml`, via `cargo add`)
- `tokio` += `rt-multi-thread` (AppCore builds a multi-thread Runtime in non-test code).
- `zip` (latest) — `export_drive_zip`.
- `tracing-subscriber` (latest, `registry` feature) — the LogSink layer.
- `uniffi` stays `features=["tokio"]`. `async_compat` comes transitively via uniffi's `tokio` feature.

## Critical correctness fixes (from the design critique)
1. **Offline `list_drives` must use `engine.reconcile_device(&dev)`**, NOT `group_local` —
   `group_local`/`finalize` hardcodes `sync_state = NotDownloaded`, so a local scan would mislabel
   mirrored drives. `reconcile_device` reclassifies from disk (offline-capable) and returns hydrated
   drives with correct state.
3. **Add `SyncEngine::mirror_root()`** — without it `export_drive_zip`/`remove_device` can't resolve
   the per-device mirror path.
5. **`ProgressSink` `&str`→`String`** (foreign-trait arg rule) — ripples to engine call sites.
6. **`it_appcore` must use `#[tokio::test(flavor = "multi_thread")]`** + poll the recorder with a
   bounded timeout (the handle only cancels; you can't `await` download completion).
- **`NoopSink`** defined in `ffi/mod.rs` (used when no progress sink is set).
- **`run_maintenance` time** uses `model::time::now_ms()`.

## Files
- **new:** `rust/core/src/ffi/mod.rs`, `rust/core/src/ffi/callbacks.rs`, `rust/core/src/settings/mod.rs`, `rust/core/src/logging/mod.rs`, `rust/core/tests/it_appcore.rs`.
- **edit:** `rust/core/src/lib.rs` (module decls), `rust/core/src/error.rs` (uniffi::Error),
  `rust/core/src/model/mod.rs` + `model/ids.rs` + `model/file_kind.rs` (derives), `model/time.rs`
  (`now_ms`), `rust/core/src/connectivity/mod.rs` (`DeviceConnectivity` Record derive),
  `rust/core/src/sync_engine/mod.rs` (`mirror_root()`, call-site `String` change) +
  `download_job.rs` (`ProgressSink` derive + `String` args + `DownloadProgress` Record),
  `rust/core/src/db/mod.rs` (`update_device`/`delete_device`/`get_drive`), `rust/core/Cargo.toml`,
  `.github/workflows/rust-ci.yml` + `tools/gen-bindings.sh` (smoke greps).

## Verification (TDD where practical)
- **Unit:** `DeviceSettings` round-trip (`settings()`/`apply_settings()`); `update_device`/
  `delete_device` (+ cascade) in `it_db.rs`; `now_ms` sanity; `LogLevel`/`ConnDot` etc. derive-compile.
- **Integration `it_appcore.rs`** (`#[tokio::test(flavor="multi_thread")]`, `mock-copyparty`):
  `AppCore::new(temp db, temp mirror)` → `add_device(@srv.addr)` → `list_devices` → `sync_now`/
  `list_drives(online)` → `set_progress_sink(Arc<Recorder>)` → `start_drive_download` → poll recorder
  (bounded timeout) until `on_completed` → assert files mirrored + `check_connectivity` is Blue mid /
  Green after / **Red** when the server is dropped → `get_settings`/`set_settings` round-trip →
  `set_preserved` → `export_drive_zip(dest)` then reopen the zip and assert the selected entries are
  present → `update_device` → `remove_device` (drive rows gone, mirror dir removed). A `LogSink`
  recorder asserts `on_log` fires. (AppCore async methods are plain async Rust — directly callable
  in the test; the uniffi export wrapper doesn't affect in-crate calls.)
- **Offline:** populate a mirror, server down, `list_drives(offline=true)` returns the same grouping
  with correct `sync_state` (proves fix #1).
- **Bindings smoke (CI + `tools/gen-bindings.sh`):** keep generate + grep; **update the
  `rust-ci.yml` grep block** to also assert (case-insensitive substrings, tolerant of
  `open class`/`public func`) in both `.kt` and `.swift`: `AppCore`, `addDevice`, `syncNow`,
  `startDriveDownload`, `checkConnectivity`, `SyncHandle`, `cancel`, `ProgressSink`, `LogSink`,
  `CoreError`, `DeviceSettings`; plus `suspend fun syncNow` in `.kt` (proves the tokio-async path
  generated). Keep the existing `ping`/`version` greps (those exports remain).
- **Gates:** `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`;
  `cargo test --workspace`; `bash tools/gen-bindings.sh` + the greps. Then commit, PR, watch CI
  (build/test/cross-compile/bindgen + claude-review), squash-merge. Rename this plan to
  `.claude/plans/m8-uniffi-surface.md` during implementation. **Phase A is complete at merge.**

## Risks / verify during implementation
1. **`spawn_blocking` under `Compat`** — every data method offloads the sync `Repo` via
   `spawn_blocking`; it resolves on async-compat's runtime during polling (the engine already relies
   on this in `download_drive`). Load-bearing but exercised by `it_appcore`.
2. **Detached download lifetime** — confirm the `runtime.handle().spawn(...)` task keeps running
   after `start_drive_download` returns (the whole reason for the owned runtime). The bounded-timeout
   recorder poll in the test proves it.
3. **`&str`→`String` ripple** — verify uniffi 0.31 actually rejects `&str` in foreign-trait methods;
   if it accepts `&str`, the `String` change is still harmless. Update both engine call sites either way.
4. **`uniffi::Error` on `CoreError`** — confirm the non-`flat` derive generates (all variants are
   `String`/unit, so variant fields cross fine); exported `Result<_, CoreError>` lowers to a thrown
   error on the foreign side.
5. **`tracing-subscriber` global install idempotency** — `install()` must be safe to call once per
   `AppCore::new` across multiple instances/tests (use `OnceLock`; don't panic on re-init).
6. **Android cross-compile** — the new `zip`/`tracing-subscriber` deps must cross-compile for the 4
   ABIs (pure-Rust; low risk). CI's `cargo ndk` step covers it.
7. **`zip` entry paths** — use forward-slash, drive-relative entry names; don't leak absolute mirror
   paths into the archive.
