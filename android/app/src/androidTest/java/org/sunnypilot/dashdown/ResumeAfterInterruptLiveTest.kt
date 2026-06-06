package org.sunnypilot.dashdown

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.dashdown_core.ConnMode
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.FileSelection
import uniffi.dashdown_core.SyncStatus

/**
 * Step-8 resume acceptance: model an interrupted/lost transfer by **deleting a downloaded file**
 * from the mirror, confirm the offline reclassify drops the drive to `Partial`, then re-download
 * and confirm it returns to `Complete` (resume re-fetches only what's missing). Deterministic and
 * hardware-exercised; gated on `mockPort`.
 */
@RunWith(AndroidJUnit4::class)
class ResumeAfterInterruptLiveTest {
  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun resumeAfterFileLossReachesComplete() = runBlocking {
    val port = InstrumentationRegistry.getArguments().getString("mockPort")
    assumeTrue("requires mockPort + fixture + adb reverse", port != null)
    val device =
        repo.addDevice(
            Device(
                id = 0,
                name = "Resume-${System.nanoTime()}",
                dongleLabel = null,
                hotspotIp = "127.0.0.1",
                wifiIp = null,
                port = port!!.toUShort(),
                activeMode = ConnMode.HOTSPOT,
                password = null,
                autoSync = false,
                fileSelection =
                    FileSelection(false, false, false, true, false, false, false, false),
                retentionMaxMinutes = null,
                autoDeleteFromComma = false,
                autoDeleteMinAgeMin = 60,
            ))
    try {
      val key = repo.listDrives(device.id, offline = false).first().driveKey
      repo.startDriveDownload(device.id, key)
      awaitStatus(device.id, key, SyncStatus.COMPLETE)

      // Lose one downloaded file (simulates an interrupted/partial transfer).
      val drive = repo.getDrive(device.id, key)
      val seg = drive.segments.first().name
      val lost =
          File(
              app.locator.mirrorRoot,
              "${device.id}/realdata/${seg.routeId}--${seg.segmentNum}/qcamera.ts")
      assertTrue("the downloaded file should exist before deletion", lost.exists())
      assertTrue("delete should succeed", lost.delete())

      // Offline reclassify must see the drive as no longer complete.
      val afterLoss = repo.listDrives(device.id, offline = true).first { it.driveKey == key }
      assertNotEquals(SyncStatus.COMPLETE, afterLoss.syncState)

      // Resume → Complete again.
      repo.startDriveDownload(device.id, key)
      awaitStatus(device.id, key, SyncStatus.COMPLETE)
      assertEquals(SyncStatus.COMPLETE, repo.getDriveStatus(device.id, key).status)
    } finally {
      repo.removeDevice(device.id)
    }
  }

  private suspend fun awaitStatus(deviceId: Long, key: String, target: SyncStatus) {
    withTimeout(30_000) {
      while (true) {
        val s = repo.getDriveStatus(deviceId, key).status
        if (s == target) break
        assertNotEquals("download failed while awaiting $target", SyncStatus.FAILED, s)
        delay(200)
      }
    }
  }
}
