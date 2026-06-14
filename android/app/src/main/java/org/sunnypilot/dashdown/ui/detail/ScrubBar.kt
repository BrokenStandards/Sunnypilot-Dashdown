package org.sunnypilot.dashdown.ui.detail

import android.graphics.Bitmap
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import kotlin.math.roundToInt
import kotlinx.coroutines.delay

private val BAR_TOUCH_HEIGHT = 28.dp
private val TRACK_HEIGHT = 3.dp
private val FINE_PULL_THRESHOLD = 24.dp // pull-up past this enters fine-seek + shows the strip
private val FULL_SCALE = 160.dp // pull-up distance that reaches the finest seek factor
private val OVERLAY_GAP = 10.dp

private const val STRIP_COUNT = 5
private const val STRIP_SPACING_MS = 1_000L // 1 s ≈ the comma GOP — the real thumbnail granularity
private val PREVIEW_W = 132.dp
private val PREVIEW_IMG_H = 74.dp
private val STRIP_THUMB_W = 64.dp
private val STRIP_THUMB_H = 38.dp

/**
 * A YouTube-style scrub bar that **replaces both the old `Slider` and the `Filmstrip`**: a thin
 * seek bar whose thumb tracks the finger, with a thumbnail preview that floats above while
 * scrubbing and — on **pull-up** — a filmstrip strip of nearby frames plus a finer (slower) seek.
 *
 * The player is NOT seeked during the drag (no decode thrash); the bar shows preview thumbnails
 * from [thumbAt] and commits a single [onSeek] on release. While a drag is in progress
 * [onScrubChange] `true` is raised so the owner suppresses clock-driven position updates (the bar
 * follows the finger, not the clock). The seek math is the pure, unit-tested helpers in
 * `RouteTimeline.kt` ([coarseTargetMs]/[applyFineDelta]/[fineSeekFactor]/[stripTicks]).
 */
@Composable
fun ScrubBar(
    positionMs: Long,
    totalMs: Long,
    thumbAt: (Long) -> Bitmap?,
    requestThumbs: (List<Long>) -> Unit,
    onScrubChange: (Boolean) -> Unit,
    onSeek: (Long) -> Unit,
    modifier: Modifier = Modifier,
) {
  val density = LocalDensity.current
  val fullScalePx = with(density) { FULL_SCALE.toPx() }
  val thresholdPx = with(density) { FINE_PULL_THRESHOLD.toPx() }

  var scrubTargetMs by remember { mutableStateOf<Long?>(null) }
  var pullUpPx by remember { mutableStateOf(0f) }
  var barWidthPx by remember { mutableStateOf(0) }
  // Bumped on a timer while scrubbing so a thumbnail that finishes decoding shows even if the
  // finger
  // is held still (no pointer event, hence no other recomposition, would otherwise re-read the
  // cache).
  var thumbTick by remember { mutableStateOf(0) }

  val scrubbing = scrubTargetMs != null
  val cap = totalMs.coerceAtLeast(0L)
  val displayMs = (scrubTargetMs ?: positionMs).coerceIn(0L, cap)
  val fineMode = scrubbing && pullUpPx > thresholdPx
  val frac = if (totalMs > 0L) displayMs.toFloat() / totalMs else 0f

  LaunchedEffect(scrubbing) {
    while (scrubbing) {
      delay(120)
      thumbTick++
    }
  }

  // Latest values read inside the long-lived (keyed-on-Unit) gesture coroutine without restarting
  // it.
  val totalState = rememberUpdatedState(totalMs)
  val onSeekState = rememberUpdatedState(onSeek)
  val onScrubState = rememberUpdatedState(onScrubChange)
  val requestState = rememberUpdatedState(requestThumbs)

  Box(modifier.fillMaxWidth().testTag("drive_scrubber")) {
    Canvas(
        Modifier.fillMaxWidth()
            .height(BAR_TOUCH_HEIGHT)
            .align(Alignment.BottomCenter)
            .onSizeChanged { barWidthPx = it.width }
            .pointerInput(Unit) {
              awaitEachGesture {
                val down = awaitFirstDown(requireUnconsumed = false)
                if (totalState.value <= 0L) return@awaitEachGesture
                val w = size.width.toFloat()
                var target = coarseTargetMs(down.position.x, w, totalState.value)
                var lastX = down.position.x
                val downY = down.position.y
                onScrubState.value(true)
                scrubTargetMs = target
                pullUpPx = 0f
                requestState.value(listOf(target))
                while (true) {
                  val ev = awaitPointerEvent()
                  val ch = ev.changes.firstOrNull { it.id == down.id }
                  if (ch == null || !ch.pressed) {
                    ch?.consume()
                    break
                  }
                  val dx = ch.position.x - lastX
                  lastX = ch.position.x
                  val pull = (downY - ch.position.y).coerceAtLeast(0f)
                  target =
                      applyFineDelta(
                          target, dx, w, totalState.value, fineSeekFactor(pull, fullScalePx))
                  pullUpPx = pull
                  scrubTargetMs = target
                  requestState.value(
                      if (pull > thresholdPx)
                          stripTicks(target, totalState.value, STRIP_COUNT, STRIP_SPACING_MS)
                      else listOf(target))
                  ch.consume()
                }
                onSeekState.value(target)
                onScrubState.value(false)
                scrubTargetMs = null
                pullUpPx = 0f
              }
            },
    ) {
      val cy = size.height / 2f
      val trackH = TRACK_HEIGHT.toPx()
      val r = CornerRadius(trackH / 2f, trackH / 2f)
      drawRoundRect(
          color = Color.White.copy(alpha = 0.3f),
          topLeft = Offset(0f, cy - trackH / 2f),
          size = Size(size.width, trackH),
          cornerRadius = r)
      drawRoundRect(
          color = Color.White,
          topLeft = Offset(0f, cy - trackH / 2f),
          size = Size(size.width * frac, trackH),
          cornerRadius = r)
      drawCircle(
          color = Color.White,
          radius = (if (scrubbing) 8.dp else 6.dp).toPx(),
          center = Offset(size.width * frac, cy))
    }

    if (scrubbing && barWidthPx > 0) {
      val thumbX = (frac * barWidthPx).roundToInt()
      if (fineMode) {
        ScrubStripPopup(displayMs, totalMs, thumbX, barWidthPx, thumbAt, thumbTick)
      } else {
        ScrubPreviewPopup(displayMs, thumbX, barWidthPx, thumbAt, thumbTick)
      }
    }
  }
}

