# hdplayer-3 — qcamera fallback for HD tiles past the frontier / on camera gaps

_Part of [hdplayer-hardening](hdplayer-hardening.md). Effort: **S** (pure Compose/UI). Subsumes the
Area-4 genuine-gap UX._

## Problem

When the playhead enters a window where an **enabled** HD camera has no source — past the download
frontier, or a segment where the camera was never recorded (driver cam off mid-route) — that
renderer is disabled, `firstFrameRendered` stays `false`
([MultiRenderersFactory.java:94-96](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiRenderersFactory.java#L94-L96)),
and the tile shows an **indefinite "Preparing HD…" spinner with no video** — even though qcamera
exists for every window ([MultiCamPlayer.kt:214](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L214)).
This is **indistinguishable** in the UI from the genuine transient remux (~1.6 s), which uses the
same spinner.

Both causes reduce to one predicate: **`currentSegmentNum ∉ track.segmentNums`** for that tile's
camera. The fix treats them identically.

## Approach (Compose overlay — no media-graph change)

Do **not** route qcamera through an HD renderer (a renderer feeds one Surface; reusing the qcamera
group needs a second decoder + selector changes — see Alternatives). Classify the tile from data the
composable already has.

**1. Publish the current segment number.** In the clock loop
([MultiCamPlayer.kt:273-285](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L273-L285))
add `currentSegmentNum = qcamera.getOrNull(player.currentMediaItemIndex)?.segmentNum` (windows are
1:1 with qcamera, so `currentMediaItemIndex` maps straight to it — assert/comment this invariant).

**2. Classify a tri-state per slot** at the `CameraTile` call site
([:318-326](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L318-L326)):

```
enum class TileState { Ready, Preparing, NotDownloaded }
// QcamVideo slot: Ready iff ready[idx].
// Hd(id) slot: present = currentSegmentNum in hdCameras.first{it.id==id}.segmentNums.toSet()
//   !present           -> NotDownloaded   (frontier OR genuine gap — same handling)
//   present && !ready   -> Preparing       (real ~1.6 s remux)
//   present && ready    -> Ready
```

(The `segmentNums.toSet()` lookup is the same set `segsOf` already builds at
[:203-206](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L203-L206).)

**3. Branch the overlay.** Change `CameraTile`'s signature
([:459-466](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L459-L466))
to take `TileState` instead of `ready: Boolean`, and the overlay block
([:498-505](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L498-L505)):
- `Preparing` → keep the spinner + "Preparing HD…" (now it means what it says).
- `NotDownloaded` → **qcamera preview poster** (recommended) or a quiet hint, no spinner.
- `Ready` → no overlay.

**4. Poster (recommended).** Reuse the existing Coil `videoFrameMillis` path that `Filmstrip` already
uses ([:690-704](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L690-L704)):
render the qcamera frame at the current offset into the HD tile (`ContentScale` to fit the
HD-aspect box). No extra MediaCodec decoder, no frame-lock concern. A small "Preview" / "No driver
cam" badge keeps it honest. Minimal variant: static text only, no poster.

## Files

- **Edit** [MultiCamPlayer.kt](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt)
  only: clock loop (:273-285), `CameraTile` call site (:318-326) + composable (:459-517). No changes
  to `TileMultiCamSelector.java`, `MultiRenderersFactory.java`, the media graph, or remux.
- **Test** add a pure-JVM classifier test alongside
  [RouteClockTest.kt:36-61](../../android/app/src/test/java/org/sunnypilot/dashdown/ui/detail/RouteClockTest.kt#L36-L61).

## Alternatives (rejected/deferred)

- **Route qcamera into the empty HD tile via the selector** — true moving video, but needs a second
  qcamera `ProgressiveMediaSource` child + extra `VideoSlot` per absent-cam window (extra HW decoder,
  selector-invariant complexity, low-res upscale). Heaviest; not worth it over a poster.
- **Hide the absent tile** — reflows the grid (`planTiles`) as the playhead crosses holes; jarring.

## Risks / open questions

- **Copy/UX:** wording for `NotDownloaded` ("Preview only" vs "Not downloaded" vs "No driver cam").
  A truly live "downloading this exact segment now" needs `repo.progress` wired into the tile — out
  of scope; `segmentNums` is a snapshot.
- **Boundary flicker:** classification is from `segmentNums` (deterministic), not the volatile flag,
  so prefer it over `ready` to avoid a 1-tick spinner flash at window changes.
- **Invariant:** depends on windows staying 1:1 with qcamera (true today, :209) — assert it.

## Verification

- **On-device:** the 100-min drive with an HD prefix (this session's fixture) — seek past the HD
  edge → tile shows the **qcamera preview/badge**, no dead spinner; seek back into HD → real HD; the
  genuine remux still shows "Preparing HD…" for ~1.6 s only. If `hdplayer-1` found a Complete drive
  with a real `dcamera` hole, verify the driver tile shows the fallback exactly on those segments.
- **Gates:** `:app:testDebugUnitTest` + `:app:ktfmtCheck`.
