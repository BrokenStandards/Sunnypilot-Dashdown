package org.sunnypilot.dashdown.ui.edit

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.sunnypilot.dashdown.data.DashdownRepository
import org.sunnypilot.dashdown.data.UiError
import uniffi.dashdown_core.ConnMode
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.FileSelection

/**
 * Editable identity + connection form. Settings (auto-sync, file selection, retention, …) are
 * edited on the separate settings screen and preserved across an edit here.
 */
data class DeviceEditState(
    val name: String = "",
    val dongleLabel: String = "",
    val hotspotIp: String = "192.168.43.1",
    val wifiIp: String = "",
    val port: String = "3923",
    val activeMode: ConnMode = ConnMode.HOTSPOT,
    val password: String = "",
    val isEdit: Boolean = false,
    val loading: Boolean = false,
    val saving: Boolean = false,
    val error: String? = null,
    val saved: Boolean = false,
) {
  val portValue: UShort?
    get() = port.trim().toUShortOrNull()

  val canSave: Boolean
    get() = name.isNotBlank() && hotspotIp.isNotBlank() && portValue != null && !saving
}

class DeviceEditViewModel(private val repo: DashdownRepository, private val deviceId: Long?) :
    ViewModel() {
  private val _state =
      MutableStateFlow(DeviceEditState(isEdit = deviceId != null, loading = deviceId != null))
  val state: StateFlow<DeviceEditState> = _state.asStateFlow()

  /** The device as loaded, so an edit preserves the settings fields not shown on this form. */
  private var original: Device? = null

  init {
    if (deviceId != null) load()
  }

  private fun load() {
    viewModelScope.launch {
      try {
        val d = repo.listDevices().firstOrNull { it.id == deviceId }
        if (d == null) {
          _state.update { it.copy(loading = false, error = "Device not found") }
          return@launch
        }
        original = d
        _state.update {
          it.copy(
              name = d.name,
              dongleLabel = d.dongleLabel ?: "",
              hotspotIp = d.hotspotIp,
              wifiIp = d.wifiIp ?: "",
              port = d.port.toString(),
              activeMode = d.activeMode,
              password = "",
              loading = false,
          )
        }
      } catch (t: Throwable) {
        _state.update { it.copy(loading = false, error = UiError.from(t).message) }
      }
    }
  }

  fun onName(v: String) = _state.update { it.copy(name = v) }

  fun onDongleLabel(v: String) = _state.update { it.copy(dongleLabel = v) }

  fun onHotspotIp(v: String) = _state.update { it.copy(hotspotIp = v) }

  fun onWifiIp(v: String) = _state.update { it.copy(wifiIp = v) }

  fun onPort(v: String) = _state.update { it.copy(port = v.filter(Char::isDigit)) }

  fun onMode(mode: ConnMode) = _state.update { it.copy(activeMode = mode) }

  fun onPassword(v: String) = _state.update { it.copy(password = v) }

  fun save() {
    val s = _state.value
    val port = s.portValue ?: return
    viewModelScope.launch {
      _state.update { it.copy(saving = true, error = null) }
      try {
        val base = original
        val device =
            if (base != null) {
              // Preserve the settings fields (file selection, retention, auto-*) from the loaded
              // device; a blank password keeps the existing one.
              base.copy(
                  name = s.name.trim(),
                  dongleLabel = s.dongleLabel.ifBlank { null },
                  hotspotIp = s.hotspotIp.trim(),
                  wifiIp = s.wifiIp.ifBlank { null },
                  port = port,
                  activeMode = s.activeMode,
                  password = s.password.ifBlank { base.password },
              )
            } else {
              Device(
                  id = 0,
                  name = s.name.trim(),
                  dongleLabel = s.dongleLabel.ifBlank { null },
                  hotspotIp = s.hotspotIp.trim(),
                  wifiIp = s.wifiIp.ifBlank { null },
                  port = port,
                  activeMode = s.activeMode,
                  password = s.password.ifBlank { null },
                  autoSync = false,
                  // Lightweight default: previews only (qcamera + muxed audio).
                  fileSelection =
                      FileSelection(false, false, false, true, false, false, false, false),
                  retentionMaxMinutes = null,
                  autoDeleteFromComma = false,
                  autoDeleteMinAgeMin = 60,
              )
            }
        if (base != null) repo.updateDevice(device) else repo.addDevice(device)
        _state.update { it.copy(saving = false, saved = true) }
      } catch (t: Throwable) {
        _state.update { it.copy(saving = false, error = UiError.from(t).message) }
      }
    }
  }
}
