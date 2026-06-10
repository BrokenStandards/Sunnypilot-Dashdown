@file:OptIn(ExperimentalMaterial3Api::class)

package org.sunnypilot.dashdown.ui.detail

import android.net.Uri
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
import androidx.media3.common.Player
import androidx.media3.common.Timeline
import androidx.media3.common.Tracks
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.AspectRatioFrameLayout
import androidx.media3.ui.PlayerView
import coil3.compose.AsyncImage
import coil3.request.ImageRequest
import coil3.request.crossfade
import coil3.video.videoFrameMillis
import java.io.File
import kotlinx.coroutines.delay
import uniffi.dashdown_core.FileKind

private const val FILMSTRIP_TICKS = 12
private const val TICK_MS = 120L

// openpilot writes ~60 s segments. ExoPlayer reports a `.ts` window's duration
// only once that segment buffers, so unbuffered windows read C.TIME_UNSET; we
// fall back to this estimate so the scrubber spans ALL segments from the start
// (it's replaced by the exact duration as each window prepares; HD MP4 windows
// always report exact durations immediately).
private const val DEFAULT_SEGMENT_MS = 60_000L

/** A visible tile: either an HD camera or the qcamera fallback (when no HD is on). */
private sealed interface Tile {
  data class Hd(val id: CameraId) : Tile

  data object Qcam : Tile
}

/**
 * The **multi-camera, drive-wide** player (RP3). The always-present `qcamera` stream is the
 * clock/audio/filmstrip source; the HD cameras (road/wide/driver) are toggled on as tiles, each its
 * own ExoPlayer fed the drive's segments as a continuous playlist and **kept frame-synced to the
 * master clock** — so toggling a camera shows the *same frame* and play/seek span all segments at
 * once. HD streams are raw HEVC, so each segment is remuxed to MP4 lazily on first enable (via
 * [resolveHd]); a tile shows a spinner until its first segment is ready.
 *
 * Audio (only the qcamera carries a track, when sunnypilot `RecordAudio` was on) is opt-in and
 * plays in sync with whatever camera is shown.
 */
