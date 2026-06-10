package org.sunnypilot.dashdown

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.work.testing.TestListenableWorkerBuilder
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.sunnypilot.dashdown.work.SyncSessionWorker
import uniffi.dashdown_core.FileKind
import uniffi.dashdown_core.SyncStatus

/**
 * B2 live test for **"a segment added to an active drive gets synced"** — the background-sync core
 * the user asked for. Download the fixture's drive, inject a new segment on its route via the mock
 * control port (as the comma would by recording more footage), run another session, and confirm the
 * new segment is mirrored locally — all worker-driven, no UI.
 *
 * Skipped unless both `mockPort` and `controlPort` are supplied. Run locally (host):
 * ```
 * cargo run -q -p mock-copyparty -- --fixture single_drive --port 8099 --control-port 8098 &
 * adb reverse tcp:8099 tcp:8099 && adb reverse tcp:8098 tcp:8098
 * ./gradlew -p android :app:connectedDebugAndroidTest \
 *   -Pandroid.testInstrumentationRunnerArguments.class=org.sunnypilot.dashdown.SyncSessionSegmentPickupTest \
 *   -Pandroid.testInstrumentationRunnerArguments.mockPort=8099 \
 *   -Pandroid.testInstrumentationRunnerArguments.controlPort=8098
 * ```
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
    val device = repo.addDevice(autoSyncDevice("SegPickup-${System.nanoTime()}", port!!))
    try {
      // First session: the fixture's initial drive downloads to COMPLETE.
      TestListenableWorkerBuilder<SyncSessionWorker>(app).build().doWork()
      val key = repo.listDrives(device.id, offline = true).first().driveKey
      assertEquals(SyncStatus.COMPLETE, repo.getDriveStatus(device.id, key).status)
      val before = repo.driveLocalPaths(device.id, key, FileKind.Q_CAMERA).size
      assertTrue("fixture drive should have segments", before > 0)

      // The comma records another segment on the same route, then a fresh session runs.
      withContext(Dispatchers.IO) { MockControl.post(controlPort!!.toInt(), "/add_segment", "{}") }
      TestListenableWorkerBuilder<SyncSessionWorker>(app).build().doWork()

      // The new segment must now be mirrored, and the drive still COMPLETE.
      val after = repo.driveLocalPaths(device.id, key, FileKind.Q_CAMERA).size
      assertEquals("the added segment should have been synced", before + 1, after)
      assertEquals(SyncStatus.COMPLETE, repo.getDriveStatus(device.id, key).status)
    } finally {
      repo.removeDevice(device.id)
    }
  }
}
