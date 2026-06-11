package org.sunnypilot.dashdown.ui.settings

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.sunnypilot.dashdown.data.DashdownRepository
import org.sunnypilot.dashdown.data.UiError
import uniffi.dashdown_core.DeviceSettings
import uniffi.dashdown_core.FileSelection

data class DeviceSettingsState(
    val autoSync: Boolean = false,
    // Lightweight default mirrors a new device: previews only (qcamera).
    val fileSelection: FileSelection =
        FileSelection(false, false, false, true, false, false, false, false),
    val retentionMinutes: String = "", // blank = unlimited (null)
    val localMinutes: Long = 0L, // minutes (≈ segments) of footage on disk right now
    val autoDeleteFromComma: Boolean = false,
    val autoDeleteMinAgeMin: String = "60",
    val capWarnEnabled: Boolean = true,
    val capWarnThresholdMinutes: String = "10",
    val loading: Boolean = true,
    val saving: Boolean = false,
    val error: String? = null,
    val saved: Boolean = false,
)

class DeviceSettingsViewModel(private val repo: DashdownRepository, private val deviceId: Long) :
    ViewModel() {
  private val _state = MutableStateFlow(DeviceSettingsState())
  val state: StateFlow<DeviceSettingsState> = _state.asStateFlow()

  init {
    load()
  }

  private fun load() {
    viewModelScope.launch {
      try {
        val s = repo.getSettings(deviceId)
        val localMin = runCatching { repo.retentionStatus(deviceId).localMinutes }.getOrDefault(0L)
        _state.update {
          it.copy(
              autoSync = s.autoSync,
              fileSelection = s.fileSelection,
              retentionMinutes = s.retentionMaxMinutes?.toString() ?: "",
              localMinutes = localMin,
              autoDeleteFromComma = s.autoDeleteFromComma,
              autoDeleteMinAgeMin = s.autoDeleteMinAgeMin.toString(),
              capWarnEnabled = s.capWarnEnabled,
              capWarnThresholdMinutes = s.capWarnThresholdMinutes.toString(),
              loading = false,
          )
        }
      } catch (t: Throwable) {
        _state.update { it.copy(loading = false, error = UiError.from(t).message) }
      }
    }
  }

  fun onAutoSync(v: Boolean) = _state.update { it.copy(autoSync = v) }

  fun onFileSelection(fs: FileSelection) = _state.update { it.copy(fileSelection = fs) }

  fun onRetention(v: String) = _state.update { it.copy(retentionMinutes = v.filter(Char::isDigit)) }

  fun onAutoDelete(v: Boolean) = _state.update { it.copy(autoDeleteFromComma = v) }

  fun onMinAge(v: String) = _state.update { it.copy(autoDeleteMinAgeMin = v.filter(Char::isDigit)) }

  fun onCapWarnEnabled(v: Boolean) = _state.update { it.copy(capWarnEnabled = v) }

  fun onCapWarnThreshold(v: String) =
      _state.update { it.copy(capWarnThresholdMinutes = v.filter(Char::isDigit)) }

  fun save() {
    viewModelScope.launch {
      _state.update { it.copy(saving = true, error = null) }
      try {
        val s = _state.value
        repo.setSettings(
            deviceId,
            DeviceSettings(
                autoSync = s.autoSync,
                fileSelection = s.fileSelection,
                retentionMaxMinutes = s.retentionMinutes.toLongOrNull(),
                autoDeleteFromComma = s.autoDeleteFromComma,
                autoDeleteMinAgeMin = s.autoDeleteMinAgeMin.toLongOrNull() ?: 0L,
                capWarnEnabled = s.capWarnEnabled,
                capWarnThresholdMinutes = s.capWarnThresholdMinutes.toLongOrNull() ?: 10L,
            ),
        )
        _state.update { it.copy(saving = false, saved = true) }
      } catch (t: Throwable) {
        _state.update { it.copy(saving = false, error = UiError.from(t).message) }
      }
    }
  }
}
