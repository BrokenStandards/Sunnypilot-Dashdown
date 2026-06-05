# M4 — Sync/Download Engine

> **Filename note:** rename this file to `m4-sync-download-engine.md` as the first implementation step.

## Context

M1–M3 gave us a copyparty client, a domain model + SQLite index, drive grouping, and a crash-safe
mirror (`MirrorStore`/`PartFile`). Nothing yet *downloads*. M4 builds the transfer engine that ties
them together: refresh a device's index (`sync_now`), download a drive's selected files atomically
with **progress** + **cancellation**, verify sizes (re-fetch on truncation), and persist a
`download_job` row. The Rust core owns the engine (awaitable + cancellable); native scheduling
(Foreground Service / BGTask) is Phase B. Resume-classification/restart-recovery is M5; retention is M6.

## Notable change from the master plan — per-stream selection (user decision)

The master plan's `FileSelection { PreviewsOnly, FullVideo, FullVideoPlusLogs }` enum is **replaced
by per-stream toggles**: one boolean per downloadable file kind. Verified against sunnypilot source:
**audio is muxed into `qcamera.ts`** (`system/loggerd/video_writer.h` `write_audio`; "audio included
in the dashcam video"), so it is *not* a separate file — the `qcamera` toggle carries preview+audio.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSelection {           // a UniFFI Record in M8
    pub fcamera: bool,   // road cam (.hevc)
    pub ecamera: bool,   // wide road cam (.hevc)
    pub dcamera: bool,   // driver/cabin cam (.hevc, only if recorded)
    pub qcamera: bool,   // low-res preview (.ts) + muxed audio
    pub rlog: bool, pub qlog: bool, pub bootlog: bool,
    pub other: bool,     // files matching no known kind
}
impl FileSelection {
    pub fn includes(&self, kind: FileKind) -> bool;       // LockMarker => always false
    pub fn previews_only() -> Self;                        // qcamera only (Default)
    pub fn as_str(&self) -> String;                        // CSV of enabled kinds, sorted
    pub fn parse(s: &str) -> Result<Self>;                 // CSV; also accepts legacy preset names
}
impl Default for FileSelection { /* previews_only */ }
```
Persisted in the **existing** `device.file_selection` TEXT column as a sorted CSV (e.g. `"qcamera,rlog"`)
— no schema migration; `parse` also maps legacy `previews_only`/`full_video`/`full_video_plus_logs`
for safety (no production data exists yet). **Reconcile the master plan** (data model + UniFFI
surface) to the toggle model as part of this milestone.

## Scope

**In:** `sync_engine::{SyncEngine, SyncHandle}`; `sync_now` (index refresh); `download_drive`
(awaitable, cancellable) + `start_drive_download` (spawns, returns `SyncHandle`); `download_file`
(size-verify + bounded retry + cancel); `ProgressSink`/`DownloadProgress`; per-stream
`FileSelection`; migration **v3** (`download_job`) + Repo job/seg_file methods; tests.
**Out (later):** resume classification / restart recovery from stale `running` jobs (**M5**);
retention/auto-delete (**M6**); connectivity dot (**M7**); UniFFI `#[export]` (**M8**). No HTTP Range
(re-verified in M5). `sync_now` refreshes only — it does **not** auto-download.

## Settled design decisions (incl. Plan-agent refinements)

- **`SyncEngine { repo: Arc<Repo>, mirror_root: PathBuf }`, `#[derive(Clone)]`.** `mirror_for(device)`
  = `MirrorStore::new(mirror_root.join(device.id.to_string()))` (per-device subdir lands here; M8
  passes the app root). `client_for(device)` builds creds from `device.password` (Some→Password,
  None→Anonymous). `const REALDATA_REL = "realdata/"` (TODO M8: make device-configurable if needed).
- **Cancellation = `tokio::select!` (biased) racing the download future vs `cancel.cancelled()`**, plus
  an `is_cancelled()` check before each file. On cancel the raced `async` block (which **owns** the
  `PartFile`) is dropped → in-flight reqwest stream closes, `.part` left for M5. `download_to` is
  unchanged. `tokio-util = { version = "0.7", features = ["sync"] }` (already in lockfile; `sync`
  needed for `CancellationToken`).
