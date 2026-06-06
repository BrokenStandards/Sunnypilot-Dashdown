package org.sunnypilot.dashdown.ui.edit

import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import org.junit.Rule
import org.junit.Test
import org.sunnypilot.dashdown.ui.theme.DashdownTheme

/**
 * Step-3 Compose test for the stateless edit-device form: the fields are present and Save is gated
 * on a non-blank name + valid port.
 */
class DeviceEditScreenTest {
  @get:Rule val rule = createComposeRule()

  private fun render(state: DeviceEditState) {
    rule.setContent {
      DashdownTheme {
        DeviceEditScreen(
            state = state,
            onName = {},
            onDongleLabel = {},
            onHotspotIp = {},
            onWifiIp = {},
            onPort = {},
            onMode = {},
            onPassword = {},
            onSave = {},
            onBack = {},
        )
      }
    }
  }

  @Test
  fun fieldsPresent() {
    render(DeviceEditState())
    rule.onNodeWithTag("device_form_name").assertExists()
    rule.onNodeWithTag("device_form_hotspot_ip").assertExists()
    rule.onNodeWithTag("device_form_port").assertExists()
    rule.onNodeWithTag("device_form_mode_toggle").assertExists()
    rule.onNodeWithTag("device_form_save").assertExists()
  }

  @Test
  fun saveDisabledWhenNameBlank() {
    render(DeviceEditState(name = "", port = "3923"))
    rule.onNodeWithTag("device_form_save").assertIsNotEnabled()
  }

  @Test
  fun saveDisabledWhenPortInvalid() {
    render(DeviceEditState(name = "Comma", port = ""))
    rule.onNodeWithTag("device_form_save").assertIsNotEnabled()
  }

  @Test
  fun saveEnabledWhenValid() {
    render(DeviceEditState(name = "Comma", hotspotIp = "192.168.43.1", port = "3923"))
    rule.onNodeWithTag("device_form_save").assertIsEnabled()
  }
}
