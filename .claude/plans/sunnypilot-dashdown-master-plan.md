# Comma4 Sunnypilot Copyparty Dashcam Downloader — Architecture & Implementation Plan

## What this document is

This is the **overall architecture plan** that drives all future development. It is the
**meta-plan**: each phase/milestone below will get its own detailed implementation plan when we reach it

**Operating principle for every future plan derived from this one:**
> Each phase **builds its functionality completely and tests it completely before the next
> phase begins** — including background downloads, resume, retention/auto-delete, and
> connectivity behavior. "Done" for a phase means built, wired end-to-end, and verified by
> automated tests (unit + integration +, for UI phases, on-device/emulator UI tests),
> including the hard cases (app backgrounded/killed mid-download, interrupted transfer
> resumed, device unreachable). No functionality is stubbed and carried forward.

Reference source (sunnypilot, copyparty, uniffi-rs, uniffi-starter) lives in a gitignored `ref/` dir — reconstructable via `tools/fetch-refs.sh`, searchable with `tools/refgrep`, and kept out of normal code searches (see `docs/REFERENCES.md`). We should also write tests using snipits that run real sunnypilot code/copypart server with fixtures for testing. Each phase should enter plan mode and create its own plan files. Each phase should make use of agents to research, verify code, run tests, debug, etc. After each milestone of each phase we should git commit. If we create branches, we should merge back into the main one at the end of the phase.  If an MCP needs to be developed, we should advise the developer to create it. For instance, we might want an MCPs for a UI to verify elements alighn correctly and assisting, or for finding relevent documentation/code from the parent codebase.

---

## Context

We are building a mobile app (iPhone + Android) that downloads dashcam footage from
Comma devices running the Sunnypilot openpilot fork, where footage is served over a
**copyparty** file server.

**What the app must do (product requirements):**
- Show all "drives" for a device. A *drive* = a maximal run of consecutive 1-minute
  segments with **no time gap**; a 1-minute gap splits into a separate drive.
- On request, download a whole drive's files in the background (single-zip export on demand).
- Compare the device's local download folder vs files on copyparty; if remote has a later
  contiguous segment we lack (or a file is incomplete), mark the drive **partially
  downloaded / resumable**.
- Store files mirroring copyparty's layout (raw files, not zips); browse them offline,
  grouped per drive by the same gap rule.
- Let the user add multiple Comma devices, each with a hotspot IP + wifi IP + copyparty
  port, with a quick toggle between hotspot/wifi.
- Show a per-device connectivity/status dot (green / blue / red).
- Per-device settings: auto-sync toggle; max-minutes-to-keep retention with a "preserve"
  switch; auto-delete-from-comma after a whole drive is copied, only if the drive is older
  than N minutes (never delete an actively-recording drive).

**Decisions already locked with the user:**
1. **Architecture:** shared **Rust core** (UniFFI) + **native UIs** — SwiftUI (iOS),
   Jetpack Compose (Android). Maximizes Rust for logic while keeping native UX per platform.
2. **File scope:** **configurable per device** (default to lightweight `qcamera.ts`
   previews; full HEVC + logs + other/unmatched optional).
3. **Local storage:** **raw files mirrored** to a tree matching copyparty exactly — no
   zips for storage. "Export zip" is an on-demand **export** action that exports the entire consecutive drive/route .
4. **Platform order:** Rust core first, then **both native shells in parallel**.

---

## Domain facts (researched — treat as ground truth)

- **copyparty API** (M1-verified against source): JSON dir listing `GET .../?ls=j` returns
  `{dirs, files}`; each entry has `href` (percent-encoded), `sz` (bytes), `ts` (mtime, Unix
  **seconds**) — **`name` is omitted from the JSON**, so derive the filename from `href`.
  Per-folder archive `?zip` / `?tar`; auth via `?pw=<pw>` query or `PW:` header (anonymous OK;
  **401** anon-denied / **403** authed-denied). **HTTP Range IS supported** in current source
  (206 / `Content-Range`) — the earlier "#329 unreliable" claim isn't visible in this version;
  we still default to **file-granular** resume for safety and **re-verify Range in M5** before
  relying on byte-range. Username/password may be empty for anonymous access.
