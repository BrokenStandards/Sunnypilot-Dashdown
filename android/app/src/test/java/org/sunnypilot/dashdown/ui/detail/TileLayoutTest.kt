package org.sunnypilot.dashdown.ui.detail

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Pure-JVM geometry tests for [planTiles] across a matrix of device aspect ratios. */
class TileLayoutTest {

  private val ASPECT = 1.6f // comma cameras ≈ 1928×1208 ≈ 16:10
  private val EPS = 3e-3f

  // Representative containers (w,h in px): phone P/L, tablet P/L, square, foldable-ish.
  private val containers =
      listOf(
          1080f to 2400f,
          2400f to 1080f,
          1536f to 2048f,
          2048f to 1536f,
          1500f to 1500f,
          2208f to 1840f,
      )

  private fun overlaps(a: TileBox, b: TileBox): Boolean {
    val e = 1e-3f
    val xOverlap = a.xFrac < b.xFrac + b.wFrac - e && b.xFrac < a.xFrac + a.wFrac - e
    val yOverlap = a.yFrac < b.yFrac + b.hFrac - e && b.yFrac < a.yFrac + a.hFrac - e
    return xOverlap && yOverlap
  }

  private fun assertInBounds(boxes: List<TileBox>) {
    val e = 1e-3f
    boxes.forEach {
      assertTrue("x≥0", it.xFrac >= -e)
      assertTrue("y≥0", it.yFrac >= -e)
      assertTrue("w>0", it.wFrac > 0f)
      assertTrue("h>0", it.hFrac > 0f)
      assertTrue("x+w≤1", it.xFrac + it.wFrac <= 1f + e)
      assertTrue("y+h≤1", it.yFrac + it.hFrac <= 1f + e)
    }
  }

  private fun assertNoOverlap(boxes: List<TileBox>) {
    for (i in boxes.indices) for (j in i + 1 until boxes.size) {
      assertTrue("tiles must not overlap", !overlaps(boxes[i], boxes[j]))
    }
  }

  /** Every tile carries the video's aspect ratio (in px) → the surface fills it with no stretch. */
  private fun assertAspect(boxes: List<TileBox>, w: Float, h: Float) {
    boxes.forEach {
      val px = (it.wFrac * w) / (it.hFrac * h)
      assertEquals("tile must keep the video aspect", ASPECT.toDouble(), px.toDouble(), 0.01)
    }
  }

  @Test
  fun singleTileIsAspectCorrectAndCentered() {
    for ((w, h) in containers) {
      val b = planTiles(1, w, h, ASPECT).single()
      assertEquals(ASPECT.toDouble(), ((b.wFrac * w) / (b.hFrac * h)).toDouble(), 0.01)
      assertEquals("centered x", 0.5, (b.xFrac + b.wFrac / 2f).toDouble(), 1e-3)
      assertEquals("centered y", 0.5, (b.yFrac + b.hFrac / 2f).toDouble(), 1e-3)
    }
  }

  @Test
  fun gridTilesAspectCorrectInBoundsNoOverlap() {
    for ((w, h) in containers) for (n in 1..4) {
      val boxes = planTiles(n, w, h, ASPECT, TileStrategy.GRID)
      assertEquals("one box per tile", n, boxes.size)
      assertInBounds(boxes)
      assertNoOverlap(boxes)
      assertAspect(boxes, w, h)
    }
  }

  @Test
  fun twoTilesStackTouchingInPortrait() {
    // Tall container → stacked, equal, TOUCHING (no gap between them).
    val boxes = planTiles(2, 1080f, 2400f, ASPECT, TileStrategy.GRID)
    assertEquals(boxes[0].xFrac, boxes[1].xFrac, EPS)
    assertEquals(boxes[0].wFrac, boxes[1].wFrac, EPS)
    assertEquals("no vertical gap", boxes[0].yFrac + boxes[0].hFrac, boxes[1].yFrac, EPS)
  }

  @Test
  fun twoTilesSideBySideTouchingInLandscape() {
    // Wide container → side by side, equal, TOUCHING (no gap between them).
    val boxes = planTiles(2, 2400f, 1080f, ASPECT, TileStrategy.GRID)
    assertEquals(boxes[0].yFrac, boxes[1].yFrac, EPS)
    assertEquals(boxes[0].hFrac, boxes[1].hFrac, EPS)
    assertEquals("no horizontal gap", boxes[0].xFrac + boxes[0].wFrac, boxes[1].xFrac, EPS)
  }

  @Test
  fun fourTilesTouchInGrid() {
    // Square → 2×2; row-mates touch horizontally and rows touch vertically.
    val boxes = planTiles(4, 1500f, 1500f, ASPECT, TileStrategy.GRID)
    assertEquals(4, boxes.size)
    assertEquals("row tiles touch", boxes[0].xFrac + boxes[0].wFrac, boxes[1].xFrac, EPS)
    assertEquals("rows touch", boxes[0].yFrac + boxes[0].hFrac, boxes[2].yFrac, EPS)
  }

  @Test
  fun featureTilesAspectCorrectMainLargestAndTouching() {
    for ((w, h) in containers) for (n in 3..4) {
      val boxes = planTiles(n, w, h, ASPECT, TileStrategy.FEATURE)
      assertEquals(n, boxes.size)
      assertInBounds(boxes)
      assertNoOverlap(boxes)
      assertAspect(boxes, w, h)
      val mainArea = boxes[0].wFrac * boxes[0].hFrac
      assertTrue("main is the largest", boxes.drop(1).all { it.wFrac * it.hFrac < mainArea })
      if (w >= h) {
        assertEquals(
            "strip touches main's right", boxes[0].xFrac + boxes[0].wFrac, boxes[1].xFrac, EPS)
      } else {
        assertEquals(
            "strip touches main's bottom", boxes[0].yFrac + boxes[0].hFrac, boxes[1].yFrac, EPS)
      }
    }
  }

  @Test
  fun countIsClampedToOneThroughFour() {
    assertEquals(1, planTiles(0, 1080f, 2400f, ASPECT).size)
    assertEquals(4, planTiles(9, 1080f, 2400f, ASPECT).size)
  }
}
