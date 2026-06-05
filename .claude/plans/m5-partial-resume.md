# M5 — Partial / Resume (byte-range)

> **Filename note:** rename this file to `m5-partial-resume.md` as the first implementation step.

## Context

M4 downloads drives but only knows "Complete vs not" and re-fetches whole files. M5 adds the
**resume layer**: classify each file's local state, compute a drive's `Partial`/`Complete`/
`NotDownloaded` status from disk, recover after a crash (stale `running` jobs + `Downloading` drives),
and — per the maintainer's decision — **resume interrupted downloads byte-for-byte via HTTP Range**
instead of re-fetching the whole file (a 76 MB camera file interrupted at 90% resumes the last ~7 MB).
M1 found copyparty's current source honors Range (206/Content-Range); M5 re-verifies it and relies on
it, with an automatic **file-granular fallback** when a server ignores Range (responds 200).

## Scope

**In:** `resume::classify_file` + `resume::drive_status`; `SyncEngine::reconcile_device` (+ folded into
`sync_now`); **byte-range resume** in `download_file` (resume from `.part` offset, 206→append /
200→restart) with `MirrorStore::part_size`/`open_part_append` and `CopypartyClient::fetch`;
`CopypartyClient::probe_range` (Range verification); tests.
**Out (later):** retention/auto-delete (**M6**); connectivity dot (**M7**); UniFFI `#[export]` +
per-file DB `download_state` surfacing (**M8**). No new deps, **no DB migration** (DownloadState/
SyncStatus/JobState already exist; `download_job` v3 already reserves stale-`running` recovery for M5).

## Settled design decisions (incl. Plan-agent refinements)

- **`resume::classify_file(mirror, rel, remote_size) -> DownloadState`** — final-first, infallible
  (bad path → `Missing`, logged): final present → `Complete` if `local_size == Some(remote_size)` else
  `SizeMismatch`; final absent → `.part` exists (`part_path(rel)?.exists()`) → `InProgress`, else
  `Missing`. (`local_size == None` with a final present falls to `SizeMismatch` — safe.)
- **`resume::drive_status(mirror, drive, selection, realdata_rel) -> SyncStatus`** — classify every
  *selected* file (`selection.includes(kind)`, rel via `file_rel`); **all Complete (≥1) → Complete;
  all exactly Missing → NotDownloaded; otherwise → Partial.** Empty selection → `Complete` (nothing
  outstanding). Returns only those three (never Downloading/Failed — those are job-lifecycle states).
  The "later contiguous remote segment missing → Partial" case needs **no separate check**: `sync_now`
  refreshes the index so a grown drive includes the new segment whose files classify `Missing` → mixed
  → `Partial`.