- **Comma dashcam storage** (M1-verified against sunnypilot source): segment dirs are **flat**
  under `/data/media/0/realdata/`, named `{route}--{N}` where `route = {8hexcounter}--{10hexrandom}`
  (e.g. `000001a3--c20ba54385--0`; N = 0-indexed) — **no timestamp/dongle-id in the on-disk
  name**; the `dongleid|YYYY-MM-DD--HH-MM-SS` form is the **comma-cloud** representation only.
  Each segment = **exactly 1 minute**; wall-clock time comes from copyparty's `ts` mtime. Files
  per segment: `fcamera.hevc` (~76MB road), `ecamera.hevc` (~76MB wide), `dcamera.hevc` (driver,
  if enabled), `qcamera.ts` (~12MB preview), `rlog.zst`/`rlog.bz2` (~3MB), `qlog.zst`/`qlog.bz2`
  (~1MB), and a transient **`rlog.lock`** present only while the segment is still recording.
- **Connectivity:** comma hotspot IP defaults to `192.168.43.1`; copyparty port is user-configured. WiFi IP is user configured. 
- **copyparty DELETE endpoint:** exact method is uncertain across versions — the M6 plan
  **verifies it against a real copyparty instance and implements + tests real deletion**
  (WebDAV `DELETE` is the most standards-compliant candidate) behind the age/complete safety
  guards. This is a verification task inside M6, if code inspection or test reveal this is not possible advise the developer and request input.

---

## Overall architecture

```
                ┌─────────────────────────────────────────┐
                │              RUST CORE (lib)              │
                │  copyparty_client · drive_grouping ·      │
                │  storage(mirror) · sync_engine(resume) ·  │
                │  db(rusqlite) · settings · connectivity · │
                │  logging(tracing) · ffi(UniFFI facade)    │
                └───────────────┬──────────────┬────────────┘
            xtool (Linux)       │              │  cargo-ndk
              MyCore.xcframework │              │  libcore.so
                    ┌───────────┴───┐     ┌─────┴──────────────┐
                    │  iOS (SwiftUI)│     │ Android (Compose)  │
                    │  UniFFI-Swift │     │ UniFFI-Kotlin      │
                    │  URLSession/  │     │ WorkManager +      │
                    │  BGTask, AVPlayer   │ Foreground Service,│
                    │  Keychain     │     │ Media3, Keystore   │
                    └───────────────┘     └────────────────────┘
```

**Tech stack (specific, 2025–2026 mature):**
- FFI: **UniFFI ** (proc-macro `#[uniffi::export]`, async via `async_runtime="tokio"`).
- HTTP: **reqwest** (`rustls-tls`, `stream`) + **tokio**.
- DB: **rusqlite** (`bundled`) + **r2d2** pool, WAL mode. (Bundled SQLite avoids
  iOS/Android system-sqlite linkage problems; our DB ops are tiny — async value of `sqlx`
  is marginal; run blocking calls via `spawn_blocking`.)
- Logging: **tracing** + custom layer forwarding to native via a `LogSink` callback.
- Builds: **cargo-ndk** (Android `.so`); **xtool** builds/signs the iOS `.xcframework` **on
  Linux** (replaces cargo-xcframework, which needs macOS — Swift-on-Linux SDK setup recipe in
  `docs/REFERENCES.md`).
- Testing: **wiremock**/**proptest**/**mockall** (Rust), a standalone **mock-copyparty**
  fixture server (axum), **XCUITest** (iOS, run on Linux via xtool/Swift), **Espresso**
  (Android), **Maestro** (cross).
  Use cargo commands for rust dependencies to install the latest versions.

---

## Workspace layout

