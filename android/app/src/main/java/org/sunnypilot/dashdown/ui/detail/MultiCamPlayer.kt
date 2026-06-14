@file:OptIn(ExperimentalMaterial3Api::class, UnstableApi::class)

package org.sunnypilot.dashdown.ui.detail

import android.net.Uri
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.zIndex
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.Timeline
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.Renderer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.exoplayer.source.MediaSource
import androidx.media3.exoplayer.source.MergingMediaSource
import androidx.media3.exoplayer.source.ProgressiveMediaSource
import coil3.compose.AsyncImage
import coil3.request.ImageRequest
import coil3.request.crossfade
import coil3.video.videoFrameMillis
import java.io.File
import kotlin.math.roundToInt
import kotlinx.coroutines.delay
import uniffi.dashdown_core.FileKind

private const val FILMSTRIP_TICKS = 12
private const val TICK_MS = 100L

// Auto-hide the immersive controls this long after they appear while playing.
private const val CONTROLS_HIDE_MS = 3500L

// Representative camera aspect (comma cameras ≈ 1928×1208) used to size the tiles before the first
// frame reports the exact aspect; the tiles are then sized to it so the surfaces fill them with no
// gaps and no stretch.
private const val DEFAULT_TILE_ASPECT = 1928f / 1208f

// openpilot writes ~60 s segments. ExoPlayer reports a window's duration only once that segment
// buffers, so unbuffered windows read C.TIME_UNSET; we fall back to this estimate so the scrubber
// spans ALL segments from the start (replaced by the exact duration as each window prepares).
private const val DEFAULT_SEGMENT_MS = 60_000L

/**
 * The **multi-camera, drive-wide** player. ONE [ExoPlayer] drives N video renderers (one per camera
 * tile) plus the audio renderer from a single clock: the playlist is one [MergingMediaSource] per
 * segment (the enabled HD cameras + qcamera), so every visible tile and the audio are
 * **frame-locked by construction** — no master/follower, no corrective seeks, no decoder flushes.
 * Toggling a camera re-selects its renderer's track (same-frame, no seek) and frees/creates just
 * that HW decoder.
 *
 * HD cameras (road/wide/driver) are raw HEVC; each segment is remuxed to MP4 **in memory, lazily**,
 * by [HevcRemuxDataSource] as ExoPlayer reaches each window (no `.hevc.mp4` file is written) — a
 * tile shows a spinner until its first frame renders, and opening/seeking starts at the current
 * segment. The always-present qcamera carries the audio (opt-in) and the drive timeline/filmstrip,
 * and is the preview tile when no HD camera is on.
 */
