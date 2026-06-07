package org.sunnypilot.dashdown.ui.detail

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
import uniffi.dashdown_core.DriveSyncStatus
import uniffi.dashdown_core.FileKind

data class DriveDetailUiState(
    val drive: Drive? = null,
    val status: DriveSyncStatus? = null,
    val loading: Boolean = true,
    val error: String? = null,
    /** Ordered absolute paths of every downloaded `qcamera.ts` — one continuous drive timeline. */
    val playablePaths: List<String> = emptyList(),
)

class DriveDetailViewModel(
    private val repo: DashdownRepository,
    private val deviceId: Long,
    private val driveKey: String,
) : ViewModel() {
  private val _state = MutableStateFlow(DriveDetailUiState())
  val state: StateFlow<DriveDetailUiState> = _state.asStateFlow()

  val progress: StateFlow<Map<String, DriveProgress>> = repo.progress

  init {
    load()
    viewModelScope.launch { repo.terminalEvents.collect { if (it.driveKey == driveKey) load() } }
  }

  fun load() {
    viewModelScope.launch {
      try {
        val drive = repo.getDrive(deviceId, driveKey)
        val status = runCatching { repo.getDriveStatus(deviceId, driveKey) }.getOrNull()
        _state.update {
          it.copy(
              drive = drive,
              status = status,
              loading = false,
              error = null,
              playablePaths = resolvePlayables(),
          )
        }
      } catch (t: Throwable) {
        _state.update { it.copy(loading = false, error = UiError.from(t).message) }
      }
    }
  }

  fun togglePreserve() {
    val drive = _state.value.drive ?: return
    viewModelScope.launch {
      runCatching { repo.setPreserved(deviceId, driveKey, !drive.preserved) }
      load()
    }
  }

  /**
   * Every downloaded `qcamera.ts` in this drive, ordered by segment — resolved by the core (the
   * single source of truth for on-disk paths). The player treats them as one drive-wide timeline.
   */
  private suspend fun resolvePlayables(): List<String> =
      runCatching { repo.driveLocalPaths(deviceId, driveKey, FileKind.Q_CAMERA).map { it.path } }
          .getOrElse { emptyList() }
}
