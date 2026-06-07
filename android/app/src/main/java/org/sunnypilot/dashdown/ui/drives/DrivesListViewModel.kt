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
import uniffi.dashdown_core.FileKind

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

  /**
   * Per-drive thumbnail source: the first complete `qcamera.ts` path once resolved. A row
   * (re)requests whenever its drive's sync state changes, so a freshly-downloaded drive gets a
   * frame; an entry is stored only once a real path is found (absent = not resolvable yet).
   */
  private val _thumbnails = MutableStateFlow<Map<String, String?>>(emptyMap())
  val thumbnails: StateFlow<Map<String, String?>> = _thumbnails.asStateFlow()

  init {
    // First visit: show the local mirror immediately, then auto-sync online if it's empty (a
    // freshly-added device has nothing locally yet, and pull-to-refresh can't help an empty list).
    viewModelScope.launch {
      try {
        val offline = repo.listDrives(deviceId, offline = true)
        _state.update { it.copy(drives = offline, loading = false, error = null) }
        if (offline.isEmpty()) refreshOnline()
      } catch (t: Throwable) {
        _state.update { it.copy(loading = false, error = UiError.from(t).message) }
      }
    }
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

  /** Resolve [drive]'s thumbnail source (called by its row on first composition + sync changes). */
  fun requestThumbnail(drive: Drive) {
    if (_thumbnails.value[drive.driveKey] != null) return // already have a frame source
    viewModelScope.launch {
      val path =
          runCatching {
                repo
                    .driveLocalPaths(deviceId, drive.driveKey, FileKind.Q_CAMERA)
                    .firstOrNull()
                    ?.path
              }
              .getOrNull()
      if (path != null) _thumbnails.update { it + (drive.driveKey to path) }
    }
  }
}