@Composable
fun MultiCamPlayer(
    qcamera: List<QSegment>,
    hdCameras: List<CameraTrack>,
    deviceId: Long,
    driveKey: String,
    remuxBytes: (Long, String, UInt, FileKind) -> ByteArray?,
    modifier: Modifier = Modifier,
    onControlsVisibleChange: (Boolean) -> Unit = {},
) {
  val context = LocalContext.current

  // One player PER DRIVE, keyed on (deviceId, driveKey) — NOT on the camera/segment lists. As a
  // download lands more HD segments, `hdCameras`/`qcamera` change; the player must NOT be torn down
  // and rebuilt for that (it resets position to 0:00 and re-spins every tile). We update its
  // playlist in place instead (the delta effect below). N video renderers (MultiRenderersFactory) +
  // a custom selector that routes one merged video group to each renderer (per window). Released on
  // dispose.
  val factory =
      remember(deviceId, driveKey) { MultiRenderersFactory(context, VIDEO_RENDERER_COUNT) }
  val selector =
      remember(deviceId, driveKey) {
        TileMultiCamSelector(emptyList(), BooleanArray(VIDEO_RENDERER_COUNT), false)
      }
  val player =
      remember(deviceId, driveKey) {
        ExoPlayer.Builder(context).setRenderersFactory(factory).setTrackSelector(selector).build()
      }
  DisposableEffect(player) { onDispose { player.release() } }

  // HD source factory: each HD window is a ProgressiveMediaSource over an in-memory data source
  // that
  // remuxes that segment's raw HEVC to MP4 bytes on demand (no file written). The shared LRU lives
  // here, so it's released when leaving the drive.
  val hdSourceFactory =
      remember(deviceId, driveKey) {
        HevcRemuxDataSource.Factory(
            HdRemuxer { d, key, seg, kindOrd ->
              remuxBytes(d, key, seg.toUInt(), FileKind.entries[kindOrd])
            },
            lruMaxBytes(hdCameras.size),
        )
      }
  // Grow the remux-cache budget as the available HD-camera set settles (a drive opened mid-download
  // goes 0→1→2→3 cams) — sized to cache ~2 full N-camera windows. resize() keeps cached bytes, so
  // this never drops the cache (which would re-introduce the churn the player itself avoids).
  LaunchedEffect(hdCameras.size) { hdSourceFactory.resize(lruMaxBytes(hdCameras.size)) }

  // HD cameras merged into the playlist (grow-only): adding a camera rebuilds the playlist to
  // include its lazy sources; toggling one back off is a same-frame visibility change (no rebuild),
  // so its source stays merged. Which tiles actually show is driven by `enabled`/`visible`. All
  // player state is keyed on (deviceId, driveKey) so it survives `hdCameras`/`qcamera` growth
  // during
  // a download — only navigating to a different drive resets it.
  var mergedCams by remember(deviceId, driveKey) { mutableStateOf(emptyList<CameraId>()) }
  var initialized by remember(deviceId, driveKey) { mutableStateOf(false) }
  // Signature of the playlist last applied to the player; the delta effect rebuilds only when it
  // changes (a merged camera gained a segment, or qcamera grew), so a download flap can't churn it.
  var lastWindowsKey by remember(deviceId, driveKey) { mutableStateOf<List<Any>?>(null) }

  var enabled by remember(deviceId, driveKey) { mutableStateOf(defaultEnabled(hdCameras)) }
  // True once the user has touched a camera toggle, so the auto-enable effect never overrides their
  // choice when a camera's first segment arrives mid-download.
  var userTouchedEnabled by remember(deviceId, driveKey) { mutableStateOf(false) }
  var positionMs by remember(deviceId, driveKey) { mutableStateOf(0L) }
  var totalMs by remember(deviceId, driveKey) { mutableStateOf(0L) }
  var playing by remember(deviceId, driveKey) { mutableStateOf(false) }
  var audioOn by remember(deviceId, driveKey) { mutableStateOf(false) }
  var hasAudio by remember(player) { mutableStateOf(false) }
  // Per-renderer readiness (first frame rendered) — drives each tile's "Preparing HD…" spinner.
  val ready = remember(deviceId, driveKey) { mutableStateMapOf<Int, Boolean>() }
  // Per-renderer decoded display aspect (w/h), reported on first frame — sizes the tiles exactly.
  val tileAspect = remember(deviceId, driveKey) { mutableStateMapOf<Int, Float>() }
  // Tiling strategy: auto-default is the area-maximizing grid; the user can cycle to FEATURE.
  var strategy by remember(deviceId, driveKey) { mutableStateOf(DEFAULT_TILE_STRATEGY) }
  // Immersive controls: tap the video to reveal; auto-hides while playing. Reported up so the
  // screen
  // can show/hide its top chrome in lock-step.
  var controlsVisible by remember(deviceId, driveKey) { mutableStateOf(true) }
  LaunchedEffect(controlsVisible) { onControlsVisibleChange(controlsVisible) }
  // The user's drag-and-drop tile order (session-only); reconciled against the cameras currently
  // visible, so toggling a camera on/off doesn't lose a manual arrangement.
  var slotOrder by remember(deviceId, driveKey) { mutableStateOf(emptyList<VideoSlot>()) }

  // The drive-segment the playhead is in (windows are 1:1 with qcamera), published by the clock —
  // lets each tile tell "HD not downloaded for this segment" (a quiet preview) from "remuxing now".
  var currentSegmentNum by remember(deviceId, driveKey) { mutableStateOf<UInt?>(null) }
  // Downloaded segments per HD camera, for the per-tile NotDownloaded classification.
  val hdSegs = remember(hdCameras) { hdCameras.associate { it.id to it.segmentNums.toSet() } }

  val visible: List<VideoSlot> = orderedVisibleSlots(visibleSlots(enabled), slotOrder)

  // Build the playlist: one source per segment (the merged cameras present for it + qcamera), with
  // the matching per-window video-slot layout for the selector. qcamera is added last so its video
  // group follows the HD groups and its audio group feeds the audio renderer. HD children are LAZY
  // —
  // a ProgressiveMediaSource over the in-memory remux data source — so building all windows is
  // cheap
  // and ExoPlayer only remuxes the windows the playhead reaches (current + look-ahead).
  fun buildWindows(): Pair<List<MediaSource>, List<List<VideoSlot>>> {
    val qFactory = DefaultMediaSourceFactory(context)
    val hdFactory = ProgressiveMediaSource.Factory(hdSourceFactory)
    fun qsrc(path: String) = qFactory.createMediaSource(MediaItem.fromUri(Uri.fromFile(File(path))))
    fun hdsrc(cam: CameraId, seg: UInt) =
        hdFactory.createMediaSource(
            MediaItem.fromUri(HdMediaUri.build(deviceId, driveKey, seg, cam.kind.ordinal)))
    // Which segments each merged camera has downloaded (mirror presence), precomputed once.
    val segsOf: Map<CameraId, Set<UInt>> =
        mergedCams.associateWith { cam ->
          hdCameras.firstOrNull { it.id == cam }?.segmentNums?.toSet() ?: emptySet()
        }
    val windows = ArrayList<MediaSource>(qcamera.size)
    val layouts = ArrayList<List<VideoSlot>>(qcamera.size)
    for (q in qcamera) {
      val sources = ArrayList<MediaSource>()
      for (cam in mergedCams) if (q.segmentNum in (segsOf[cam] ?: emptySet())) {
        sources.add(hdsrc(cam, q.segmentNum))
      }
      sources.add(qsrc(q.path)) // qcamera last
      windows.add(
          if (sources.size == 1) sources[0]
          else MergingMediaSource(true, true, *sources.toTypedArray()))
      layouts.add(
          windowVideoLayout(mergedCams, q.segmentNum) { c, s -> s in (segsOf[c] ?: emptySet()) })
    }
    return windows to layouts
  }

  // The signature of the playlist the player currently shows (see [playlistSignature]); the delta
  // effect compares it to the last applied one so it skips a rebuild when an `hdCameras`/`qcamera`
  // change wouldn't alter the windows — which is what keeps a download flap from churning.
  fun windowsKey(): List<Any> = playlistSignature(qcamera, mergedCams, hdCameras)

  fun applyVisibility() {
    val v = BooleanArray(VIDEO_RENDERER_COUNT)
    visible.forEach { if (it.rendererIndex < v.size) v[it.rendererIndex] = true }
    selector.visibleRenderers = v
    selector.audioEnabled = audioOn
    selector.reselect()
  }

  fun seekGlobal(globalMs: Long) {
    val (idx, off) = locate(windowsOf(player), globalMs)
    player.seekTo(idx.coerceIn(0, (player.mediaItemCount - 1).coerceAtLeast(0)), off)
    positionMs = globalMs
  }

  // React to the enabled set. There is no eager remux anymore: building the playlist just wires up
  // lazy HD sources, so the player only remuxes the current window (+ look-ahead) and seeking
  // starts
  // at the seeked segment. First run merges the default-enabled camera so its current segment shows
  // immediately; enabling another camera rebuilds the playlist to add its lazy sources, preserving
  // position. Toggling a camera OFF stays a same-frame reselection (it remains merged but hidden).
  LaunchedEffect(enabled) {
    if (!initialized) {
      mergedCams = enabled.sortedBy { it.ordinal }
      val (w, l) = buildWindows()
      selector.windowLayouts = l
      applyVisibility()
      player.setMediaSources(w)
      player.prepare()
      lastWindowsKey = windowsKey()
      initialized = true
      return@LaunchedEffect
    }

    val toMerge = enabled.filter { it !in mergedCams }
    if (toMerge.isNotEmpty()) {
      // Rebuild the playlist with the newly-merged cameras' lazy sources, preserving position.
      val savedIdx = player.currentMediaItemIndex
      val savedOff = player.currentPosition
      mergedCams = (mergedCams + toMerge).distinct().sortedBy { it.ordinal }
      val (w, l) = buildWindows()
      selector.windowLayouts = l
      player.setMediaSources(w, savedIdx.coerceAtLeast(0), savedOff.coerceAtLeast(0))
      player.prepare()
      lastWindowsKey = windowsKey()
    }

    applyVisibility()
  }

  // Auto-enable the default camera when its first segment lands mid-download, so a drive opened as
  // qcamera-only doesn't stay preview-only once HD starts arriving — unless the user already chose
  // a camera set (then we never override it).
  LaunchedEffect(hdCameras) {
    if (!userTouchedEnabled && enabled.isEmpty() && hdCameras.isNotEmpty()) {
      enabled = defaultEnabled(hdCameras)
    }
  }

  // Apply newly-downloaded segments WITHOUT recreating the player. As a download lands HD/qcamera
  // segments, `hdCameras`/`qcamera` change; rebuild the playlist in place, preserving the current
  // segment + offset — but only when the windows actually change (snapshot compare), so a failed→
  // resume download flap (which re-runs the ViewModel's load() repeatedly) can't churn the player.
  // Already-remuxed windows are LRU hits, so this re-prepares only the current window.
  LaunchedEffect(qcamera, hdCameras) {
    if (!initialized) return@LaunchedEffect
    val key = windowsKey()
    if (key == lastWindowsKey) return@LaunchedEffect
    val savedIdx = player.currentMediaItemIndex
    val savedOff = player.currentPosition
    val (w, l) = buildWindows()
    selector.windowLayouts = l
    player.setMediaSources(w, savedIdx.coerceAtLeast(0), savedOff.coerceAtLeast(0))
    player.prepare()
    applyVisibility()
    lastWindowsKey = key
  }

  // Clock: publish the drive-global position + total every tick (smooth scrubber), and mirror each
  // renderer's "first frame" flag so tiles clear their spinner. No corrective seeks — one clock.
  LaunchedEffect(player) {
    while (true) {
      val windows = windowsOf(player)
      totalMs = windows.sum()
      positionMs = globalPosition(windows, player.currentMediaItemIndex, player.currentPosition)
      currentSegmentNum = qcamera.getOrNull(player.currentMediaItemIndex)?.segmentNum
      hasAudio = selector.sawAudio
      for (i in 0 until VIDEO_RENDERER_COUNT) {
        ready[i] = factory.stats[i].firstFrameRendered
        factory.stats[i].displayAspect().let { if (it > 0f) tileAspect[i] = it }
      }
      delay(TICK_MS)
    }
  }

  // Auto-hide the controls a few seconds after they appear while playing; any tap re-reveals them.
  LaunchedEffect(controlsVisible, playing) {
    if (controlsVisible && playing) {
      delay(CONTROLS_HIDE_MS)
      controlsVisible = false
    }
  }

  if (qcamera.isEmpty() && hdCameras.isEmpty()) {
    Box(modifier, contentAlignment = Alignment.Center) {
      Text("No playable video downloaded", Modifier.padding(16.dp))
    }
    return
  }

  Box(modifier.background(Color.Black)) {
    // Tiles fill the whole area (letterboxed, never cropped); tapping the video toggles the
    // controls.
    // Size all tiles to one representative aspect (the primary tile's, once known) so they tile
    // edge-to-edge with no gaps; comma cameras are uniform so this is also their true aspect.
    val gridAspect =
        visible.firstOrNull()?.let { tileAspect[it.rendererIndex] }?.takeIf { it > 0f }
            ?: DEFAULT_TILE_ASPECT
    TileGrid(
        slots = visible,
        strategy = strategy,
        tileAspect = gridAspect,
        onToggleControls = { controlsVisible = !controlsVisible },
        onReorder = { from, to -> slotOrder = swapSlots(visible, from, to) },
        modifier = Modifier.fillMaxSize().testTag("drive_detail_player"),
    ) { slot, tileModifier ->
      val slotState =
          tileStateFor(slot, currentSegmentNum, hdSegs, ready[slot.rendererIndex] == true)
      // For a segment a camera hasn't downloaded, show that segment's qcamera frame as a quiet
      // preview poster (one decode per segment) behind the badge — never an indefinite spinner.
      val previewModel =
          if (slotState == TileState.NotDownloaded) {
            qcamera
                .firstOrNull { it.segmentNum == currentSegmentNum }
                ?.path
                ?.let { p ->
                  remember(p) {
                    ImageRequest.Builder(context)
                        .data(File(p))
                        .videoFrameMillis(0L)
                        .crossfade(false)
                        .build()
                  }
                }
          } else {
            null
          }
      CameraTile(
          player = player,
          renderer = factory.videoRenderers[slot.rendererIndex],
          label = slotLabel(slot),
          state = slotState,
          previewModel = previewModel,
          modifier = tileModifier,
      )
    }

    // Tap-to-reveal control overlay, pinned to the bottom over a legibility scrim.
    AnimatedVisibility(
        visible = controlsVisible,
        enter = fadeIn(),
        exit = fadeOut(),
        modifier = Modifier.align(Alignment.BottomCenter),
    ) {
      Column(
          Modifier.fillMaxWidth()
              .background(Color.Black.copy(alpha = 0.45f))
              .padding(horizontal = 12.dp, vertical = 8.dp),
      ) {
        // Camera toggles (downloaded HD cameras) + the layout-strategy cycle.
        if (hdCameras.isNotEmpty()) {
          Row(
              verticalAlignment = Alignment.CenterVertically,
              horizontalArrangement = Arrangement.spacedBy(8.dp),
              modifier = Modifier.fillMaxWidth().testTag("camera_toggles"),
          ) {
            hdCameras.forEach { track ->
              FilterChip(
                  selected = track.id in enabled,
                  onClick = {
                    userTouchedEnabled = true
                    enabled = if (track.id in enabled) enabled - track.id else enabled + track.id
                  },
                  label = { Text(track.id.label) },
                  modifier = Modifier.testTag("camera_toggle_${track.id.label.lowercase()}"),
              )
            }
            Spacer(Modifier.weight(1f))
            if (visible.size > 1) {
              FilterChip(
                  selected = false,
                  onClick = { strategy = strategy.next() },
                  label = { Text(if (strategy == TileStrategy.FEATURE) "Feature" else "Grid") },
                  modifier = Modifier.testTag("drive_layout_toggle"),
              )
            }
          }
        }

        // Transport: play/pause, clock label, audio toggle.
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
        ) {
          IconButton(
              onClick = {
                playing = !playing
                player.playWhenReady = playing
              },
              modifier = Modifier.testTag("drive_play_toggle"),
          ) {
            if (playing) {
              val barColor = LocalContentColor.current
              Canvas(Modifier.size(20.dp)) {
                val barW = size.width * 0.24f
                val h = size.height * 0.78f
                val top = (size.height - h) / 2f
                val gap = size.width * 0.16f
                drawRect(
                    barColor,
                    topLeft = Offset(size.width / 2f - gap / 2f - barW, top),
                    size = Size(barW, h))
                drawRect(
                    barColor,
                    topLeft = Offset(size.width / 2f + gap / 2f, top),
                    size = Size(barW, h))
              }
            } else {
              Icon(Icons.Filled.PlayArrow, contentDescription = "play")
            }
          }
          Text(
              "${fmtTime(positionMs)} / ${fmtTime(totalMs)}",
              style = MaterialTheme.typography.bodySmall)
          Spacer(Modifier.weight(1f))
          if (hasAudio) {
            FilterChip(
                selected = audioOn,
                onClick = {
                  audioOn = !audioOn
                  applyVisibility() // audio is a track selection on the same clock — no seek
                },
                label = { Text("Audio") },
                modifier = Modifier.testTag("drive_audio_toggle"),
            )
          }
        }

        Slider(
            value = if (totalMs > 0) positionMs.toFloat().coerceIn(0f, totalMs.toFloat()) else 0f,
            valueRange = 0f..totalMs.coerceAtLeast(1L).toFloat(),
            enabled = totalMs > 0,
            onValueChange = { v -> seekGlobal(v.toLong()) },
            modifier = Modifier.fillMaxWidth().testTag("drive_scrubber"),
        )

        if (totalMs > 0 && qcamera.isNotEmpty()) {
          Filmstrip(qcamera.map { it.path }, windowsOf(player), totalMs) { t -> seekGlobal(t) }
        }
      }
    }
  }
}