- **`download_file(client, mirror, rel, expected_size, cancel, max_attempts) -> Result<FileOutcome>`**
  (`FileOutcome { Complete, Canceled }`): loop ≤ `max_attempts` (const 2 = one retry):
  `create_part` (truncates stale `.part`) → `download_to` → if `written == expected_size` → `commit`,
  return `Complete`; if `written != expected` (too short **or** too long) → drop pf, retry; transport/
  IO `Err` → retry if `attempt < max`; `AuthRequired`/`Forbidden`/`NotFound`/`Parse` → **fail fast**.
  Exhausted size-mismatch → `Err`. No `abort()` between attempts (next `create_part` truncates); leave
  `.part` on terminal failure (M5 breadcrumb). `fn is_retriable(&CoreError) -> matches!(Http|Io)`.
- **`download_drive`**: `get_drives` **once**, find by `drive_key`; selected files = those with
  `device.file_selection.includes(file.kind)`; `bytes_total` = Σ selected `remote_size`; pre-credit
  `bytes_done`/`files_done` for files already complete on disk
  (`mirror.is_complete(rel) && local_size==remote_size` — fast `stat`s, called directly, no
  spawn_blocking). Empty selection ⇒ immediate `Complete` (guard div-by-zero). Set
  `drive.sync_state=Downloading` + `upsert_job(running)` at start; per committed file →
  `set_file_complete` + `bump_job_progress` + `sink.on_progress`. All done → `sync_state=Complete`,
  `set_job_state(complete)`, `on_completed`. Cancel → `set_job_state(canceled)`, return `Canceled`.
  Err → `sync_state=Failed`, `set_job_state(failed,err)`, `on_failed`, return `Failed`.
- **`start_drive_download(device: Device, drive_key: String, sink) -> SyncHandle`**: `let e=self.clone();`
  `tokio::spawn(async move { e.download_drive(&device,&drive_key,sink,token.clone()).await })`; return
  `SyncHandle { token }`. **Join-less** (native owns task lifecycle per the background contract);
  the cancel test spawns `download_drive` directly to get a `JoinHandle`.
- **`sync_now`**: list once; do upsert_segments + group_segments + replace_drives in **one**
  `spawn_blocking` closure (segments moved in, avoids a borrow/move conflict and double-listing);
  return `get_drives` (hydrated, preserving stored `sync_state`/`preserved`).
- **DB calls wrapped in `spawn_blocking`** via `async fn db<T>(repo: Arc<Repo>, f: FnOnce(&Repo)->Result<T>)`
  mapping `JoinError`→`CoreError::Db`. Filesystem `stat`s stay direct (microseconds).
- **Progress = per-file** (emit after each commit + one initial). `DownloadProgress { drive_key,
  files_done, files_total, bytes_done, bytes_total, current_file }`. Byte-level intra-file progress is
  a later `ProgressWriter<W>` newtype — no trait change needed.

## Module / API sketch

`sync_engine/mod.rs`: `SyncEngine` (+ `new`, `sync_now`, `download_drive`, `start_drive_download`,
`client_for`/`mirror_for`, `db` helper, consts), `SyncHandle { token }` + `cancel()`.
`sync_engine/download_job.rs`: `download_file`, `ProgressSink` (trait: `on_progress`/`on_completed`/
`on_failed`), `DownloadProgress`, `FileOutcome`, `JobOutcome { Complete, Canceled, Failed(String) }`,
`JobState` enum (as_str/parse). Add `pub mod sync_engine;` to `lib.rs`.

## DB additions

**Migration v3** — new `db/schema_job.sql`, appended to `migrations.rs`:
```sql
CREATE TABLE IF NOT EXISTS download_job (
    id          INTEGER PRIMARY KEY,
    device_id   INTEGER NOT NULL REFERENCES device(id) ON DELETE CASCADE,
    drive_key   TEXT    NOT NULL,
    state       TEXT    NOT NULL DEFAULT 'running', -- running|complete|failed|canceled
    files_total INTEGER NOT NULL DEFAULT 0,
    files_done  INTEGER NOT NULL DEFAULT 0,
    bytes_total INTEGER NOT NULL DEFAULT 0,
    bytes_done  INTEGER NOT NULL DEFAULT 0,
    error       TEXT,
    updated_s   INTEGER NOT NULL DEFAULT 0,        -- epoch seconds (std::time::SystemTime)
    UNIQUE(device_id, drive_key)
);
```
**Repo methods** (`db/mod.rs`):
- `set_file_complete(device_id, route_id, segment_num, file_name, local_size)` — single `UPDATE seg_file
  SET local_size=?, download_state='complete' WHERE name=? AND segment_id=(SELECT id FROM segment WHERE
  device_id=? AND route_id=? AND segment_num=?)` (no segment_id juggling; FK-safe).
