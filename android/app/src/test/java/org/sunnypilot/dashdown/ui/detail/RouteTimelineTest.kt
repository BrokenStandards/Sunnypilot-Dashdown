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

  // --- Scrub-bar math ---

  @Test
  fun coarseTargetMapsFractionOfWidth() {
    assertEquals(0L, coarseTargetMs(0f, 1000f, 120_000L))
    assertEquals(60_000L, coarseTargetMs(500f, 1000f, 120_000L)) // mid bar → mid drive
    assertEquals(120_000L, coarseTargetMs(1000f, 1000f, 120_000L))
  }

  @Test
  fun coarseTargetClampsOutOfRangeAndDegenerate() {
    assertEquals(0L, coarseTargetMs(-50f, 1000f, 120_000L))
    assertEquals(120_000L, coarseTargetMs(5000f, 1000f, 120_000L))
    assertEquals(0L, coarseTargetMs(500f, 0f, 120_000L)) // zero width
    assertEquals(0L, coarseTargetMs(500f, 1000f, 0L)) // empty drive
  }

  @Test
  fun fineSeekFactorEndpointsAndFloor() {
    assertEquals(1f, fineSeekFactor(0f, 200f), 1e-4f) // no pull → 1:1
    assertEquals(0.1f, fineSeekFactor(200f, 200f), 1e-4f) // full pull → floor
    assertEquals(0.1f, fineSeekFactor(400f, 200f), 1e-4f) // beyond full pull stays at floor
    assertEquals(0.55f, fineSeekFactor(100f, 200f), 1e-4f) // halfway → midpoint of 1.0..0.1
  }

  @Test
  fun fineSeekFactorIsMonotonicDecreasing() {
    var prev = fineSeekFactor(0f, 200f)
    for (px in 1..200) {
      val f = fineSeekFactor(px.toFloat(), 200f)
      assertTrue("factor never increases as pull-up grows", f <= prev + 1e-6f)
      prev = f
    }
  }

  @Test
  fun applyFineDeltaCoarseIsOneToOne() {
    // factor 1: dragging half the bar width moves half the drive.
    assertEquals(90_000L, applyFineDelta(60_000L, 250f, 1000f, 120_000L, 1f))
    assertEquals(30_000L, applyFineDelta(60_000L, -250f, 1000f, 120_000L, 1f))
  }

  @Test
  fun applyFineDeltaScalesAndClamps() {
    // factor 0.1: same drag moves a tenth → finer control.
    assertEquals(63_000L, applyFineDelta(60_000L, 250f, 1000f, 120_000L, 0.1f))
    // clamps at both ends.
    assertEquals(0L, applyFineDelta(10_000L, -5000f, 1000f, 120_000L, 1f))
    assertEquals(120_000L, applyFineDelta(110_000L, 5000f, 1000f, 120_000L, 1f))
  }

  @Test
  fun stripTicksCenteredAndClamped() {
    // Odd count → middle tick is exactly the center; spaced by spacingMs.
    assertEquals(
        listOf(58_000L, 59_000L, 60_000L, 61_000L, 62_000L),
        stripTicks(60_000L, 120_000L, 5, 1_000L))
    // Near the start clamps the left ticks to 0; near the end clamps to total.
    assertEquals(listOf(0L, 0L, 0L, 1_000L, 2_000L), stripTicks(0L, 120_000L, 5, 1_000L))
    assertEquals(
        listOf(118_000L, 119_000L, 120_000L, 120_000L, 120_000L),
        stripTicks(120_000L, 120_000L, 5, 1_000L))
    assertTrue(stripTicks(60_000L, 120_000L, 0, 1_000L).isEmpty())
  }
}
