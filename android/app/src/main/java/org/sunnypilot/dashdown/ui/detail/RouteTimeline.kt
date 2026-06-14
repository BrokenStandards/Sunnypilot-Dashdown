package org.sunnypilot.dashdown.ui.detail

/**
 * Pure math for the **drive-wide** playback timeline. The player concatenates a drive's qcamera
 * segments (each ~1 minute) into one ExoPlayer playlist; these helpers convert between a single
 * drive-global millisecond offset and the `(segmentIndex, offsetWithinSegment)` pair ExoPlayer
 * needs — so play and seek span ALL segments as one continuous timeline. Kept free of Compose /
 * Media3 so it is plain-JVM unit-testable (see `RouteTimelineTest`).
 */

/** Map a drive-global ms offset to `(segmentIndex, offsetWithinSegment)`, clamped into range. */
internal fun locate(windows: LongArray, globalMs: Long): Pair<Int, Long> {
  if (windows.isEmpty()) return 0 to 0L
  var remaining = globalMs.coerceAtLeast(0L)
  for (i in windows.indices) {
    val d = windows[i]
    // Land in this segment if the offset falls within it, or it's the last segment (clamp).
    if (remaining < d || i == windows.lastIndex) {
      return i to remaining.coerceIn(0L, (d - 1).coerceAtLeast(0L))
    }
    remaining -= d
  }
  return windows.lastIndex to 0L
}

/** Drive-global position = sum of prior segment durations + position within the current segment. */
internal fun globalPosition(
    windows: LongArray,
    segmentIndex: Int,
    positionInSegmentMs: Long
): Long {
  var prior = 0L
  for (i in 0 until segmentIndex.coerceIn(0, windows.size)) prior += windows[i]
  return prior + positionInSegmentMs.coerceAtLeast(0L)
}

/** `m:ss` clock label for a millisecond offset. */
internal fun fmtTime(ms: Long): String {
  val totalSec = (ms / 1000).coerceAtLeast(0L)
  return "%d:%02d".format(totalSec / 60, totalSec % 60)
}

// --- Scrub-bar math (YouTube-style seek). Pure so it is unit-tested in RouteTimelineTest. ---
//
// The scrubber tracks a target ms as the finger drags. On touch-down it jumps absolutely to the
// touched x ([coarseTargetMs]); every subsequent move adds a factor-scaled increment
// ([applyFineDelta]) so the thumb tracks the finger 1:1 when near the bar (factor 1) and moves
// finer the further the finger is pulled UP off the bar ([fineSeekFactor]) — one continuous,
// jump-free model with no discontinuity between coarse and fine seeking.

/** Default floor for [fineSeekFactor]: a full pull-up makes horizontal drag 10× finer. */
internal const val FINE_SEEK_FLOOR: Float = 0.1f

/**
 * Absolute map: pointer [xPx] within a bar of [widthPx] → drive-global ms, clamped to 0..totalMs.
 */
internal fun coarseTargetMs(xPx: Float, widthPx: Float, totalMs: Long): Long {
  if (widthPx <= 0f || totalMs <= 0L) return 0L
  val frac = (xPx / widthPx).coerceIn(0f, 1f)
  return (frac * totalMs).toLong().coerceIn(0L, totalMs)
}

/**
 * Fine-seek speed factor for a pull-up of [pullUpPx] (finger distance above the bar), reaching
 * [floor] at [fullScalePx]: 1.0 at no pull (1:1 seek) → [floor] (very fine) at/above full scale,
 * linear and monotonic between. Clamped to [floor]..1.
 */
internal fun fineSeekFactor(
    pullUpPx: Float,
    fullScalePx: Float,
    floor: Float = FINE_SEEK_FLOOR,
): Float {
  if (pullUpPx <= 0f) return 1f
  if (fullScalePx <= 0f) return floor
  val t = (pullUpPx / fullScalePx).coerceIn(0f, 1f)
  return (1f - t * (1f - floor)).coerceIn(floor, 1f)
}

/**
 * Incremental map: from [anchorMs], a horizontal drag of [dxPx] within a bar of [widthPx] moves
 * `dxPx/widthPx * totalMs` scaled by [factor] (≤1 = finer). Clamped to 0..totalMs.
 */
internal fun applyFineDelta(
    anchorMs: Long,
    dxPx: Float,
    widthPx: Float,
    totalMs: Long,
    factor: Float,
): Long {
  val cap = totalMs.coerceAtLeast(0L)
  if (widthPx <= 0f || totalMs <= 0L) return anchorMs.coerceIn(0L, cap)
  val deltaMs = (dxPx / widthPx) * totalMs * factor
  return (anchorMs + deltaMs.toLong()).coerceIn(0L, cap)
}

/**
 * The [count] thumbnail tick times for the pull-up filmstrip strip, centered on [centerMs] and
 * spaced [spacingMs] apart, each clamped to 0..totalMs. For an odd [count] the middle tick is
 * exactly [centerMs].
 */
internal fun stripTicks(centerMs: Long, totalMs: Long, count: Int, spacingMs: Long): List<Long> {
  if (count <= 0) return emptyList()
  val cap = totalMs.coerceAtLeast(0L)
  val half = (count - 1) / 2
  return (0 until count).map { i -> (centerMs + (i - half) * spacingMs).coerceIn(0L, cap) }
}
