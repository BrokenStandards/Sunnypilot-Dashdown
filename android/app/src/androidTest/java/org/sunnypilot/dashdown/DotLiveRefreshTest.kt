package org.sunnypilot.dashdown

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.test.core.app.ApplicationProvider
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.sunnypilot.dashdown.ui.devices.DeviceListRoute
import org.sunnypilot.dashdown.ui.theme.DashdownTheme
import uniffi.dashdown_core.ConnMode
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.FileSelection

/**
 * Phase C live test: the connectivity dot re-probes **on its own** while the device list is open
 * and reflects the server **stopping and restarting** with no manual refresh — exactly the
 * foreground poll mechanism.
 *
 * The server is stopped/started via the mock's `/reachable` control knob (which drops/rebinds the
 * data listener). To make that outage observable we point the data port at the emulator's host-
 * loopback alias **10.0.2.2** (→ host 127.0.0.1) rather than `127.0.0.1` over `adb reverse`: a
 * reverse tunnel accepts the device-side TCP connection even when the host listener is closed,
 * which would mask the outage (the same reason the hermetic `unreachableDeviceShowsRed` test uses a
 * closed unreversed port). The control port is still reached at `127.0.0.1:controlPort` over
 * reverse.
 *
 * Skipped unless `mockPort`+`controlPort` are supplied; emulator-only (relies on the 10.0.2.2
 * alias).
 */
class DotLiveRefreshTest {
  @get:Rule val rule = createComposeRule()

  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun dotReflectsServerStopStartWithoutManualRefresh() {
    val args = InstrumentationRegistry.getArguments()
    val port = args.getString("mockPort")
    val controlPort = args.getString("controlPort")
    assumeTrue(
        "requires mockPort + controlPort + fixture + adb reverse",
        port != null && controlPort != null)
    val cp = controlPort!!.toInt()
    // Data port via the emulator host alias (no adb reverse) so /reachable outages are observable.
    val device = runBlocking {
      repo.addDevice(probeDevice("Dot-${System.nanoTime()}", port!!.toUShort(), host = "10.0.2.2"))
    }
    try {
      rule.setContent { DashdownTheme { DeviceListRoute({}, {}, {}, {}, dotPollMs = 150L) } }

      // Server up → green.
      rule.waitUntil(15_000) {
        rule.onAllNodesWithContentDescription("conn_dot_green").fetchSemanticsNodes().isNotEmpty()
      }
      // Server stops → the poll re-probes and flips the dot to red on its own.
      MockControl.post(cp, "/reachable", "{\"up\":false}")
      rule.waitUntil(15_000) {
        rule.onAllNodesWithContentDescription("conn_dot_red").fetchSemanticsNodes().isNotEmpty()
      }
      // Server restarts → back to green, again with no manual refresh.
      MockControl.post(cp, "/reachable", "{\"up\":true}")
      rule.waitUntil(15_000) {
        rule.onAllNodesWithContentDescription("conn_dot_green").fetchSemanticsNodes().isNotEmpty()
      }
    } finally {
      runCatching { MockControl.post(cp, "/reachable", "{\"up\":true}") }
      runBlocking { repo.removeDevice(device.id) }
    }
  }
}

/** A reachable (autoSync=OFF, so B2 never touches it) device at [host]:[port]. */
internal fun probeDevice(name: String, port: UShort, host: String = "127.0.0.1"): Device =
    Device(
        id = 0,
        name = name,
        dongleLabel = null,
        hotspotIp = host,
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
