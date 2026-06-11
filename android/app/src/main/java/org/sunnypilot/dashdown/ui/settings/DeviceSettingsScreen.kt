@file:OptIn(ExperimentalMaterial3Api::class)

package org.sunnypilot.dashdown.ui.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import org.sunnypilot.dashdown.ui.rememberRepository
import uniffi.dashdown_core.FileSelection

@Composable
fun DeviceSettingsRoute(deviceId: Long, onDone: () -> Unit) {
  val repo = rememberRepository()
  val vm: DeviceSettingsViewModel =
      viewModel(
          factory = viewModelFactory { initializer { DeviceSettingsViewModel(repo, deviceId) } })
  val state by vm.state.collectAsStateWithLifecycle()
  LaunchedEffect(state.saved) { if (state.saved) onDone() }
  DeviceSettingsScreen(
      state = state,
      onAutoSync = vm::onAutoSync,
      onFileSelection = vm::onFileSelection,
      onRetention = vm::onRetention,
      onAutoDelete = vm::onAutoDelete,
      onMinAge = vm::onMinAge,
      onSave = vm::save,
      onBack = onDone,
  )
}

@Composable
fun DeviceSettingsScreen(
    state: DeviceSettingsState,
    onAutoSync: (Boolean) -> Unit,
    onFileSelection: (FileSelection) -> Unit,
    onRetention: (String) -> Unit,
    onAutoDelete: (Boolean) -> Unit,
    onMinAge: (String) -> Unit,
    onSave: () -> Unit,
    onBack: () -> Unit,
) {
  val fs = state.fileSelection
  Scaffold(
      topBar = {
        TopAppBar(
            title = { Text("Device settings") },
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
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
          SwitchRow("Auto-sync over Wi-Fi", state.autoSync, onAutoSync, "settings_autosync")

          HorizontalDivider()
          Text("Files to download")
          FileRow("Road camera (fcamera)", fs.fcamera, "settings_file_fcamera") {
            onFileSelection(fs.copy(fcamera = it))
          }
          FileRow("Wide road camera (ecamera)", fs.ecamera, "settings_file_ecamera") {
            onFileSelection(fs.copy(ecamera = it))
          }
          FileRow("Driver camera (dcamera)", fs.dcamera, "settings_file_dcamera") {
            onFileSelection(fs.copy(dcamera = it))
          }
          FileRow("Preview + audio (qcamera)", fs.qcamera, "settings_file_qcamera") {
            onFileSelection(fs.copy(qcamera = it))
          }
          FileRow("Raw log (rlog)", fs.rlog, "settings_file_rlog") {
            onFileSelection(fs.copy(rlog = it))
          }
          FileRow("Quick log (qlog)", fs.qlog, "settings_file_qlog") {
            onFileSelection(fs.copy(qlog = it))
          }
          FileRow("Boot log (bootlog)", fs.bootlog, "settings_file_bootlog") {
            onFileSelection(fs.copy(bootlog = it))
          }
          FileRow("Other files", fs.other, "settings_file_other") {
            onFileSelection(fs.copy(other = it))
          }

          HorizontalDivider()
          OutlinedTextField(
              value = state.retentionMinutes,
              onValueChange = onRetention,
              label = { Text("Keep local footage up to (minutes)") },
              placeholder = { Text("Unlimited") },
              singleLine = true,
              keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
              modifier = Modifier.fillMaxWidth().testTag("settings_retention"),
          )
          Text(
              buildString {
                append("Using ~${state.localMinutes} min on this phone")
                state.retentionMinutes.toLongOrNull()?.let { append(" · budget $it min") }
              },
              style = MaterialTheme.typography.bodySmall,
              modifier = Modifier.testTag("settings_storage_usage"),
          )

          HorizontalDivider()
          SwitchRow(
              "Auto-delete from device after download",
              state.autoDeleteFromComma,
              onAutoDelete,
              "settings_autodelete",
          )
          OutlinedTextField(
              value = state.autoDeleteMinAgeMin,
              onValueChange = onMinAge,
              label = { Text("…only footage older than (minutes)") },
              singleLine = true,
              enabled = state.autoDeleteFromComma,
              keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
              modifier = Modifier.fillMaxWidth().testTag("settings_min_age"),
          )

          state.error?.let { Text("Error: $it") }
          Button(
              onClick = onSave,
              enabled = !state.saving && !state.loading,
              modifier = Modifier.fillMaxWidth().testTag("settings_save"),
          ) {
            Text(if (state.saving) "Saving…" else "Save")
          }
        }
      }
}

@Composable
private fun SwitchRow(label: String, checked: Boolean, onChange: (Boolean) -> Unit, tag: String) {
  Row(
      Modifier.fillMaxWidth(),
      horizontalArrangement = Arrangement.SpaceBetween,
      verticalAlignment = Alignment.CenterVertically,
  ) {
    Text(label, modifier = Modifier.weight(1f))
    Switch(checked = checked, onCheckedChange = onChange, modifier = Modifier.testTag(tag))
  }
}

@Composable
private fun FileRow(label: String, checked: Boolean, tag: String, onChange: (Boolean) -> Unit) {
  Row(
      Modifier.fillMaxWidth(),
      horizontalArrangement = Arrangement.SpaceBetween,
      verticalAlignment = Alignment.CenterVertically,
  ) {
    Text(label, modifier = Modifier.weight(1f))
    Checkbox(checked = checked, onCheckedChange = onChange, modifier = Modifier.testTag(tag))
  }
}
