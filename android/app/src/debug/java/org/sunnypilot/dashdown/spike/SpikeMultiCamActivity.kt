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

    val mf = DefaultMediaSourceFactory(this)

    // Two modes:
    //  • MULTI-SEGMENT (R1): driveDir + segs + cams → a player PLAYLIST of per-segment
    //    MergingMediaSources (Design C). Each window is one segment's multi-cam merge, so every
    //    merge is uniformly 1-period (MergingMediaSource never hits REASON_PERIOD_COUNT_MISMATCH,
    //    even with ragged/lazy availability), and the drive-wide timeline = sum of window durations
    //    (reuses RP3's windowsOf/locate/globalPosition math). This is the candidate production shape.
    //  • SINGLE-SEGMENT (A0 default): videos[] (+ optional audio) merged into one window.
    val driveDir = intent.getStringExtra("driveDir")
    val segs = intent.getIntExtra("segs", 0)
    val cams = intent.getStringArrayExtra("cams")
    val audioName = intent.getStringExtra("audioName") // per-segment audio filename (multi mode)

    val mediaSources = ArrayList<MediaSource>()
    val audioPresent: Boolean

    if (driveDir != null && segs > 0 && cams != null) {
      tileCount = cams.size
      audioPresent = audioName != null
      for (s in 0 until segs) {
        val segDir = "$driveDir--$s"
        val segSources = ArrayList<MediaSource>()
        for (cam in cams) segSources.add(mf.createMediaSource(item("$segDir/$cam")))
        if (audioName != null) segSources.add(mf.createMediaSource(item("$segDir/$audioName")))
        mediaSources.add(MergingMediaSource(true, true, *segSources.toTypedArray()))
      }
      Log.i(tag, "MULTI-SEG drive=$driveDir segs=$segs cams=${cams.size} audio=$audioPresent")
    } else {
      val videos = intent.getStringArrayExtra("videos") ?: defaultVideos()
      val audio = intent.getStringExtra("audio")
      tileCount = videos.size
      audioPresent = audio != null
      val segSources = ArrayList<MediaSource>()
      for (p in videos) segSources.add(mf.createMediaSource(item(p)))
      if (audio != null) segSources.add(mf.createMediaSource(item(audio)))
      // adjustPeriodTimeOffsets + clipDurations: align independently-timed clips to a common window.
      mediaSources.add(MergingMediaSource(true, true, *segSources.toTypedArray()))
      Log.i(tag, "SINGLE-SEG videos=${videos.size} audio=$audioPresent")
    }

    // Real tiles are the first `tileCount` video renderers; the trailing one (qcamera's own video,
    // when audio is merged) stays disabled — we only want qcamera's audio.
    val videoRendererCount = tileCount + if (audioPresent) 1 else 0
    factory = MultiRenderersFactory(this, videoRendererCount)
    enabled = BooleanArray(videoRendererCount) { it < tileCount }
    selector = TileTrackSelector(enabled, audioPresent)

    player =
        ExoPlayer.Builder(this)
            .setRenderersFactory(factory)
            .setTrackSelector(selector)
            .build()
    player.setMediaSources(mediaSources)
    player.playWhenReady = false
    player.prepare()

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
    buttons.addView(
        button("Next seg") {
          // Cross-WINDOW seek: jump to the next segment's start (tests boundary seek + same-frame).
          val next = (player.currentMediaItemIndex + 1).coerceAtMost(player.mediaItemCount - 1)
          player.seekTo(next, 0L)
        })
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
    sb.append(
        "pos=${player.currentPosition}ms win=${player.currentMediaItemIndex}/${player.mediaItemCount} " +
            "state=${player.playbackState} pwr=${player.playWhenReady}\n")
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
