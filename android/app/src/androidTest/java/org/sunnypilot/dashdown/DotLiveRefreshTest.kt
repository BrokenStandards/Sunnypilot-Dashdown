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
 * Phase C live test: the connectivity dot **re-probes on its own** while the device list is open
 * and reflects reachability changes with **no manual refresh** — the foreground poll mechanism.
 *
 * Reachability is flipped by pointing the device at a dead, *unreversed* port (genuine connection
 * refused → red) vs. the live mock port (→ green). We can't toggle the mock's `/reachable` here
 * because `adb reverse` accepts the device-side TCP connection even when the host listener is
 * closed, masking the outage (this is why the hermetic `unreachableDeviceShowsRed` test also uses a
 * closed unreversed port). After the device becomes reachable, only the **poll** re-probes —
 * nothing calls `refresh()` — so a green→ here proves the poll is driving the update.
 *
 * Skipped unless `mockPort` is supplied (see docs/TESTING.md / tools/run-android-e2e.sh).
 */
class DotLiveRefreshTest {
  @get:Rule val rule = createComposeRule()

  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun dotReflectsReachabilityChangesWithoutManualRefresh() {
    val port = InstrumentationRegistry.getArguments().getString("mockPort")
    assumeTrue("requires mockPort + fixture + adb reverse", port != null)
    val deadPort =
        9099.toUShort() // unreversed → genuine connection-refused on the emulator loopback
    val livePort = port!!.toUShort()
    // Start UNREACHABLE so the initial (loud) load shows red.
    val device = runBlocking { repo.addDevice(probeDevice("Dot-${System.nanoTime()}", deadPort)) }
    try {
      rule.setContent { DashdownTheme { DeviceListRoute({}, {}, {}, {}, dotPollMs = 150L) } }

      rule.waitUntil(15_000) {
        rule.onAllNodesWithContentDescription("conn_dot_red").fetchSemanticsNodes().isNotEmpty()
      }
      // Device becomes reachable → ONLY the poll re-probes (no manual refresh) → green.
      runBlocking { repo.updateDevice(device.copy(port = livePort)) }
      rule.waitUntil(15_000) {
        rule.onAllNodesWithContentDescription("conn_dot_green").fetchSemanticsNodes().isNotEmpty()
      }
      // Device goes away again → poll → red.
      runBlocking { repo.updateDevice(device.copy(port = deadPort)) }
      rule.waitUntil(15_000) {
        rule.onAllNodesWithContentDescription("conn_dot_red").fetchSemanticsNodes().isNotEmpty()
      }
    } finally {
      runBlocking { repo.removeDevice(device.id) }
    }
  }
}

/**
 * A reachable (autoSync=OFF, so B2 never touches it) device on 127.0.0.1:[port] via adb reverse.
 */
internal fun probeDevice(name: String, port: UShort): Device =
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
    )
