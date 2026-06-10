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
import uniffi.dashdown_core.FileKind
import uniffi.dashdown_core.SyncStatus

/**
 * B2 live test for **"a segment added to an active drive gets synced"** — the background-sync core
 * the user asked for. Stage a 2-segment drive on a **dedicated route**, download it, append a third
 * segment via the mock control port (as the comma would by recording more footage), run another
 * session, and confirm the appended segment is mirrored — all worker-driven, no UI. The dedicated
 * route is removed in `finally` so the shared fixture stays pristine for the other live tests.
 *
 * Skipped unless both `mockPort` and `controlPort` are supplied (see [tools/run-android-e2e.sh] /
 * docs/TESTING.md for the runbook).
 */
@RunWith(AndroidJUnit4::class)
class SyncSessionSegmentPickupTest {
  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun segmentAddedToActiveDriveGetsSynced() = runBlocking {
    val args = InstrumentationRegistry.getArguments()
    val port = args.getString("mockPort")
    val controlPort = args.getString("controlPort")
    assumeTrue(
        "requires mockPort + controlPort + fixture + adb reverse",
        port != null && controlPort != null)
    val cp = controlPort!!.toInt()
    val route = "000009ee--segpickup01" // dedicated route, isolated from the single_drive fixture
    val device = repo.addDevice(autoSyncDevice("SegPickup-${System.nanoTime()}", port!!))
    try {
      // Stage a 2-segment drive on its own route and download everything.
      withContext(Dispatchers.IO) {
        MockControl.post(cp, "/add_drive", "{\"route\":\"$route\",\"segs\":2}")
      }
      TestListenableWorkerBuilder<SyncSessionWorker>(app).build().doWork()
      val key =
          repo
              .listDrives(device.id, offline = true)
              .first { it.driveKey.startsWith(route) }
              .driveKey
      assertEquals(SyncStatus.COMPLETE, repo.getDriveStatus(device.id, key).status)
      val before = repo.driveLocalPaths(device.id, key, FileKind.Q_CAMERA).size

      // The comma records another segment on that drive; a fresh session must pick it up.
      withContext(Dispatchers.IO) {
        MockControl.post(cp, "/add_segment", "{\"route\":\"$route\",\"n\":1}")
      }
      TestListenableWorkerBuilder<SyncSessionWorker>(app).build().doWork()

      val after = repo.driveLocalPaths(device.id, key, FileKind.Q_CAMERA).size
      assertEquals("the appended segment should have been synced", before + 1, after)
      assertEquals(SyncStatus.COMPLETE, repo.getDriveStatus(device.id, key).status)
    } finally {
      withContext(Dispatchers.IO) {
        runCatching { MockControl.post(cp, "/remove_drive", "{\"route\":\"$route\"}") }
      }
      repo.removeDevice(device.id)
    }
  }
}
