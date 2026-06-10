package org.sunnypilot.dashdown.data

import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext
import org.sunnypilot.dashdown.ServiceLocator
import uniffi.dashdown_core.ConnMode
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.DeviceConnectivity
import uniffi.dashdown_core.DeviceSettings
import uniffi.dashdown_core.Drive
import uniffi.dashdown_core.DriveSyncStatus
import uniffi.dashdown_core.FileKind
import uniffi.dashdown_core.SegmentPath
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

  /** One-shot completed/failed events (cancellation emits nothing). */
  val terminalEvents: SharedFlow<TerminalEvent>
    get() = locator.progressBus.terminal

  /** Mirror root the core writes to; used to resolve local file paths for playback. */
  val mirrorRoot: File
    get() = locator.mirrorRoot

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

  /** Connect and return the detected copyparty hostname (e.g. `comma-e0e384a`), or null. */
  suspend fun detectDeviceName(deviceId: Long): String? = io {
    locator.core.detectDeviceName(deviceId)
  }

  suspend fun setPreserved(deviceId: Long, driveKey: String, preserved: Boolean) = io {
    locator.core.setPreserved(deviceId, driveKey, preserved)
  }

  /** Absolute path of a downloaded file (one stream of one segment), or null if not mirrored. */
  suspend fun localFilePath(
      deviceId: Long,
      driveKey: String,
      segmentNum: UInt,
      kind: FileKind,
  ): String? = io { locator.core.localFilePath(deviceId, driveKey, segmentNum, kind) }

  /** Ordered absolute paths of every mirrored file of [kind] in the drive (complete only). */
  suspend fun driveLocalPaths(deviceId: Long, driveKey: String, kind: FileKind): List<SegmentPath> =
      io {
        locator.core.driveLocalPaths(deviceId, driveKey, kind)
      }

  /**
   * Path to a player-openable file for this segment's [kind] stream, remuxing the raw HEVC HD
   * cameras to MP4 on first use (qcamera/others return their source path). Null if not mirrored.
   * The remux is CPU/IO-heavy; it runs on the core's blocking pool.
   */
  suspend fun ensurePlayable(
      deviceId: Long,
      driveKey: String,
      segmentNum: UInt,
      kind: FileKind,
  ): String? = io { locator.core.ensurePlayable(deviceId, driveKey, segmentNum, kind) }

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
