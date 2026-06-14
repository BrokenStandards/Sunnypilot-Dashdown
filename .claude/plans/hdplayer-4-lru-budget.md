# hdplayer-4 — N-aware remux LRU budget

_Part of [hdplayer-hardening](hdplayer-hardening.md). Effort: **S**._

## Problem

The remux cache budget is a blind fraction of heap:
`lruMaxBytes() = (maxMemory/8).coerceIn(64MB, 192MB)`
([HevcRemuxDataSource.kt:155-158](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/HevcRemuxDataSource.kt#L155-L158)).
One HD-cam segment is ~37 MB, so a **3-cam window ≈ 111 MB > the 64 MB floor** — the LRU can't cache
even one full window. Every fresh seek / boundary-return re-remuxes (the ~1.6 s spinner). The budget
knows nothing about **N** (enabled cameras) or per-segment size.

Memory shape (worst case N=3): the current window's 3 open `DataSource`s pin ~111 MB (same arrays the
LRU counts), and at a boundary the next window's 3 cams remux → **~222 MB transient**. No
`android:largeHeap` is set ([AndroidManifest.xml:11-17](../../android/app/src/main/AndroidManifest.xml#L11-L17)),
so the app gets the OEM standard memory class; on the emulator `maxMemory/8` clamps to the 64 MB
floor.

## Approach

**1. Make `lruMaxBytes` N- and size-aware** ([HevcRemuxDataSource.kt:155-158](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/HevcRemuxDataSource.kt#L155-L158)):

```
fun lruMaxBytes(context: Context, camCount: Int, segMB: Int = 40): Int {
  val want = camCount.coerceAtLeast(1) * segMB * 2          // two full N-cam windows (current + look-ahead)
  val am = context.getSystemService<ActivityManager>()!!
  val classMB = if (largeHeapEnabled) am.largeMemoryClass else am.memoryClass
  val cap = (classMB / 3)                                   // leave room for ExoPlayer buffers, Coil, UI
  return want.coerceIn(2 * segMB, cap).coerceAtMost(256) * 1024 * 1024   // floor ~80MB, hard ceiling 256MB
}
```

Per-segment MP4 ≈ input `.hevc` (~37 MB) — grounded in
[remux.rs:84-176](../../rust/core/src/video/remux.rs#L84-L176) (NAL-rewrap, no decode); 40 MB is a
safe constant for uniform comma segments.

**2. Pass N at construction.** In `MultiCamPlayer`, `hdCameras.size` (available HD cams, ≤3) is known
where the Factory is built
([:149-157](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L149-L157)).
Size for the **available** count (worst case reachable without rebuilding the Factory), not the live
enabled count:

```
remember(deviceId, driveKey, hdCameras.size) {
  HevcRemuxDataSource.Factory(HdRemuxer { … }, HevcRemuxDataSource.lruMaxBytes(context, hdCameras.size))
}
```

> Note the interaction with [`hdplayer-2`](hdplayer-2-live-playlist.md): after step 2 re-keys the
> Factory on `(deviceId, driveKey)`, add `hdCameras.size` to that same key. Because `hdCameras.size`
> only changes when the *set* of available cameras changes (rare — a camera's first segment ever
> landing), the cache drops at most a few times early in a download, not per-segment. Land step 2
> first so this composes cleanly.

**3. Optional — `android:largeHeap="true"`** on `<application>`
([AndroidManifest.xml:11-17](../../android/app/src/main/AndroidManifest.xml#L11-L17)) so
`largeMemoryClass` (often 2–4× standard) is the cap on real devices like the test Pixel, giving room
for two full 3-cam windows without OOM. Pair with reading `largeMemoryClass` in the formula. Measure
GC impact before committing to it.

## Files

- **Edit** [HevcRemuxDataSource.kt](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/HevcRemuxDataSource.kt):
  `lruMaxBytes` signature + body (:155-158).
- **Edit** [MultiCamPlayer.kt:149-157](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L149-L157):
  add `hdCameras.size` to the Factory `remember` key; pass `context` + count.
- **Maybe** [AndroidManifest.xml](../../android/app/src/main/AndroidManifest.xml): `largeHeap`.

## Risks / open questions

- **OOM** if the cap fraction is too generous on a device without `largeHeap` — cap by the **memory
  class**, never unbounded `maxMemory/8`. Keep the hard 256 MB ceiling.
- **GC pressure:** 37 MB arrays land in large-object space; a bigger LRU keeps more alive — the hard
  ceiling bounds pause time. Measure on the Pixel + the 64 MB-floor emulator.
- **`DefaultLoadControl`** buffers ~50 s ahead across N renderers and competes for the same heap —
  that's why the cap is ~1/3 of the class, not 1/2.
- **Exact fraction** and whether to `largeHeap` project-wide need a measured pass; `segMB` could be
  `resize()`d from the first real remux size if the 40 MB estimate proves off (probably unnecessary).

## Verification

- **On-device:** the 3-cam fixture — seek away and back across several segments; with the bigger
  budget, **seek-back to a recently-played segment is an LRU hit (instant)**, not a re-remux; confirm
  via the `HevcRemux` `hit` vs `remux` logs. Watch `dumpsys meminfo` / no OOM over a few minutes of
  seeking on both the Pixel and the emulator.
- **Gates:** `:app:testDebugUnitTest` + `:app:ktfmtCheck`.
