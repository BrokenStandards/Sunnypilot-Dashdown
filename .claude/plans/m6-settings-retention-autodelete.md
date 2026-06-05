# M6 — Settings + Retention + Auto-Delete

## Context

Phase A milestones M0–M5 are merged: the Rust core can index a device, group drives,
mirror files locally, and download/resume them crash-safely. M6 is the last *storage-management*
milestone before the UniFFI surface (M8): it makes the per-device settings that were merely
**stored** since M1 actually **do** something.

Two distinct deletions land here:

1. **Local retention pruning** — when locally-mirrored footage exceeds the device's
   `retention_max_minutes` budget, prune the oldest drives first (keep newest), **skipping
   `preserved`**, freeing space *on the phone*. The remote is untouched.
2. **Auto-delete-from-comma** — once a drive is fully copied and old enough, delete its footage
   *from the Comma device* to reclaim its (small) disk, behind three safety guards. The local
   copy is kept.

The master plan flags copyparty's DELETE endpoint as a **verification task**: it must be
confirmed against a real copyparty instance and exercised by tests. That verification is done
(see below) — WebDAV `DELETE` is confirmed.

**Decision (user, this session): auto-delete removes _whole segment directories_ recursively**
(all streams, including ones never downloaded), maximizing reclaimed device space — not just the
selected streams. The "drive is Complete for the active selection" guard still gates the delete;
the user accepts losing un-selected streams from the device once the selection is safely mirrored.

Out of scope (do **not** pull forward): M7 (connectivity dots), M8 (UniFFI/`AppCore`). No
scheduling — these are explicit engine methods the Phase-B native layer will trigger.

## Verified facts (grounding)

- **copyparty DELETE** (from `ref/copyparty`): WebDAV `DELETE /vpath` → `handle_rm`, returns
  **200 OK** plain-text body, **recursive** for directories. Needs the volume `d` flag
  (`-v PATH:/:rwd`; we currently boot tests `:r`). Auth via `PW:`; 401 (no auth) / 403 (no perm
  or `--no-del`) / 404 (missing). A `POST /vpath?delete` JSON-batch variant also exists — not used.
- **No schema migration** — all columns already exist: `device.{retention_max_minutes:Option<i64>,
  auto_delete_from_comma:bool, auto_delete_min_age_min:i64}`, `drive.{preserved, end_ms:Option<i64>(ms),
  sync_state, recording, segment_count}`. `migrations.rs` stays `LATEST_VERSION = 3`.
