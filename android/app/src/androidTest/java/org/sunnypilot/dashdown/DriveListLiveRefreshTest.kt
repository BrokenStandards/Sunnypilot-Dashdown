package org.sunnypilot.dashdown

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.test.core.app.ApplicationProvider
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.sunnypilot.dashdown.ui.drives.DrivesListRoute
import org.sunnypilot.dashdown.ui.theme.DashdownTheme

/**
 * Phase C live test: the drive **list membership** updates on its own (no manual refresh) while the
 * drives screen is open — a drive added on the device appears, and a removed drive disappears.
 * Drives the mock control port `/add_drive` // `/remove_drive` on a dedicated route (cleaned up in
 * `finally`), and relies on the foreground poll (injected short interval).
 *
 * Skipped unless `mockPort`+`controlPort` are supplied (see docs/TESTING.md /
 * tools/run-android-e2e.sh).
 */
class DriveListLiveRefreshTest {
  @get:Rule val rule = createComposeRule()

  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun driveAppearsAndDisappearsWithoutManualRefresh() {
    val args = InstrumentationRegistry.getArguments()
    val port = args.getString("mockPort")
    val controlPort = args.getString("controlPort")
    assumeTrue(
        "requires mockPort + controlPort + fixture + adb reverse",
        port != null && controlPort != null)
    val cp = controlPort!!.toInt()
    val route = "000009ee--phasecadd" // dedicated route, isolated from the single_drive fixture
    val newTag = "drive_row_$route--0" // drive_key = first segment dir name
    val device = runBlocking {
      repo.addDevice(probeDevice("DriveLive-${System.nanoTime()}", port!!.toUShort()))
    }
    try {
      rule.setContent { DashdownTheme { DrivesListRoute(device.id, {}, {}, drivesPollMs = 150L) } }

      // Baseline: the fixture's drive loads (init does the first online sync).
      rule.waitUntil(20_000) {
        rule
            .onAllNodesWithTag("drive_row_000001a3--c20ba54385--0")
            .fetchSemanticsNodes()
            .isNotEmpty()
      }
      // A new drive appears on the device → the poll adds the row, no manual refresh.
      MockControl.post(cp, "/add_drive", "{\"route\":\"$route\",\"segs\":1}")
      rule.waitUntil(20_000) { rule.onAllNodesWithTag(newTag).fetchSemanticsNodes().isNotEmpty() }
      // Removed from the device (models the comma's low-space auto-prune) → the poll drops the row.
      MockControl.post(cp, "/remove_drive", "{\"route\":\"$route\"}")
      rule.waitUntil(20_000) { rule.onAllNodesWithTag(newTag).fetchSemanticsNodes().isEmpty() }
    } finally {
      runCatching { MockControl.post(cp, "/remove_drive", "{\"route\":\"$route\"}") }
      runBlocking { repo.removeDevice(device.id) }
    }
  }
}
