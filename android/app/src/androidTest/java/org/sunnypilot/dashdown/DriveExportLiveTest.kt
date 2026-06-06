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
 * Step-6 live test: download the `single_drive` fixture, then `exportDriveZip` to a temp file and
 * confirm a valid (PK-headed, non-empty) zip is produced. Skipped unless `mockPort` is supplied.
 */
@RunWith(AndroidJUnit4::class)
class DriveExportLiveTest {
  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun downloadThenExportProducesZip() = runBlocking {
    val port = InstrumentationRegistry.getArguments().getString("mockPort")
    assumeTrue("requires mockPort + fixture + adb reverse", port != null)
    val device =
        repo.addDevice(
            Device(
                id = 0,
                name = "Export-${System.nanoTime()}",
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
    val temp = File.createTempFile("export", ".zip", app.cacheDir)
    try {
      val key = repo.listDrives(device.id, offline = false).first().driveKey
      repo.startDriveDownload(device.id, key)
      withTimeout(30_000) {
        while (true) {
          val status = repo.getDriveStatus(device.id, key).status
          if (status == SyncStatus.COMPLETE) break
          assertNotEquals("download failed", SyncStatus.FAILED, status)
          delay(200)
        }
      }
      repo.exportDriveZip(device.id, key, temp.absolutePath)
      assertTrue("zip should be non-empty", temp.length() > 0)
      val head = ByteArray(2)
      temp.inputStream().use { it.read(head) }
      assertEquals('P'.code.toByte(), head[0])
      assertEquals('K'.code.toByte(), head[1])
    } finally {
      temp.delete()
      repo.removeDevice(device.id)
    }
  }
}
