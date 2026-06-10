package org.sunnypilot.dashdown.ui.detail

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Pure-JVM tests for the single-player multi-camera layout + mapping helpers. */
class RouteClockTest {

  @Test
  fun tilePlanSingle() {
    assertEquals(TilePlan.SINGLE, tilePlan(1, landscape = false))
    assertEquals(TilePlan.SINGLE, tilePlan(1, landscape = true))
  }

  @Test
  fun tilePlanTwoDependsOnOrientation() {
    assertEquals(TilePlan.STACK2, tilePlan(2, landscape = false))
    assertEquals(TilePlan.ROW2, tilePlan(2, landscape = true))
  }

  @Test
  fun tilePlanThreeIsPrimaryPlusTwo() {
    assertEquals(TilePlan.PRIMARY_BOTTOM2, tilePlan(3, landscape = false))
    assertEquals(TilePlan.PRIMARY_RIGHT2, tilePlan(3, landscape = true))
  }

  @Test
  fun tilePlanFourIsGrid() {
    assertEquals(TilePlan.GRID4, tilePlan(4, landscape = false))
    assertEquals(TilePlan.GRID4, tilePlan(4, landscape = true))
  }

  @Test
  fun tilePlanClampsOutOfRange() {
    assertEquals(TilePlan.SINGLE, tilePlan(0, landscape = false))
    assertEquals(TilePlan.GRID4, tilePlan(9, landscape = true))
  }

  @Test
  fun camerasPinToStableRenderers() {
    // The toggle bar covers exactly the three HD cameras, each on a distinct, fixed renderer.
    assertEquals(3, CameraId.entries.size)
    assertEquals(0, CameraId.ROAD.rendererIndex)
    assertEquals(1, CameraId.WIDE.rendererIndex)
    assertEquals(2, CameraId.DRIVER.rendererIndex)
    // qcamera's video renderer is distinct from all HD cameras, and all fit in
    // VIDEO_RENDERER_COUNT.
    val indices = CameraId.entries.map { it.rendererIndex } + QCAM_VIDEO_RENDERER_INDEX
    assertEquals(indices.size, indices.toSet().size)
    assertTrue(indices.all { it < VIDEO_RENDERER_COUNT })
  }

  @Test
  fun visibleSlotsAreEnabledHdCamerasInCanonicalOrder() {
    // Enabled out of canonical order still renders road-before-wide-before-driver.
    val slots = visibleSlots(setOf(CameraId.DRIVER, CameraId.ROAD))
    assertEquals(listOf(VideoSlot.Hd(CameraId.ROAD), VideoSlot.Hd(CameraId.DRIVER)), slots)
  }

  @Test
  fun visibleSlotsFallBackToQcameraPreview() {
    assertEquals(listOf<VideoSlot>(VideoSlot.QcamVideo), visibleSlots(emptySet()))
  }

  @Test
  fun windowLayoutOrdersMergedHdThenQcamera() {
    // Merged road+wide, both present this segment: video groups are [road, wide, qcam] in order.
    val layout =
        windowVideoLayout(listOf(CameraId.ROAD, CameraId.WIDE), segmentNum = 3u) { _, _ -> true }
    assertEquals(
        listOf(VideoSlot.Hd(CameraId.ROAD), VideoSlot.Hd(CameraId.WIDE), VideoSlot.QcamVideo),
        layout,
    )
  }

  @Test
  fun windowLayoutSkipsCamerasMissingThisSegment() {
    // Ragged download: wide lacks segment 7, so this window merges only road + qcamera.
    val layout =
        windowVideoLayout(listOf(CameraId.ROAD, CameraId.WIDE), segmentNum = 7u) { cam, _ ->
          cam == CameraId.ROAD
        }
    assertEquals(listOf(VideoSlot.Hd(CameraId.ROAD), VideoSlot.QcamVideo), layout)
  }

  @Test
  fun windowLayoutQcameraOnlyWhenNoHdMerged() {
    assertEquals(
        listOf<VideoSlot>(VideoSlot.QcamVideo), windowVideoLayout(emptyList(), 0u) { _, _ -> true })
  }
}
