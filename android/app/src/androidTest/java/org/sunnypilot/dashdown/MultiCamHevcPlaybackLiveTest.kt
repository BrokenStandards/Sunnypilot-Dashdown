package org.sunnypilot.dashdown

import android.media.MediaMetadataRetriever
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.dashdown_core.ConnMode
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.FileKind
import uniffi.dashdown_core.FileSelection
import uniffi.dashdown_core.SyncStatus

/**
 * RP3 on-device acceptance: the core's HEVC→MP4 remux ([uniffi…AppCore.ensurePlayable]) produces a
 * file the **platform HEVC decoder** plays and seeks. Downloads a real road-camera segment from a
 * comma device, remuxes it, then decodes a frame at t=0 (sync) and mid-segment (seek into a GOP)
 * via `MediaMetadataRetriever` — the same decode path Coil/Media3 use. Heavy + needs the real
 * device, so it's gated on `commaHost`/`commaPort` instrumentation args and self-skips otherwise.
 *
 * Run (device on the same LAN as the comma):
 * ```
 * ./gradlew :app:connectedDebugAndroidTest \
 *   -Pandroid.testInstrumentationRunnerArguments.commaHost=192.168.1.181 \
 *   -Pandroid.testInstrumentationRunnerArguments.commaPort=8080
 * ```
 */
@RunWith(AndroidJUnit4::class)
class MultiCamHevcPlaybackLiveTest {
  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun remuxedRoadCameraDecodesAndSeeksOnDevice() = runBlocking {
    val args = InstrumentationRegistry.getArguments()
    val host = args.getString("commaHost")
    val port = args.getString("commaPort")
    assumeTrue(
        "requires commaHost + commaPort (a reachable comma device)", host != null && port != null)

    val device =
        repo.addDevice(
            Device(
                id = 0,
                name = "RP3-${System.nanoTime()}",
                dongleLabel = null,
                hotspotIp = host!!,
                wifiIp = null,
                port = port!!.toUShort(),
                activeMode = ConnMode.HOTSPOT,
                password = null,
                autoSync = false,
                // Road camera + qcamera only (keep the download modest).
                fileSelection = FileSelection(true, false, false, true, false, false, false, false),
                retentionMaxMinutes = null,
                autoDeleteFromComma = false,
                autoDeleteMinAgeMin = 60,
                capWarnEnabled = true,
                capWarnThresholdMinutes = 10,
            ))
    try {
      // Smallest drive → fewest segments to download.
      val drive = repo.listDrives(device.id, offline = false).minByOrNull { it.segmentCount }!!
      repo.startDriveDownload(device.id, drive.driveKey)
      awaitStatus(device.id, drive.driveKey, SyncStatus.COMPLETE, timeoutMs = 240_000)

      val seg0 = repo.getDrive(device.id, drive.driveKey).segments.first().name.segmentNum
      val mp4 = repo.ensurePlayable(device.id, drive.driveKey, seg0, FileKind.F_CAMERA)
      assertNotNull("remux should yield an MP4 path for the road camera", mp4)

      val r = MediaMetadataRetriever()
      try {
        r.setDataSource(mp4!!)
        val width =
            r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toIntOrNull()
        val durationMs =
            r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull()
        assertTrue("HD width expected, got $width", (width ?: 0) >= 1000)
        // One ~60 s segment.
        assertTrue("≈60 s duration expected, got $durationMs", (durationMs ?: 0) in 55_000..65_000)

        // Decode at the start (sync sample) and mid-segment (seek into a GOP) — proves the platform
        // HEVC decoder accepts our hvcC + sample tables and can seek frame-accurately.
        assertNotNull("frame at t=0 should decode", r.getFrameAtTime(0))
        assertNotNull(
            "frame at t=30s should decode (seek into segment)",
            r.getFrameAtTime(30_000_000L, MediaMetadataRetriever.OPTION_CLOSEST))
      } finally {
        r.release()
      }
    } finally {
      repo.removeDevice(device.id)
    }
  }

  private suspend fun awaitStatus(
      deviceId: Long,
      key: String,
      target: SyncStatus,
      timeoutMs: Long,
  ) {
    withTimeout(timeoutMs) {
      while (true) {
        val s = repo.getDriveStatus(deviceId, key).status
        if (s == target) break
        assertNotEquals("download failed while awaiting $target", SyncStatus.FAILED, s)
        delay(500)
      }
    }
  }
}
