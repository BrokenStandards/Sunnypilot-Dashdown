package org.sunnypilot.dashdown

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.work.Constraints
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.sunnypilot.dashdown.work.SyncSessionWorker
import uniffi.dashdown_core.SyncStatus

/**
 * B2 live test exercising the **real scheduled path** (not
 * [androidx.work.testing.TestListenableWorkerBuilder], which no-ops `setForeground`): enqueue a
 * [SyncSessionWorker] through `WorkManager`, which promotes it to a `dataSync` foreground service
 * via WorkManager's own `SystemForegroundService` and runs the session.
 *
 * This is the regression guard for the manifest fix: without the `tools:node="merge"` overlay
 * adding `foregroundServiceType="dataSync"` to `SystemForegroundService`, this path throws
 * `IllegalArgumentException: foregroundServiceType 0x1 is not a subset of 0x0` and the app process
 * crashes — failing the run hard. With the fix it downloads the fixture's drive to COMPLETE.
 *
 * Skipped unless `mockPort` is supplied (see docs/TESTING.md for the runbook).
 */
@RunWith(AndroidJUnit4::class)
class SyncSessionScheduledFgsTest {
  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun scheduledWorkerPromotesDataSyncFgsAndDownloads() = runBlocking {
    val port = InstrumentationRegistry.getArguments().getString("mockPort")
    assumeTrue("requires mockPort + fixture + adb reverse", port != null)
    val device = repo.addDevice(autoSyncDevice("SchedFgs-${System.nanoTime()}", port!!))
    val wm = WorkManager.getInstance(app)
    try {
      val request =
          OneTimeWorkRequestBuilder<SyncSessionWorker>()
              .setConstraints(
                  Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
              .build()
      wm.enqueueUniqueWork(UNIQUE, ExistingWorkPolicy.REPLACE, request)

      // The real WorkManager FGS path syncs the index then downloads; wait for it to drain.
      withTimeout(90_000) {
        while (true) {
          val drives = repo.listDrives(device.id, offline = true)
          val done =
              drives.isNotEmpty() &&
                  drives.all {
                    repo.getDriveStatus(device.id, it.driveKey).status == SyncStatus.COMPLETE
                  }
          if (done) break
          delay(1000)
        }
      }
      val drives = repo.listDrives(device.id, offline = true)
      assertTrue("scheduled session should have indexed the fixture's drive", drives.isNotEmpty())
      drives.forEach {
        assertEquals(SyncStatus.COMPLETE, repo.getDriveStatus(device.id, it.driveKey).status)
      }
    } finally {
      wm.cancelUniqueWork(UNIQUE)
      repo.removeDevice(device.id)
    }
  }

  private companion object {
    const val UNIQUE = "sync-session-fgs-test"
  }
}