/** Single magnified preview frame floating above the thumb (coarse scrub). */
@Composable
private fun ScrubPreviewPopup(
    targetMs: Long,
    thumbX: Int,
    barWidthPx: Int,
    thumbAt: (Long) -> Bitmap?,
    tick: Int,
) {
  val density = LocalDensity.current
  val wPx = with(density) { PREVIEW_W.roundToPx() }
  val hPx = with(density) { (PREVIEW_IMG_H + 18.dp).roundToPx() }
  val gapPx = with(density) { OVERLAY_GAP.roundToPx() }
  val x = (thumbX - wPx / 2).coerceIn(0, (barWidthPx - wPx).coerceAtLeast(0))
  val frame = remember(targetMs, tick) { thumbAt(targetMs) }
  Popup(
      alignment = Alignment.TopStart,
      offset = IntOffset(x, -(hPx + gapPx)),
      properties = PopupProperties(focusable = false, clippingEnabled = false),
  ) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
      ThumbFrame(frame, PREVIEW_W, PREVIEW_IMG_H, highlighted = true)
      Text(
          fmtTime(targetMs),
          color = Color.White,
          style = MaterialTheme.typography.labelMedium,
          fontWeight = FontWeight.SemiBold,
          modifier = Modifier.padding(top = 2.dp),
      )
    }
  }
}

/** Filmstrip strip of nearby frames floating above the bar (pull-up fine seek). */
@Composable
private fun ScrubStripPopup(
    targetMs: Long,
    totalMs: Long,
    thumbX: Int,
    barWidthPx: Int,
    thumbAt: (Long) -> Bitmap?,
    tick: Int,
) {
  val density = LocalDensity.current
  val gap = 3.dp
  val stripWpx =
      with(density) { (STRIP_THUMB_W * STRIP_COUNT + gap * (STRIP_COUNT - 1)).roundToPx() }
  val stripHpx = with(density) { (STRIP_THUMB_H + 20.dp).roundToPx() }
  val gapPx = with(density) { OVERLAY_GAP.roundToPx() }
  val x = (thumbX - stripWpx / 2).coerceIn(0, (barWidthPx - stripWpx).coerceAtLeast(0))
  val ticks =
      remember(targetMs, totalMs) { stripTicks(targetMs, totalMs, STRIP_COUNT, STRIP_SPACING_MS) }
  val frames = remember(ticks, tick) { ticks.map { it to thumbAt(it) } }
  Popup(
      alignment = Alignment.TopStart,
      offset = IntOffset(x, -(stripHpx + gapPx)),
      properties = PopupProperties(focusable = false, clippingEnabled = false),
  ) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
      Row(horizontalArrangement = Arrangement.spacedBy(gap)) {
        frames.forEachIndexed { i, (_, bmp) ->
          ThumbFrame(bmp, STRIP_THUMB_W, STRIP_THUMB_H, highlighted = i == STRIP_COUNT / 2)
        }
      }
      Text(
          fmtTime(targetMs),
          color = Color.White,
          style = MaterialTheme.typography.labelMedium,
          fontWeight = FontWeight.SemiBold,
          modifier = Modifier.padding(top = 2.dp),
      )
    }
  }
}

/** One thumbnail cell: the decoded frame (or a dark placeholder until it decodes), framed. */
@Composable
private fun ThumbFrame(bmp: Bitmap?, width: Dp, height: Dp, highlighted: Boolean) {
  val shape = RoundedCornerShape(4.dp)
  val borderColor = if (highlighted) Color.White else Color.White.copy(alpha = 0.5f)
  Box(
      Modifier.size(width, height)
          .clip(shape)
          .background(Color.Black.copy(alpha = 0.85f))
          .border(BorderStroke(if (highlighted) 2.dp else 1.dp, borderColor), shape)) {
        if (bmp != null) {
          Image(
              bitmap = bmp.asImageBitmap(),
              contentDescription = null,
              contentScale = ContentScale.Crop,
              modifier = Modifier.fillMaxSize(),
          )
        }
      }
}
