package org.sunnypilot.dashdown.ui.devices

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import org.junit.Rule
import org.junit.Test
import org.sunnypilot.dashdown.ui.theme.DashdownTheme
import uniffi.dashdown_core.ConnDot
import uniffi.dashdown_core.ConnMode
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.FileSelection

/**
 * Step-2 Compose UI test for the stateless device-list screen. Renders fabricated state so it
 * verifies row rendering, the connectivity-dot accessibility labels, and the empty state without
 * touching the core (the VM's real data path is covered by the integration tests in later steps).
 */
class DeviceListScreenTest {
  @get:Rule val rule = createComposeRule()

  private fun device(id: Long, name: String) =
      Device(
          id = id,
          name = name,
          dongleLabel = null,
          hotspotIp = "192.168.43.1",
          wifiIp = null,
          port = 3000.toUShort(),
          activeMode = ConnMode.HOTSPOT,
          password = null,
          autoSync = false,
          fileSelection = FileSelection(false, false, false, true, false, false, false, false),
          retentionMaxMinutes = null,
          autoDeleteFromComma = false,
          autoDeleteMinAgeMin = 0L,
          capWarnEnabled = true,
          capWarnThresholdMinutes = 10,
      )

  @Test
  fun showsRowsAndConnectivityDots() {
    val rows =
        listOf(
            DeviceRow(device(1, "Comma 3X"), ConnDot.GREEN, "2 drives · 1 complete"),
            DeviceRow(device(2, "Garage"), ConnDot.RED, "No drives yet"),
        )
    rule.setContent {
      DashdownTheme { DeviceListScreen(DeviceListUiState(rows = rows), {}, {}, {}, {}, {}) }
    }

    rule.onNodeWithTag("add_device_fab").assertExists()
    rule.onNodeWithText("Comma 3X").assertIsDisplayed()
    rule.onNodeWithTag("device_row_1").assertExists()
    rule.onNodeWithTag("device_row_2").assertExists()
    rule.onNodeWithContentDescription("conn_dot_green").assertExists()
    rule.onNodeWithContentDescription("conn_dot_red").assertExists()
  }

  @Test
  fun showsEmptyState() {
    rule.setContent {
      DashdownTheme { DeviceListScreen(DeviceListUiState(loading = false), {}, {}, {}, {}, {}) }
    }

    rule.onNodeWithText("No devices yet", substring = true).assertExists()
    rule.onNodeWithTag("add_device_fab").assertExists()
  }
}
