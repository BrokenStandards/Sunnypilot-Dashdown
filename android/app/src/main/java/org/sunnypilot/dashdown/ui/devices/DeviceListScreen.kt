@file:OptIn(ExperimentalMaterial3Api::class)

package org.sunnypilot.dashdown.ui.devices

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import org.sunnypilot.dashdown.ui.components.ConnDotIndicator
import org.sunnypilot.dashdown.ui.rememberRepository

/** Route wrapper: builds the VM from the app repository and refreshes on each resume. */
@Composable
fun DeviceListRoute(
    onAddDevice: () -> Unit,
    onDeviceClick: (Long) -> Unit,
    onDeviceEdit: (Long) -> Unit,
    onDeviceSettings: (Long) -> Unit,
) {
  val repo = rememberRepository()
  val vm: DeviceListViewModel =
      viewModel(factory = viewModelFactory { initializer { DeviceListViewModel(repo) } })
  val state by vm.state.collectAsStateWithLifecycle()
  // Reload whenever the screen resumes (initial show + return from add/edit/drives).
  LifecycleResumeEffect(Unit) {
    vm.refresh()
    onPauseOrDispose {}
  }
  DeviceListScreen(
      state = state,
      onAddDevice = onAddDevice,
      onDeviceClick = onDeviceClick,
      onDeviceEdit = onDeviceEdit,
      onDeviceSettings = onDeviceSettings,
      onRemove = vm::remove,
  )
}

@Composable
fun DeviceListScreen(
    state: DeviceListUiState,
    onAddDevice: () -> Unit,
    onDeviceClick: (Long) -> Unit,
    onDeviceEdit: (Long) -> Unit,
    onDeviceSettings: (Long) -> Unit,
    onRemove: (Long) -> Unit,
) {
  Scaffold(
      topBar = { TopAppBar(title = { Text("Devices") }) },
      floatingActionButton = {
        FloatingActionButton(onClick = onAddDevice, modifier = Modifier.testTag("add_device_fab")) {
          Icon(Icons.Filled.Add, contentDescription = "Add device")
        }
      },
  ) { padding ->
    Box(Modifier.fillMaxSize().padding(padding)) {
      when {
        state.rows.isEmpty() && state.loading ->
            CircularProgressIndicator(Modifier.align(Alignment.Center))
        state.rows.isEmpty() ->
            Text(
                "No devices yet. Tap + to add your Comma device.",
                modifier = Modifier.align(Alignment.Center).padding(24.dp),
                style = MaterialTheme.typography.bodyLarge,
            )
        else ->
            LazyColumn(Modifier.fillMaxSize()) {
              items(state.rows, key = { it.device.id }) { row ->
                DeviceRowItem(
                    row = row,
                    onClick = { onDeviceClick(row.device.id) },
                    onEdit = { onDeviceEdit(row.device.id) },
                    onSettings = { onDeviceSettings(row.device.id) },
                    onRemove = { onRemove(row.device.id) },
                )
                HorizontalDivider()
              }
            }
      }
      state.error?.let { err ->
        Text(
            "Error: $err",
            color = MaterialTheme.colorScheme.error,
            modifier = Modifier.align(Alignment.BottomCenter).padding(16.dp),
        )
      }
    }
  }
}

@Composable
private fun DeviceRowItem(
    row: DeviceRow,
    onClick: () -> Unit,
    onEdit: () -> Unit,
    onSettings: () -> Unit,
    onRemove: () -> Unit,
) {
  var menu by remember { mutableStateOf(false) }
  ListItem(
      leadingContent = { ConnDotIndicator(row.dot) },
      headlineContent = { Text(row.device.name) },
      supportingContent = { if (row.summary.isNotEmpty()) Text(row.summary) },
      trailingContent = {
        Box {
          IconButton(onClick = { menu = true }) {
            Icon(Icons.Filled.MoreVert, contentDescription = "More")
          }
          DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            DropdownMenuItem(
                text = { Text("Edit") },
                onClick = {
                  menu = false
                  onEdit()
                },
            )
            DropdownMenuItem(
                text = { Text("Settings") },
                onClick = {
                  menu = false
                  onSettings()
                },
            )
            DropdownMenuItem(
                text = { Text("Remove") },
                onClick = {
                  menu = false
                  onRemove()
                },
            )
          }
        }
      },
      modifier = Modifier.clickable(onClick = onClick).testTag("device_row_${row.device.id}"),
  )
}