```
sunnypilot-dashdown/
  Cargo.toml                  # [workspace]
  rust/
    core/                     # crate-type = ["staticlib","cdylib","lib"]
      src/
        lib.rs                # uniffi::setup_scaffolding!()
        ffi/{mod,callbacks,errors}.rs   # AppCore facade, ProgressSink/LogSink, CoreError
        model/{mod,ids,time}.rs         # Device/Drive/Segment/SegmentFile + enums
        copyparty_client/{mod,listing,download,delete,auth}.rs
        drive_grouping/{mod,remote,local}.rs   # shared group_segments()
        storage/{mod,paths,scan}.rs            # MirrorStore, atomic .part writes
        sync_engine/{mod,download_job,resume,retention}.rs
        connectivity/mod.rs   # TCP-connect reachability + dot logic
        db/{mod,migrations}.rs; schema.sql
        settings/mod.rs
        logging/mod.rs
      tests/                  # it_listing, it_download, it_resume, it_retention, ...
    mock-copyparty/           # reusable axum fixture server (also drives UI tests)
      src/main.rs; fixtures/  # single-drive, gap-split, partial trees
    bindgen/                  # uniffi_bindgen wrapper -> swift/ + kotlin/ bindings
  ios/                        # SwiftPM app (SwiftUI), built on Linux via xtool — Phase B
  android/                    # Gradle project (Compose) — built in Phase B
  tools/                      # fetch-refs.sh, refgrep, Maestro flows, CI scripts
  docs/                       # REFERENCES.md (reference manifest + iOS-on-Linux recipe)
  ref/                        # gitignored third-party reference source (via fetch-refs.sh)
  CLAUDE.md                   # repo working conventions
```

---

## Data model & DB (summary)

**Boundary-crossing types** (UniFFI Records/Enums): `Device`, `DeviceSettings`,
`Drive`, `Segment`, `SegmentFile`, `DownloadProgress`, `DeviceConnectivity`,
`DriveSyncStatus`, `LogEvent`; enums `ConnMode{Hotspot,Wifi}`, `FileKind`,
`DownloadState{Missing,InProgress,Complete,SizeMismatch}`,
`JobState{Running,Complete,Failed,Canceled}`,
`SyncStatus{NotDownloaded,Partial,Complete,Downloading,Failed}`,
`ConnDot{Green,Blue,Red}`, `LogLevel`; error `CoreError`. Objects (Arc): `AppCore`,
`SyncHandle` (holds a `CancellationToken`).

> **M4 correction:** `FileSelection` is a **per-stream toggle struct** (a UniFFI Record) — one
> bool per downloadable `FileKind` (`fcamera/ecamera/dcamera/qcamera/rlog/qlog/bootlog/other`), not
> the original 3-level `{PreviewsOnly,FullVideo,FullVideoPlusLogs}` enum. Audio is muxed into
> `qcamera.ts` upstream (sunnypilot `RecordAudio`), so it rides the `qcamera` toggle. Persisted as a
> sorted CSV in the existing `device.file_selection` column (legacy preset names still parse).

**SQLite (metadata only — mirror folder is source of truth, DB is a rebuildable index):**
tables `device`, `drive`, `segment`, `seg_file`, `download_job`, `schema_version`.
Key columns: `seg_file(remote_size, local_size, download_state)` drives status computation;
`device(hotspot_ip, wifi_ip, port, active_mode, password, auto_sync, file_selection,
retention_max_minutes, auto_delete_from_comma, auto_delete_min_age_min)`;
`drive(drive_key, start_ms, end_ms, segment_count, preserved, sync_state)`.

**Connectivity dot meaning:** **Green** = reachable (TCP connect to active (ip,port) ok) &
idle; **Blue** = reachable & a download/sync active for this device; **Red** = unreachable.
Reachability uses `TcpStream::connect` with timeout (not ICMP — ping needs raw sockets,
blocked on mobile).

---

## Key algorithms

**Drive grouping (shared, online listing & offline mirror scan use the SAME function).**
*(on-disk names carry no timestamp, so grouping keys on **route-id + segment
index** with the `ts` mtime as the time signal)* Within a route, two segments are contiguous iff their `segment_num`s are
consecutive; a **new `route_id`** (a new loggerd session) or a **missing index** starts a new
drive. The segment `ts` mtime is a secondary sanity signal (gap-check:
`abs(next_mtime − prev_mtime − 60_000)` should be small). `drive_id` = first segment's key
(stable as the drive grows). Property tests assert: union of drives' segments == input; no
internal index gap within a drive; idempotent & order-independent.

**Partial / resume (no HTTP range).** Download to `<file>.part` → fsync → atomic rename to
`<file>`. So a committed file = no `.part`. `classify_file`: `.part` present → `InProgress`
(re-fetch whole); final missing → `Missing`; final present & size==remote_size →
`Complete`; size mismatch → `SizeMismatch` (re-fetch). Drive is `Partial`/`resumable` if any
selected file is Missing/InProgress/SizeMismatch **or** a later contiguous remote segment is
missing locally.