- **Classification is purely disk-based** (`resume::classify_file` only stats the FS; `seg_file.local_size`
  is never read for classification). So deleting local files + `reconcile` reclassifies to
  `NotDownloaded` with zero DB-row cleanup. Confirmed at [resume.rs:19](rust/core/src/sync_engine/resume.rs#L19).
- **`replace_drives`** ([db/mod.rs:219](rust/core/src/db/mod.rs#L219)) currently prunes *any* drive_key
  absent from the new listing (both an empty-set wipe-all branch and a `NOT IN (...)` branch). Its
  INSERT touches only derived columns; `ON CONFLICT` leaves `preserved`/`sync_state` intact.
- **`resume::drive_status`** ([resume.rs:43](rust/core/src/sync_engine/resume.rs#L43)) aggregates a
  drive against a selection from disk → reused verbatim for the remote re-verify guard.
- **mock-copyparty** ([lib.rs:80](rust/mock-copyparty/src/lib.rs#L80)) is a live `.fallback(handle)`
  server reading a real temp dir; `safe_join` + `check_auth` reusable; no DELETE handler yet.

## Design

### Index preservation (the key correctness point)
Auto-delete removes a drive's remote files while **keeping the local copy**, so the drive must
stay in the library after the next `sync_now` re-lists a now-empty remote. Refine
`replace_drives` pruning to **keep any drive with local data** — prune only purely-remote,
unpinned, vanished drives:

```sql
-- both the empty-set branch AND the NOT IN (...) branch gain this guard:
... AND preserved = 0 AND sync_state IN ('not_downloaded','failed')
```

This keeps a comma-deleted drive (`sync_state='complete'`) visible **without** touching the
user's `preserved` pin — so local retention can still reclaim it later. (Deliberately **not**
auto-setting `preserved=true`, which would permanently exempt the drive from local pruning and
risk filling the phone.) `upsert_segments` needs no change (it only inserts/updates; segment rows
persist, so `get_drives` keeps hydrating the kept drive's segments).

### Pure logic — new `rust/core/src/sync_engine/retention.rs`
```rust
/// Drives to prune to fit `budget_minutes` of local data. None ⇒ never prune.
/// Considers only Complete|Partial drives; sorts newest-first (end_ms desc,
/// then start_ms, last_segment_num, drive_key for a total order); accumulates
/// segment_count as the minute cost; once the running total exceeds budget,
/// every older NON-preserved drive is pruned. Preserved drives consume budget
/// (they occupy disk) but are never returned. Pure + unit-testable.
pub fn plan_prune(drives: &[Drive], budget_minutes: Option<i64>) -> Vec<String>;

/// Guard: Complete && !recording && end_ms ≥ min_age old. now_ms/min_age in ms.
pub fn eligible_for_remote_delete(drive: &Drive, now_ms: i64, min_age_min: i64) -> bool;

/// Remote paths to delete for one drive: the WHOLE segment directories
/// ("<realdata_rel><seg.dir_name()>/") — recursive on the server. (User choice.)
fn remote_delete_targets(drive: &Drive, realdata_rel: &str) -> Vec<String>;
```
Wire `mod retention;` into [sync_engine/mod.rs](rust/core/src/sync_engine/mod.rs#L6) beside `resume`.

### Engine methods — `rust/core/src/sync_engine/mod.rs`
Standalone + explicitly callable (Phase B schedules them); reuse `client_for`, `mirror_for`,
`reconcile`, the `db()` helper, `group_segments`, `resume::drive_status`.

```rust
/// Prune local drives beyond retention_max_minutes (newest-first, skip preserved).
/// Deletes local files only; remote untouched. Reconciles so pruned drives
/// reclassify NotDownloaded. Returns pruned drive_keys.
pub async fn enforce_retention(&self, device: &Device) -> Result<Vec<String>>;

/// Delete eligible drives' footage from the comma (whole segment dirs), keeping
/// the local copy. Per drive: guard (Complete+old+!recording) → fresh re-verify
/// still Complete remotely → delete each segment dir. Returns deleted drive_keys.
pub async fn auto_delete_from_comma(&self, device: &Device, now_ms: i64) -> Result<Vec<String>>;

/// Convenience for Phase B: enforce_retention then auto_delete_from_comma.
pub async fn run_maintenance(&self, device: &Device, now_ms: i64) -> Result<()>;
```

- **`enforce_retention`**: `get_drives` (one `db()` checkout) → `plan_prune` (pure) → for each
  pruned key, `mirror.remove_dir(seg_dir)` for its segments (FS ops, outside any db closure) →
  `reconcile` (its own single closure). Returns pruned keys. No-op when budget is `None`.
- **`auto_delete_from_comma`**: no-op unless `device.auto_delete_from_comma`. `get_drives` →
  filter by `eligible_for_remote_delete(.., now_ms, device.auto_delete_min_age_min)` → one fresh
  `client.list_segments(REALDATA_REL)` + `group_segments` → per candidate: require it's still in
  the fresh listing, `!fresh.recording`, and `drive_status(&mirror, &fresh, &selection, REALDATA_REL)
  == Complete` (re-verify; a grown drive → Partial → skipped) → `client.delete(seg_dir)` for each
  segment dir. Local mirror untouched → `sync_state` stays Complete → drive kept by the
  `replace_drives` refinement. `now_ms` injected (tests fixed; native passes `SystemTime` epoch-ms —
  allowed in core Rust).
- **spawn_blocking discipline** (in-memory pool `max_size=1`): each Repo call in its own `db()`
  closure; FS deletes outside closures; never nest checkouts (mirror `reconcile`'s pattern).

### Client — `rust/core/src/copyparty_client/mod.rs`
```rust
/// Delete a remote path (file or directory; server-recursive) via WebDAV DELETE.
/// 200/204 ⇒ Ok; 404 ⇒ Ok (idempotent, already gone); 401⇒AuthRequired,
/// 403⇒Forbidden, else Http. Requires the volume's `d` permission.
pub async fn delete(&self, rel: &str) -> Result<()>;
```
`auth::apply_auth(self.http.delete(self.url_for(rel)?), &creds).send()` + a local status match
(don't reuse `check_status` — 404 must be success here). Update the module header note ("DELETE
lands in M6"). No `auth.rs` change.

### Local removal — `rust/core/src/storage/mod.rs`
```rust
pub async fn remove_file(&self, rel: &str) -> Result<()>;     // final + stray .part; NotFound⇒Ok
pub async fn remove_dir(&self, rel_dir: &str) -> Result<()>;  // remove_dir_all; NotFound⇒Ok; safe_join
```
Reuse `final_path`/`part_path`/`safe_join`; mirror `PartFile::abort`'s NotFound-is-Ok pattern.

### Mock DELETE — `rust/mock-copyparty/src/lib.rs`
Add `method: axum::http::Method` to `handle`'s extractors (valid with `.fallback`). Before the
ls/GET logic, branch: `if method == Method::DELETE { remove_dir_all|remove_file on target;
Ok⇒200 plain-text, NotFound⇒404, else⇒500 }`. `check_auth` already runs first. Update the doc
comment. Lets `it_retention` assert files actually vanish via a follow-up list/GET.

## Files

- **edit** [rust/core/src/db/mod.rs](rust/core/src/db/mod.rs) — `replace_drives` prune guard (both branches) + doc.
- **edit** [rust/core/src/copyparty_client/mod.rs](rust/core/src/copyparty_client/mod.rs) — `delete`.
- **edit** [rust/core/src/storage/mod.rs](rust/core/src/storage/mod.rs) — `remove_file`, `remove_dir` (+ unit tests).
- **edit** [rust/core/src/sync_engine/mod.rs](rust/core/src/sync_engine/mod.rs) — `mod retention;`, three methods.
- **new**  rust/core/src/sync_engine/retention.rs — pure planner/guard/targets (+ unit tests).
- **edit** [rust/mock-copyparty/src/lib.rs](rust/mock-copyparty/src/lib.rs) — DELETE handler.
- **new**  rust/core/tests/it_retention.rs — integration tests.
- **edit** [rust/core/tests/it_real_copyparty.rs](rust/core/tests/it_real_copyparty.rs) — writable boot + real DELETE verify.

## Verification (TDD — write tests first, then code)

**Unit — `retention.rs`:**
- `plan_prune`: None⇒empty; under-budget⇒empty; over-budget prunes oldest, keeps newest; boundary
  (total == budget keeps all; +1 over prunes oldest); preserved skipped but consumes budget;
  only Complete/Partial considered; deterministic tie-break.
- `eligible_for_remote_delete`: age `==` min_age ⇒ eligible (`>=`); 1ms under ⇒ no; recording ⇒ no;
  non-Complete ⇒ no; `end_ms None` ⇒ no.

**Unit — `storage`:** `remove_file` deletes final+part, idempotent on missing; `remove_dir`
recursive + idempotent; both reject `..`.

**Integration — `it_retention.rs`** (mock-copyparty with DELETE; reuse the `setup()`/`device_at`
conventions from [it_resume.rs](rust/core/tests/it_resume.rs) / [it_download.rs](rust/core/tests/it_download.rs)):
- (a) **retention prune**: download two drives; budget below combined cost; `enforce_retention`
  deletes the oldest drive's local files (assert `!is_complete`), keeps newest, keeps a
  `preserved` old drive even over budget; pruned drive reclassifies `NotDownloaded`.
- (b) **auto-delete happy path**: download a drive; `now_ms` past the age guard;
  `auto_delete_from_comma` deletes its segment dirs from the mock (assert a follow-up
  `list_segments` shows them gone) while the local mirror remains; a follow-up `sync_now`
  (remote now empty) leaves the drive **still present + Complete** in `get_drives` (proves the
  `replace_drives` refinement, with `preserved` untouched).
- (c) **age guard**: a too-recent drive (`now_ms - end_ms < min_age*60_000`) is NOT deleted.
- (d) **not-Complete guard**: a `Partial` drive (delete one local file + reconcile) is NOT deleted.

**Integration — `it_real_copyparty.rs`** (skips if copyparty absent): add `boot_copyparty_writable`
(`-v PATH:/:rwd`); `real_copyparty_delete_removes_segment_dir` — boot writable, `client.delete`
a segment directory, assert Ok (real **200**) and a re-list no longer shows the segment —
authoritatively verifying the real DELETE endpoint, recursive dir delete, and status.

**Gates:** `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`. Then commit, PR, watch CI (build/test/cross-compile/bindgen +
claude-review), squash-merge to `main`. Rename this plan file to
`.claude/plans/m6-settings-retention-autodelete.md` during implementation.

## Risks / verify during implementation

1. **Kept-drive hydration**: after (b)'s second `sync_now`, the kept drive's `segment`/`seg_file`
   rows must persist (upsert never deletes) so `get_drives` hydrates non-empty segments. Confirm.
2. **`replace_drives` empty-branch parity**: `empty_input_clears_device_drives` and
   `regroup_preserves_user_flags_and_prunes_orphans` must stay green (their orphans are
   `NotDownloaded`/unpinned → still pruned). Verify both.
3. **axum `Method` extractor** composes with `.fallback(handle)`; GET/ls paths unaffected.
4. **ms units**: `Drive.end_ms`, injected `now_ms`, and `min_age*60_000` all in epoch-ms.
5. **Re-verify under current selection**: guard uses `device.file_selection` (current), so a
   grown/changed drive can't slip through. Whole-dir delete still only fires when the *selection*
   is Complete — un-selected streams are intentionally discarded per the user's choice.
6. **Dir-path DELETE**: confirm copyparty + mock accept the segment-dir path (trailing slash
   handled by `safe_join`); the real test asserts recursive dir removal end-to-end.