- `upsert_job(device_id, drive_key, files_total, bytes_total)` (ON CONFLICT → reset to running/0),
  `set_job_state(device_id, drive_key, JobState, error: Option<&str>)` (+stamp `updated_s`),
  `bump_job_progress(device_id, drive_key, files_done, bytes_done)` (absolute set, +stamp), `get_job →
  Option<RawJob>`. `RawJob` struct + `JobState::parse` outside the row closure (mirrors `raw_to_device`).
- Bump `migrations` array → v3; **update both `it_db` `schema_version()` asserts 2 → 3.**

## Files changed

- new `rust/core/src/sync_engine/{mod,download_job}.rs`, `rust/core/src/db/schema_job.sql`,
  `rust/core/tests/it_download.rs`
- `rust/core/src/lib.rs` (+`pub mod sync_engine;`)
- `rust/core/src/model/mod.rs` (replace `FileSelection` enum with the toggle struct + `includes`)
- `rust/core/src/db/mod.rs` (job + `set_file_complete` methods; `RawDevice` file_selection parse
  already routes through `FileSelection::parse`), `rust/core/src/db/migrations.rs` (+v3)
- `rust/core/Cargo.toml` (+`tokio-util` `sync`)
- `rust/core/tests/it_db.rs` (schema_version 2→3; FileSelection builder), `tests/it_drive_grouping.rs`
  (test_device FileSelection builder)
- `.claude/plans/sunnypilot-dashdown-master-plan.md` (reconcile FileSelection → toggle model)

## Tests (`it_download.rs` unless noted)

- **`FileSelection::includes`** unit (model): previews→qcamera only; an all-on set → every downloadable
  kind; LockMarker→false; CSV `as_str`/`parse` round-trip + legacy preset parse.
- **`download_file` retry** (wiremock): truncated-then-full (short body once via `up_to_n_times(1)`+
  priority, full thereafter) → `Complete`, committed file full size, no `.part`; always-short →
  `Err`, **no** committed final, `.part` left; 404 → `Err(NotFound)` after exactly 1 request (no retry).
- **`download_file` cancel** (wiremock `set_delay(30s)`): spawn, `cancel()`, returns `Canceled` well
  under 30s, no committed final.
- **`sync_now`** (mock-copyparty): device → `srv.addr()`; assert returned/persisted drives match
  `single_drive` (1 drive, 3 segs).
- **`it_download` happy path** (mock-copyparty): `sync_now` then `download_drive` with a recording
  `ProgressSink` (Arc<Mutex<…>>); assert every selected file committed at exact fixture sizes
  (1200/300/100/7600/7600), `drive.sync_state==Complete`, `download_job.state=='complete'` &
  `files_done==files_total`, `on_completed` fired, `files_done` monotonic.
- **previews-only filter** (mock-copyparty): selection = qcamera only → only `qcamera.ts` on disk,
  cameras/logs absent, `files_total==3`.
- **engine cancel mid-download** (wiremock + delay): seed index directly (`upsert_segments`/
  `replace_drives`); spawn `download_drive`; `cancel()`; assert `Canceled`, no committed final for the
  in-flight file, `download_job.state=='canceled'`, drive not `Complete`.

## Verification (gates)

`cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace`.
CI already runs these + Android cross-compile + bindgen; `tokio-util`/`tokio fs` are std-backed and
cross-compile cleanly — no CI change.

## Implementation order (TDD)

1. Rename plan → `m4-sync-download-engine.md`; branch `phase-a/m4-sync-download-engine`.
2. Migration v3 (`schema_job.sql` + array); bump `it_db` schema_version 2→3. Run `it_db`.
3. Replace `FileSelection` with the toggle struct (+ `includes`/`as_str`/`parse`/`Default`) + unit
   tests; fix `db` insert/raw, `it_db`/`it_drive_grouping` device builders. Build green.
4. Repo methods (`set_file_complete`, job CRUD, `RawJob`/`JobState`) + a round-trip test.
5. `Cargo` +`tokio-util`; `download_job.rs` (`ProgressSink`/`DownloadProgress`/outcomes/`download_file`)
   + wiremock retry & cancel tests (isolated, before the engine).
6. `sync_engine/mod.rs` (`SyncEngine`/`SyncHandle`/`sync_now`/`download_drive`/`start_drive_download`/
   `db` helper); wire `pub mod sync_engine;`.
7. `it_download.rs`: sync_now, happy path, previews-only, engine cancel.
8. Reconcile master plan (FileSelection). Gates green → PR → CI green → squash-merge to `main`.
