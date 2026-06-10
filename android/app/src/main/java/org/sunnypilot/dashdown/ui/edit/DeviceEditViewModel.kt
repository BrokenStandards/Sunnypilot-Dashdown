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
    val port: String = "8080", // sunnypilot's copyparty default (HTTP+HTTPS on one port)
    val activeMode: ConnMode = ConnMode.HOTSPOT,
    val password: String = "",
    val isEdit: Boolean = false,
    val loading: Boolean = false,
    val saving: Boolean = false,
    val error: String? = null,
    val saved: Boolean = false,
    /** Non-null while a just-added device's auto-detected id awaits confirmation. */
    val pendingDongle: String? = null,
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

  /** The just-added device awaiting dongle-id confirmation (set by [save]). */
  private var added: Device? = null

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

  fun onPassword(v: String) = _state.update { it.copy(password = v) }

  fun save() {
    val s = _state.value
    val port = s.portValue ?: return
    viewModelScope.launch {
      _state.update { it.copy(saving = true, error = null) }
      try {
        val base = original
        if (base != null) {
          // Edit: preserve the settings fields not shown here; a blank password keeps the current.
          repo.updateDevice(
              base.copy(
                  name = s.name.trim(),
                  dongleLabel = s.dongleLabel.ifBlank { null },
                  hotspotIp = s.hotspotIp.trim(),
                  wifiIp = s.wifiIp.ifBlank { null },
                  port = port,
                  activeMode = s.activeMode,
                  password = s.password.ifBlank { base.password },
              ))
          _state.update { it.copy(saving = false, saved = true) }
          return@launch
        }

        // New device: add it, then connect and offer the auto-detected device id
        // for confirmation if the user left the Dongle ID blank.
        val device =
            repo.addDevice(
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
                ))
        val detected =
            if (s.dongleLabel.isBlank()) {
              runCatching { repo.detectDeviceName(device.id) }.getOrNull()?.ifBlank { null }
            } else null
        if (detected != null) {
          added = device
          _state.update { it.copy(saving = false, pendingDongle = detected) }
        } else {
          _state.update { it.copy(saving = false, saved = true) }
        }
      } catch (t: Throwable) {
        _state.update { it.copy(saving = false, error = UiError.from(t).message) }
      }
    }
  }

  /** Accept the detected device id as the dongle label, then finish. */
  fun confirmDongle() {
    val name = _state.value.pendingDongle ?: return
    val dev = added ?: return
    viewModelScope.launch {
      runCatching { repo.updateDevice(dev.copy(dongleLabel = name)) }
      _state.update { it.copy(pendingDongle = null, saved = true) }
    }
  }

  /** Dismiss the detected id (leave the dongle label unset) and finish. */
  fun dismissDongle() = _state.update { it.copy(pendingDongle = null, saved = true) }
}
