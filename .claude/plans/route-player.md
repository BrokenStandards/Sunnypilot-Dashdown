# Route Media Player (camera-focused)

> **Naming:** a post-B2 UX track, added now that the basic app shell works. Its milestones are
> labeled **RP1/RP2/RP3** to avoid colliding with the master plan's core milestones (M0–M8) and
> Phase B/C. It builds on top of the master-plan roadmap; it does not change it.

## Context

Downloaded routes are currently a dead end: the drives list shows only text rows, and the
detail screen plays a single `qcamera.ts` via one ExoPlayer. We want downloaded routes to be
**watchable** — a thumbnail in the list, and a real player that lets you switch between the
road / wide / driver cameras instantly at the same frame, scrub with a keyframe filmstrip, and
hear audio when it was recorded, with enabled cameras tiling to fit.

There is also a **live regression to fix**: PR #21 moved the on-disk footage base from
`realdata/` to `routes/`, but Android's [DriveDetailViewModel.resolvePlayable()](android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/DriveDetailViewModel.kt#L70-L78)
still hardcodes `realdata/`. Once #21 merges, local playback path resolution breaks. The fix
(centralizing path resolution in the Rust core) is the first step of this feature and the single
source of truth everything else builds on.

### Scope (locked with the user)
- **IN:** list thumbnails; a real detail player; thumbnail-keyframe scrubbing; per-stream audio
  (when recorded); same-frame camera switching; tap-toggle road/wide/driver; intelligent tiling.
- **OUT (explicit user decision — "we will not do this in the app"):** the "what the comma was
  showing" model-replay overlay, and the wheel-position / acceleration real-vs-requested telemetry.
  → **No log/cereal parsing, no capnp/zstd, no charts, no projection math** anywhere in this work.
  Logs remain *downloadable* (existing `FileSelection`) but are not visualized in-app.
