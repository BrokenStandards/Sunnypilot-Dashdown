# Player controls redesign — center play/pause + YouTube-style thumbnail scrub bar

## Context

On a dark-themed Pixel the player's play/pause control is **near-invisible**: the play icon
([MultiCamPlayer.kt:497](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L497))
and the hand-drawn pause bars
([MultiCamPlayer.kt:481-495](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L481-L495))
tint from `LocalContentColor`, which resolves near-black on the 0.45 black scrim. The controls also
**waste vertical height**: the bottom overlay stacks a transport row (play/pause + clock + audio), a
Material3 `Slider`, AND a 12-thumb `Filmstrip` as three separate vertical bands
([MultiCamPlayer.kt:469-527](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L469-L527)).

Goal: a compact, legible control surface. Move play/pause to a **big white center button** revealed
by tapping the video; **merge the Slider + Filmstrip into one YouTube-style scrub bar** that shows a
thumbnail preview while scrubbing and, on **pull-up**, a filmstrip strip of nearby frames with a
finer (slower) seek. Thumbnails decode **in the background at low priority, cached in memory**, so
they never compete with playback.

Decisions confirmed with the user:
- **Tap = show/hide controls; play/pause is a big center button** (not a bare video tap). Long-press
  + drag still reorders tiles — unchanged.
- **Pull-up = filmstrip of nearby frames + slower seek** (not just a single magnified preview).

## What's already true (don't redo)

- `qcamera.ts` (low-res HEVC-in-MPEG-TS, audio muxed) is on disk per segment (`QSegment.path`) and is
  **already frame-decodable** — the current `Filmstrip` uses Coil `videoFrameMillis` on it
  ([MultiCamPlayer.kt:869-883](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L869-L883)),
  which uses `MediaMetadataRetriever` under the hood — so MMR on these files is proven.
- `locate(windows, globalMs) -> (segIdx, offsetMs)` and `globalPosition(...)` in
  [RouteTimeline.kt](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/RouteTimeline.kt)
  already map drive-global ms ↔ (segment, offset). Reuse for thumbnail lookup + scrub math.
