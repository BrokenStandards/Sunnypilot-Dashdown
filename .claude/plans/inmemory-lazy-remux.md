# In-memory, lazy HEVC remux — instant open & seek-anywhere

_(suggested final filename: `.claude/plans/inmemory-lazy-remux.md`)_

## Context

The drive player plays comma HD cameras (road/wide/driver) by losslessly remuxing each
raw `.hevc` segment to MP4. Today that remux is **eager and disk-backed**:

- When an HD camera is enabled, `MultiCamPlayer`'s `LaunchedEffect(enabled)` loops over **every**
  segment of that camera and calls `resolveHd`/`ensure_playable` for each
  ([MultiCamPlayer.kt:223-231](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L223-L231)),
  **before** building the playlist. For a 100-minute drive that's ~100 segments × 3 cameras
  remuxed up front — the ~40 s the user observed.
- Each remux **writes a `<src>.hevc.mp4` file** next to the source
  ([video/mod.rs `ensure_playable_mp4`](../../rust/core/src/video/mod.rs#L31-L45)), used as a disk cache.

The remux itself is **cheap**: `hevc_annexb_to_mp4(input: &[u8]) -> Vec<u8>`
([remux.rs:84](../../rust/core/src/video/remux.rs#L84)) is a pure NAL-rewrap (no decode), already
returns bytes in memory, and produces an `moov`-at-front MP4 with a full sample table (so seeking
to any frame within a segment works). The only disk step is the file write, and the only reason a
100-min drive is slow is the **app-level eager loop** — ExoPlayer itself prepares a long playlist
lazily (confirmed: it opens a window's `DataSource` only when the playhead reaches it).

**Goal:** remux **in memory (no file written)**, **lazily**, **starting at the current segment**,
so opening a drive and seeking to any timestamp plays with minimal delay regardless of drive length.

**Scope:** footage already mirrored to disk (the normal case). Streaming *un-downloaded* footage
directly from copyparty is a separate future extension (the client already has ranged `fetch` /
in-memory `download` if we pursue it) — out of scope here.

## Approach

Replace the file path that ExoPlayer opens with a **custom in-memory `DataSource`** that remuxes the
raw `.hevc` on demand, and **delete the eager remux loop**. ExoPlayer then opens (and the core
remuxes) only the window(s) at the playhead.

### Rust core

1. **`video/mod.rs`** — add `remux_hevc_to_mp4_bytes(src: &Path) -> Result<Vec<u8>>`:
   `std::fs::read(src)` → `remux::hevc_annexb_to_mp4(&input)`. No file write. (Leave
   `ensure_playable_mp4` in place; still used by `ensure_playable` for iOS/exports.)
2. **`ffi/mod.rs`** — add a **synchronous** exported method
   `remux_hd_bytes(device_id, drive_key, segment_num, kind) -> Result<Option<Vec<u8>>>`:
   - `Ok(None)` if `kind` isn't `FCamera|ECamera|DCamera`.
   - Resolve the single segment's mirrored path with the **same logic** as
     [`resolve_local_paths`](../../rust/core/src/ffi/mod.rs#L413-L447) (factor a sync helper:
     `repo.get_drive` → `MirrorStore::new(mirror_root.join(device_id))` → `file_rel(REALDATA_REL,…)`
     → `is_complete` → `final_path`); `Ok(None)` if not mirrored.
   - Else `Ok(Some(remux_hevc_to_mp4_bytes(&path)?))`.
   - **Sync** (no `async`/`spawn_blocking`): it's called from ExoPlayer's background loader thread,
     which must block on it. `repo.get_drive` is already a sync call. Returns `ByteArray?` in Kotlin.
   Bindings regenerate automatically during the Android Gradle build (cargo-ndk + in-workspace bindgen).

### Android

3. **New `ui/detail/HevcRemuxDataSource.java`** (Java, matching `TileMultiCamSelector.java`):
   - `HevcRemuxDataSource extends BaseDataSource`: `open(dataSpec)` parses the opaque URI →
     `(deviceId, driveKey, segNum, kindOrdinal)`, gets the full MP4 `byte[]` from a byte-bounded LRU
     (remux via the sync core call on miss — blocking on the loader thread is correct), serves
     `read()` from the held array honoring `dataSpec.position`/length, returns the length. Must hold
     the **full** bytes (ExoPlayer re-opens at byte offsets for in-window seeks). `close()` drops the
     array ref (LRU owns the bytes). Throw `IOException` on miss/parse-failure.
   - `HevcRemuxDataSource.Factory implements DataSource.Factory`, owning the
     `androidx.collection.LruCache<Key, byte[]>` (`sizeOf = value.length`,
     `maxSize = (maxMemory/8).coerceIn(64MB, 192MB)`) + a `remux(deviceId,driveKey,seg,kindOrdinal)->byte[]?`
     callable. Per-key miss-fill lock (small `ConcurrentHashMap` of monitors) so concurrent merge
     children don't double-remux the same key, without serializing different cameras.
4. **New `ui/detail/HdMediaUri.kt`** (pure Kotlin): opaque-URI `build`/`parse` for the four fields
   (URL-encode `driveKey`). Unit-testable without Android `Uri`.
5. **`MultiCamPlayer.kt`** — make sources lazy, drop eager remux:
   - `buildWindows()`: HD child = `ProgressiveMediaSource.Factory(hevcFactory).createMediaSource(
     MediaItem.fromUri(HdMediaUri.build(deviceId, driveKey, q.segmentNum, cam.kind.ordinal)))`,
     gated by **`CameraTrack.segmentNums`** (the camera has that segment). qcamera child stays
     `DefaultMediaSourceFactory` + `Uri.fromFile`. `MergingMediaSource(true,true,…)` per segment is
     unchanged. `windowVideoLayout`'s predicate switches from `resolvedHd[c]?.containsKey(s)` to the
     `segmentNums` check (same `windowVideoLayout` signature — [RouteClock.kt:67](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/RouteClock.kt#L67)).
   - `LaunchedEffect(enabled)`: **remove the `toMerge` remux loop**. First build is qcamera-only +
     default-enabled HD; on enable, rebuild windows for the merged set and
     `setMediaSources(w, savedIdx, savedOff)` + `prepare()` (preserves the current segment → only it
     remuxes). Keep a **grow-only `merged` set** so toggling a camera **off** stays a same-frame
     `applyVisibility()` (no rebuild), and toggling **on** a new camera rebuilds.
   - Drop the `resolvedHd` map and the `resolveHd` suspend param; add params `deviceId: Long`,
     `driveKey: String`, and the sync `remux` callable (build the `HevcRemuxDataSource.Factory` +
     LRU with `remember(qcamera, hdCameras)` so they're released on leaving the drive).
6. **`DashdownRepository.kt`** — add a **non-suspend** `remuxHdBytes(deviceId, driveKey, segmentNum, kind): ByteArray?`
   = `locator.core.remuxHdBytes(...)`. Deliberately *not* wrapped in `io {}` (the caller is already a
   background loader thread that must block) — document the exception to the dispatch rule.
7. **`DriveDetailScreen.kt`** — update the `ImmersivePlayer`→`MultiCamPlayer` call
   ([DriveDetailScreen.kt:202-208](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/DriveDetailScreen.kt#L202-L208)):
   stop passing `resolveHd = vm::ensurePlayable`; pass `deviceId`, `driveKey`, and `repo::remuxHdBytes`
   (thread `deviceId`/`driveKey` from `DriveDetailRoute`, which already has them).

### Why this works (validated)

`ExoPlayer.setMediaSources(~100 per-segment MergingMediaSources)` does **not** open all DataSources:
each `ProgressiveMediaSource` publishes a `C.TIME_UNSET` timeline until prepared (so the full-drive
scrubber keeps working off `DEFAULT_SEGMENT_MS` estimates, unchanged), and `MergingMediaSource`
creates its children's `MediaPeriod`s — which is what opens a `DataSource` — only when the
`MediaPeriodQueue` reaches that window (current + ~1 look-ahead). `seekTo(idx, off)` relocates the
queue, so a far seek remuxes only the target window. Time-to-first-frame becomes **one segment's
remux**, and seek latency is **one segment's remux** (or zero on an LRU hit).

## Files

- **Rust:** `rust/core/src/video/mod.rs` (new bytes helper), `rust/core/src/ffi/mod.rs` (sync
  `remux_hd_bytes` + a sync single-segment path resolver).
- **Android new:** `ui/detail/HevcRemuxDataSource.java`, `ui/detail/HdMediaUri.kt`.
- **Android edit:** `ui/detail/MultiCamPlayer.kt` (lazy sources, drop eager loop + `resolvedHd`),
  `data/DashdownRepository.kt` (sync `remuxHdBytes`), `ui/detail/DriveDetailScreen.kt` (call site).
- Reuse: `hevc_annexb_to_mp4`, `resolve_local_paths` body, `file_rel`/`final_path`/`is_complete`,
  `windowVideoLayout`, `CameraTrack.segmentNums`, `TileMultiCamSelector` (unchanged).

## Trade-offs & risks

- **No disk cache** (by design): re-seeking to an LRU-evicted window re-remuxes (~tens–hundreds of
  ms; the "Preparing HD…" spinner covers it). This is the explicit intent ("without creating a file").
- **Memory:** live ≈ LRU buffers + ExoPlayer's per-active-period `SampleQueue`s. Keep `maxSize`
  conservative (`maxMemory/8`, ≤192 MB) and verify no OOM on-device; lower if pressure shows.
- **Merge failure granularity:** a corrupt segment that *is* mirrored would fail its whole merged
  window (rare — `hevc_annexb_to_mp4` is robust on real footage). Missing `(cam,seg)` never reaches
  the DataSource (excluded by `windowVideoLayout`/`segmentNums`).

## Verification

- **Rust unit test** (`video/mod.rs`): write `remux::minimal_stream()` to a temp `.hevc`; assert
  `remux_hevc_to_mp4_bytes(&src)` == the bytes `ensure_playable_mp4` writes (byte-identical input to
  ExoPlayer whether file or memory). Plus: `remux_hd_bytes` returns `Ok(None)` for a non-HD kind.
- **Android pure tests:** `HdMediaUri` round-trip (incl. a `driveKey` with `--`/`|`); LRU byte-eviction
  via `androidx.collection.LruCache`. Existing `RouteClockTest` `windowVideoLayout` tests still pass.
- **Gates:** `:app:testDebugUnitTest` + `:app:ktfmtCheck`; `cargo test`/`fmt`/`clippy` for the core.
- **On-device (Pixel `192.168.1.210:5555`, comma at `192.168.1.181`, drive `0000004f` or a ~100-min
  drive, 3 cameras):**
  1. Open drive → enable 3 cameras: **time-to-first-frame ≈ one segment**, not ~40 s
     (screen-record the spinner clearing).
  2. **No `.hevc.mp4` files** appear under the mirror dir after playback (headline check —
     `tools/dd-db.sh` / `adb` to inspect; the new path never writes them).
  3. Seek to ~80%: target window plays within a short delay; intermediate windows did **not** remux
     (only current + look-ahead).
  4. Play across a 60 s window boundary: seamless, tiles stay frame-locked.
  5. Toggle cameras mid-play: enable shows within ~one segment; disable is instant; position preserved.
  6. No OOM over a few minutes of seeking.

## Out of scope

Streaming un-downloaded footage from copyparty on demand; iOS parity (the Rust `remux_hd_bytes` is
reusable when the iOS player lands); changing the remux algorithm or the qcamera path.
