# hdplayer-2 — stop rebuilding the player while a drive downloads

_Part of [hdplayer-hardening](hdplayer-hardening.md). Effort: **M**._

## Problem

Playing a drive **while it is downloading** sticks the player at `0:00` with every tile showing
"Preparing HD…", clearing only when downloads stop. Root cause: the whole ExoPlayer (plus its
factory, selector, and all UI state) is `remember(qcamera, hdCameras)`-keyed
([MultiCamPlayer.kt:133-184](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L133-L184)),
and `CameraTrack`'s equality includes `segmentNums`
([RouteClock.kt:38](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/RouteClock.kt#L38)).
Each time `DriveDetailViewModel.load()` recomputes `hdCameras` from disk with more segments
([:91-97](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/DriveDetailViewModel.kt#L91-L97)),
the value changes → **the player is recreated from scratch** (new ExoPlayer at `0:00`,
`initialized=false`, fresh renderers all `firstFrameRendered=false`).

`load()` is **not** per-segment. It fires on init, on `terminalEvents`
([DriveDetailViewModel.kt:40](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/DriveDetailViewModel.kt#L40)),
and on `LifecycleResumeEffect`
([DriveDetailScreen.kt:83-86](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/DriveDetailScreen.kt#L83-L86)).
`ProgressBus` emits terminal only on per-drive `onCompleted`/`onFailed`
([ProgressBus.kt:50,58](../../android/app/src/main/java/org/sunnypilot/dashdown/core/ProgressBus.kt#L50-L58)).
So the *permanent* spinner is the **failed→resume flap**: each `onFailed`→`load()` rebuilds; the
service re-queues ([DownloadService.kt:59-66](../../android/app/src/main/java/org/sunnypilot/dashdown/service/DownloadService.kt#L59-L66))
→ another failure → another rebuild, forever.

## Approach

**1. Re-key the player and durable state on stable identity.** Change every
`remember(qcamera, hdCameras)` / `remember(hdCameras)` at
[MultiCamPlayer.kt:133-184](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L133-L184)
— `factory`, `selector`, **`player`**, `mergedCams`, `initialized`, `enabled`, `positionMs`,
`totalMs`, `playing`, `audioOn`, `ready`, `tileAspect`, `strategy`, `controlsVisible`, `slotOrder`
— to **`remember(deviceId, driveKey)`**, matching `hdSourceFactory`
([:149-157](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L149-L157))
which is already keyed that way. Leave `hasAudio` (`remember(player)`). The player now persists
across `hdCameras` growth and is recreated only on navigating to a different drive.

**2. Add a position-preserving delta handler.** Add `LaunchedEffect(qcamera, hdCameras)` that, once
`initialized`, applies new segments **without** recreating the player, reusing the exact pattern the
camera-toggle path already proves
([:259-265](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L259-L265)):

```
val savedIdx = player.currentMediaItemIndex
val savedOff = player.currentPosition
val (w, l) = buildWindows()
selector.windowLayouts = l            // MUST be set before prepare (TileMultiCamSelector indexes by period)
player.setMediaSources(w, savedIdx.coerceAtLeast(0), savedOff.coerceAtLeast(0))
player.prepare()
applyVisibility()
```

Guard it with a **snapshot compare** so it only fires when `buildWindows()` output would actually
change (a merged camera gained a segment, or qcamera grew) — keep a `remember`ed snapshot of
`mergedCams → segsOf` and of `qcamera.size`; skip the rebuild when unchanged. This keeps the single
re-prepare (which re-buffers only the current window) from firing on every `hdCameras` identity or
every `LifecycleResumeEffect` resume.

**3. Make the default-enable robust to late HD arrival.** `enabled` no longer resets on `hdCameras`
change, so initialize it from `defaultEnabled(hdCameras)`
([:165,436-441](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L436-L441));
if `hdCameras` is empty at first composition (qcamera-only at download start), auto-enable the
default camera once the first HD segment lands — `LaunchedEffect(hdCameras) { if (!userTouchedEnabled
&& enabled.isEmpty() && hdCameras.isNotEmpty()) enabled = defaultEnabled(hdCameras) }`. Track a
`userTouchedEnabled` flag so this never overrides a user who turned everything off.

**Why it's cheap:** the LRU is keyed by `HdMediaUri` and survives (already
`remember(deviceId, driveKey)`), so re-`setMediaSources` re-reads already-remuxed windows as **cache
hits** ([HevcRemuxDataSource.kt:72-74](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/HevcRemuxDataSource.kt#L72-L74));
only the just-arrived window legitimately remuxes (~1.6 s).

## Files

- **Edit** [MultiCamPlayer.kt](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt):
  re-key remembers (:133-184); add the delta `LaunchedEffect` + snapshot guard; `userTouchedEnabled`
  flag in the camera-toggle handler. Reuse `buildWindows`/`applyVisibility` (:195-230) verbatim.
- No Rust, ViewModel, or selector changes required (the VM already produces the right `hdCameras`).

## Risks / open questions

- **Re-prepare glitch:** a brief spinner on the *active* tile each time a delta applies; minimized by
  the snapshot guard. Acceptable.
- **qcamera growth:** qcamera is also a key today; the delta handler keys on both `qcamera` and
  `hdCameras`, and `buildWindows` iterates `qcamera` (:209), so a growing window count is handled —
  but verify `setMediaSources(list, savedIdx, savedOff)` with a longer list keeps position (coerce as
  existing code does).
- **Auto-enable timing:** confirm a camera whose first segment arrives mid-download should auto-appear
  (product call) vs. only extend already-enabled cameras.
- **Optional finer grain:** replace only changed windows via `removeMediaItem(i)+addMediaSources(i, w)`
  + `windowLayouts[i]` instead of whole-list `setMediaSources` — lower re-buffer surface, more
  bookkeeping; keep as a later optimization only if the single re-prepare proves visible.

## Verification

- **Unit:** existing `RouteClockTest` (`windowVideoLayout`) still green; add a pure test for the
  snapshot-compare "did the window set change?" helper.
- **On-device (emulator + real comma, 100-min drive with a partial HD download, flapping):** open the
  drive **while downloading** → total shows real length immediately, current segment renders HD,
  **no stuck `0:00`**; as later segments land, playback keeps position and only re-buffers briefly;
  seeking is unaffected. Compare against the pre-fix behavior captured this session.
- **Gates:** `:app:testDebugUnitTest` + `:app:ktfmtCheck`.
