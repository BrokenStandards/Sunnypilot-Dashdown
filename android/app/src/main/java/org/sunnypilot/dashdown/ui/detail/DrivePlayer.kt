@file:OptIn(ExperimentalMaterial3Api::class)

package org.sunnypilot.dashdown.ui.detail

import android.net.Uri
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PlayArrow
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.layout.ContentScale
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
import androidx.media3.ui.PlayerView
import coil3.compose.AsyncImage
import coil3.request.ImageRequest
import coil3.request.crossfade
import coil3.video.videoFrameMillis
import java.io.File
import kotlinx.coroutines.delay

private const val FILMSTRIP_TICKS = 12

/**
 * Continuous, **drive-wide** qcamera player. Plays ALL of a drive's qcamera segments as one
 * ExoPlayer playlist, and presents the scrubber + thumbnail filmstrip over the *whole* drive in
 * route milliseconds — play and seek cross 1-minute segment boundaries seamlessly, never
 * per-segment. The built-in controller is disabled in favor of these drive-wide controls. The audio
 * toggle appears only when the qcamera actually carries a track (sunnypilot `RecordAudio`).
 */
@Composable
fun DrivePlayer(paths: List<String>, modifier: Modifier = Modifier) {
  val context = LocalContext.current
  val player =
      remember(paths) {
        ExoPlayer.Builder(context).build().apply {
          setMediaItems(paths.map { MediaItem.fromUri(Uri.fromFile(File(it))) })
          playWhenReady = false
          volume = 0f // audio off until the user opts in (and only if a track exists)
          prepare()
        }
      }
  DisposableEffect(paths) { onDispose { player.release() } }

  // Per-segment (window) durations → a single cumulative drive timeline.
  var windowDurations by remember(paths) { mutableStateOf(longArrayOf()) }
  var positionMs by remember(paths) { mutableStateOf(0L) }
  var playing by remember(paths) { mutableStateOf(false) }
  var hasAudio by remember(paths) { mutableStateOf(false) }
  var audioOn by remember(paths) { mutableStateOf(false) }

  DisposableEffect(player) {
    val listener =
        object : Player.Listener {
          override fun onTimelineChanged(timeline: Timeline, reason: Int) {
            if (!timeline.isEmpty) {
              val w = Timeline.Window()
              windowDurations =
                  LongArray(timeline.windowCount) {
                    val d = timeline.getWindow(it, w).durationMs
                    if (d == C.TIME_UNSET) 0L else d
                  }
            }
          }

          override fun onTracksChanged(tracks: Tracks) {
            hasAudio = tracks.groups.any { it.type == C.TRACK_TYPE_AUDIO }
          }

          override fun onIsPlayingChanged(isPlaying: Boolean) {
            playing = isPlaying
          }
        }
    player.addListener(listener)
    onDispose { player.removeListener(listener) }
  }

  // Poll the global position (cumulative prior segments + position within the current one).
  LaunchedEffect(player) {
    while (true) {
      positionMs =
          globalPosition(windowDurations, player.currentMediaItemIndex, player.currentPosition)
      delay(250)
    }
  }

  val totalMs = remember(windowDurations) { windowDurations.sum() }

  Column(modifier) {
    AndroidView(
        factory = {
          PlayerView(it).apply {
            this.player = player
            useController = false
          }
        },
        modifier = Modifier.fillMaxWidth().aspectRatio(16f / 9f).testTag("drive_detail_player"),
    )
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
    ) {
      IconButton(
          onClick = { if (player.isPlaying) player.pause() else player.play() },
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
              player.volume = if (audioOn) 1f else 0f
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
        onValueChange = { v ->
          positionMs = v.toLong()
          seekGlobal(player, windowDurations, v.toLong())
        },
        modifier = Modifier.fillMaxWidth().testTag("drive_scrubber"),
    )
    if (totalMs > 0 && windowDurations.isNotEmpty()) {
      Filmstrip(paths, windowDurations, totalMs) { t -> seekGlobal(player, windowDurations, t) }
    }
  }
}

/** Evenly-spaced keyframe thumbnails across the whole drive; tap to seek to that time. */
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

private fun seekGlobal(player: ExoPlayer, windows: LongArray, globalMs: Long) {
  val (idx, offset) = locate(windows, globalMs)
  player.seekTo(idx, offset)
}
