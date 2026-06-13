package org.sunnypilot.dashdown.ui.detail

import kotlin.math.ceil
import kotlin.math.min

/**
 * Pure, Compose-free tile layout: given how many camera tiles to show, the container's size, and
 * the (fixed) video aspect ratio, decide each tile's rectangle. Every tile is sized to the video
 * aspect ([tileAspect]) and tiles are placed **edge-to-edge (touching) with the whole block
 * centered**, so the videos **never crop, never stretch, and have no gaps between them** — any
 * leftover space is a margin at the container's edge, never between two videos. Kept plain-JVM so
 * the geometry is unit-testable (see `TileLayoutTest`); the composable maps these fractions to px
 * and a surface fills each box.
 *
 * The default [TileStrategy.GRID] uses the video-conference "maximize fitted tile area" approach —
 * for each candidate column count it computes the resulting tile size and keeps the columns that
 * maximize it. Because comma cameras are wide (~16:10), this naturally **stacks in portrait and
 * sits side-by-side in landscape**. [TileStrategy.FEATURE] gives the first tile a larger share with
 * the rest in a touching strip beside/under it.
 */

/** A tile's rectangle as fractions of the container, in [0,1]. */
data class TileBox(val xFrac: Float, val yFrac: Float, val wFrac: Float, val hFrac: Float)

/**
 * GRID = equal tiles (max area). FEATURE = one large main tile + the rest in a touching side strip.
 */
enum class TileStrategy {
  GRID,
  FEATURE,
}

/** The default strategy: the area-maximizing grid. FEATURE is offered as a manual override. */
val DEFAULT_TILE_STRATEGY: TileStrategy = TileStrategy.GRID

/** Cycle strategies for the manual override control. */
fun TileStrategy.next(): TileStrategy =
    when (this) {
      TileStrategy.GRID -> TileStrategy.FEATURE
      TileStrategy.FEATURE -> TileStrategy.GRID
    }

/**
 * Lay out [count] tiles (clamped 1..4) of aspect [tileAspect] (= w/h) inside a
 * [containerW]×[containerH] box. Returns one [TileBox] per tile in input order; each box already
 * has the video's aspect ratio, so the caller's surface fills it exactly (no letterbox between
 * tiles).
 */
fun planTiles(
    count: Int,
    containerW: Float,
    containerH: Float,
    tileAspect: Float,
    strategy: TileStrategy = DEFAULT_TILE_STRATEGY,
): List<TileBox> {
  val n = count.coerceIn(1, 4)
  val w = containerW.coerceAtLeast(1f)
  val h = containerH.coerceAtLeast(1f)
  val a = tileAspect.coerceIn(0.1f, 10f)
  return when {
    n == 1 -> listOf(centered(w, h, fitTile(w, h, a), a))
    strategy == TileStrategy.FEATURE -> featurePlan(n, w, h, a)
    else -> gridPlan(n, w, h, a)
  }
}

/** The largest width for a tile of aspect [a] fitting a [cw]×[ch] cell. */
private fun fitTile(cw: Float, ch: Float, a: Float): Float = min(cw, ch * a)

/** A single tile of width [tileW] (aspect [a]) centered in the [w]×[h] container, as fractions. */
private fun centered(w: Float, h: Float, tileW: Float, a: Float): TileBox {
  val tileH = tileW / a
  return TileBox((w - tileW) / 2f / w, (h - tileH) / 2f / h, tileW / w, tileH / h)
}

/**
 * Equal, aspect-sized tiles in the area-maximizing grid, placed touching and centered as a block.
 */
private fun gridPlan(n: Int, w: Float, h: Float, a: Float): List<TileBox> {
  val landscape = w >= h
  // Choose the column count whose resulting tile is largest (max fitted area), biased by
  // orientation.
  var bestCols = 1
  var bestW = -1f
  for (cols in 1..n) {
    val rows = ceil(n / cols.toFloat()).toInt()
    val tw = fitTile(w / cols, h / rows, a)
    if (tw > bestW + 1e-3f ||
        (kotlin.math.abs(tw - bestW) <= 1e-3f && pref(cols, bestCols, landscape))) {
      bestW = tw
      bestCols = cols
    }
  }
  val cols = bestCols
  val rows = ceil(n / cols.toFloat()).toInt()
  val tileW = fitTile(w / cols, h / rows, a)
  val tileH = tileW / a
  val originY = (h - rows * tileH) / 2f // center the whole grid vertically
  val boxes = ArrayList<TileBox>(n)
  var placed = 0
  for (r in 0 until rows) {
    val inRow = min(cols, n - placed) // last row may hold fewer tiles; center it under the rest
    val originX = (w - inRow * tileW) / 2f
    for (c in 0 until inRow) {
      val x = originX + c * tileW
      val y = originY + r * tileH
      boxes.add(TileBox(x / w, y / h, tileW / w, tileH / h))
    }
    placed += inRow
  }
  return boxes
}

private fun pref(cols: Int, bestCols: Int, landscape: Boolean): Boolean =
    if (landscape) cols > bestCols else cols < bestCols

/** One large main tile (input[0]) + the remaining tiles in a touching strip beside/under it. */
private fun featurePlan(n: Int, w: Float, h: Float, a: Float): List<TileBox> {
  val landscape = w >= h
  val side = n - 1
  val boxes = ArrayList<TileBox>(n)
  if (landscape) {
    // Main fills the height on the left; strip is a touching column on the right.
    var mainH = h
    var mainW = mainH * a
    var stripH = mainH / side
    var stripW = stripH * a
    val blockW = mainW + stripW
    if (blockW > w) {
      val f = w / blockW
      mainH *= f
      mainW *= f
      stripH *= f
      stripW *= f
    }
    val originX = (w - (mainW + stripW)) / 2f
    val originY = (h - mainH) / 2f
    boxes.add(TileBox(originX / w, originY / h, mainW / w, mainH / h))
    for (i in 0 until side) {
      boxes.add(TileBox((originX + mainW) / w, (originY + i * stripH) / h, stripW / w, stripH / h))
    }
  } else {
    // Main fills the width on top; strip is a touching row underneath.
    var mainW = w
    var mainH = mainW / a
    var stripW = mainW / side
    var stripH = stripW / a
    val blockH = mainH + stripH
    if (blockH > h) {
      val f = h / blockH
      mainW *= f
      mainH *= f
      stripW *= f
      stripH *= f
    }
    val originX = (w - mainW) / 2f
    val originY = (h - (mainH + stripH)) / 2f
    boxes.add(TileBox(originX / w, originY / h, mainW / w, mainH / h))
    for (i in 0 until side) {
      boxes.add(TileBox((originX + i * stripW) / w, (originY + mainH) / h, stripW / w, stripH / h))
    }
  }
  return boxes
}
