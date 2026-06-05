# M1 — copyparty Client, Domain Model & DB

## Context

M0 gave us a building, cross-compiling Rust core with a smoke FFI surface. **M1 builds the
first real engine layer** (master plan milestone M1): talk to a copyparty server (`?ls=j`
listing + auth + streamed download), define the domain **model** (devices, segments, files),
persist an **index** in SQLite (schema + migrations + Repo), and build the reusable
**`mock-copyparty`** axum fixture server that later milestones and UI tests reuse.

**Scope is data acquisition + persistence only.** Explicit **non-goals** (each its own
milestone): drive *grouping* (M2), storage mirror / atomic `.part` writes (M3), sync/download
engine + cancel/progress (M4), resume classification (M5), DELETE + retention (M6),
connectivity dot (M7), the UniFFI `AppCore` surface (M8). `lib.rs` keeps the M0 smoke exports;
M1's new modules are ordinary **`pub` Rust** (reachable by the `tests/` crate) but **not yet
`#[uniffi::export]`ed** — the FFI boundary is M8.

### Source-verified contract (this drives the code — corrections to the master plan below)

- **copyparty `?ls=j`** (`ref/copyparty/copyparty/httpcli.py:7289`, `:6734`): `GET {base}/{path}/?ls=j`
  → `{"dirs":[…],"files":[…], …}`. Each JSON entry = `{ "href": <url-encoded>, "sz": <int bytes>,
  "ts": <int st_mtime SECONDS>, "ext", "lead" }`. **`name` is popped from JSON** → derive the
  filename from `href` (percent-decode; dir hrefs end `/`). Auth: `?pw=` query **or** `PW:` header;
  anonymous OK; **401** anon-denied / **403** authed-denied. Download = plain `GET` (Range *is*
  supported in source — see correction #3).
- **sunnypilot on-disk layout** (`ref/sunnypilot/system/loggerd/logger.cc:98,166,185`,
  `loggerd.h`, `hw.py:15`): root `/data/media/0/realdata/`; **segment dirs are flat** and named
  `{route}--{N}` where `route = {8hexcounter}--{10hexrandom}` (e.g. `000001a3--c20ba54385--0`) —
  **no timestamp in the name** (confirmed by user). Per-segment files: `fcamera.hevc`,
  `ecamera.hevc`, `dcamera.hevc` (if RecordFront), `qcamera.ts`, `rlog.zst`, `qlog.zst` (legacy
  `.bz2` per `route.py:15`), and `rlog.lock` (**present ⇒ segment still recording**). Segment =
  exactly 60 s.

### Corrections to fold into the master plan as part of M1
1. **Naming:** segment dirs are `{counter}--{random}--{N}` (cloud `dongleid|timestamp` is comma-API only). Segment **time = copyparty `ts` mtime**, not parsed from the name.
2. **Grouping (M2 preview):** key on **route-id + segment index** (a new route = new drive; missing index or mtime gap = split), with `ts` mtime as the time signal — replaces the "route_start_ms + N·60 000 parsed from name" algorithm.
3. **Log files are `.zst`** (current) / `.bz2` (legacy), not bare `rlog`/`qlog`; add `rlog.lock` as the in-progress marker.
4. **Range is supported** in copyparty source (206/Content-Range) — the "#329 unreliable" claim isn't visible in this version. Doesn't change M1; **re-verify in M5** before committing to file-granular-only resume.

---

## File layout added in M1

```
rust/core/src/
  lib.rs                       # + pub mod error/model/copyparty_client/db  (M0 smoke exports stay)
  error.rs                     # CoreError (thiserror) — mapped to UniFFI error in M8
  model/
    mod.rs                     # Device, DeviceSettings, Segment, SegmentFile, enums
    ids.rs                     # SegmentName parse (route_id + segment_num), naming-agnostic
    time.rs                    # SEGMENT_MS=60_000, mtime→ms helpers, segment-start estimate
    file_kind.rs               # filename → FileKind mapping (.zst/.bz2/.hevc/.ts/.lock)
  copyparty_client/
    mod.rs                     # CopypartyClient { base_url, creds, reqwest::Client } + list_segments()
    auth.rs                    # Credentials { Anonymous | Password } → PW header (redacted in logs)
    listing.rs                 # serde structs for ?ls=j; parse → DirListing; href percent-decode
    download.rs                # streamed GET → AsyncWrite sink (basic; atomic .part is M3)
  db/
    mod.rs                     # Repo: r2d2 + r2d2_sqlite pool, WAL; device/segment/seg_file CRUD
    migrations.rs              # embedded migration runner + schema_version tracking
    schema.sql                 # v1: schema_version, device, segment, seg_file
  tests/                       # integration crate (uses pub API)
    it_listing.rs              # axum mock-copyparty: list_segments parses dirs/files/sizes/mtimes
    it_listing_auth.rs         # wiremock: asserts ?ls=j query + PW header; 401/403 handling
    it_real_copyparty.rs       # boots REAL copyparty over a fixture dir (gated/skippable)
    it_db.rs                   # migrate fresh DB; idempotent re-open; device+segment round-trip
rust/mock-copyparty/           # NEW workspace member (lib + bin)
  src/lib.rs                   # spawn(fixture) -> (base_url, handle); axum ?ls=j + file GET + pw
  src/fixtures.rs              # builders: single_drive, gap_split, partial (temp-dir trees)
  src/main.rs                  # standalone binary (reused by UI tests / mock-comma-mcp later)
```
Workspace `members` gains `"rust/mock-copyparty"`.

---

## Design details

**model::ids** — `SegmentName { route_id: String, segment_num: u32 }`. Parse by `rsplit_once("--")`
on a trailing all-digits suffix → `(route_id, segment_num)`; everything before is `route_id`
(works for `000001a3--c20ba54385--0` **and** legacy `dongleid|ts--0`). Reject names without a
numeric segment suffix.

**model::file_kind** — `FileKind { FCamera, ECamera, DCamera, QCamera, RLog, QLog, BootLog,
LockMarker, Other }`. Map by filename incl. both `.zst`/`.bz2`. `rlog.lock` ⇒ `LockMarker`
(flags "segment recording", never a download target).

**model::mod** — plain structs (no uniffi derives yet): `Segment { route_id, segment_num,
files: Vec<SegmentFile>, recording: bool /*rlog.lock present*/ }`; `SegmentFile { kind, name,
remote_size: u64, mtime_s: i64 }`; `Device` (connection: name, dongle/label, hotspot_ip,
wifi_ip, port, active_mode, password + the later-milestone settings columns, defined now but
only connection fields exercised in M1); enums `ConnMode{Hotspot,Wifi}`,
`FileSelection{PreviewsOnly,FullVideo,FullVideoPlusLogs}`, `DownloadState{Missing,InProgress,
Complete,SizeMismatch}`.

**copyparty_client** — `reqwest` (`rustls-tls`, `stream`). `list_dir(rel_path) -> DirListing`
(GET `?ls=j`, serde parse, derive names from `href` via `percent-encoding`). `list_segments(realdata_rel)`
= list realdata → keep dir entries whose name parses as a `SegmentName` → list each segment dir →
build `Vec<Segment>` with per-file size/mtime (one request per segment; fine for M1). `download(rel, sink)`
streams the body chunk-by-chunk. `auth` adds the `PW:` header when `Password`; `tracing` logs with
the password redacted.

**db** — `rusqlite` (`bundled`) + `r2d2`/`r2d2_sqlite`, WAL + `foreign_keys=ON` on each conn.
`migrations.rs` applies embedded SQL whose `version` > the `schema_version` table value, inside a
transaction (idempotent; M2/M4 append v2/v3). `Repo` exposes sync CRUD (upsert device; bulk
upsert segments+seg_files from a listing; query state); the async core calls these via
`tokio::task::spawn_blocking` (documented; the wrapping itself lands with M4/M8 callers).
`seg_file(remote_size, local_size, download_state)` columns exist now; `local_size`/state stay
default until M3/M5 populate them.

**mock-copyparty** — `axum`: `GET /{*path}?ls=j` → copyparty-shaped JSON built from a temp-dir
fixture (incl. `name` popped, `href` percent-encoded, `sz`, `ts` from file mtime, dirs vs files);
`GET /{*path}` → file bytes; optional `pw` (query/header) → 401/403. `fixtures.rs` builds
`single_drive` (one route, N consecutive segments w/ qcamera+logs), `gap_split` (missing index /
mtime gap), `partial` (some files missing, one `rlog.lock` present). `spawn(fixture)` binds an
ephemeral port and returns `(base_url, JoinHandle)` for tests; `main.rs` runs it standalone.

---

## Dependencies (add via `cargo add` for latest; pin in Cargo.lock)

- **core:** `reqwest` (`default-features=false, features=["rustls-tls","stream"]`), `serde`
  (`derive`), `serde_json`, `rusqlite` (`bundled`), `r2d2`, `r2d2_sqlite` (version matched to
  rusqlite), `thiserror`, `tracing`, `percent-encoding`, `url`. (tokio already.)
- **core dev-deps:** `wiremock`, `tempfile`, `tokio` (`macros,rt`), `mock-copyparty` (path).
- **mock-copyparty:** `axum`, `tokio`, `serde_json`, `tempfile`, `tower`/`hyper` as needed.

---

## Tests & verification

1. **Unit (`cargo test -p dashdown-core`):** `ids` parsing (counter--random--N, legacy, rejects bad), `file_kind` mapping (.zst/.bz2/.hevc/.ts/.lock), `time` conversions/segment-start.
2. **`it_listing`** (axum mock): `single_drive` + `gap_split` fixtures → `list_segments` returns correct route_id/segment_num, file kinds, `remote_size`, `mtime_s`, `recording` flag.
3. **`it_listing_auth`** (wiremock): assert outgoing request carries `?ls=j` + `PW:` header; map 401/403 → `CoreError::Auth`/`Forbidden`.
4. **`it_real_copyparty`** (gated): boot real copyparty (`pipx run copyparty` / `python -m copyparty`, or the `ref/` source) over a temp fixture dir; `list_dir`/`list_segments` parse its real `?ls=j`. `#[ignore]`-or-skip if copyparty absent locally; **CI installs copyparty and runs it**.
5. **`it_db`**: migrate fresh temp DB → `schema_version` set + tables exist; re-open is idempotent; round-trip a device + a listing's segments/seg_files.
6. **Workspace gates:** `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`; the M0 cross-compile steps still green. **Extend `rust-ci.yml`** with a copyparty install step for #4.
7. **Master-plan reconciliation:** apply corrections #1–#4 to `sunnypilot-dashdown-master-plan.md` (domain facts + M2 algorithm note) as part of the M1 PR.

**Branch/PR:** `phase-a/m1-copyparty-model-db` → PR → CI green → merge to `main`.

## Risks
- **Real-copyparty in CI:** pin a copyparty version and install via `pipx`/`pip`; keep the test hermetic (temp fixture dir, ephemeral port) and tolerant of minor JSON additions (parse only the fields we use; `serde` ignores unknown keys).
- **`r2d2_sqlite`/`rusqlite` version coupling:** add together and let cargo resolve a compatible pair; `bundled` adds C-compile time (cached in CI).
- **`href` decoding:** copyparty `quotep` percent-encodes; decode defensively and derive the basename, don't assume `name`.
