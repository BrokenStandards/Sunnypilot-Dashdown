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
    /** Absolute path to a downloaded, playable `qcamera.ts` in this drive, if any. */
    val playablePath: String? = null,
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
              playablePath = resolvePlayable(),
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
   * First complete `qcamera.ts` in this drive, resolved by the core (the single source of truth for
   * on-disk paths — see `AppCore.driveLocalPaths`). Returns null if nothing is mirrored yet.
   */
  private suspend fun resolvePlayable(): String? =
      runCatching {
            repo.driveLocalPaths(deviceId, driveKey, FileKind.Q_CAMERA).firstOrNull()?.path
          }
          .getOrNull()
}
