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
import org.sunnypilot.dashdown.work.Maintenance
import org.sunnypilot.dashdown.work.SyncSessionWorker
import uniffi.dashdown_core.DeviceSettings
import uniffi.dashdown_core.FileKind
import uniffi.dashdown_core.FileSelection

/**
 * Phase D live test: **segment-level retention**. Stages a long (5-segment) non-preserved drive and
 * an old 2-segment preserved drive (deterministic ages via `/add_drive`'s `mtime_s`), downloads
 * everything, then enforces a budget of 3 — proving:
 * - the long drive is kept **partially** (oldest segments pruned, newest 3 kept),
 * - the **preserved** drive survives in full and doesn't count toward the budget,
 * - clear-down runs with the device **unreachable** (`Maintenance.sweep` is local-only), and
 * - the pruned segments are **not re-downloaded** by a later session (the loop is impossible).
 *
 * Skipped unless both `mockPort` and `controlPort` are supplied.
 */
@RunWith(AndroidJUnit4::class)
class RetentionLiveTest {
  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun prunesOldSegmentsKeepsPreservedRunsOfflineAndNeverReDownloads() = runBlocking {
    val args = InstrumentationRegistry.getArguments()
    val port = args.getString("mockPort")
    val controlPort = args.getString("controlPort")
    assumeTrue(
        "requires mockPort + controlPort + fixture + adb reverse",
        port != null && controlPort != null)
    val cp = controlPort!!.toInt()
    val dRoute = "000000dd--retentiond1" // 5 segments, NEWEST (future mtime), non-preserved
    val pRoute = "000000aa--retentionp1" // 2 segments, old, preserved
    val dKey = "$dRoute--0"
    val pKey = "$pRoute--0"

    val device = repo.addDevice(autoSyncDevice("Retention-${System.nanoTime()}", port!!))
    val id = device.id
    suspend fun qcount(key: String) = repo.driveLocalPaths(id, key, FileKind.Q_CAMERA).size
    try {
      // Stage the two dedicated drives with explicit ages (before the first sync).
      withContext(Dispatchers.IO) {
        MockControl.post(
            cp, "/add_drive", "{\"route\":\"$dRoute\",\"segs\":5,\"mtime_s\":2000000000}")
        MockControl.post(cp, "/add_drive", "{\"route\":\"$pRoute\",\"segs\":2,\"mtime_s\":1000}")
      }

      // Budget unlimited → download everything; D and P fully local.
      TestListenableWorkerBuilder<SyncSessionWorker>(app).build().doWork()
      assertEquals("D fully downloaded", 5, qcount(dKey))
      assertEquals("P fully downloaded", 2, qcount(pKey))

      // Star P, take the device OFFLINE, set budget 3, then clear down locally.
      repo.setPreserved(id, pKey, true)
      repo.updateDevice(device.copy(port = 9099.toUShort())) // unreachable (unreversed port)
      repo.setSettings(
          id,
          DeviceSettings(
              autoSync = true,
              fileSelection = FileSelection(false, false, false, true, false, false, false, false),
              retentionMaxMinutes = 3,
              autoDeleteFromComma = false,
              autoDeleteMinAgeMin = 60,
              capWarnEnabled = true,
              capWarnThresholdMinutes = 10,
          ),
      )
      Maintenance.sweep(app, repo, repo.listDevices().first { it.id == id }) // local, no network
      assertEquals("D kept partially (newest 3 segments)", 3, qcount(dKey))
      assertEquals("preserved P survives in full", 2, qcount(pKey))

      // Loop guard: back online (re-fetch so the budget=3 set above is preserved — updateDevice
      // writes the whole row), a fresh session must NOT re-fetch the pruned segments.
      repo.updateDevice(repo.listDevices().first { it.id == id }.copy(port = port.toUShort()))
      TestListenableWorkerBuilder<SyncSessionWorker>(app).build().doWork()
      assertEquals("pruned segments are not re-downloaded", 3, qcount(dKey))
      assertEquals("preserved P unchanged", 2, qcount(pKey))
    } finally {
      withContext(Dispatchers.IO) {
        runCatching { MockControl.post(cp, "/remove_drive", "{\"route\":\"$dRoute\"}") }
        runCatching { MockControl.post(cp, "/remove_drive", "{\"route\":\"$pRoute\"}") }
      }
      repo.removeDevice(id)
    }
  }
}