**Retention + auto-delete safety guard.** Retention prunes oldest-beyond-budget local drives
(newest-first), skipping `preserved`. Auto-delete-from-comma fires only when: (1) drive
`sync_state == Complete` for the active selection, (2) `now − drive.end_ms >= min_age*60_000`
(protects an actively-recording drive — its last segment ends ~now), and (3) a fresh remote
re-verify still shows Complete. M6 implements and tests the real copyparty deletion call
behind these guards.

---

## UniFFI surface (native-facing API)

Root `AppCore::new(db_path, mirror_root)`. Async methods (Swift `async throws` / Kotlin
`suspend`): `list_devices`, `add_device`, `update_device`, `remove_device`,
`set_active_mode`, `get_settings`, `set_settings`, `list_drives(device_id, offline)`,
`get_drive`, `sync_now(device_id)`, `start_drive_download(device_id, drive_id) -> SyncHandle`,
`set_preserved`, `export_drive_zip(device_id, drive_id, dest_path)`,
`run_maintenance(device_id)`, `check_connectivity(device_id)`. Sync: `set_progress_sink`,
`set_log_sink(level)`. `SyncHandle::cancel()`. Callback traits `ProgressSink`
(`on_progress/on_completed/on_failed`) and `LogSink` (`on_log`) are implemented natively.

---

## Background execution contract (full background download on both platforms)

The **Rust core owns the transfer engine** (reqwest streaming + the file-granular resume
engine). The native layer provides background scheduling and true-suspension execution. Full
background download is built and tested within each platform's phase:

- **Android:** a **Foreground Service** (persistent notification) hosts a coroutine that
  calls `start_drive_download` — runs long while user-visible/backgrounded. **WorkManager**
  schedules opportunistic auto-sync (constraints: unmetered/WiFi, not-low battery) that calls
  `sync_now` + `run_maintenance`. The Android phase tests: start a drive, background the app,
  confirm it completes; kill/restart, confirm it resumes only the missing files.
- **iOS:** while foregrounded, Rust downloads freely. For unattended work the iOS phase builds
  **both** mechanisms and tests them end-to-end: **BGProcessingTask** windows that call
  `sync_now`/`start_drive_download`, **and** a Swift **`URLSession` background** downloader
  that completes large transfers while the app is suspended/killed and hands each finished
  file to the core for commit + status update. The core's file-granular resume engine makes
  any interrupted transfer safe to continue on the next window/launch. The iOS phase tests:
  start a drive, suspend/kill the app, confirm completion and resume.

Rust owns the transfer engine and resume logic everywhere; the native layer adds the
platform's true-background execution path on top. Both are required deliverables of their
phases.

---

## Native UI plan (native UX per platform)

**Screens (shared IA):** (1) **Device list** — each row shows name + colored connectivity
dot + sync summary; add-device button. (2) **Add/Edit device** — name, dongle id, hotspot
IP / wifi IP fields with a **segmented quick-toggle** for active mode, port, password,
per-device settings. (3) **Drives list** — grouped drives with duration, sync badge
(NotDownloaded / Partial-resumable / Complete / Downloading), preserve star; works offline
from the local mirror. (4) **Drive detail** — segments/files, Download/Resume, Cancel,
Preserve toggle, Export-as-zip, inline playback. (5) **Per-device settings** — auto-sync,
file selection, retention max-minutes, preserve default, auto-delete-from-comma + min-age.

**iOS (SwiftUI):** `NavigationStack`, `List` with swipe actions, SF Symbols, segmented
`Picker` for hotspot/wifi, `.refreshable` pull-to-refresh, native **share sheet** for zip
export, **AVPlayer** for `qcamera.ts`/HEVC playback, **Keychain** for the copyparty password,
progress via `ProgressSink` → `@Observable` view models. Background: BGTaskScheduler +
background URLSession.

**Android (Jetpack Compose):** Material 3, `LazyColumn`, `NavHost`, `SegmentedButton` for
hotspot/wifi, `PullToRefresh`, **Storage Access Framework** for zip export, **Media3/ExoPlayer**
for playback, **EncryptedSharedPreferences/Keystore** for the password, progress via
`ProgressSink` → `StateFlow`. Background: WorkManager + Foreground Service.