/**
 * The signature `buildWindows()` actually branches on — each qcamera segment+path, and for each
 * MERGED camera which of those segments it has on disk. Equal signatures ⇒ an `hdCameras`/`qcamera`
 * change wouldn't alter the playlist, so the delta effect can skip a player rebuild. That guard is
 * what stops a failed→resume download flap (which re-runs the ViewModel's `load()` repeatedly) from
 * churning the player. Pure, so it is unit-tested.
 */
internal fun playlistSignature(
    qcamera: List<QSegment>,
    mergedCams: List<CameraId>,
    hdCameras: List<CameraTrack>,
): List<Any> =
    qcamera.map { it.segmentNum to it.path } +
        mergedCams.map { cam ->
          cam to (hdCameras.firstOrNull { it.id == cam }?.segmentNums?.toSet() ?: emptySet<UInt>())
        }

/** Default-on camera: road if present, else the first available HD, else none (qcamera preview). */
private fun defaultEnabled(hdCameras: List<CameraTrack>): Set<CameraId> =
    when {
      hdCameras.any { it.id == CameraId.ROAD } -> setOf(CameraId.ROAD)
      hdCameras.isNotEmpty() -> setOf(hdCameras.first().id)
      else -> emptySet()
    }

/** Per-window (segment) durations of the player's current playlist, for the drive-wide timeline. */
private fun windowsOf(player: ExoPlayer): LongArray {
  val tl = player.currentTimeline
  if (tl.isEmpty) return LongArray(0)
  val w = Timeline.Window()
  return LongArray(tl.windowCount) {
    val d = tl.getWindow(it, w).durationMs
    if (d == C.TIME_UNSET) DEFAULT_SEGMENT_MS else d
  }
}

