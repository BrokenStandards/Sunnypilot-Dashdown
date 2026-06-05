# M3 — Storage Mirror

> **Filename note:** rename this file to `m3-storage-mirror.md` as the first implementation step
> (plan mode auto-named it), matching the `m0-*`/`m1-*`/`m2-*` convention.

## Context

M1 reads a device's segments over copyparty; M2 groups them into drives. We still can't **store**
footage locally or read it back **offline**. M3 builds the mirror: a local tree that matches
copyparty's layout exactly, written **crash-safely** (`<file>.part` → fsync → atomic rename), plus a
**scan** that reconstructs `Vec<Segment>` from disk and an offline `group_local` that reuses M2's
`group_segments` verbatim. The headline guarantee: **offline grouping == online grouping** for the
same tree. This `MirrorStore`/`PartFile` write primitive is what the M4 download engine streams into.

> Store files mirroring copyparty's layout (raw files, not zips); browse them offline, grouped per
> drive by the same gap rule. Download to `<file>.part` → fsync → atomic rename to `<file>`.
> (master plan)

## Scope

**In:** `storage::paths` (rel ↔ (segment,file) mapping + safe-join), `storage::MirrorStore`/`PartFile`
(atomic `.part` write), `storage::scan::scan_segments` (local twin of `list_segments`),
`drive_grouping::local::group_local`; unit tests + `it_offline_grouping`.
**Out (later):** partial/resume *classification*, `seg_file.local_size`/`download_state`
reconciliation, and any DB writes from a scan → **M5**. Deletion/retention → **M6**. Per-device mirror
subdir selection + `list_drives(offline)` wiring → **M8**. Byte-range resume → re-verified in **M5**.

## Settled design decisions

- **Reuse `Segment` for the scan output (no new `SegInfo` type).** Lets `group_segments` be shared
  verbatim and makes the offline==online test an exact `assert_eq!` on `Vec<Drive>`. `SegmentFile.remote_size`
  carries the **on-disk length** in the local-scan context (for a complete mirror local == remote; the
  real local/remote split is the M5 DB `local_size` column). Documented on `scan_segments`.
- **Crash-safe write order** (`PartFile::commit`): `flush()` → `file.sync_all()` (fsync data) → drop
  handle → `tokio::fs::rename(part, final)` → **fsync parent dir, best-effort** (non-fatal: log at
  debug and proceed — the data fsync is the load-bearing guarantee; dir fsync isn't portable on all
  filesystems). `create_part` opens the `.part` with `create+write+truncate` (no pre-remove; M3 always
  re-fetches from 0 — resume is M5). A failed commit may leave a `.part` — intentional and benign
  (scan ignores `.part`; M5 re-fetches).
- **`scan_segments` mirrors `list_segments` field-for-field:** two-level `std::fs::read_dir`; keep
  subdirs that `SegmentName::parse`; **skip `*.part`**; `rlog.lock` → `recording=true` (not listed);
  `mtime_s = modified().duration_since(UNIX_EPOCH).as_secs() as i64` (identical to the mock server's
  `ts`), `remote_size = len()`; **sort files by name**, sort segments by (route,num).
- **Sync scan, async writes.** `scan_segments`/`group_local` are sync `fn` (filesystem walk; callers
  `spawn_blocking` later, per the DB convention). `MirrorStore`/`PartFile` are async (`tokio::fs`)
  because they sit on the awaited download-streaming path.
- **`MirrorStore` is device-agnostic** — operates on copyparty-relative paths against one `root`;
  per-device subdir is M8's concern.
- **No DB writes in M3.** `group_local` returns drives in-memory; persisting offline drives would
  clobber remote-derived `sync_state`, and `local_size`/`download_state` is M5.
- **`is_complete(rel)` = "final file exists"** (ignore a stray `.part`; mismatch logic is M5).

## API (new module `storage/`, add `pub mod storage;` to `lib.rs`)

`storage/paths.rs` (free fns; `realdata_rel` is the only "config", already threaded as `&str`):
```rust
pub fn file_rel(realdata_rel: &str, seg: &SegmentName, file_name: &str) -> String; // "realdata/<seg>/<file>"
pub fn parse_file_rel(realdata_rel: &str, rel: &str) -> Option<(SegmentName, String)>; // round-trip inverse
pub(crate) fn safe_join(root: &Path, rel: &str) -> Option<PathBuf>; // rejects `..` (mirror mock-copyparty)
```

