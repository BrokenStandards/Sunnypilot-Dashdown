package org.sunnypilot.dashdown

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.work.testing.TestListenableWorkerBuilder
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.sunnypilot.dashdown.work.SyncSessionWorker
import uniffi.dashdown_core.SyncStatus

/**
 * B2 live test for **"automatic download of a newly-appeared drive"**. After a baseline session,
 * inject a brand-new drive (route) via the mock control port and run another session; the new drive
 * must be indexed and downloaded to COMPLETE without any UI.
 *
 * Skipped unless both `mockPort` and `controlPort` are supplied. Run locally (host):
 * ```
 * cargo run -q -p mock-copyparty -- --fixture single_drive --port 8099 --control-port 8098 &
 * adb reverse tcp:8099 tcp:8099 && adb reverse tcp:8098 tcp:8098
 * ./gradlew -p android :app:connectedDebugAndroidTest \
 *   -Pandroid.testInstrumentationRunnerArguments.class=org.sunnypilot.dashdown.SyncSessionDriveAddTest \
 *   -Pandroid.testInstrumentationRunnerArguments.mockPort=8099 \
 *   -Pandroid.testInstrumentationRunnerArguments.controlPort=8098
 * ```
 */
@RunWith(AndroidJUnit4::class)
class SyncSessionDriveAddTest {
  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun newDriveGetsDownloaded() = runBlocking {
    val args = InstrumentationRegistry.getArguments()
    val port = args.getString("mockPort")
    val controlPort = args.getString("controlPort")
    assumeTrue(
        "requires mockPort + controlPort + fixture + adb reverse",
        port != null && controlPort != null)
    val device = repo.addDevice(autoSyncDevice("DriveAdd-${System.nanoTime()}", port!!))
    try {
      TestListenableWorkerBuilder<SyncSessionWorker>(app).build().doWork() // baseline
      val baseline = repo.listDrives(device.id, offline = true).size

      // A new drive appears on the device (distinct route), then a fresh session runs.
      withContext(Dispatchers.IO) {
        MockControl.post(
            controlPort!!.toInt(), "/add_drive", "{\"route\":\"000009ff--b2sessADD\",\"segs\":1}")
      }
      TestListenableWorkerBuilder<SyncSessionWorker>(app).build().doWork()

      val drives = repo.listDrives(device.id, offline = true)
      assertEquals("the new drive should be indexed", baseline + 1, drives.size)
      drives.forEach {
        assertEquals(
            "drive ${it.driveKey} should be auto-downloaded",
            SyncStatus.COMPLETE,
            repo.getDriveStatus(device.id, it.driveKey).status,
        )
      }
    } finally {
      repo.removeDevice(device.id)
    }
  }
}
