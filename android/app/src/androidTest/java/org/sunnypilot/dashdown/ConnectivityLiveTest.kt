package org.sunnypilot.dashdown

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.dashdown_core.ConnDot
import uniffi.dashdown_core.ConnMode
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.FileSelection

/**
 * Step-8 connectivity acceptance. `unreachableDeviceShowsRed` runs everywhere (incl. CI — it points
 * at a closed local port, so no server is needed) and proves the unreachable→**Red** case.
 * `reachableDeviceShowsGreen` is gated on `mockPort` (fixture + adb reverse) and proves reachable
 * idle→**Green**.
 */
@RunWith(AndroidJUnit4::class)
class ConnectivityLiveTest {
  private val repo
    get() = ApplicationProvider.getApplicationContext<DashdownApp>().locator.repository

  private fun device(name: String, port: UShort) =
      Device(
          id = 0,
          name = name,
          dongleLabel = null,
          hotspotIp = "127.0.0.1",
          wifiIp = null,
          port = port,
          activeMode = ConnMode.HOTSPOT,
          password = null,
          autoSync = false,
          fileSelection = FileSelection(false, false, false, true, false, false, false, false),
          retentionMaxMinutes = null,
          autoDeleteFromComma = false,
          autoDeleteMinAgeMin = 60,
          capWarnEnabled = true,
          capWarnThresholdMinutes = 10,
      )

  @Test
  fun unreachableDeviceShowsRed() = runBlocking {
    // Nothing listens on 127.0.0.1:1 → the TCP probe fails → Red. No fixture needed.
    val d = repo.addDevice(device("Unreachable-${System.nanoTime()}", 1u))
    try {
      val c = repo.checkConnectivity(d.id)
      assertFalse("closed port must not be reachable", c.reachable)
      assertEquals(ConnDot.RED, c.dot)
    } finally {
      repo.removeDevice(d.id)
    }
  }

  @Test
  fun reachableDeviceShowsGreen() = runBlocking {
    val port = InstrumentationRegistry.getArguments().getString("mockPort")
    assumeTrue("requires mockPort + fixture + adb reverse", port != null)
    val d = repo.addDevice(device("Reachable-${System.nanoTime()}", port!!.toUShort()))
    try {
      val c = repo.checkConnectivity(d.id)
      assertTrue("fixture port must be reachable", c.reachable)
      assertEquals(ConnDot.GREEN, c.dot)
    } finally {
      repo.removeDevice(d.id)
    }
  }
}