`storage/mod.rs`:
```rust
pub struct MirrorStore { root: PathBuf }
impl MirrorStore {
    pub fn new(root: impl Into<PathBuf>) -> Self;
    pub fn root(&self) -> &Path;
    pub fn final_path(&self, rel: &str) -> Result<PathBuf>;   // safe_join or Err(Io traversal)
    pub fn part_path(&self, rel: &str) -> Result<PathBuf>;    // final + ".part" (OsString push)
    pub fn is_complete(&self, rel: &str) -> bool;             // final exists
    pub fn local_size(&self, rel: &str) -> Option<u64>;
    pub async fn create_part(&self, rel: &str) -> Result<PartFile>; // create_dir_all(parent) + open .part
    pub async fn write_all(&self, rel: &str, bytes: &[u8]) -> Result<()>; // create_part+write+commit (tests/M4)
}
pub struct PartFile { /* file, part, final_ */ }
impl PartFile {
    pub fn writer(&mut self) -> &mut tokio::fs::File;  // client.download_to(rel, pf.writer()).await?
    pub async fn commit(self) -> Result<()>;           // flush→sync_all→rename→dir fsync(best-effort)
    pub async fn abort(self) -> Result<()>;            // remove .part (ignore NotFound)
}
```

`storage/scan.rs`:
```rust
/// Local twin of CopypartyClient::list_segments. `remote_size` holds on-disk len.
/// Skips `*.part` and `rlog.lock` (→recording). Sync; caller spawn_blocks.
pub fn scan_segments(realdata_dir: &Path) -> Result<Vec<Segment>>;
```

`drive_grouping/local.rs` (add `pub mod local;` beside `pub mod remote;`):
```rust
/// Offline mirror of `group_remote`.
pub fn group_local(realdata_dir: &Path) -> Result<Vec<Drive>> { Ok(group_segments(scan_segments(realdata_dir)?)) }
```

## Files changed

- new `rust/core/src/storage/{mod,paths,scan}.rs`, `rust/core/src/drive_grouping/local.rs`,
  `rust/core/tests/it_offline_grouping.rs`
- `rust/core/src/lib.rs` (+`pub mod storage;`)
- `rust/core/src/drive_grouping/mod.rs` (+`pub mod local;`; doc note local is now implemented)
- `rust/core/Cargo.toml` — `tokio = { workspace = true, features = ["io-util", "fs"] }`

## Tests

- **`paths`** unit: `parse_file_rel(rr, file_rel(rr, &seg, f)) == Some((seg, f))` for on-disk
  (`000001a3--c20ba54385--0`) and legacy `dongleid|ts--N` names; `None` on prefix mismatch/`..`;
  `safe_join` rejects `..`, accepts nested rels.
- **`MirrorStore`** unit (tempfile): atomic write — pre-commit final ABSENT & `.part` PRESENT; post-commit
  final PRESENT w/ exact bytes & `.part` ABSENT; `is_complete`/`local_size` correct; `create_part` creates
  missing segment dirs; drop-without-commit & `abort` leave `.part`/no final; `write_all` commits;
  `create_part("../escape")` → `Err`.
- **`scan`** unit (tempfile tree): 2-segment tree → 2 segments; **`.part` excluded**; `rlog.lock` →
  recording, not listed; files sorted; `remote_size`/`mtime_s` correct.
- **`group_local`** unit: empty dir → `Ok(vec![])`; missing dir → `Err(Io)`.
- **`it_offline_grouping.rs`** (headline): for `single_drive`/`gap_split`/`partial` — capture
  `realdata_dir = fixture.path().join("realdata")` **before** `MockServer::spawn` consumes the fixture,
  then `assert_eq!(group_remote(&client, "realdata/").await?, group_local(&realdata_dir)?)` (full
  `Vec<Drive>` equality — same inodes ⇒ identical `sz`/`as_secs()` mtime ⇒ identical drives). **`.part`
  case is scan-only (NO server):** build an inline tree, drop `qcamera.ts.part`, assert `group_local`
  equals the same tree without it. (The mock server lists `.part` and the client keeps it as
  `FileKind::Other`, so a remote==local assertion would fail for a non-product reason.)

## Verification (gates)

`cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace`.
CI already runs these + Android cross-compile + bindgen; `tokio/fs` is std-backed and cross-compiles
cleanly — no CI change.

## Implementation order (TDD)

1. Rename plan → `m3-storage-mirror.md`; branch `phase-a/m3-storage-mirror`.
2. Cargo: add `fs` to core `tokio` features (compile gate).
3. `storage/paths.rs` + unit tests.
4. `storage/mod.rs` (`MirrorStore`/`PartFile`) + unit tests (depends on `safe_join`).
5. `storage/scan.rs` + unit tests; wire `pub mod storage;` into `lib.rs`.
6. `drive_grouping/local.rs` + `pub mod local;` + small unit test.
7. `tests/it_offline_grouping.rs` (3 fixtures full-equality + scan-only `.part` case).
8. Gates green → PR → CI green → squash-merge to `main`.