/**
 * A tile's display state: the camera is rendering, still remuxing, or has no HD for this segment.
 */
enum class TileState {
  Ready,
  Preparing,
  NotDownloaded,
}

/**
 * Classify a tile for the current segment. A camera with no source this segment — past the download
 * frontier, or a segment the route never recorded it — is [TileState.NotDownloaded] (a quiet
 * preview, not a dead spinner); otherwise it is [TileState.Preparing] until its first frame
 * renders, then [TileState.Ready]. qcamera is present in every window, and an unknown segment
 * (before the first clock tick) is treated as present so a freshly-opened HD tile shows the
 * spinner, not the "not downloaded" poster. Pure, so it is unit-tested.
 */
internal fun tileStateFor(
    slot: VideoSlot,
    currentSegmentNum: UInt?,
    hdSegs: Map<CameraId, Set<UInt>>,
    ready: Boolean,
): TileState {
  val present =
      when (slot) {
        is VideoSlot.Hd ->
            currentSegmentNum == null || currentSegmentNum in (hdSegs[slot.id] ?: emptySet())
        VideoSlot.QcamVideo -> true
      }
  return when {
    !present -> TileState.NotDownloaded
    ready -> TileState.Ready
    else -> TileState.Preparing
  }
}

/**
 * A single camera surface (its own renderer) filling its box, with a label and — until ready — a
 * spinner, or a qcamera preview poster when this segment's HD isn't downloaded. The box is already
 * sized to the video aspect by [TileGrid]/[planTiles], so the surface fills it exactly — no crop,
 * no stretch, and no letterbox gap between adjacent tiles.
 */
