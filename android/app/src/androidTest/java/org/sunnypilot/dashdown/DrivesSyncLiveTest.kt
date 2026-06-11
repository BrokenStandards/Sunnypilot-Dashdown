package org.sunnypilot.dashdown

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.dashdown_core.ConnMode
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.FileSelection

/**
 * Step-4 live integration test: sync a device against a real `mock-copyparty` fixture reached over
 * `adb reverse`, and assert the core groups the `single_drive` fixture (3 consecutive segments)
 * into one drive. Exercises the full device→adb-reverse→host-mock→core path on hardware.
 *
 * Skipped unless a `mockPort` instrumentation arg is supplied, so it is a no-op in CI (where no
 * fixture is running). To run locally:
 *
 * cargo run -q -p mock-copyparty -- --fixture single_drive --port 8099 & # on the host adb reverse
 * tcp:8099 tcp:8099 ./gradlew -p android :app:connectedDebugAndroidTest \
 * -Pandroid.testInstrumentationRunnerArguments.mockPort=8099
 */
@RunWith(AndroidJUnit4::class)
class DrivesSyncLiveTest {
  private val repo
    get() = ApplicationProvider.getApplicationContext<DashdownApp>().locator.repository

  @Test
  fun syncGroupsSingleDriveFixture() = runBlocking {
    val port = InstrumentationRegistry.getArguments().getString("mockPort")
    assumeTrue(
        "set -Pandroid.testInstrumentationRunnerArguments.mockPort=<port> with the fixture + adb reverse running",
        port != null,
    )
    val device =
        repo.addDevice(
            Device(
                id = 0,
                name = "LiveFixture-${System.nanoTime()}",
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
      val drives = repo.listDrives(device.id, offline = false) // network sync_now
      assertTrue("expected at least one drive from the fixture", drives.isNotEmpty())
      assertEquals(
          "single_drive fixture has 3 consecutive segments", 3u, drives.first().segmentCount)
    } finally {
      repo.removeDevice(device.id)
    }
  }
}