@Composable
fun MultiCamPlayer(
    qcameraPaths: List<String>,
    hdCameras: List<CameraTrack>,
    resolveHd: suspend (FileKind, UInt) -> String?,
    modifier: Modifier = Modifier,
) {
  val context = LocalContext.current
  val landscape =
      LocalConfiguration.current.screenWidthDp > LocalConfiguration.current.screenHeightDp

  // One ExoPlayer for qcamera (clock/audio master) and one per available HD camera.
  val qPlayer =
      remember(qcameraPaths) {
        if (qcameraPaths.isEmpty()) null
        else
            ExoPlayer.Builder(context).build().apply {
              setMediaItems(qcameraPaths.map { MediaItem.fromUri(Uri.fromFile(File(it))) })
              playWhenReady = false
              volume = 0f
              prepare()
            }
      }
  val hdPlayers =
      remember(hdCameras) {
        hdCameras.associate { track ->
          track.id to
              ExoPlayer.Builder(context).build().apply {
                playWhenReady = false
                volume = 0f
              }
        }
      }
  DisposableEffect(qPlayer, hdPlayers) {
    onDispose {
      qPlayer?.release()
      hdPlayers.values.forEach { it.release() }
    }
  }

  // Lazily-remuxed MP4 paths per HD camera, cached so re-enabling is instant.
  val resolvedHd = remember(hdCameras) { mutableStateMapOf<CameraId, List<String>>() }

  var enabled by remember(hdCameras) { mutableStateOf(defaultEnabled(hdCameras)) }
  var positionMs by remember(qcameraPaths, hdCameras) { mutableStateOf(0L) }
  var totalMs by remember(qcameraPaths, hdCameras) { mutableStateOf(0L) }
  var playing by remember(qcameraPaths, hdCameras) { mutableStateOf(false) }
  var audioOn by remember(qcameraPaths, hdCameras) { mutableStateOf(false) }
  var hasAudio by remember(qPlayer) { mutableStateOf(false) }
  // Per-HD-camera readiness — true once the player has rendered its first frame.
  // The tile's "Preparing HD…" overlay reads this (Compose) state instead of the
  // player's `mediaItemCount`, which isn't observable and so never recomposed the
  // tile when the lazy remux finished (the spinner stuck until an unrelated toggle).
  val ready = remember(hdCameras) { mutableStateMapOf<CameraId, Boolean>() }

  // Visible tiles: enabled HD cameras (stable enum order), or qcamera if none on.
  val visible: List<Tile> =
      hdCameras
          .map { it.id }
          .filter { it in enabled }
          .map { Tile.Hd(it) }
          .ifEmpty { if (qPlayer != null) listOf(Tile.Qcam) else emptyList() }

  fun playerFor(t: Tile): ExoPlayer? =
      when (t) {
        is Tile.Hd -> hdPlayers[t.id]
        Tile.Qcam -> qPlayer
      }

  // The canonical drive timeline: qcamera if present (the lightweight, usually
  // most-complete stream), else the first visible HD player.
  val timelineSource = qPlayer ?: visible.firstOrNull()?.let { playerFor(it) }
  // The clock master drives the scrubber; followers chase it.
  val master = visible.firstOrNull()?.let { playerFor(it) }

  // Players that should actually be running: the visible tiles, plus the qcamera
  // (as a hidden audio source) when audio is on and it isn't already a tile.
  fun activePlayers(): List<ExoPlayer> {
    val list = visible.mapNotNull { playerFor(it) }.toMutableList()
    if (audioOn && qPlayer != null && visible.none { it == Tile.Qcam }) list.add(qPlayer)
    return list
  }

  fun seekGlobal(globalMs: Long) {
    val (idx, off) = locate(windowsOf(timelineSource), globalMs)
    activePlayers().forEach { p ->
      p.seekTo(idx.coerceIn(0, (p.mediaItemCount - 1).coerceAtLeast(0)), off)
    }
    positionMs = globalMs
  }

  // qcamera audio track presence (gates the Audio toggle).
  DisposableEffect(qPlayer) {
    val p = qPlayer ?: return@DisposableEffect onDispose {}
    val l =
        object : Player.Listener {
          override fun onTracksChanged(tracks: Tracks) {
            hasAudio = tracks.groups.any { it.type == C.TRACK_TYPE_AUDIO }
          }
        }
    p.addListener(l)
    onDispose { p.removeListener(l) }
  }

  // Clear each HD tile's "Preparing HD…" overlay the moment it renders a frame.
  DisposableEffect(hdPlayers) {
    val attached =
        hdPlayers.map { (id, player) ->
          val l =
              object : Player.Listener {
                override fun onRenderedFirstFrame() {
                  ready[id] = true
                }
              }
          player.addListener(l)
          Triple(id, player, l)
        }
    onDispose { attached.forEach { (_, player, l) -> player.removeListener(l) } }
  }

  // Keep qcamera muted/unmuted per the toggle.
  LaunchedEffect(audioOn, qPlayer) { qPlayer?.volume = if (audioOn) 1f else 0f }

  // Populate each enabled HD camera's playlist (lazy remux), then align it to the
  // current position + play state. Clearing on disable stops its decoder.
  hdCameras.forEach { track ->
    key(track.id) {
      val on = track.id in enabled
      LaunchedEffect(track.id, on) {
        if (!on) return@LaunchedEffect
        val player = hdPlayers[track.id] ?: return@LaunchedEffect
        val paths =
            resolvedHd[track.id]
                ?: track.segmentNums
                    .mapNotNull { resolveHd(track.id.kind, it) }
                    .also { resolvedHd[track.id] = it }
        if (player.mediaItemCount == 0 && paths.isNotEmpty()) {
          player.setMediaItems(paths.map { MediaItem.fromUri(Uri.fromFile(File(it))) })
          player.prepare()
          val (idx, off) = locate(windowsOf(timelineSource), positionMs)
          player.seekTo(idx.coerceIn(0, (paths.size - 1)), off)
          player.playWhenReady = playing
        }
      }
      DisposableEffect(track.id) { onDispose { hdPlayers[track.id]?.clearMediaItems() } }
    }
  }

  // Master clock: publish the global position + total, and re-seek any follower
  // (other tiles + the audio player) that has drifted more than ~1 frame.
  LaunchedEffect(visible, audioOn) {
    while (true) {
      val src = qPlayer ?: master
      if (src != null) {
        val w = windowsOf(src)
        totalMs = w.sum()
      }
      val m = master
      if (m != null) {
        val w = windowsOf(timelineSource)
        positionMs = globalPosition(w, m.currentMediaItemIndex, m.currentPosition)
        activePlayers()
            .filter { it !== m }
            .forEach { f ->
              val fg = globalPosition(w, f.currentMediaItemIndex, f.currentPosition)
              if (shouldResync(positionMs, fg)) {
                val (idx, off) = locate(w, positionMs)
                f.seekTo(idx.coerceIn(0, (f.mediaItemCount - 1).coerceAtLeast(0)), off)
              }
            }
      }
      delay(TICK_MS)
    }
  }

  Column(modifier) {
    if (visible.isEmpty()) {
      Text("No playable video downloaded", Modifier.padding(16.dp))
      return@Column
    }

    TileGrid(
        tiles = visible,
        plan = tilePlan(visible.size, landscape),
        modifier = Modifier.fillMaxWidth().testTag("drive_detail_player"),
    ) { tile ->
      CameraTile(
          tile = tile,
          player = playerFor(tile),
          // qcamera is prepared up front; HD tiles wait for their first rendered frame.
          ready = if (tile is Tile.Hd) ready[tile.id] == true else true,
      )
    }

    // Camera toggle bar (only the HD cameras that were downloaded for this drive).
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
            activePlayers().forEach { it.playWhenReady = playing }
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
          style = MaterialTheme.typography.bodySmall,
      )
      Spacer(Modifier.weight(1f))
      if (hasAudio) {
        FilterChip(
            selected = audioOn,
            onClick = { audioOn = !audioOn },
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

    if (totalMs > 0 && qcameraPaths.isNotEmpty()) {
      Filmstrip(qcameraPaths, windowsOf(qPlayer), totalMs) { t -> seekGlobal(t) }
    }
  }
}

/** Default-on camera: road if present, else the first available HD, else none. */
private fun defaultEnabled(hdCameras: List<CameraTrack>): Set<CameraId> =
    when {
      hdCameras.any { it.id == CameraId.ROAD } -> setOf(CameraId.ROAD)
      hdCameras.isNotEmpty() -> setOf(hdCameras.first().id)
      else -> emptySet()
    }

/** Per-window (segment) durations of a prepared player, as a cumulative-able array. */
private fun windowsOf(player: ExoPlayer?): LongArray {
  val p = player ?: return LongArray(0)
  val tl = p.currentTimeline
  if (tl.isEmpty) return LongArray(0)
  val w = Timeline.Window()
  return LongArray(tl.windowCount) {
    val d = tl.getWindow(it, w).durationMs
    if (d == C.TIME_UNSET) DEFAULT_SEGMENT_MS else d
  }
}

/** A single camera surface with a label and a "preparing" spinner until ready. */
@Composable
private fun CameraTile(
    tile: Tile,
    player: ExoPlayer?,
    ready: Boolean,
    modifier: Modifier = Modifier,
) {
  val label =
      when (tile) {
        is Tile.Hd -> tile.id.label
        Tile.Qcam -> "Preview"
      }
  Box(
      modifier
          .fillMaxWidth()
          .aspectRatio(16f / 9f)
          .background(MaterialTheme.colorScheme.surfaceVariant)
          .testTag("drive_tile_${label.lowercase()}"),
  ) {
    AndroidView(
        factory = {
          PlayerView(it).apply {
            useController = false
            resizeMode = AspectRatioFrameLayout.RESIZE_MODE_FIT
          }
        },
        update = { it.player = player },
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

/** Arrange `tiles` per [plan] using nested rows/columns; `render` draws each. */
@Composable
private fun TileGrid(
    tiles: List<Tile>,
    plan: TilePlan,
    modifier: Modifier = Modifier,
    render: @Composable (Tile) -> Unit,
) {
  Column(modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
    when (plan) {
      TilePlan.SINGLE -> render(tiles[0])
      TilePlan.STACK2 -> {
        render(tiles[0])
        render(tiles[1])
      }
      TilePlan.ROW2 ->
          Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Box(Modifier.weight(1f)) { render(tiles[0]) }
            Box(Modifier.weight(1f)) { render(tiles[1]) }
          }
      TilePlan.PRIMARY_BOTTOM2 -> {
        render(tiles[0])
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
          Box(Modifier.weight(1f)) { render(tiles[1]) }
          Box(Modifier.weight(1f)) { render(tiles[2]) }
        }
      }
      TilePlan.PRIMARY_RIGHT2 ->
          Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Box(Modifier.weight(2f)) { render(tiles[0]) }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
              render(tiles[1])
              render(tiles[2])
            }
          }
      TilePlan.GRID4 -> {
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
          Box(Modifier.weight(1f)) { render(tiles[0]) }
          Box(Modifier.weight(1f)) { render(tiles[1]) }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
          Box(Modifier.weight(1f)) { render(tiles[2]) }
          Box(Modifier.weight(1f)) { render(tiles[3]) }
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
