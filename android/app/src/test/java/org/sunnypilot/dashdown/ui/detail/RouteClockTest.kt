package org.sunnypilot.dashdown.ui.detail

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Pure-JVM tests for the multi-camera layout + sync helpers (RP3). */
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
    // 0 enabled tiles can't happen (we always show ≥1) but must not crash.
    assertEquals(TilePlan.SINGLE, tilePlan(0, landscape = false))
    // More than 4 (shouldn't occur: only road/wide/driver + qcamera) → grid.
    assertEquals(TilePlan.GRID4, tilePlan(9, landscape = true))
  }

  @Test
  fun resyncOnlyPastOneFrame() {
    // Within ~1 frame (≤60 ms): leave it alone.
    assertFalse(shouldResync(1_000L, 1_000L))
    assertFalse(shouldResync(1_000L, 1_050L))
    assertFalse(shouldResync(1_000L, 940L))
    // Beyond the threshold (either direction): re-seek.
    assertTrue(shouldResync(1_000L, 1_200L))
    assertTrue(shouldResync(2_000L, 1_000L))
  }

  @Test
  fun cameraIdsMapToHevcKinds() {
    // The toggle bar covers exactly the three HD cameras.
    assertEquals(3, CameraId.entries.size)
    assertEquals("Road", CameraId.ROAD.label)
    assertEquals("Wide", CameraId.WIDE.label)
    assertEquals("Driver", CameraId.DRIVER.label)
  }
}
