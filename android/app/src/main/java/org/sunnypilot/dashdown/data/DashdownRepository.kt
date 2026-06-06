package org.sunnypilot.dashdown.data

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext
import org.sunnypilot.dashdown.ServiceLocator
import uniffi.dashdown_core.ConnMode
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.DeviceConnectivity
import uniffi.dashdown_core.DeviceSettings
import uniffi.dashdown_core.Drive
import uniffi.dashdown_core.DriveSyncStatus
import uniffi.dashdown_core.SyncHandle

/**
 * Thin 1:1 wrapper over [uniffi.dashdown_core.AppCore]. Every call hops to [Dispatchers.IO] (the
 * suspend FFI must not run on the main thread, and the first call builds the core there) and lets
 * [uniffi.dashdown_core.CoreException] propagate; callers classify it for display via
 * [UiError.from]. Live progress is exposed straight from the
 * [org.sunnypilot.dashdown.core.ProgressBus].
 */
class DashdownRepository(private val locator: ServiceLocator) {

  /** Live per-drive download progress, keyed by driveKey. */
  val progress: StateFlow<Map<String, DriveProgress>>
    get() = locator.progressBus.states

  // --- Devices ---
  suspend fun listDevices(): List<Device> = io { locator.core.listDevices() }

  suspend fun addDevice(device: Device): Device = io { locator.core.addDevice(device) }

  suspend fun updateDevice(device: Device) = io { locator.core.updateDevice(device) }

  suspend fun removeDevice(deviceId: Long) = io { locator.core.removeDevice(deviceId) }

  suspend fun setActiveMode(deviceId: Long, mode: ConnMode) = io {
    locator.core.setActiveMode(deviceId, mode)
  }

  suspend fun getSettings(deviceId: Long): DeviceSettings = io {
    locator.core.getSettings(deviceId)
  }

  suspend fun setSettings(deviceId: Long, settings: DeviceSettings) = io {
    locator.core.setSettings(deviceId, settings)
  }

  // --- Drives ---
  suspend fun listDrives(deviceId: Long, offline: Boolean): List<Drive> = io {
    locator.core.listDrives(deviceId, offline)
  }

  suspend fun getDrive(deviceId: Long, driveKey: String): Drive = io {
    locator.core.getDrive(deviceId, driveKey)
  }

  suspend fun getDriveStatus(deviceId: Long, driveKey: String): DriveSyncStatus = io {
    locator.core.getDriveStatus(deviceId, driveKey)
  }

  suspend fun syncNow(deviceId: Long): List<Drive> = io { locator.core.syncNow(deviceId) }

  suspend fun setPreserved(deviceId: Long, driveKey: String, preserved: Boolean) = io {
    locator.core.setPreserved(deviceId, driveKey, preserved)
  }

  // --- Downloads / maintenance / connectivity ---
  suspend fun startDriveDownload(deviceId: Long, driveKey: String): SyncHandle = io {
    locator.core.startDriveDownload(deviceId, driveKey)
  }

  suspend fun exportDriveZip(deviceId: Long, driveKey: String, destPath: String) = io {
    locator.core.exportDriveZip(deviceId, driveKey, destPath)
  }

  suspend fun runMaintenance(deviceId: Long) = io { locator.core.runMaintenance(deviceId) }

  suspend fun checkConnectivity(deviceId: Long): DeviceConnectivity = io {
    locator.core.checkConnectivity(deviceId)
  }

  /** Run [block] on [Dispatchers.IO]; the first such call lazily builds the core there. */
  private suspend fun <T> io(block: suspend () -> T): T = withContext(Dispatchers.IO) { block() }
}