@Composable
private fun CameraTile(
    player: ExoPlayer,
    renderer: Renderer,
    label: String,
    state: TileState,
    previewModel: ImageRequest? = null,
    modifier: Modifier = Modifier,
) {
  Box(modifier.background(Color.Black).testTag("drive_tile_${label.lowercase()}")) {
    AndroidView(
        // Raw SurfaceView routed to THIS renderer (not PlayerView, whose setVideoSurface would
        // broadcast to every video renderer). Re-routes on surface (re)creation, e.g. rotation.
        // Fills the box; the box already has the video's aspect, so the frame is undistorted.
        factory = { ctx ->
          SurfaceView(ctx).apply {
            holder.addCallback(
                object : SurfaceHolder.Callback {
                  override fun surfaceCreated(h: SurfaceHolder) {
                    player
                        .createMessage(renderer)
                        .setType(Renderer.MSG_SET_VIDEO_OUTPUT)
                        .setPayload(h.surface)
                        .send()
                  }

                  override fun surfaceChanged(h: SurfaceHolder, f: Int, w: Int, ht: Int) {}

                  override fun surfaceDestroyed(h: SurfaceHolder) {
                    player
                        .createMessage(renderer)
                        .setType(Renderer.MSG_SET_VIDEO_OUTPUT)
                        .setPayload(null)
                        .send()
                  }
                })
          }
        },
        modifier = Modifier.fillMaxSize(),
    )
    when (state) {
      TileState.Ready -> {}
      TileState.Preparing -> {
        // Present for this segment, first frame not yet rendered (the ~1.6 s in-memory remux).
        CircularProgressIndicator(Modifier.align(Alignment.Center).size(28.dp))
        Text(
            "Preparing HD…",
            style = MaterialTheme.typography.labelSmall,
            modifier = Modifier.align(Alignment.BottomCenter).padding(4.dp),
        )
      }
      TileState.NotDownloaded -> {
        // HD for this segment isn't on disk (past the download frontier, or a segment this route
        // never recorded the camera) — show the qcamera preview frame + a quiet badge, not a
        // dead spinner.
        if (previewModel != null) {
          AsyncImage(
              model = previewModel,
              contentDescription = null,
              contentScale = ContentScale.Crop,
              modifier = Modifier.fillMaxSize(),
          )
        }
        Text(
            "HD not downloaded",
            style = MaterialTheme.typography.labelSmall,
            modifier =
                Modifier.align(Alignment.BottomCenter)
                    .padding(4.dp)
                    .background(MaterialTheme.colorScheme.surface.copy(alpha = 0.6f))
                    .padding(horizontal = 4.dp),
        )
      }
    }
    Text(
        label,
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier =
            Modifier.align(Alignment.TopStart)
                .padding(4.dp)
                .background(MaterialTheme.colorScheme.surface.copy(alpha = 0.6f))
                .padding(horizontal = 4.dp),
    )
  }
}

