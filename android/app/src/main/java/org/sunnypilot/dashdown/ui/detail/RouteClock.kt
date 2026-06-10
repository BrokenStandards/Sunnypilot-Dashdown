package org.sunnypilot.dashdown.ui.detail

import uniffi.dashdown_core.FileKind

/**
 * Pure, Compose-free helpers for the **multi-camera** player (RP3): which HD cameras exist, how
 * enabled tiles are arranged, and when a follower player has drifted far enough from the master
 * clock to need a re-seek. Kept free of Media3/Compose so it is plain-JVM unit-testable (see
 * `RouteClockTest`). The drive-wide timeline math lives next door in `RouteTimeline.kt`.
 */

/**
 * The comma HD cameras that can be toggled on as tiles. qcamera is not here — it is the
 * always-present clock/audio/filmstrip source, not a toggle.
 */
enum class CameraId(val kind: FileKind, val label: String) {
  ROAD(FileKind.F_CAMERA, "Road"),
  WIDE(FileKind.E_CAMERA, "Wide"),
  DRIVER(FileKind.D_CAMERA, "Driver"),
}

/**
 * An HD camera available for this drive plus the segment numbers downloaded for it (ordered) — the
 * player remuxes these lazily when the camera is enabled.
 */
data class CameraTrack(val id: CameraId, val segmentNums: List<UInt>)

/**
 * How the enabled tiles are laid out. The composable renders each plan with nested rows/columns;
 * this stays a pure value so the mapping is testable. `PRIMARY_*` give the first (primary) tile
 * extra area with the rest stacked beside/under it.
 */
enum class TilePlan {
  SINGLE, // 1 tile, full area
  STACK2, // 2 tiles, vertically stacked (portrait)
  ROW2, // 2 tiles, side by side (landscape)
  PRIMARY_BOTTOM2, // 3 tiles: primary on top, two below (portrait)
  PRIMARY_RIGHT2, // 3 tiles: primary on left, two stacked on the right (landscape)
  GRID4, // 4 tiles, 2x2
}

/**
 * Pick the tile arrangement for `n` enabled tiles in the given orientation. `n` is clamped to 1..4
 * (there are at most road/wide/driver + qcamera).
 */
fun tilePlan(n: Int, landscape: Boolean): TilePlan =
    when (n.coerceIn(1, 4)) {
      1 -> TilePlan.SINGLE
      2 -> if (landscape) TilePlan.ROW2 else TilePlan.STACK2
      3 -> if (landscape) TilePlan.PRIMARY_RIGHT2 else TilePlan.PRIMARY_BOTTOM2
      else -> TilePlan.GRID4
    }

/**
 * Followers are re-seeked to the master only past this drift (≈ one 20 fps frame), so we don't
 * fight ExoPlayer with constant micro-seeks.
 */
const val SYNC_THRESHOLD_MS: Long = 60L

/**
 * Whether a follower at `followerGlobalMs` has drifted from `masterGlobalMs` enough to warrant a
 * corrective seek.
 */
fun shouldResync(masterGlobalMs: Long, followerGlobalMs: Long): Boolean =
    kotlin.math.abs(masterGlobalMs - followerGlobalMs) > SYNC_THRESHOLD_MS
