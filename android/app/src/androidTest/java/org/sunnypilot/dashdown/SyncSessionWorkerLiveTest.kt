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
import org.sunnypilot.dashdown.work.SyncSessionWorker
import uniffi.dashdown_core.ConnMode
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.FileSelection
import uniffi.dashdown_core.SyncStatus

/**
 * B2 live test: run a background [SyncSessionWorker] directly (via the WorkManager test harness)
 * against an `autoSync` device pointing at the `single_drive` fixture, and confirm the session
 * syncs **and** auto-downloads the drive to `Complete` — the "automatic downloads, no app open"
 * acceptance. Skipped unless `mockPort` is supplied.
 *
 * Run locally (host):
 * ```
 * cargo run -q -p mock-copyparty -- --fixture single_drive --port 8099 &
 * adb reverse tcp:8099 tcp:8099
 * ./gradlew -p android :app:connectedDebugAndroidTest \
 *   -Pandroid.testInstrumentationRunnerArguments.class=org.sunnypilot.dashdown.SyncSessionWorkerLiveTest \
 *   -Pandroid.testInstrumentationRunnerArguments.mockPort=8099
 * ```
 */
@RunWith(AndroidJUnit4::class)
class SyncSessionWorkerLiveTest {
  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun sessionSyncsAndDownloads() = runBlocking {
    val port = InstrumentationRegistry.getArguments().getString("mockPort")
    assumeTrue("requires mockPort + fixture + adb reverse", port != null)
    val device = repo.addDevice(autoSyncDevice("Session-${System.nanoTime()}", port!!))
    try {
      TestListenableWorkerBuilder<SyncSessionWorker>(app).build().doWork()

      val drives = repo.listDrives(device.id, offline = true)
      assertTrue("session should have indexed the fixture's drive", drives.isNotEmpty())
      assertEquals(
          "session should have downloaded the drive",
          SyncStatus.COMPLETE,
          repo.getDriveStatus(device.id, drives.first().driveKey).status,
      )
    } finally {
      repo.removeDevice(device.id)
    }
  }
}

/** An `autoSync` device on 127.0.0.1:[port] (reached via `adb reverse`), qcamera-only selection. */
internal fun autoSyncDevice(name: String, port: String): Device =
    Device(
        id = 0,
        name = name,
        dongleLabel = null,
        hotspotIp = "127.0.0.1",
        wifiIp = null,
        port = port.toUShort(),
        activeMode = ConnMode.HOTSPOT,
        password = null,
        autoSync = true,
        fileSelection = FileSelection(false, false, false, true, false, false, false, false),
        retentionMaxMinutes = null,
        autoDeleteFromComma = false,
        autoDeleteMinAgeMin = 60,
        capWarnEnabled = true,
        capWarnThresholdMinutes = 10,
    )