/** Tile label: the HD camera's name, or "Preview" for the always-present qcamera tile. */
private fun slotLabel(slot: VideoSlot): String =
    if (slot is VideoSlot.Hd) slot.id.label else "Preview"

/**
 * Place each slot at the rectangle [planTiles] computes for the available space, [strategy], and
 * [tileAspect] — tiles are sized to the video aspect and placed touching (no gaps between them).
 * `render` receives the slot and an aspect-sized modifier the surface fills.
 *
 * Gestures live on one transparent layer above the tiles: a tap toggles the controls
 * ([onToggleControls]); a long-press lifts the tile under the finger and, on release over another
 * tile, calls [onReorder] to swap them. The live [SurfaceView]s never move during a drag (a moving
 * surface punches through overlays) — a lightweight labeled placeholder floats under the finger
 * instead, and hit-testing uses the engine's px rects so it works for GRID and FEATURE alike.
 */
@Composable
private fun TileGrid(
    slots: List<VideoSlot>,
    strategy: TileStrategy,
    tileAspect: Float,
    onToggleControls: () -> Unit,
    onReorder: (Int, Int) -> Unit,
    modifier: Modifier = Modifier,
    render: @Composable (VideoSlot, Modifier) -> Unit,
) {
  BoxWithConstraints(modifier) {
    val density = LocalDensity.current
    val haptics = LocalHapticFeedback.current
    val w = maxWidth
    val h = maxHeight
    val wPx = with(density) { w.toPx() }
    val hPx = with(density) { h.toPx() }
    val boxes =
        remember(slots.size, wPx, hPx, strategy, tileAspect) {
          planTiles(slots.size, wPx, hPx, tileAspect, strategy)
        }

    // Latest geometry/order read from inside the long-lived gesture coroutine *without* restarting
    // it. The drag pointerInput is keyed on Unit (never restarts), so an in-progress drag is never
    // stranded mid-gesture; these holders make each callback see the current rects, tile count, and
    // reorder callback. (Keying the gesture on a changing value instead let a drop compute against
    // a
    // stale snapshot — swapping the wrong tiles, or no-op after the first swap.)
    val hit =
        rememberUpdatedState<(Offset) -> Int> { p ->
          boxes.indexOfFirst {
            val x0 = it.xFrac * wPx
            val y0 = it.yFrac * hPx
            p.x in x0..(x0 + it.wFrac * wPx) && p.y in y0..(y0 + it.hFrac * hPx)
          }
        }
    val tileCount = rememberUpdatedState(slots.size)
    val reorder = rememberUpdatedState(onReorder)

    // Live tiles, each pinned to its engine rect. Untouched during a drag.
    slots.forEachIndexed { i, slot ->
      val b = boxes.getOrElse(i) { boxes.last() }
      key(slot.rendererIndex) {
        render(
            slot,
            Modifier.offset(x = w * b.xFrac, y = h * b.yFrac)
                .size(width = w * b.wFrac, height = h * b.hFrac),
        )
      }
    }

    var dragIndex by remember { mutableStateOf(-1) }
    var targetIndex by remember { mutableStateOf(-1) }
    var fingerPx by remember { mutableStateOf(Offset.Zero) }

    // One gesture layer over everything: tap → toggle controls; long-press + drag → reorder.
    Box(
        Modifier.matchParentSize()
            .pointerInput(Unit) { detectTapGestures { onToggleControls() } }
            .pointerInput(Unit) {
              detectDragGesturesAfterLongPress(
                  onDragStart = { pos ->
                    val idx = hit.value(pos)
                    if (idx >= 0 && tileCount.value > 1) {
                      dragIndex = idx
                      targetIndex = idx
                      fingerPx = pos
                      haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                    }
                  },
                  onDrag = { change, _ ->
                    if (dragIndex >= 0) {
                      change.consume()
                      fingerPx = change.position
                      targetIndex = hit.value(change.position)
                    }
                  },
                  onDragEnd = {
                    val n = tileCount.value
                    // Bounds-guard: a stale index can never swap the wrong (or a vanished) tile.
                    if (dragIndex in 0 until n &&
                        targetIndex in 0 until n &&
                        targetIndex != dragIndex) {
                      reorder.value(dragIndex, targetIndex)
                      haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                    }
                    dragIndex = -1
                    targetIndex = -1
                  },
                  onDragCancel = {
                    dragIndex = -1
                    targetIndex = -1
                  },
              )
            },
    )

    // Drag affordances: dim the lifted tile, outline the drop target, and float a labeled
    // placeholder under the finger (a regular view, so it composites above the SurfaceViews).
    if (dragIndex >= 0) {
      val src = boxes.getOrElse(dragIndex) { boxes.last() }
      Box(
          Modifier.offset(x = w * src.xFrac, y = h * src.yFrac)
              .size(width = w * src.wFrac, height = h * src.hFrac)
              .background(Color.Black.copy(alpha = 0.5f)))
      if (targetIndex >= 0 && targetIndex != dragIndex) {
        val tgt = boxes.getOrElse(targetIndex) { boxes.last() }
        Box(
            Modifier.offset(x = w * tgt.xFrac, y = h * tgt.yFrac)
                .size(width = w * tgt.wFrac, height = h * tgt.hFrac)
                .border(BorderStroke(3.dp, Color.White)))
      }
      val pw = w * src.wFrac
      val ph = h * src.hFrac
      val pwPx = with(density) { pw.toPx() }
      val phPx = with(density) { ph.toPx() }
      Box(
          Modifier.zIndex(1f)
              .offset {
                IntOffset(
                    (fingerPx.x - pwPx / 2f).roundToInt(), (fingerPx.y - phPx / 2f).roundToInt())
              }
              .size(width = pw, height = ph)
              .background(Color.Black.copy(alpha = 0.7f))
              .border(BorderStroke(2.dp, Color.White)),
          contentAlignment = Alignment.Center,
      ) {
        Text(
            slotLabel(slots[dragIndex]),
            color = Color.White,
            style = MaterialTheme.typography.titleMedium)
      }
    }
  }
}

