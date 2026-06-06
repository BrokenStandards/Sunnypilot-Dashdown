@file:OptIn(ExperimentalMaterial3Api::class)

package org.sunnypilot.dashdown.ui.edit

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import org.sunnypilot.dashdown.ui.rememberRepository
import uniffi.dashdown_core.ConnMode

@Composable
fun DeviceEditRoute(deviceId: Long?, onDone: () -> Unit) {
  val repo = rememberRepository()
  val vm: DeviceEditViewModel =
      viewModel(factory = viewModelFactory { initializer { DeviceEditViewModel(repo, deviceId) } })
  val state by vm.state.collectAsStateWithLifecycle()
  LaunchedEffect(state.saved) { if (state.saved) onDone() }
  DeviceEditScreen(
      state = state,
      onName = vm::onName,
      onDongleLabel = vm::onDongleLabel,
      onHotspotIp = vm::onHotspotIp,
      onWifiIp = vm::onWifiIp,
      onPort = vm::onPort,
      onMode = vm::onMode,
      onPassword = vm::onPassword,
      onSave = vm::save,
      onBack = onDone,
  )
}

@Composable
fun DeviceEditScreen(
    state: DeviceEditState,
    onName: (String) -> Unit,
    onDongleLabel: (String) -> Unit,
    onHotspotIp: (String) -> Unit,
    onWifiIp: (String) -> Unit,
    onPort: (String) -> Unit,
    onMode: (ConnMode) -> Unit,
    onPassword: (String) -> Unit,
    onSave: () -> Unit,
    onBack: () -> Unit,
) {
  Scaffold(
      topBar = {
        TopAppBar(
            title = { Text(if (state.isEdit) "Edit device" else "Add device") },
            navigationIcon = {
              IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
              }
            },
        )
      }) { padding ->
        Column(
            Modifier.fillMaxSize()
                .padding(padding)
                .padding(16.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
          OutlinedTextField(
              value = state.name,
              onValueChange = onName,
              label = { Text("Name") },
              singleLine = true,
              isError = state.name.isBlank(),
              modifier = Modifier.fillMaxWidth().testTag("device_form_name"),
          )
          OutlinedTextField(
              value = state.dongleLabel,
              onValueChange = onDongleLabel,
              label = { Text("Dongle ID (optional)") },
              singleLine = true,
              modifier = Modifier.fillMaxWidth().testTag("device_form_dongle"),
          )
          SingleChoiceSegmentedButtonRow(modifier = Modifier.testTag("device_form_mode_toggle")) {
            SegmentedButton(
                selected = state.activeMode == ConnMode.HOTSPOT,
                onClick = { onMode(ConnMode.HOTSPOT) },
                shape = SegmentedButtonDefaults.itemShape(index = 0, count = 2),
            ) {
              Text("Hotspot")
            }
            SegmentedButton(
                selected = state.activeMode == ConnMode.WIFI,
                onClick = { onMode(ConnMode.WIFI) },
                shape = SegmentedButtonDefaults.itemShape(index = 1, count = 2),
            ) {
              Text("Wi-Fi")
            }
          }
          OutlinedTextField(
              value = state.hotspotIp,
              onValueChange = onHotspotIp,
              label = { Text("Hotspot IP") },
              singleLine = true,
              isError = state.hotspotIp.isBlank(),
              modifier = Modifier.fillMaxWidth().testTag("device_form_hotspot_ip"),
          )
          OutlinedTextField(
              value = state.wifiIp,
              onValueChange = onWifiIp,
              label = { Text("Wi-Fi IP (optional)") },
              singleLine = true,
              modifier = Modifier.fillMaxWidth().testTag("device_form_wifi_ip"),
          )
          OutlinedTextField(
              value = state.port,
              onValueChange = onPort,
              label = { Text("Port") },
              singleLine = true,
              isError = state.portValue == null,
              keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
              modifier = Modifier.fillMaxWidth().testTag("device_form_port"),
          )
          OutlinedTextField(
              value = state.password,
              onValueChange = onPassword,
              label = { Text("Password") },
              singleLine = true,
              visualTransformation = PasswordVisualTransformation(),
              supportingText =
                  if (state.isEdit) {
                    { Text("Leave blank to keep the current password") }
                  } else {
                    null
                  },
              modifier = Modifier.fillMaxWidth().testTag("device_form_password"),
          )
          state.error?.let { Text("Error: $it") }
          Button(
              onClick = onSave,
              enabled = state.canSave,
              modifier = Modifier.fillMaxWidth().testTag("device_form_save"),
          ) {
            Text(if (state.saving) "Saving…" else "Save")
          }
        }
      }
}