---

## MCPs & reliable UI testing

**Phase A (Rust core) needs no MCP** — it's verified locally with `cargo test`, wiremock, and
the in-repo `mock-copyparty` fixture. (The "doc-lookup MCP for the parent codebase" idea is
replaced by the gitignored `ref/` dir + `tools/refgrep`.)

**MCPs for later phases:**
- **GitHub MCP** — PRs, CI status, review handling. Set up in Phase 0 as a header-token HTTP
  server (`api.githubcopilot.com/mcp/`); GitHub's hosted MCP doesn't support OAuth/DCR, so it
  authenticates via a `gh` token header. Repo `BrokenStandards/Sunnypilot-Dashdown` (private);
  Claude GitHub Actions installed.
- **~~XcodeBuildMCP~~ — N/A:** requires Xcode/macOS, incompatible with our iOS-on-Linux path.
  Agentic iOS build/run/screenshot uses the **xtool** CLI + **libimobiledevice** (real device).
- **mobile-mcp** (mobile-next) — Android emulator/device UI automation on Linux (its iOS path
  needs a Mac/simulator, so it's Android-focused here); complements native test suites.

**MCP we will develop:** a thin **`mock-comma-mcp`** wrapper around the `mock-copyparty`
fixture server so the agent can, during automated UI runs, provision device fixtures, inject
states (single drive / 1-min-gap split / partial / size-mismatch), and **toggle reachability
up/down** to exercise the green/blue/red dot — giving deterministic, hermetic UI tests. (The
underlying server is a plain binary reused by Rust integration tests too.) Built in M1
(server) and wrapped as an MCP in Phase B.

**Reliable UI testing approach:**
- Point both apps at the **mock-copyparty** server on localhost → hermetic, deterministic
  backend with known fixture trees (no real Comma needed).
- **Native** layer: XCUITest (iOS) + Espresso (Android) keyed on accessibility IDs.
- **Cross-platform** smoke/regression: **Maestro** YAML flows run on both in CI.
- **Agentic**: **xtool**/libimobiledevice (iOS) + **mobile-mcp** (Android) let Claude launch,
  screenshot, and validate flows end-to-end against the mock server, toggling connectivity to
  verify status dots.

---

## Testing strategy (Rust core)

- **Unit:** `model::time` parsing/offsets; `drive_grouping` table-driven (gap split, route
  change mid-drive, tolerance boundary); `resume::classify_*` permutations; `retention`
  budget/preserve/age-guard boundaries; `storage::paths` round-trips; connectivity dot matrix.
- **Property (proptest):** drive-grouping invariants (partition completeness, no internal
  gap, idempotence, order-independence).
- **Integration (wiremock + mock-copyparty):** `it_listing` (asserts `?ls=j` + `PW:`),
  `it_download` (atomic `.part`→rename, truncated body → re-fetch), `it_resume` (fills only
  missing files), `it_retention` (guards + real delete against the mock), `it_offline_grouping`
  (mirror scan == online).
- **Bindings smoke:** generate Swift + Kotlin, load lib, call `ping()` / `list_devices()`.
- Thorough **debug logging** throughout via `tracing` (pw redacted), surfaced to native via
  `LogSink` for on-device diagnostics.

---

## Implementation order (milestones)

Each milestone below is **built and fully tested before the next starts**. Each will receive
its own detailed plan when reached, with full scope as described.

**Phase 0 — Environment bootstrap (done):** toolchain (Rust Android/iOS targets, cargo-ndk,
NDK r27.3), reference source in gitignored `ref/` (copyparty / sunnypilot / uniffi-rs /
uniffi-starter, pinned), repo conventions (`CLAUDE.md`, `.gitignore`/`.ignore`,
`tools/fetch-refs.sh` + `refgrep`, `docs/REFERENCES.md`), GitHub repo + MCP. See
`.claude/plans/Phase 0 — Environment Bootstrap & Reference Setup.md`.

**Phase A — Rust core (sequential, test-first each step):**
- **M0 Scaffolding** — workspace, `core` crate (`staticlib/cdylib/lib`), deps, CI
  cross-compile for `aarch64-apple-ios(-sim)` + `aarch64-linux-android` /
  `armv7-linux-androideabi` / `x86_64-linux-android` / `i686-linux-android`, exported `ping()`.
  (iOS targets compile std on Linux; linking the `.xcframework` is via xtool's Swift SDK in
  Phase B.) *Test:* ping + bindgen runs green on all targets.
- **M1 copyparty client + model + db** — `?ls=j` listing/parse, streamed download, auth;
  schema + migrations + Repo; build the `mock-copyparty` fixture server. *Tests:* `it_listing`,
  time parse, migration.
- **M2 Drive grouping** — shared `group_segments` (remote). *Tests:* table-driven + proptest.
- **M3 Storage mirror** — path mapping, atomic `.part` writes, mirror scan → `SegInfo`,
  offline grouping. *Tests:* `it_offline_grouping`, atomic write, path round-trip.
- **M4 Sync/download engine** — `download_job`, `SyncHandle`+cancel, progress, `sync_now`,
  selection filtering. *Tests:* `it_download`, cancel mid-download, previews-only filter.
- **M5 Partial/resume** — `resume` classification, job persistence/restart recovery. *Tests:*
  `it_resume`, truncated→re-fetch, later-contiguous→Partial, restart-resumes-missing-only.
- **M6 Settings + retention + auto-delete** — retention pruning; **verify copyparty DELETE
  against a real instance and implement + test the real deletion** behind the age/complete
  guards. *Tests:* `it_retention` (budget, preserve, age guard, real delete on mock + verified
  endpoint).
- **M7 Connectivity** — TCP-connect reachability + dot logic. *Tests:* up/down, dot matrix,
  Blue while downloading.
- **M8 UniFFI surface** — `AppCore` facade, callbacks, tracing→`LogSink`, generate
  Swift+Kotlin. *Tests:* end-to-end against mock-copyparty + per-platform load smoke.

**Phase B — Native shells (parallel, after M8) — each builds & tests full background:**
- **iOS:** SwiftPM app built on Linux via **xtool** (one-time `Xcode.xip` SDK extraction +
  `libxadi` Apple-auth — see `docs/REFERENCES.md`), integrate `.xcframework`, SwiftUI screens,
  ProgressSink/LogSink, BGTask + background URLSession, AVPlayer, Keychain. XCUITest/Swift
  Testing (including background/suspend/kill → complete/resume) vs mock server.
- **Android:** Gradle project, integrate `.so`, Compose screens, WorkManager + Foreground
  Service, Media3, Keystore. Espresso (including background/kill → complete/resume) vs mock
  server.
- **Shared:** Maestro cross-platform flows; develop `mock-comma-mcp`; wire
  xtool/libimobiledevice (iOS) + mobile-mcp (Android) agentic checks.

**Phase C — Integration & CI:** GitHub Actions builds Rust for all targets, copies artifacts
into the iOS (xtool/SwiftPM) and Android (Gradle) projects, runs `cargo test` + iOS tests +
`./gradlew connectedAndroidTest` + Maestro; release packaging. **OPEN QUESTION (decide at
Phase C):** run iOS build/test on a Linux runner via xtool, or a macOS runner via xcodebuild.

---

## Verification (end-to-end)

1. `cargo test --workspace` green (unit + proptest + wiremock integration).
2. Launch `mock-copyparty` with fixture trees; run the M8 end-to-end test: add device →
   `list_drives` (verify gap-split grouping) → `start_drive_download` → ProgressSink fires →
   status `Complete`; re-run after deleting one file → status `Partial`/resumable → resume
   fills only the missing file.
3. Offline: scan the populated mirror with the server down → `list_drives(offline:true)`
   returns identical grouping.
4. Connectivity: toggle mock server up/down → `check_connectivity` returns Green/Red; start a
   download → Blue.
5. Background (per platform): start a drive download, background then kill the app, confirm it
   completes; interrupt mid-transfer and confirm it resumes only the missing files.
6. Native: XCUITest + Espresso + Maestro flows pass against the mock server; agentic
   screenshot verification via xtool/libimobiledevice (iOS) and mobile-mcp (Android) confirms
   the device-list dots, drive grouping, partial/resume badge, and per-device settings render
   natively on each OS.
7. copyparty DELETE: the M6 verification probe confirms the real endpoint, and auto-delete
   (behind the age/complete guards) is exercised against it.
