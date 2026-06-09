package org.sunnypilot.dashdown

import android.media.MediaMetadataRetriever
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Comma-free on-device proof that the **platform HEVC decoder** accepts our HEVC→MP4 remux. Reads a
 * pre-remuxed MP4 staged in the app's external files dir and decodes a frame at t=0 (sync sample)
 * and mid-segment (seek into a GOP) via `MediaMetadataRetriever` — the same path Coil/Media3 use.
 * Self-skips if the file isn't present, so CI and ordinary runs don't require it.
 *
 * Stage the file (produced by `it_remux_local`, see rust/core/tests):
 * ```
 * adb push fcamera_seg0.mp4 \
 *   /sdcard/Android/data/org.sunnypilot.dashdown/files/remux_probe.mp4
 * ```
 *
 * Unlike [MultiCamHevcPlaybackLiveTest] this needs no comma device — it isolates the decoder check.
 */
@RunWith(AndroidJUnit4::class)
class DeviceRemuxDecodeTest {
  @Test
  fun platformDecoderPlaysAndSeeksRemuxedMp4() {
    val ctx = ApplicationProvider.getApplicationContext<DashdownApp>()
    val f = File(ctx.getExternalFilesDir(null), "remux_probe.mp4")
    assumeTrue("stage a remuxed MP4 at ${f.absolutePath} to run", f.exists())

    val r = MediaMetadataRetriever()
    try {
      r.setDataSource(f.absolutePath)
      val width = r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toIntOrNull()
      val durationMs =
          r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull()
      assertTrue("HD width expected, got $width", (width ?: 0) >= 1000)
      assertTrue("≈60 s segment expected, got $durationMs", (durationMs ?: 0) in 55_000..65_000)
      assertNotNull("frame at t=0 should decode (sync sample)", r.getFrameAtTime(0))
      assertNotNull(
          "frame at t=30s should decode (seek into segment)",
          r.getFrameAtTime(30_000_000L, MediaMetadataRetriever.OPTION_CLOSEST))
    } finally {
      r.release()
    }
  }
}