/**
 * Evenly-spaced keyframe thumbnails across the whole drive (from qcamera); tap to seek to that
 * route time.
 */
@Composable
private fun Filmstrip(
    paths: List<String>,
    windows: LongArray,
    totalMs: Long,
    onSeek: (Long) -> Unit,
) {
  val context = LocalContext.current
  val ticks = remember(totalMs) { (0 until FILMSTRIP_TICKS).map { totalMs * it / FILMSTRIP_TICKS } }
  LazyRow(
      horizontalArrangement = Arrangement.spacedBy(4.dp),
      modifier = Modifier.fillMaxWidth().padding(top = 6.dp).testTag("drive_filmstrip"),
  ) {
    items(ticks) { t ->
      val (segIdx, offsetMs) = locate(windows, t)
      if (segIdx in paths.indices) {
        AsyncImage(
            model =
                ImageRequest.Builder(context)
                    .data(File(paths[segIdx]))
                    .videoFrameMillis(offsetMs)
                    .crossfade(false)
                    .build(),
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier =
                Modifier.size(width = 80.dp, height = 45.dp)
                    .clip(RoundedCornerShape(4.dp))
                    .background(MaterialTheme.colorScheme.surfaceVariant)
                    .clickable { onSeek(t) },
        )
      }
    }
  }
}
