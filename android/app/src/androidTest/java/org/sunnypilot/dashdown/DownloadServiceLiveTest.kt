package org.sunnypilot.dashdown

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.sunnypilot.dashdown.service.DownloadService
import uniffi.dashdown_core.ConnMode
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.FileSelection
import uniffi.dashdown_core.SyncStatus

/**
 * Step-5 live test: start a download through the real [DownloadService] (the UI's path) and confirm
 * the drive reaches `Complete` — the "download runs to completion in the background" acceptance,
 * exercised on hardware against a `mock-copyparty` fixture over `adb reverse`.
 *
 * Skipped unless `mockPort` is supplied (no-op in CI). To run locally:
 *
 * cargo run -q -p mock-copyparty -- --fixture single_drive --port 8099 & adb reverse tcp:8099
 * tcp:8099 ./gradlew -p android :app:connectedDebugAndroidTest \
 * -Pandroid.testInstrumentationRunnerArguments.class=org.sunnypilot.dashdown.DownloadServiceLiveTest
 * \ -Pandroid.testInstrumentationRunnerArguments.mockPort=8099
 */
@RunWith(AndroidJUnit4::class)
class DownloadServiceLiveTest {
  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun serviceDownloadRunsToComplete() = runBlocking {
    val port = InstrumentationRegistry.getArguments().getString("mockPort")
    assumeTrue("requires mockPort + fixture + adb reverse", port != null)
    val device =
        repo.addDevice(
            Device(
                id = 0,
                name = "DlSvc-${System.nanoTime()}",
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
                capWarnEnabled = true,
                capWarnThresholdMinutes = 10,
            ))
    try {
      val drives = repo.listDrives(device.id, offline = false)
      assertTrue("fixture should expose a drive", drives.isNotEmpty())
      val key = drives.first().driveKey

      DownloadService.start(app, device.id, key)

      withTimeout(30_000) {
        while (true) {
          val status = repo.getDriveStatus(device.id, key).status
          if (status == SyncStatus.COMPLETE) break
          assertNotEquals("download failed", SyncStatus.FAILED, status)
          delay(200)
        }
      }
      assertEquals(SyncStatus.COMPLETE, repo.getDriveStatus(device.id, key).status)
    } finally {
      repo.removeDevice(device.id)
    }
  }
}
