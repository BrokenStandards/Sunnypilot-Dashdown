package org.sunnypilot.dashdown.ui.detail

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Pure-JVM tests for the single-player multi-camera layout + mapping helpers. */
class RouteClockTest {

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

  private val road = VideoSlot.Hd(CameraId.ROAD)
  private val wide = VideoSlot.Hd(CameraId.WIDE)
  private val driver = VideoSlot.Hd(CameraId.DRIVER)

  @Test
  fun swapSlotsExchangesTwoTiles() {
    // Drag road (0) onto driver (2): they trade places; the middle tile stays put.
    assertEquals(listOf(driver, wide, road), swapSlots(listOf(road, wide, driver), 0, 2))
  }

  @Test
  fun swapSlotsIsNoOpForEqualOrOutOfRange() {
    val order = listOf(road, wide)
    assertEquals(order, swapSlots(order, 1, 1)) // same index
    assertEquals(order, swapSlots(order, 0, 5)) // out of range
    assertEquals(order, swapSlots(order, -1, 0)) // out of range
  }

  @Test
  fun orderedVisibleKeepsUserOrderForStillVisibleSlots() {
    // User dragged wide before road; both still visible → that order is preserved.
    val preferred = listOf(wide, road)
    assertEquals(preferred, orderedVisibleSlots(listOf(road, wide), preferred))
  }

  @Test
  fun orderedVisibleAppendsNewlyEnabledCameraAtEnd() {
    // User order was [wide, road]; driver just toggled on → appended after the user-placed tiles.
    assertEquals(
        listOf(wide, road, driver),
        orderedVisibleSlots(listOf(road, wide, driver), listOf(wide, road)))
  }

  @Test
  fun orderedVisibleDropsSlotsNoLongerVisible() {
    // User order had driver, but it was toggled off → it disappears from the result.
    assertEquals(
        listOf(wide, road), orderedVisibleSlots(listOf(road, wide), listOf(driver, wide, road)))
  }

  // --- playlistSignature: the guard that lets the player update in place instead of rebuilding ---

  private val q01 = listOf(QSegment(0u, "/q0"), QSegment(1u, "/q1"))

  @Test
  fun playlistSignatureStableForEqualInputs() {
    val hd = listOf(CameraTrack(CameraId.ROAD, listOf(0u, 1u)))
    assertEquals(
        playlistSignature(q01, listOf(CameraId.ROAD), hd),
        playlistSignature(q01, listOf(CameraId.ROAD), hd))
  }

  @Test
  fun playlistSignatureChangesWhenMergedCameraGainsSegment() {
    // Road is merged and its download frontier advances (seg 1 lands) → the window set changes, so
    // the player must rebuild to gain road's HD child for seg 1.
    val before =
        playlistSignature(
            q01, listOf(CameraId.ROAD), listOf(CameraTrack(CameraId.ROAD, listOf(0u))))
    val after =
        playlistSignature(
            q01, listOf(CameraId.ROAD), listOf(CameraTrack(CameraId.ROAD, listOf(0u, 1u))))
    assertNotEquals(before, after)
  }

  @Test
  fun playlistSignatureUnchangedWhenUnmergedCameraGainsSegment() {
    // Wide downloads a segment but the user never enabled it (not merged) → the windows the player
    // shows are identical, so NO rebuild. This is the guard that stops a download flap from
    // churning.
    val merged = listOf(CameraId.ROAD)
    val before =
        playlistSignature(
            q01,
            merged,
            listOf(
                CameraTrack(CameraId.ROAD, listOf(0u, 1u)), CameraTrack(CameraId.WIDE, listOf(0u))))
    val after =
        playlistSignature(
            q01,
            merged,
            listOf(
                CameraTrack(CameraId.ROAD, listOf(0u, 1u)),
                CameraTrack(CameraId.WIDE, listOf(0u, 1u))))
    assertEquals(before, after)
  }

  @Test
  fun playlistSignatureChangesWhenQcameraGrows() {
    // A new qcamera segment mirrors → one more window; the player must extend its playlist.
    val hd = listOf(CameraTrack(CameraId.ROAD, listOf(0u)))
    val before = playlistSignature(listOf(QSegment(0u, "/q0")), listOf(CameraId.ROAD), hd)
    assertNotEquals(before, playlistSignature(q01, listOf(CameraId.ROAD), hd))
  }
}
