package org.sunnypilot.dashdown.ui.devices

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.sunnypilot.dashdown.data.DashdownRepository
import org.sunnypilot.dashdown.data.UiError
import uniffi.dashdown_core.ConnDot
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.Drive
import uniffi.dashdown_core.SyncStatus

/** A device row: identity + a resolved connectivity dot + a short sync summary. */
data class DeviceRow(val device: Device, val dot: ConnDot?, val summary: String)

data class DeviceListUiState(
    val rows: List<DeviceRow> = emptyList(),
    val loading: Boolean = false,
    val error: String? = null,
)

class DeviceListViewModel(private val repo: DashdownRepository) : ViewModel() {
  private val _state = MutableStateFlow(DeviceListUiState(loading = true))
  val state: StateFlow<DeviceListUiState> = _state.asStateFlow()

  /**
   * Reload the device list. Each device's connectivity dot and offline sync summary are resolved
   * concurrently and failures are tolerated (a probe that errors → unknown dot), so one unreachable
   * device never blanks the whole list.
   */
  fun refresh() {
    viewModelScope.launch {
      _state.update { it.copy(loading = true, error = null) }
      try {
        val devices = repo.listDevices()
        val rows = coroutineScope {
          devices
              .map { d ->
                async {
                  val dot = runCatching { repo.checkConnectivity(d.id).dot }.getOrNull()
                  val summary =
                      runCatching { summarize(repo.listDrives(d.id, offline = true)) }
                          .getOrDefault("")
                  DeviceRow(d, dot, summary)
                }
              }
              .awaitAll()
        }
        _state.update { DeviceListUiState(rows = rows, loading = false) }
      } catch (t: Throwable) {
        _state.update { it.copy(loading = false, error = UiError.from(t).message) }
      }
    }
  }

  fun remove(deviceId: Long) {
    viewModelScope.launch {
      runCatching { repo.removeDevice(deviceId) }
      refresh()
    }
  }
}

private fun summarize(drives: List<Drive>): String {
  if (drives.isEmpty()) return "No drives yet"
  val complete = drives.count { it.syncState == SyncStatus.COMPLETE }
  val partial = drives.count { it.syncState == SyncStatus.PARTIAL }
  return buildString {
    append(drives.size).append(if (drives.size == 1) " drive" else " drives")
    if (complete > 0) append(" · ").append(complete).append(" complete")
    if (partial > 0) append(" · ").append(partial).append(" partial")
  }
}
