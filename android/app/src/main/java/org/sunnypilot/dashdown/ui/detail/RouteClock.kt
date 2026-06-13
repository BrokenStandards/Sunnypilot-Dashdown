package org.sunnypilot.dashdown.ui.detail

import uniffi.dashdown_core.FileKind

/**
 * Pure, Compose-free helpers for the **single-player multi-camera** player. One `ExoPlayer` drives
 * N video renderers (one per camera tile) from a single clock — a playlist of per-segment
 * `MergingMediaSource`s — so every tile and the audio are frame-locked with no chase-seeks. These
 * helpers decide which camera maps to which renderer, the per-segment merge layout, the visible
 * tiles, and the tile arrangement. Kept free of Media3/Compose so it is plain-JVM unit-testable
 * (see `RouteClockTest`). Drive-wide timeline math lives next door in `RouteTimeline.kt`.
 */

/**
 * The comma HD cameras that can be toggled on as tiles. qcamera is not here — it is the
 * always-present clock/audio/filmstrip/preview source. [rendererIndex] pins each camera to a fixed
 * video renderer (and thus a fixed surface), so the camera↔renderer mapping is stable no matter
 * which cameras are currently merged or visible.
 */
enum class CameraId(val kind: FileKind, val label: String, val rendererIndex: Int) {
  ROAD(FileKind.F_CAMERA, "Road", 0),
  WIDE(FileKind.E_CAMERA, "Wide", 1),
  DRIVER(FileKind.D_CAMERA, "Driver", 2),
}

/** The video renderer that plays qcamera's own (low-res) video, shown only as the preview tile. */
const val QCAM_VIDEO_RENDERER_INDEX: Int = 3

/**
 * Number of video renderers the single player is built with: the 3 HD cameras + qcamera's video.
 */
const val VIDEO_RENDERER_COUNT: Int = 4

/**
 * An HD camera available for this drive plus the segment numbers downloaded for it (ordered) — the
 * player remuxes these lazily when the camera is first enabled, then merges them per segment.
 */
data class CameraTrack(val id: CameraId, val segmentNums: List<UInt>)

/** One qcamera segment: its segment number (for aligning HD cameras) and its on-disk `.ts` path. */
data class QSegment(val segmentNum: UInt, val path: String)

/**
 * A camera occupying a video-renderer slot within a window's (segment's) merge. Its [rendererIndex]
 * is fixed, so the same camera always decodes on the same renderer and draws to the same tile.
 */
sealed interface VideoSlot {
  val rendererIndex: Int

  data class Hd(val id: CameraId) : VideoSlot {
    override val rendererIndex: Int
      get() = id.rendererIndex
  }

  data object QcamVideo : VideoSlot {
    override val rendererIndex: Int
      get() = QCAM_VIDEO_RENDERER_INDEX
  }
}

/**
 * The ordered video slots whose track groups appear in one window's (segment's) merge — used to map
 * the k-th merged video group to the k-th slot's renderer. HD cameras (in canonical [CameraId]
 * order, only those merged AND present for this segment) come first, then qcamera's video last,
 * matching the order sources are added to the per-segment `MergingMediaSource`.
 */
fun windowVideoLayout(
    mergedCams: List<CameraId>,
    segmentNum: UInt,
    camHasSegment: (CameraId, UInt) -> Boolean,
): List<VideoSlot> {
  val hd = mergedCams.filter { camHasSegment(it, segmentNum) }.map { VideoSlot.Hd(it) }
  return hd + VideoSlot.QcamVideo
}

/**
 * The tiles to display: the enabled HD cameras (canonical order), or the qcamera preview tile when
 * no HD camera is enabled. An enabled camera still mid-remux gets a tile too (it shows a spinner
 * until its renderer has a frame) — its renderer simply isn't selected until it's merged.
 */
fun visibleSlots(enabled: Set<CameraId>): List<VideoSlot> {
  val hd = CameraId.entries.filter { it in enabled }.map { VideoSlot.Hd(it) }
  return hd.ifEmpty { listOf(VideoSlot.QcamVideo) }
}

/**
 * Apply the user's drag-and-drop [preferred] order to the currently-[visible] slots. Slots the user
 * has placed keep that relative order; any newly-visible slot not in [preferred] (e.g. a camera
 * just toggled on) is appended in canonical order; slots no longer visible are dropped. Because
 * each slot maps to a fixed renderer, this only permutes tile positions — the
 * camera↔renderer↔surface wiring is untouched. Pure so the reorder logic is unit-testable.
 */
fun orderedVisibleSlots(visible: List<VideoSlot>, preferred: List<VideoSlot>): List<VideoSlot> {
  val kept = preferred.filter { it in visible }
  val appended = visible.filter { it !in kept }
  return kept + appended
}

/**
 * Swap the slots at [i] and [j] — the drop reducer for drag-and-drop tile reordering. An equal or
 * out-of-range index pair returns [order] unchanged.
 */
fun swapSlots(order: List<VideoSlot>, i: Int, j: Int): List<VideoSlot> {
  if (i == j || i !in order.indices || j !in order.indices) return order
  return order.toMutableList().apply {
    this[i] = order[j]
    this[j] = order[i]
  }
}

// Tile arrangement now lives in TileLayout.kt (`planTiles`), which sizes each tile from the video
// aspect ratio and the device's available space instead of a fixed per-count plan.
