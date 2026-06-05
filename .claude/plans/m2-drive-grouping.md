# M2 — Drive Grouping

> **Filename note:** rename this file to `m2-drive-grouping.md` as the first implementation step
> (plan mode only let me create it under the auto-generated name), matching the `m0-*`/`m1-*` convention.

## Context

M1 gave us a copyparty client that lists a device's `realdata/` into `Vec<Segment>` (sorted by
`(route_id, segment_num)`, each with files + a `recording` flag), a domain model, and a SQLite
index (`device`/`segment`/`seg_file`). What we **cannot** yet do is the product's central concept:
group those flat 1-minute segments into **drives**.

> A *drive* = a maximal run of consecutive 1-minute segments with no time gap; a gap splits into a
> separate drive. (master plan, product requirements)

M2 builds the shared, pure `group_segments` function (the master plan's key algorithm), the `Drive`
model, the **remote** path that wires `list_segments → group_segments`, and — per the locked scope
decision — persists drives in a new `drive` table (migration v2) so M3 (offline grouping) and M8
(`list_drives`) read from a stable source. The same `group_segments` is reused by M3's offline mirror
scan (`drive_grouping/local.rs`, not built here). Outcome: segments → drives, end-to-end and tested.

## Scope

**In:** pure `group_segments`; `Drive` + `SyncStatus` model types; `drive_grouping::remote::group_remote`;
migration **v2** (`drive` table); `Repo::replace_drives` / `get_drives`; table-driven + proptest +
integration + persistence tests.
**Out (later milestones):** offline/local mirror grouping (`local.rs`, M3); populating `sync_state`
(M5); `preserved` behavior + retention/auto-delete and `drive_key` re-keying on deletion (M6); the
UniFFI `Drive` record (M8). `sync_state`/`preserved` are **stored now, behavior lands later** — the
same accepted pattern as M1's `device` retention columns.

## Settled design decisions

- **Splitter = route + index only.** Two segments continue the same drive **iff**
  `route_id == prev.route_id && segment_num == prev.segment_num + 1`. A new `route_id` or any index
  discontinuity starts a new drive. (Spec: "contiguous *iff* segment_nums consecutive".)
- **mtime is warn-only sanity, never a splitter.** When route+index say contiguous but the time gap
  is anomalous, emit `tracing::warn!` and keep them together. Helper
  `gap_is_sane(prev: Option<i64>, next: Option<i64>) -> bool`: `true` if either is `None`; else
  `(next - prev - SEGMENT_MS).abs() <= GAP_TOLERANCE_MS`. `const GAP_TOLERANCE_MS: i64 = 30_000;`
  (advisory only; 30 s absorbs sub-segment finalization lag without masking a real skipped minute).
- **Output order:** drives sorted by `(route_id, first_segment_num)` — total, always-present,
  independent of the optional time signal (keeps idempotence/order-independence clean). Segments
  within a drive stay in ascending `segment_num`.
- **`drive_key` = first segment's `dir_name()`** (e.g. `000001a3--c20ba54385--0`); stable as the
  drive grows. *Forward-ref:* if M6 deletes a drive's first segment the key drifts — handle re-keying
  in M6; out of scope here (M2 never deletes).
- **`start_ms` / `end_ms` (both `Option<i64>`, computed from first/last by `segment_num`, not mtime):**
  `start_ms = first.approx_time_ms()`; `end_ms = last.approx_time_ms().map(|t| t + SEGMENT_MS)`
  (half-open / conservative — M6's retention guard does `now - end_ms >= min_age`).
- **`recording`** on a drive = any segment recording (the last segment is the real-world case).
- **Duplicate `(route_id, segment_num)`:** dedup keep-**richest** deterministically (most files →
  `recording` wins → larger `approx_time_ms` → keep-first) so output is order-independent. Shouldn't
  occur from one `list_segments`, but the function must be total.
- **`group_segments(Vec<Segment>) -> Vec<Drive>`** is pure + infallible (segments arrive pre-parsed)
  and **takes ownership** (moves segments into drives, no clones). `Drive` owns its `Vec<Segment>`.

## Model additions — `rust/core/src/model/mod.rs`

```rust
pub enum SyncStatus { NotDownloaded, Partial, Complete, Downloading, Failed }
// as_str/parse exactly like ConnMode/DownloadState; strings: not_downloaded|partial|complete|downloading|failed

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    pub drive_key: String,        // first segment dir_name
    pub route_id: String,
    pub first_segment_num: u32,
    pub last_segment_num: u32,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub segment_count: u32,
    pub recording: bool,
    pub sync_state: SyncStatus,   // default NotDownloaded in M2 (set in M5)
    pub preserved: bool,          // default false in M2 (behavior in M6)
    pub segments: Vec<Segment>,
}
```
Derived fields must stay consistent with `segments` (only `group_segments` / DB hydration set them).

## `drive_grouping/` — `rust/core/src/drive_grouping/`

- `mod.rs`: `pub fn group_segments`, `fn gap_is_sane`, `const GAP_TOLERANCE_MS`, `pub mod remote;`,
  and `#[cfg(test)] mod tests` (unit + dedup + proptest). Doc-comment: "local mirror grouping reuses
  `group_segments` in M3."
- `remote.rs`: `pub async fn group_remote(client: &CopypartyClient, realdata_rel: &str) -> Result<Vec<Drive>>`
  = `Ok(group_segments(client.list_segments(realdata_rel).await?))`. (No DB coupling — the
  orchestrator composes `group_remote` + `replace_drives` in M4/M8.)
- Do **not** create `local.rs` yet (M3).
- `lib.rs`: add `pub mod drive_grouping;` (keep alphabetical: copyparty_client, db, drive_grouping, error, model).

Algorithm: dedup-by-key (richest) after sorting by `(route_id, segment_num)` → single walk emitting
drives on route change / index break (warn via `gap_is_sane` when contiguous-but-time-anomalous) →
finalize each drive's derived fields → sort drives by `(route_id, first_segment_num)`.

## DB additions

**Migration v2** — new `rust/core/src/db/schema_drive.sql`, appended to `migrations.rs`:
```rust
const MIGRATIONS: &[&str] = &[
    include_str!("schema.sql"),        // v1
    include_str!("schema_drive.sql"),  // v2 — drive table (M2)
];
```
```sql
CREATE TABLE IF NOT EXISTS drive (
    id            INTEGER PRIMARY KEY,
    device_id     INTEGER NOT NULL REFERENCES device(id) ON DELETE CASCADE,
    drive_key     TEXT    NOT NULL,            -- first segment dir_name
    route_id      TEXT    NOT NULL,
    first_seg     INTEGER NOT NULL,
    last_seg      INTEGER NOT NULL,
    start_ms      INTEGER,                      -- NULL when first seg has no files
    end_ms        INTEGER,                      -- NULL when last seg has no files
    segment_count INTEGER NOT NULL,
    recording     INTEGER NOT NULL DEFAULT 0,
    preserved     INTEGER NOT NULL DEFAULT 0,   -- behavior: M6
    sync_state    TEXT    NOT NULL DEFAULT 'not_downloaded', -- behavior: M5
    UNIQUE(device_id, drive_key)
);
CREATE INDEX IF NOT EXISTS idx_drive_device ON drive(device_id, route_id, first_seg);
```

**`Repo::replace_drives(device_id, &[Drive])`** (one transaction; mirrors `upsert_segments`):
1. Per drive: `INSERT … ON CONFLICT(device_id, drive_key) DO UPDATE SET` **only** the derived columns
   (route_id, first_seg, last_seg, start_ms, end_ms, segment_count, recording). **`preserved` and
   `sync_state` are intentionally NOT in the SET list** — they survive regroups.
2. Prune orphans: `DELETE FROM drive WHERE device_id = ? AND drive_key NOT IN (<placeholders>)`
   (or `DELETE … WHERE device_id = ?` when the input is empty).

**`Repo::get_drives(device_id) -> Vec<Drive>`** — `SELECT … ORDER BY route_id, first_seg`, mapped via
a `RawDrive` → `raw_to_drive` pair (so `SyncStatus::parse` surfaces `CoreError`, like `raw_to_device`).
Hydrate each drive's `segments` with a range fetch (safe — within-drive indices are contiguous by
construction): `SELECT … FROM segment WHERE device_id=? AND route_id=? AND segment_num BETWEEN ? AND ?`,
reusing the existing per-segment `seg_file` query. Factor a `get_segments_in_range(...)` helper.
Add `DRIVE_COLS` const alongside `DEVICE_COLS`.

## Files changed

- new `rust/core/src/drive_grouping/mod.rs`, `rust/core/src/drive_grouping/remote.rs`
- `rust/core/src/lib.rs` (+`pub mod drive_grouping;`)
- `rust/core/src/model/mod.rs` (+`Drive`, +`SyncStatus`)
- new `rust/core/src/db/schema_drive.sql`; `rust/core/src/db/migrations.rs` (append v2)
- `rust/core/src/db/mod.rs` (+`DRIVE_COLS`, `RawDrive`/`raw_to_drive`, `replace_drives`, `get_drives`, `get_segments_in_range`)
- `rust/core/Cargo.toml` (`cargo add --dev proptest`)
- new `rust/core/tests/it_drive_grouping.rs`
- `rust/core/tests/it_db.rs` (schema_version assertions `1 → 2`)

## Tests

**Unit (in `drive_grouping`, table-driven):** single route consecutive → 1 drive; two routes → 2;
internal gap `0,1,3` → 2 drives `[0,1]`+`[3]`; route change mid-stream; duplicate index (both input
orders → identical, richest kept); empty → empty; single segment; unordered input → same result;
no-files segment (`approx_time None`) still groups by index, `start/end` independent; `drive_key`,
`start_ms`, `end_ms`, `segment_count`, `recording` correctness; `gap_is_sane` boundary (exactly
`GAP_TOLERANCE_MS`, one over, and `None` inputs). Plus a `SyncStatus` `as_str`/`parse` round-trip.

**Proptest** (`cargo add --dev proptest`; default cases): generator builds `Vec<Segment>` from a
3-element route alphabet, `segment_num ∈ 0..8`, `mtime ∈ Option<…>` (one `qcamera.ts` file when
`Some`, none when `None`), `recording: bool`; **dedups to unique keys** in the generator so dedup
doesn't intrude on the invariants. Invariants (each its own `proptest!`):
(a) partition completeness — flattened drive segments == input as a multiset;
(b) within each drive: `route_id` constant, `segment_num` strictly `+1`, and derived fields agree
   (`segment_count == segments.len()`, `first/last_segment_num`, `route_id`, `drive_key`);
(c) idempotence — regrouping the concatenation of all drives' segments yields identical `Vec<Drive>`;
(d) order-independence — `.prop_shuffle()` the input → identical `Vec<Drive>`;
(e) every drive non-empty.
(c)/(d) rely on full `Drive: PartialEq` + the deterministic `(route_id, first_segment_num)` output sort.

**Integration `tests/it_drive_grouping.rs`** (mock-copyparty, like `it_listing.rs`):
`single_drive` → 1 drive, count 3; `gap_split` → 2 drives (counts 2+2, distinct route_ids);
`partial` → 1 drive of 2 segments with `recording == true` and sane `start_ms`/`end_ms`.

**Persistence** (same file or `it_db.rs`): `upsert_segments → replace_drives → get_drives` returns
correct summaries + hydrated segments; set `preserved=1` + non-default `sync_state`, regroup +
`replace_drives` again → flags **preserved**, derived columns **updated**; drop a route from input →
its drive is **pruned**; empty input clears the device's drives.

## Verification (gates)

`cargo fmt --check` · `cargo clippy --workspace -- -D warnings` (watch the dynamic `NOT IN`
placeholder build / `Vec<&dyn ToSql>`) · `cargo test --workspace` (unit + proptest + integration +
persistence). CI (`.github/workflows/rust-ci.yml`) already runs all of these — no CI change needed.

## Implementation order (TDD)

1. Rename plan file → `m2-drive-grouping.md`; branch `phase-a/m2-drive-grouping`.
2. Model: add `SyncStatus` + `Drive` (+ enum round-trip test). Build.
3. `drive_grouping/mod.rs`: write table-driven unit tests first, then `group_segments`/`gap_is_sane`/
   `GAP_TOLERANCE_MS` to green; wire into `lib.rs`.
4. `cargo add --dev proptest`; add generator + 5 invariants + deterministic-dedup unit test.
5. `drive_grouping/remote.rs` + `tests/it_drive_grouping.rs` (mock-server cases).
6. Persistence: `schema_drive.sql` + v2 in `migrations.rs`; bump `it_db.rs` schema_version `1→2`;
   add `RawDrive`/`raw_to_drive`/`DRIVE_COLS`/`replace_drives`/`get_drives`/`get_segments_in_range`.
7. Persistence tests (upsert→group→read; preserve-flags-on-regroup; orphan prune; empty clears).
8. Gates green → PR → CI green → squash-merge to `main` (per-milestone commit/merge).
