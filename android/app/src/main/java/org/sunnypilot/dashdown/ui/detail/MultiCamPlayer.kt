@file:OptIn(ExperimentalMaterial3Api::class, UnstableApi::class)

package org.sunnypilot.dashdown.ui.detail

import android.net.Uri
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
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
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.Timeline
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.Renderer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.exoplayer.source.MediaSource
import androidx.media3.exoplayer.source.MergingMediaSource
import coil3.compose.AsyncImage
import coil3.request.ImageRequest
import coil3.request.crossfade
import coil3.video.videoFrameMillis
import java.io.File
import kotlinx.coroutines.delay
import uniffi.dashdown_core.FileKind

private const val FILMSTRIP_TICKS = 12
private const val TICK_MS = 100L

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
 * HD cameras (road/wide/driver) are raw HEVC; each segment is remuxed to MP4 lazily on first enable
 * (via [resolveHd]) and then merged in — a tile shows a spinner until its first frame renders. The
 * always-present qcamera carries the audio (opt-in) and the drive timeline/filmstrip, and is the
 * preview tile when no HD camera is on.
 */
@Composable
fun MultiCamPlayer(
    qcamera: List<QSegment>,
    hdCameras: List<CameraTrack>,
    resolveHd: suspend (FileKind, UInt) -> String?,
    modifier: Modifier = Modifier,
) {
  val context = LocalContext.current
  val landscape =
      LocalConfiguration.current.screenWidthDp > LocalConfiguration.current.screenHeightDp

  // One player, built once: N video renderers (MultiRenderersFactory) + a custom selector that
  // routes one merged video group to each renderer (per window). Released on dispose.
  val factory =
      remember(qcamera, hdCameras) { MultiRenderersFactory(context, VIDEO_RENDERER_COUNT) }
  val selector =
      remember(qcamera, hdCameras) {
        TileMultiCamSelector(emptyList(), BooleanArray(VIDEO_RENDERER_COUNT), false)
      }
  val player =
      remember(qcamera, hdCameras) {
        ExoPlayer.Builder(context).setRenderersFactory(factory).setTrackSelector(selector).build()
      }
  DisposableEffect(player) { onDispose { player.release() } }

  // HD cameras included in the per-segment merges (grows on first enable). Kept in canonical order.
  var mergedCams by remember(qcamera, hdCameras) { mutableStateOf(emptyList<CameraId>()) }
  // Cache of remuxed MP4 paths: camera -> (segmentNum -> mp4 path). Plain cache (not observed).
  val resolvedHd =
      remember(qcamera, hdCameras) { mutableMapOf<CameraId, MutableMap<UInt, String>>() }
  var initialized by remember(qcamera, hdCameras) { mutableStateOf(false) }

  var enabled by remember(hdCameras) { mutableStateOf(defaultEnabled(hdCameras)) }
  var positionMs by remember(qcamera, hdCameras) { mutableStateOf(0L) }
  var totalMs by remember(qcamera, hdCameras) { mutableStateOf(0L) }
  var playing by remember(qcamera, hdCameras) { mutableStateOf(false) }
  var audioOn by remember(qcamera, hdCameras) { mutableStateOf(false) }
  var hasAudio by remember(player) { mutableStateOf(false) }
  // Per-renderer readiness (first frame rendered) — drives each tile's "Preparing HD…" spinner.
  val ready = remember(qcamera, hdCameras) { mutableStateMapOf<Int, Boolean>() }

  val visible: List<VideoSlot> = visibleSlots(enabled)

  // Build the playlist: one source per segment (the merged cameras present for it + qcamera), with
  // the matching per-window video-slot layout for the selector. qcamera is added last so its video
  // group follows the HD groups and its audio group feeds the audio renderer.
  fun buildWindows(): Pair<List<MediaSource>, List<List<VideoSlot>>> {
    val mf = DefaultMediaSourceFactory(context)
    fun src(path: String) = mf.createMediaSource(MediaItem.fromUri(Uri.fromFile(File(path))))
    val windows = ArrayList<MediaSource>(qcamera.size)
    val layouts = ArrayList<List<VideoSlot>>(qcamera.size)
    for (q in qcamera) {
      val sources = ArrayList<MediaSource>()
      for (cam in mergedCams) resolvedHd[cam]?.get(q.segmentNum)?.let { sources.add(src(it)) }
      sources.add(src(q.path)) // qcamera last
      windows.add(
          if (sources.size == 1) sources[0]
          else MergingMediaSource(true, true, *sources.toTypedArray()))
      layouts.add(
          windowVideoLayout(mergedCams, q.segmentNum) { c, s ->
            resolvedHd[c]?.containsKey(s) == true
          })
    }
    return windows to layouts
  }

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

  // React to the enabled set: on first run set up the qcamera-only playlist (instant timeline +
  // audio + preview), then remux & merge any newly-enabled HD cameras (rebuilding the playlist
  // while
  // preserving position), and finally re-select tracks for the current visibility. Toggling an
  // already-merged camera skips straight to reselection (instant, same-frame).
  LaunchedEffect(enabled) {
    if (!initialized) {
      val (w, l) = buildWindows()
      selector.windowLayouts = l
      applyVisibility()
      player.setMediaSources(w)
      player.prepare()
      initialized = true
    }

    val toMerge = enabled.filter { it !in mergedCams }
    if (toMerge.isNotEmpty()) {
      for (cam in toMerge) {
        val track = hdCameras.firstOrNull { it.id == cam } ?: continue
        val map = resolvedHd.getOrPut(cam) { mutableMapOf() }
        for (seg in track.segmentNums) {
          if (seg in map) continue
          resolveHd(cam.kind, seg)?.let { map[seg] = it }
        }
      }
      // Rebuild the playlist with the newly-merged cameras, preserving the current position.
      val savedIdx = player.currentMediaItemIndex
      val savedOff = player.currentPosition
      mergedCams = (mergedCams + toMerge).distinct().sortedBy { it.ordinal }
      val (w, l) = buildWindows()
      selector.windowLayouts = l
      player.setMediaSources(w, savedIdx.coerceAtLeast(0), savedOff.coerceAtLeast(0))
      player.prepare()
    }

    applyVisibility()
  }

  // Clock: publish the drive-global position + total every tick (smooth scrubber), and mirror each
  // renderer's "first frame" flag so tiles clear their spinner. No corrective seeks — one clock.
  LaunchedEffect(player) {
    while (true) {
      val windows = windowsOf(player)
      totalMs = windows.sum()
      positionMs = globalPosition(windows, player.currentMediaItemIndex, player.currentPosition)
      hasAudio = selector.sawAudio
      for (i in 0 until VIDEO_RENDERER_COUNT) ready[i] = factory.stats[i].firstFrameRendered
      delay(TICK_MS)
    }
  }

  Column(modifier) {
    if (qcamera.isEmpty() && hdCameras.isEmpty()) {
      Text("No playable video downloaded", Modifier.padding(16.dp))
      return@Column
    }

    TileGrid(
        slots = visible,
        plan = tilePlan(visible.size, landscape),
        modifier = Modifier.fillMaxWidth().testTag("drive_detail_player"),
    ) { slot ->
      CameraTile(
          player = player,
          renderer = factory.videoRenderers[slot.rendererIndex],
          label = if (slot is VideoSlot.Hd) slot.id.label else "Preview",
          // The qcamera preview is ready as soon as it renders; HD tiles wait for their first
          // frame.
          ready = ready[slot.rendererIndex] == true,
      )
    }

    // Camera toggle bar (only the HD cameras downloaded for this drive).
    if (hdCameras.isNotEmpty()) {
      Row(
          horizontalArrangement = Arrangement.spacedBy(8.dp),
          modifier = Modifier.fillMaxWidth().padding(top = 8.dp).testTag("camera_toggles"),
      ) {
        hdCameras.forEach { track ->
          FilterChip(
              selected = track.id in enabled,
              onClick = {
                enabled = if (track.id in enabled) enabled - track.id else enabled + track.id
              },
              label = { Text(track.id.label) },
              modifier = Modifier.testTag("camera_toggle_${track.id.label.lowercase()}"),
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
                barColor, topLeft = Offset(size.width / 2f + gap / 2f, top), size = Size(barW, h))
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
              applyVisibility() // audio is a track selection on the same clock — no seek, no glitch
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
 * A single camera surface (its own renderer) with a label and a "preparing" spinner until ready.
 */
@Composable
private fun CameraTile(
    player: ExoPlayer,
    renderer: Renderer,
    label: String,
    ready: Boolean,
    modifier: Modifier = Modifier,
) {
  Box(
      modifier
          .fillMaxWidth()
          .aspectRatio(16f / 9f)
          .background(MaterialTheme.colorScheme.surfaceVariant)
          .testTag("drive_tile_${label.lowercase()}"),
  ) {
    AndroidView(
        // Raw SurfaceView routed to THIS renderer (not PlayerView, whose setVideoSurface would
        // broadcast to every video renderer). Re-routes on surface (re)creation, e.g. rotation.
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
    if (!ready) {
      CircularProgressIndicator(Modifier.align(Alignment.Center).size(28.dp))
      Text(
          "Preparing HD…",
          style = MaterialTheme.typography.labelSmall,
          modifier = Modifier.align(Alignment.BottomCenter).padding(4.dp),
      )
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

/** Arrange `slots` per [plan] using nested rows/columns; `render` draws each tile. */
@Composable
private fun TileGrid(
    slots: List<VideoSlot>,
    plan: TilePlan,
    modifier: Modifier = Modifier,
    render: @Composable (VideoSlot) -> Unit,
) {
  Column(modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
    when (plan) {
      TilePlan.SINGLE -> key(slots[0].rendererIndex) { render(slots[0]) }
      TilePlan.STACK2 -> {
        key(slots[0].rendererIndex) { render(slots[0]) }
        key(slots[1].rendererIndex) { render(slots[1]) }
      }
      TilePlan.ROW2 ->
          Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Box(Modifier.weight(1f)) { key(slots[0].rendererIndex) { render(slots[0]) } }
            Box(Modifier.weight(1f)) { key(slots[1].rendererIndex) { render(slots[1]) } }
          }
      TilePlan.PRIMARY_BOTTOM2 -> {
        key(slots[0].rendererIndex) { render(slots[0]) }
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
          Box(Modifier.weight(1f)) { key(slots[1].rendererIndex) { render(slots[1]) } }
          Box(Modifier.weight(1f)) { key(slots[2].rendererIndex) { render(slots[2]) } }
        }
      }
      TilePlan.PRIMARY_RIGHT2 ->
          Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Box(Modifier.weight(2f)) { key(slots[0].rendererIndex) { render(slots[0]) } }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
              key(slots[1].rendererIndex) { render(slots[1]) }
              key(slots[2].rendererIndex) { render(slots[2]) }
            }
          }
      TilePlan.GRID4 -> {
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
          Box(Modifier.weight(1f)) { key(slots[0].rendererIndex) { render(slots[0]) } }
          Box(Modifier.weight(1f)) { key(slots[1].rendererIndex) { render(slots[1]) } }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
          Box(Modifier.weight(1f)) { key(slots[2].rendererIndex) { render(slots[2]) } }
          Box(Modifier.weight(1f)) { key(slots[3].rendererIndex) { render(slots[3]) } }
        }
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
