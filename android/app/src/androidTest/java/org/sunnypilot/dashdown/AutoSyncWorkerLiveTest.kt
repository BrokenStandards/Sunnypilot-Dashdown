package org.sunnypilot.dashdown

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.work.testing.TestListenableWorkerBuilder
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.sunnypilot.dashdown.work.AutoSyncWorker
import uniffi.dashdown_core.ConnMode
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.FileSelection
import uniffi.dashdown_core.SyncStatus

/**
 * Step-7 live test: run [AutoSyncWorker.doWork] directly (via the WorkManager test harness) against
 * an `autoSync` device pointing at the `single_drive` fixture, and confirm it syncs **and**
 * auto-downloads the drive to `Complete`. Skipped unless `mockPort` is supplied.
 */
@RunWith(AndroidJUnit4::class)
class AutoSyncWorkerLiveTest {
  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun autoSyncSyncsAndDownloads() = runBlocking {
    val port = InstrumentationRegistry.getArguments().getString("mockPort")
    assumeTrue("requires mockPort + fixture + adb reverse", port != null)
    val device =
        repo.addDevice(
            Device(
                id = 0,
                name = "AutoSync-${System.nanoTime()}",
                dongleLabel = null,
                hotspotIp = "127.0.0.1",
                wifiIp = null,
                port = port!!.toUShort(),
                activeMode = ConnMode.HOTSPOT,
                password = null,
                autoSync = true,
                fileSelection =
                    FileSelection(false, false, false, true, false, false, false, false),
                retentionMaxMinutes = null,
                autoDeleteFromComma = false,
                autoDeleteMinAgeMin = 60,
            ))
    try {
      val worker = TestListenableWorkerBuilder<AutoSyncWorker>(app).build()
      worker.doWork() // syncNow + runMaintenance + escalate-download for autoSync devices

      val drives = repo.listDrives(device.id, offline = true)
      assertTrue("auto-sync should have indexed the fixture's drive", drives.isNotEmpty())
      assertEquals(
          "auto-sync should have downloaded the drive",
          SyncStatus.COMPLETE,
          repo.getDriveStatus(device.id, drives.first().driveKey).status,
      )
    } finally {
      repo.removeDevice(device.id)
    }
  }
}
