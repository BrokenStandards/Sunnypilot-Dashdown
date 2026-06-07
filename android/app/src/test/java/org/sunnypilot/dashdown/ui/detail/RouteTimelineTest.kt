package org.sunnypilot.dashdown.ui.detail

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pure-JVM tests for the drive-wide timeline mapping — RP2's headline: the player plays/seeks
 * across ALL of a drive's segments as one continuous timeline. Models two 1-minute segments → a
 * 2-min drive (matches the on-device check: a 2-segment drive shows a 1:58 total).
 */
class RouteTimelineTest {
  private val twoMin = longArrayOf(60_000L, 60_000L)

  @Test
  fun locateWithinFirstSegment() {
    assertEquals(0 to 30_000L, locate(twoMin, 30_000L))
  }

  @Test
  fun locateCrossesIntoSecondSegment() {
    assertEquals(1 to 30_000L, locate(twoMin, 90_000L)) // 1:30 global → segment 1 @ 0:30
  }

  @Test
  fun locateAtSegmentBoundaryStartsNextSegment() {
    assertEquals(1 to 0L, locate(twoMin, 60_000L))
  }

  @Test
  fun locatePastEndClampsToLastSegment() {
    val (idx, off) = locate(twoMin, 5_000_000L)
    assertEquals(1, idx)
    assertTrue("offset stays within the last segment", off in 0 until 60_000L)
  }

  @Test
  fun locateNegativeClampsToStart() {
    assertEquals(0 to 0L, locate(twoMin, -1_000L))
  }

  @Test
  fun locateEmptyTimelineIsOrigin() {
    assertEquals(0 to 0L, locate(longArrayOf(), 1_000L))
  }

  @Test
  fun globalPositionSumsPriorSegments() {
    assertEquals(90_000L, globalPosition(twoMin, 1, 30_000L)) // seg 1 @ 0:30 → 1:30 global
    assertEquals(15_000L, globalPosition(twoMin, 0, 15_000L))
  }

  @Test
  fun fmtTimeIsMinutesSeconds() {
    assertEquals("0:00", fmtTime(0L))
    assertEquals("0:05", fmtTime(5_000L))
    assertEquals("1:30", fmtTime(90_000L))
    assertEquals("1:58", fmtTime(118_000L))
  }
}