- The MIN_PRIORITY daemon single-thread executor + byte-bounded `LruCache` pattern is established in
  [HevcRemuxDataSource.Factory](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/HevcRemuxDataSource.kt#L100-L135)
  (the prewarm thread) — the thumbnail cache mirrors it.
- `seekGlobal()` already prewarms the next segment's remux on a near-boundary seek
  ([MultiCamPlayer.kt:264-282](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L264-L282)).

## Approach

### 1. `ThumbnailCache.kt` (new) — background decode + in-memory cache

An instance **owned by the player** (created in `remember(deviceId, driveKey)`, released in a
`DisposableEffect`) — same lifetime model as the remux LRU, so MMRs/bitmaps free when leaving the
drive. NOT a global singleton.

- `ConcurrentHashMap<String, MediaMetadataRetriever>` — one MMR per qcamera path, opened lazily under
  a per-path lock, **accessed under `synchronized(mmr)`** (MMR is not thread-safe per instance).
- Decode via `getScaledFrameAtTime(offsetMs * 1000, OPTION_CLOSEST_SYNC, targetW, targetH)` —
  `CLOSEST_SYNC` snaps to the keyframe (fast; comma GOP ≈ 1 s so this is the real granularity
  anyway). Decode to ~256 px wide (16:9 ≈ 144 px tall), reused for both preview and strip.
- `LruCache<String, Bitmap>` keyed `"$path@${offsetMs / 1000 * 1000}"` (quantized to ~1 s = the GOP,
  so we never decode redundant frames), `sizeOf = bitmap.allocationByteCount`, **~24 MB budget**
  (modest, to coexist with the 80–256 MB remux LRU under `largeHeap`). **Do NOT recycle bitmaps on
  eviction** — Compose may still be drawing one; drop the strong ref and let GC reclaim (recycling a
  referenced bitmap crashes the draw).
- One MIN_PRIORITY daemon single-thread `Executor` for `prefetch(path, offsetMs)` (fire-and-forget;
  checks the cache first to coalesce). `get(path, offsetMs): Bitmap?` is cache-only (immediate, for
  the UI). `release()` releases every MMR and shuts the executor down.
- A tiny adapter the player passes to the scrub bar closes over `qcamera` paths + `windows`:
  `globalMs -> locate() -> path + offset -> cache.get/prefetch`.

### 2. `ScrubBar.kt` (new) — the merged seek bar + thumbnail scrubber

Replaces BOTH the `Slider` and the `Filmstrip`. Custom (Material3 `Slider` can't host the thumbnail
overlay or the pull-up gesture) — a `Canvas` track + `Modifier.pointerInput { awaitEachGesture { … } }`.

```
@Composable fun ScrubBar(
    positionMs: Long, totalMs: Long, windows: LongArray,
    thumbAt: (Long) -> Bitmap?,           // cache-only lookup
    requestThumbs: (List<Long>) -> Unit,  // fire-and-forget prefetch
    onScrubChange: (Boolean) -> Unit,     // raises/lowers isScrubbing
    onSeek: (Long) -> Unit,               // committed on release
    modifier: Modifier = Modifier)
```

- **Collapsed**: a thin track (~24–28 dp incl. touch target) — background + progress fill + thumb;
  follows `positionMs` from the clock when not scrubbing.
- **Coarse scrub** (touch-down + horizontal drag): `onScrubChange(true)`; thumb follows finger with an
  absolute map `targetMs = coarseTargetMs(x, width, totalMs)`; a single preview thumbnail bubble shows
  `thumbAt(targetMs)` above the thumb. **The player does not seek yet** (preview only — no decode
  thrash; YouTube-style).
- **Pull-up → fine + strip**: once the finger rises past a threshold, anchor the target and switch to an
  incremental map `applyFineDelta(anchor, dx, width, totalMs, fineSeekFactor(pullUpPx))`; show a
  horizontal **filmstrip strip** of `stripTicks(target, …)` frames (`thumbAt` each, `requestThumbs`
  the misses) centered on the target. Higher pull → smaller `fineSeekFactor` → finer seek.
- **Release**: `onSeek(targetMs)` (one real seek → existing `seekGlobal`, which also prewarms the next
  segment), `onScrubChange(false)`.
- **Tap** (down/up, no significant drag): jump-seek to the tapped x.
- Keep `testTag("drive_scrubber")` on the bar.

### 3. Pure scrub math in `RouteTimeline.kt` (extend) — unit-tested

Add small pure functions (plain JVM, no Compose/Media3) so they're covered by `RouteTimelineTest`:
- `coarseTargetMs(xPx, widthPx, totalMs): Long` — absolute map, clamped to `0..totalMs`.
- `fineSeekFactor(pullUpPx, fullScalePx): Float` — `1.0` at 0 pull → floor `~0.1` at `fullScalePx`,
  linear and monotonic, clamped.
- `applyFineDelta(anchorMs, dxPx, widthPx, totalMs, factor): Long` — incremental, clamped.
- `stripTicks(centerMs, totalMs, count, spacingMs): List<Long>` — strip frame times, centered,
  edge-clamped.

### 4. `MultiCamPlayer.kt` — wire it up, fix legibility, shrink the bar

- **State**: add `thumbnails = remember(deviceId, driveKey) { ThumbnailCache(...) }` +
  `DisposableEffect(thumbnails){ onDispose{ thumbnails.release() } }`; add
  `var isScrubbing by remember(deviceId, driveKey) { mutableStateOf(false) }`.
- **Clock loop** ([~349-362](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L349-L362)):
  skip the `positionMs` update while `isScrubbing` (don't fight the finger).
- **Auto-hide** ([~365-370](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L365-L370)):
  don't hide while `isScrubbing`.
- **Center play/pause button** (new): a large (~64 dp) button centered over the video on a
  semi-transparent circular scrim (`Color.Black.copy(alpha = 0.35f)`) for guaranteed contrast, white
  `Icons.Filled.PlayArrow` when paused / white hand-drawn bars when playing. Gate on `controlsVisible`
  as a sibling **after** `TileGrid` (so it sits above the tile gesture layer and receives the tap
  itself, while taps elsewhere still toggle controls). Keep `testTag("drive_play_toggle")` on it so
  the existing Maestro flow still finds a tap target.
- **Remove** the inline transport play/pause `IconButton` + Canvas
  ([~473-499](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L473-L499))
  — the worst legibility offender and a whole row of height.
- **Bottom overlay** now: camera-chips row (unchanged) → `ScrubBar(...)` → a compact row with the
  `m:ss / m:ss` clock (explicit `color = Color.White`) + the Audio chip. Drop the standalone
  `Filmstrip` band. Net: two stacked bands removed.
- The tile-tap (toggle controls) and long-press-drag (reorder) gesture layer
  ([~769-808](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L769-L808))
  is **unchanged**.

### 5. Tests + Maestro

- `RouteTimelineTest.kt`: cases for `coarseTargetMs` (end clamps), `fineSeekFactor` (endpoints,
  monotonic, clamp), `applyFineDelta` (anchor + scaled delta, clamp), `stripTicks` (count, centering,
  edge clamp).
- `android/maestro/play_drive.yaml`: **no change needed** — it taps `drive_play_toggle` then asserts
  the player is visible; the center button keeps that testTag and is present on open
  (`controlsVisible` starts true). The retired `drive_filmstrip` tag is referenced nowhere else
  (grep-confirmed).

## Out of scope / rejected

- Live-seeking the player on every drag pixel — rejected (decode thrash). Preview-only during drag,
  one commit on release.
- Coil prefetch for bulk thumbnails — rejected (its cache is composition-scoped; would churn and
  contend with playback). Disk-persisted thumbnails — deferred.
- Buffered-range rendering on the bar — skipped (quantizes poorly; little value here).
- No Rust core changes; no remux/timeline-window changes.

## Verification

- **Unit**: `./gradlew :app:testDebugUnitTest` (new `RouteTimelineTest` cases green) + `:app:ktfmtCheck`.
  Rust untouched, but run `cargo test`/`fmt`/`clippy` once to confirm no regression.
- **Emulator (real-comma footage — reliable screenshots)**: open the 100m drive and confirm — the
  control bar is visibly shorter; tapping the video shows/hides controls; the big white center button
  toggles play/pause and is clearly legible; dragging the scrub bar shows a live preview thumbnail and
  does not stutter playback; pulling up reveals the filmstrip strip and seeks finer; releasing commits
  exactly one seek; watch logcat to confirm the thumbnail executor runs on the background thread
  without dropping playback frames.
- **Pixel (the dark-theme device with the original bug)**: confirm the play/pause is now legible
  (white on scrim), the bar is shorter, and scrub + pull-up feel right (tune the pull-up threshold and
  `fineSeekFactor` floor on-device). Use `installDebug` (never `connectedAndroidTest`), inspect via
  mobile-mcp `list-elements`. Spot-check `getScaledFrameAtTime` on `qcamera.ts` works (low risk — Coil
  already decodes these via MMR).
- **Maestro**: `play_drive.yaml` stays green (center button keeps `drive_play_toggle`).

## Results (validated)

- **Gates**: `:app:compileDebugKotlin` + `:app:testDebugUnitTest` (new `RouteTimelineTest` scrub-math
  cases) + `:app:ktfmtCheck` all green. Rust untouched.
- **Emulator (real-comma footage, `f684dd21fc` 3-seg complete + HD decoding)**:
  - New layout confirmed via the accessibility tree on open (paused): center `drive_play_toggle`
    (168 px, screen-centered), compact `drive_scrubber` (74 px), white clock `0:00 / 2:59`, audio
    chip — the separate transport play/pause row and the filmstrip band are gone.
  - Coarse scrub: drag to ~80 % of the bar committed a seek to **2:23 / 2:58** (release-commit, exact).
  - Touch-down jumps absolutely to the touched x (~0:29); a diagonal **pull-up** drag landed at
    **1:20** — the rightward motion that would move ~1:48 at full speed was scaled to ~51 s (avg
    fine-factor ≈0.47 as the pull-up ramped 1.0→0.1). Fine-seek scaling works; **no crash** through
    the strip/Popup path.
  - Center button toggles play (controls then auto-hid while playing — the `isScrubbing` guard didn't
    break auto-hide).
  - Background `thumb-decode` MIN_PRIORITY thread runs; **zero `ThumbnailCache` warnings** → MMR
    `getScaledFrameAtTime` on raw `qcamera.ts` succeeds (the research's "critical" open question).
- **Pending — Pixel visual legibility**: the Compose control overlay doesn't composite into emulator
  screencaps (only the SurfaceView buffer does), so the white center button / white scrub-bar / white
  clock legibility on the dark theme — the original complaint — still needs an eyes-on pass on the
  Pixel (not connected during this build). Tune the pull-up threshold + `fineSeekFactor` floor there.