- **`SyncEngine::reconcile_device(device) -> Result<Vec<Drive>>`** — recompute each drive's
  `sync_state` from disk and persist; **restart recovery**: for any `download_job` with
  `state == Running` (a fresh process can't have a live job), move it to a terminal state — `Complete`
  if the drive is now Complete, else `Failed` with `error = resume::INTERRUPTED` ("interrupted"). Only
  touch `Running` jobs (never resurrect a `Canceled`/`Failed`). Offline-capable (index + disk, no
  network). **All work in ONE `db()` spawn_blocking closure** (build `MirrorStore` inside it — it's not
  `Clone`; `mirror_root` is); avoid per-drive `db()` awaits (the in-memory test pool is `max_size=1`).
- **Fold reconcile into `sync_now`** via a shared private `reconcile` that classifies + persists over a
  given drive set, run inside `sync_now`'s existing single blocking closure (one hop). `reconcile_device`
  is the standalone offline/restart entry that does `get_drives` then the same helper.
- **`download_drive` pre-credit** refactors its inline `is_complete && size` check to
  `resume::classify_file(...) == DownloadState::Complete` (single-source "Complete"); no behavior change
  — existing `it_download.rs` is the regression guard.

### Byte-range resume

- **`CopypartyClient::fetch(rel, range_from: Option<u64>) -> Result<Fetch>`** — GET with `apply_auth`;
  when `Some(start)`, add `Range: bytes={start}-`; `check_status` (206 and 200 both succeed); returns
  `Fetch { resp }`. `Fetch::partial(&self) -> bool` (`status == 206`); `Fetch::stream_to(self, &mut W)
  -> Result<u64>` (wraps `download::stream_to_writer`). Import `reqwest::header::RANGE` /
  `reqwest::StatusCode` directly (no re-export). Keep `download_to`/`download` (reimplement
  `download_to` as `fetch(rel, None)?.stream_to(w)` or leave as-is).
- **`MirrorStore`**: `part_size(rel) -> Option<u64>` (size of an existing `.part`); `open_part_append(rel)
  -> Result<PartFile>` (`create(true).append(true)` — writes go to end, existing bytes preserved).
  `create_part` (truncate) stays for fresh/restart. `PartFile::commit` is unchanged (fsync + rename
  works regardless of how the file was opened).
- **`download_file` rewrite** (same signature) — the attempt, raced against `cancel.cancelled()` in a
  `biased` `select!` so a slow `fetch`/stream is cancellable (dropping the future leaves the `.part`
  for the *next* resume):
  ```
  let existing = mirror.part_size(rel).unwrap_or(0);
  let resume_from = if existing > 0 && existing < remote_size { existing } else { 0 }; // shrunk/stale → 0
  let fetch = client.fetch(rel, (resume_from > 0).then_some(resume_from)).await?;
  let (mut pf, base) = if resume_from > 0 && fetch.partial() {
      (mirror.open_part_append(rel).await?, resume_from)   // 206: append the tail
  } else {
      (mirror.create_part(rel).await?, 0)                  // fresh, or 200 (server ignored Range): restart
  };
  let written = fetch.stream_to(pf.writer()).await?;
  let total = base + written;
  if total == remote_size { pf.commit().await?; Complete } else { /* drop pf; retry (offset recomputes) */ }
  ```
  Retry policy unchanged (transport/IO retriable; 401/403/404 fail-fast; exhausted size-mismatch →
  `Err`). Cancellation leaves a partial `.part` → progressive resume across cancels/restarts.
- **`probe_range(rel) -> Result<bool>`** — `Range: bytes=0-0`; `Ok(status == PARTIAL_CONTENT)`;
  transport errors propagate. Verification + diagnostics; the download path is self-correcting via
  206/200 so it doesn't gate runtime behavior.

## Files changed

- new `rust/core/src/sync_engine/resume.rs` (`classify_file`, `drive_status`, `INTERRUPTED`, tests)
- `rust/core/src/sync_engine/mod.rs` (+`pub mod resume;`, `reconcile_device` + private `reconcile`,
  fold into `sync_now`, pre-credit → `classify_file`)
- `rust/core/src/sync_engine/download_job.rs` (rewrite `download_file` for byte-range)
- `rust/core/src/copyparty_client/mod.rs` (+`fetch`/`Fetch`, +`probe_range`)
- `rust/core/src/storage/mod.rs` (+`part_size`, +`open_part_append`)
- new `rust/core/tests/it_resume.rs`; `rust/core/tests/it_real_copyparty.rs` (extract
  `boot_copyparty`, add Range + byte-range-resume tests)
- `.claude/plans/sunnypilot-dashdown-master-plan.md` (reconcile resume algorithm → byte-range + fallback)
- No `Cargo.toml` / migration / CI changes.

## Tests

- **`resume.rs` unit**: `classify_file` (missing / `.part`-only→InProgress / final-correct→Complete /
  wrong-size→SizeMismatch); `drive_status` (none→NotDownloaded / all→Complete / some-missing→Partial /
  wrong-size→Partial / empty-selection→Complete) — in-memory `Drive` + tempfile `MirrorStore`.
- **byte-range `download_file`** (wiremock, in `it_resume.rs`): **206 resume** — pre-place a `.part`
  of N bytes, stub matches `Range: bytes=N-` → 206 tail; assert final == `remote_size`. **200 fallback**
  — `.part` of N bytes, server returns 200 full (ignores Range) → `.part` truncated/restarted; final ==
  `remote_size`. (M4's `download_file` tests still pass: full-from-scratch, always-short→err, 404
  no-retry, cancel.)
- **`it_resume.rs` integration** (mock-copyparty): **resume-only-missing** — full download → delete 2
  distinct-kind files in ONE segment → `reconcile_device`→Partial → re-download → progress events with
  `current_file=Some` are exactly those 2 (first event `None`, `files_done==total-2`), all present
  after. **size-mismatch** — overwrite a file with wrong-size bytes → reconcile→Partial,
  `classify_file`==SizeMismatch → re-download corrects it. **later-contiguous** — capture realdata path
  before spawn; download 0..2 → write seg `--3` on disk → `sync_now`→Partial (key unchanged) →
  download fetches only seg 3 → Complete. **restart-recovery** — full → simulate crash
  (`set_drive_sync_state(Downloading)` + `upsert_job(running)` + delete a file) → `reconcile_device` →
  drive Partial + job terminal with `error==Some("interrupted")` → resume fetches only the missing file.
- **`it_real_copyparty.rs`**: factor `boot_copyparty(fixture) -> Option<(Killer, String)>`;
  `supports_byte_range` → `probe_range(...) == true`; **byte-range e2e** — download a file's first 500
  bytes into a `.part`, then `download_file` against real copyparty resumes via 206; assert the
  committed file's bytes equal the full file (authoritative correctness, not just size).

## Verification (gates)

`cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace`
(confirm `schema_version()` stays **3** — no migration). CI runs these + Android cross-compile +
bindgen; all std/reqwest-backed — no CI change.

## Implementation order (TDD)

1. Rename plan → `m5-partial-resume.md`; branch `phase-a/m5-partial-resume`.
2. `resume.rs` `classify_file` + unit tests (tempfile MirrorStore) → implement.
3. `resume.rs` `drive_status` + unit tests → implement.
4. `MirrorStore::part_size`/`open_part_append`; `CopypartyClient::fetch`/`Fetch`/`probe_range`.
5. Rewrite `download_file` for byte-range; run M4 `it_download` tests (regression) + add wiremock
   206-resume / 200-fallback tests.
6. `reconcile` helper + `reconcile_device`; fold into `sync_now`; refactor `download_drive` pre-credit
   to `classify_file`.
7. `it_resume.rs` (resume-only-missing, size-mismatch, later-contiguous, restart-recovery).
8. `it_real_copyparty.rs`: `boot_copyparty` + Range probe + byte-range e2e.
9. Reconcile master plan. Gates green → PR → CI green → squash-merge to `main`.