- **Decisions:** incremental sequencing; HD remux **lazy + gated**; **staged PR per milestone**;
  new branch `route-player` off `main` (after / rebased on PR #21's `routes/` change).

## Key technical facts (verified)
- Local layout: `<mirror_root>/<device_id>/routes/<route_id>--<segNum>/<file>`
  (`REALDATA_REL = "routes/"`, [sync_engine/mod.rs:36](rust/core/src/sync_engine/mod.rs#L36)).
- `qcamera.ts` = H.264 in MPEG-TS, 526×330, 20 fps — **directly playable by Media3** and the
  **only** stream that can carry audio (AAC mono, when sunnypilot `RecordAudio` was on).
- `fcamera.hevc` (road), `ecamera.hevc` (wide), `dcamera.hevc` (driver) = **raw HEVC Annex-B**,
  20 fps CBR, no container, no audio, ~76 MB/min. **ExoPlayer cannot play raw `.hevc`.**
- No FFI returns a local path today; `export_drive_zip` builds them internally via
  `file_rel(REALDATA_REL, seg, name)` + `MirrorStore::final_path` ([ffi/mod.rs](rust/core/src/ffi/mod.rs)).
- Android: Media3 1.10.1 present; **no** image-loading lib (add Coil); no multi-player/scrubber/
  thumbnail code. `FileKind` is already a `uniffi::Enum` (`F_CAMERA/E_CAMERA/D_CAMERA/Q_CAMERA/…`).

## HEVC playback strategy — Rust lossless remux to fragmented MP4
ExoPlayer has no raw-`.hevc` extractor. **Remux raw HEVC Annex-B → fragmented MP4 (`-c copy`,
`hvc1` tag) in the Rust core**, cached next to the source; ExoPlayer/AVPlayer then play the fMP4
with exact sample tables (→ frame-accurate `seekTo`). This is pure bytestream surgery (parse
start codes → length-prefixed samples → `moov/moof/mdat`, synthesize 20 fps timing) — no decode,
no re-encode, deterministic, unit-testable in Rust, and reused verbatim on iOS. Rejected: a
custom Media3 `Extractor` (unproven, Android-only, builds on Media3-internal `H265Reader`).
Remux **lazily** (on first HD play) and **gate** HD behind the existing `FileSelection`; the
`.mp4` is a derived artifact (deletable in maintenance, re-derivable). qcamera plays as-is.

### Master clock & same-frame switching
One `ExoPlayer` **per enabled tile** (ExoPlayer has no multi-video surface). Global timeline =
**route microseconds**: `route_us = segNum*60_000_000 + frameIndex*50_000` (20 fps → 50 ms/frame,
segment-aligned; every camera shares it). Elect a **clock master** (qcamera if audio on, else
road); a coroutine ticker reads `master.currentPosition`, the ViewModel drives followers. Switch
camera / scrub = `seekTo(route_us)` on all players; re-sync any follower that drifts > ~1 frame.
Per camera, a `ConcatenatingMediaSource` over the drive's segment files makes the drive one
seamless timeline. Audio lives only on qcamera → when "audio" is on, a qcamera source (visible
tile, or a hidden audio-only player) is slaved to the clock; the toggle shows only when the
segment's qcamera actually has an audio track (detect via Media3 `Tracks`).

---

## Milestones (each: build + test + commit; its own PR to `main`)

### RP1 — Path accessor (fixes the regression) + drives-list thumbnails — ✅ done (PR #22)
- **Rust** ([ffi/mod.rs](rust/core/src/ffi/mod.rs)): add the single source of truth for on-disk paths,
  reusing `file_rel(REALDATA_REL, …)` + `MirrorStore::final_path` (returns `None` if not complete):
  ```
  async fn local_file_path(&self, device_id: i64, drive_key: String,
                           segment_num: u32, kind: FileKind) -> Result<Option<String>>
  async fn drive_local_paths(&self, device_id: i64, drive_key: String,
                            kind: FileKind) -> Result<Vec<SegmentPath>>   // ordered, complete only
  #[derive(uniffi::Record)] struct SegmentPath { segment_num: u32, path: String }
  ```
- **Android:** replace the hardcoded `realdata/` string in `DriveDetailViewModel.resolvePlayable()`
  with the new accessor (via `DashdownRepository`). Add **Coil** (`coil-compose`) to
  [libs.versions.toml](android/gradle/libs.versions.toml) + app build. Generate a per-drive thumbnail
  on Android from the first complete `qcamera.ts` via `MediaMetadataRetriever.getFrameAtTime`, cache
  in app `cacheDir/thumbs/<driveKey>.jpg` (out of the mirror tree), show in
  [DrivesListScreen.kt](android/app/src/main/java/org/sunnypilot/dashdown/ui/drives/DrivesListScreen.kt)
  `DriveRow` leading slot.
- **Tests:** Rust unit (path complete / missing / `..` traversal). Android instrumented: detail
  screen resolves a real qcamera path from a mock mirror; list thumbnail renders. Manual: open a
  downloaded drive on the Pixel and confirm playback works again (regression gone).

### RP2 — Real qcamera detail player: scrubber + filmstrip + audio (Android-only)
- **Spans the whole drive (headline requirement):** the player + scrubber treat ALL of a drive's
  qcamera segments as ONE continuous timeline in route µs — never per-segment. Play and seek cross
  1-minute segment boundaries seamlessly (no gap, no stutter, no per-segment UI).
- Rebuild `DrivePlayer` in [DriveDetailScreen.kt](android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/DriveDetailScreen.kt):
  `ConcatenatingMediaSource` over the drive's qcamera segments → one continuous timeline; a Compose
  scrubber bound to `route_us`; a `LazyRow` thumbnail **filmstrip** (frames every N seconds via
  `MediaMetadataRetriever`, cached under `cacheDir/filmstrip/<driveKey>/`), tap-to-seek.
- Audio: detect via Media3 `Tracks` after prepare; show a mute/unmute toggle only when a track
  exists. No core work (qcamera plays natively).
- **Tests:** instrumented scrub (seek lands within 1 frame), filmstrip count + cache hit, audio
  toggle hidden when no track. Manual on Pixel.

### RP3 — Multi-camera HD: lazy remux + tiling + same-frame switching
- **Rust** new module `rust/core/src/video/{mod.rs,remux.rs}`: raw HEVC Annex-B → fragmented MP4
  (`-c copy`, `hvc1`), VPS/SPS/PPS parse, 20 fps timing, `moov/moof/mdat` writer. Use a pure-Rust
  mp4 muxer crate (evaluate `mp4`/`re-mp4` for `hvc1`; else a small hand-rolled box writer) — **no
  ffmpeg/C**. Cache `<file>.hevc.mp4`. FFI:
  ```
  async fn ensure_playable(&self, device_id: i64, drive_key: String,
                          segment_num: u32, kind: FileKind) -> Result<Option<String>>
  // qcamera → its path as-is; HD camera → cached/created fMP4 path (remux on the tokio runtime)
  ```
  Maintenance (`run_maintenance`) deletes derived `.mp4` when pruning.
- **Android** new `ui/detail/{MultiCamPlayer.kt, RouteClock.kt}`: one ExoPlayer per enabled tile,
  master clock + sync controller (above), tap-toggle bar for **road / wide / driver**, and a pure
  `tilesFor(enabled, orientation): List<TileSlot>` layout function rendered via `BoxWithConstraints`.
  Tiling: **N=1** full; **N=2** stacked (portrait) / side-by-side (landscape); **N=3** primary-large
  (road) + two stacked thumbnails; **N=4** 2×2.
- **Tests:** Rust remux unit (HEVC fixture → valid fMP4; `sample_count == frame_count`). Media3
  instrumented: the fMP4 plays + seeks; cross-camera switch lands the same frame (±1). `tilesFor`
  snapshot tests N=1..4 × orientations. Manual on Pixel + the real comma-4 @ `192.168.1.181:8080`
  with a real HD drive (watch thermals/battery — consider capping simultaneous HD decoders).

---

## Cross-platform
Heavy/derivable logic stays in the Rust core (path resolution, HEVC→fMP4 remux) returning plain
paths so **iOS (later) reuses it verbatim**. Native per platform: players (ExoPlayer/AVPlayer),
tiling, scrubber, filmstrip extraction (MediaMetadataRetriever/AVAssetImageGenerator), and the
master-clock/sync controller. Both consume the same FFI + the same `route_us` timeline.

## Risks / verify early
- Verify a round-tripped fMP4 actually **plays + seeks frame-accurately** on the Pixel decoder
  before building the multi-cam UI on it (do this first thing in RP3).
- Pick the mp4 muxer carefully — confirm `hvc1`/HEVC sample-entry support; hand-roll a minimal box
  writer if no crate fits. No C deps (clean Android cross-compile).
- Multi-HEVC decode is the worst case for battery/thermals — measure; cap simultaneous HD tiles if
  the Pixel throttles (e.g. road HD + others as qcamera).
- qcamera audio is optional — never assume a track; detect and gate the toggle.
- Without logs we use the `50 ms/frame` arithmetic clock; exact only if no frames were dropped —
  acceptable here since we're not doing log-aligned overlays.

## Verification & delivery
Per milestone: `cargo test` (Rust) + instrumented/Maestro (Android) green, then a PR to `main`
(staged review like B2); `main` always has working software. Branch: `route-player` off `main`
(rebased on PR #21). Manual acceptance on the Pixel and the real comma-4 @ `192.168.1.181:8080`.
