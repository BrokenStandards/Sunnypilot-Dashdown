@file:OptIn(UnstableApi::class)

package org.sunnypilot.dashdown.spike

import android.app.Activity
import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.ViewGroup
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import androidx.media3.common.MediaItem
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.Renderer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.exoplayer.source.MediaSource
import androidx.media3.exoplayer.source.MergingMediaSource
import java.io.File

/**
 * SPIKE A0 (throwaway, debug-only) — the go/no-go gate for the single-player multi-camera rewrite.
 *
 * Proves on real hardware whether ONE [ExoPlayer] with N video renderers ([MultiRenderersFactory])
 * + a custom [TileTrackSelector] + a [MergingMediaSource] can render N camera tiles **frame-locked
 * under one clock** (no chase-seeks), play [qcamera] audio on the same clock, and **release a tile's
 * decoder** when its track is deselected. We measure per-tile [androidx.media3.exoplayer.DecoderCounters]
 * (rendered/dropped frames) to quantify "smooth".
 *
 * Launch (reads cached MP4s straight from the app mirror; tiles first, qcamera audio appended last):
 * ```
 * adb shell am start -n org.sunnypilot.dashdown/.spike.SpikeMultiCamActivity \
 *   --esa videos /sdcard/Android/data/org.sunnypilot.dashdown/files/dashdown/mirror/2/routes/00000043--050c69d7d8--0/fcamera.hevc.mp4,/sdcard/.../ecamera.hevc.mp4 \
 *   --es audio  /sdcard/.../00000043--050c69d7d8--0/qcamera.ts
 * ```
 * With no extras it defaults to road+wide of `00000043--050c69d7d8--0` and no audio.
 */
class SpikeMultiCamActivity : Activity() {

  private val tag = "SpikeMultiCam"
  private lateinit var player: ExoPlayer
  private lateinit var factory: MultiRenderersFactory
  private lateinit var selector: TileTrackSelector
  private lateinit var enabled: BooleanArray
  private lateinit var statusView: TextView
  private val ui = Handler(Looper.getMainLooper())
  private var tileCount = 0

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)

    val videos = intent.getStringArrayExtra("videos") ?: defaultVideos()
    val audio = intent.getStringExtra("audio") // optional qcamera.ts for the audio test
    tileCount = videos.size
    val videoRendererCount = tileCount + if (audio != null) 1 else 0

    factory = MultiRenderersFactory(this, videoRendererCount)
    // Real tiles are the first `tileCount` video renderers; the trailing one (qcamera's own video,
    // when an audio source is merged) stays disabled — we only want qcamera's audio.
    enabled = BooleanArray(videoRendererCount) { it < tileCount }
    selector = TileTrackSelector(enabled, /* audioEnabled= */ audio != null)

    player =
        ExoPlayer.Builder(this)
            .setRenderersFactory(factory)
            .setTrackSelector(selector)
            .build()

    val mf = DefaultMediaSourceFactory(this)
    val sources = ArrayList<MediaSource>()
    for (p in videos) sources.add(mf.createMediaSource(item(p)))
    if (audio != null) sources.add(mf.createMediaSource(item(audio)))
    // adjustPeriodTimeOffsets + clipDurations: align independently-timed clips to a common window.
    val merged = MergingMediaSource(true, true, *sources.toTypedArray())
    player.setMediaSource(merged)
    player.playWhenReady = false
    player.prepare()

    Log.i(tag, "videos=${videos.size} audio=${audio != null} videoRenderers=$videoRendererCount")

    setContentView(buildUi())
    routeSurfacesWhenReady()
    startStatsLoop()
  }

  private fun item(path: String) = MediaItem.fromUri(Uri.fromFile(File(path)))

  private fun buildUi(): LinearLayout {
    val root =
        LinearLayout(this).apply {
          orientation = LinearLayout.VERTICAL
          setBackgroundColor(Color.BLACK)
        }
    for (i in 0 until tileCount) {
      val frame = FrameLayout(this)
      val sv = SurfaceView(this)
      frame.addView(sv, FrameLayout.LayoutParams(MATCH, MATCH))
      frame.addView(
          TextView(this).apply {
            text = "tile $i"
            setTextColor(Color.YELLOW)
            setBackgroundColor(Color.argb(120, 0, 0, 0))
          },
          FrameLayout.LayoutParams(WRAP, WRAP),
      )
      root.addView(frame, LinearLayout.LayoutParams(MATCH, 0, 1f))

      val target: Renderer = factory.videoRenderers[i]
      sv.holder.addCallback(
          object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) = routeSurface(target, holder.surface)
            override fun surfaceChanged(h: SurfaceHolder, f: Int, w: Int, ht: Int) {}
            override fun surfaceDestroyed(holder: SurfaceHolder) = routeSurface(target, null)
          })
    }

    statusView =
        TextView(this).apply {
          setTextColor(Color.WHITE)
          textSize = 11f
          setBackgroundColor(Color.argb(160, 0, 0, 0))
        }
    root.addView(statusView, LinearLayout.LayoutParams(MATCH, WRAP))

    val buttons = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
    buttons.addView(button("Play/Pause") { player.playWhenReady = !player.playWhenReady })
    buttons.addView(
        button("Toggle tile ${tileCount - 1}") {
          val idx = tileCount - 1
          enabled[idx] = !enabled[idx]
          selector.videoTileEnabled = enabled
          selector.reselect() // re-select tracks → decoder released/created, no seek
          Log.i(tag, "tile $idx enabled=${enabled[idx]}")
        })
    buttons.addView(button("+5s") { player.seekTo(player.currentPosition + 5_000) })
    buttons.addView(button("Dump") { dumpStats() })
    root.addView(buttons, LinearLayout.LayoutParams(MATCH, WRAP))
    return root
  }

  private fun button(label: String, onClick: () -> Unit) =
      Button(this).apply {
        text = label
        setOnClickListener { onClick() }
      }

  private fun routeSurfacesWhenReady() {
    // Surfaces route via their SurfaceHolder.Callback (above); nothing to do here yet.
  }

  private fun routeSurface(target: Renderer, surface: Surface?) {
    player.createMessage(target).setType(Renderer.MSG_SET_VIDEO_OUTPUT).setPayload(surface).send()
  }

  private fun startStatsLoop() {
    ui.post(
        object : Runnable {
          override fun run() {
            dumpStats()
            ui.postDelayed(this, 1_000)
          }
        })
  }

  private fun dumpStats() {
    val sb = StringBuilder()
    sb.append("pos=${player.currentPosition}ms state=${player.playbackState} pwr=${player.playWhenReady}\n")
    factory.stats.forEachIndexed { i, s ->
      val c = s.counters
      c?.ensureUpdated()
      sb.append(
          "r$i first=${s.firstFrameRendered} rendered=${c?.renderedOutputBufferCount} " +
              "dropped=${c?.droppedBufferCount} lastBatch=${s.lastDroppedBatch}\n")
    }
    val txt = sb.toString()
    statusView.text = txt
    Log.i(tag, txt.replace("\n", " | "))
  }

  override fun onDestroy() {
    super.onDestroy()
    ui.removeCallbacksAndMessages(null)
    player.release()
  }

  private fun defaultVideos(): Array<String> {
    val base = "${getExternalFilesDir(null)}/dashdown/mirror/2/routes/00000043--050c69d7d8--0"
    return arrayOf("$base/fcamera.hevc.mp4", "$base/ecamera.hevc.mp4")
  }

  private companion object {
    const val MATCH = ViewGroup.LayoutParams.MATCH_PARENT
    const val WRAP = ViewGroup.LayoutParams.WRAP_CONTENT
  }
}
