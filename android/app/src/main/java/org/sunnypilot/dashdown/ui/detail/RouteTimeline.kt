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
