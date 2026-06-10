package org.sunnypilot.dashdown

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.work.testing.TestListenableWorkerBuilder
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.sunnypilot.dashdown.work.SyncSessionWorker
import uniffi.dashdown_core.SyncStatus

/**
 * B2 live test for **"automatic download of a newly-appeared drive"**. After a baseline session,
 * inject a brand-new drive on a **dedicated route** via the mock control port and run another
 * session; the new drive must be indexed and downloaded to COMPLETE without any UI. The dedicated
 * route is removed in `finally` so the shared fixture stays pristine for the other live tests.
 *
 * Skipped unless both `mockPort` and `controlPort` are supplied (see [tools/run-android-e2e.sh] /
 * docs/TESTING.md for the runbook).
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
    val route = "000009ff--driveadd01" // dedicated route, isolated from the single_drive fixture
    val device = repo.addDevice(autoSyncDevice("DriveAdd-${System.nanoTime()}", port!!))
    try {
      TestListenableWorkerBuilder<SyncSessionWorker>(app).build().doWork() // baseline

      // A new drive appears on the device; a fresh session must index and download it.
      withContext(Dispatchers.IO) {
        MockControl.post(controlPort!!.toInt(), "/add_drive", "{\"route\":\"$route\",\"segs\":1}")
      }
      TestListenableWorkerBuilder<SyncSessionWorker>(app).build().doWork()

      val mine =
          repo.listDrives(device.id, offline = true).firstOrNull { it.driveKey.startsWith(route) }
      assertNotNull("the new drive should be indexed", mine)
      assertEquals(
          "the new drive should be auto-downloaded",
          SyncStatus.COMPLETE,
          repo.getDriveStatus(device.id, mine!!.driveKey).status,
      )
    } finally {
      withContext(Dispatchers.IO) {
        runCatching {
          MockControl.post(controlPort!!.toInt(), "/remove_drive", "{\"route\":\"$route\"}")
        }
      }
      repo.removeDevice(device.id)
    }
  }
}
