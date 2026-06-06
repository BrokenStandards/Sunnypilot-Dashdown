package org.sunnypilot.dashdown.ui.drives

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.sunnypilot.dashdown.data.DashdownRepository
import org.sunnypilot.dashdown.data.DriveProgress
import org.sunnypilot.dashdown.data.UiError
import uniffi.dashdown_core.Drive

data class DrivesUiState(
    val drives: List<Drive> = emptyList(),
    val loading: Boolean = true, // initial offline load (from the local mirror)
    val refreshing: Boolean = false, // pull-to-refresh online sync
    val error: String? = null,
)

class DrivesListViewModel(private val repo: DashdownRepository, private val deviceId: Long) :
    ViewModel() {
  private val _state = MutableStateFlow(DrivesUiState())
  val state: StateFlow<DrivesUiState> = _state.asStateFlow()

  /** Live download progress keyed by driveKey (drives the Downloading badge). */
  val progress: StateFlow<Map<String, DriveProgress>> = repo.progress

  init {
    loadOffline()
    // A download finishing (complete/failed) changes sync state — reclassify from disk.
    viewModelScope.launch { repo.terminalEvents.collect { loadOffline() } }
  }

  /** Fast, network-free load that reclassifies sync state from the local mirror. */
  fun loadOffline() {
    viewModelScope.launch {
      try {
        val drives = repo.listDrives(deviceId, offline = true)
        _state.update { it.copy(drives = drives, loading = false, error = null) }
      } catch (t: Throwable) {
        _state.update { it.copy(loading = false, error = UiError.from(t).message) }
      }
    }
  }

  /** Pull-to-refresh: hit the device over the network to refresh the index. */
  fun refreshOnline() {
    viewModelScope.launch {
      _state.update { it.copy(refreshing = true, error = null) }
      try {
        val drives = repo.listDrives(deviceId, offline = false)
        _state.update { it.copy(drives = drives, refreshing = false) }
      } catch (t: Throwable) {
        _state.update { it.copy(refreshing = false, error = UiError.from(t).message) }
      }
    }
  }

  fun togglePreserve(drive: Drive) {
    viewModelScope.launch {
      runCatching { repo.setPreserved(deviceId, drive.driveKey, !drive.preserved) }
      loadOffline()
    }
  }
}
