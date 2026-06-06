package org.sunnypilot.dashdown.core

import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import org.sunnypilot.dashdown.data.DriveProgress
import org.sunnypilot.dashdown.data.Terminal
import org.sunnypilot.dashdown.data.TerminalEvent
import uniffi.dashdown_core.DownloadProgress
import uniffi.dashdown_core.ProgressSink

/**
 * The single [ProgressSink] the Rust core sees. UniFFI delivers these callbacks on the core's tokio
 * threads, so updates use the atomic [MutableStateFlow.update].
 * - [states]: hot, app-lifetime, keyed by driveKey; collected by ViewModels and the Foreground
 *   Service notification.
 * - [terminal]: one-shot completed/failed events so the Service/Worker can react (e.g. `stopSelf`)
 *   without polling the map.
 */
class ProgressBus : ProgressSink {
  private val _states = MutableStateFlow<Map<String, DriveProgress>>(emptyMap())
  val states: StateFlow<Map<String, DriveProgress>> = _states.asStateFlow()

  private val _terminal = MutableSharedFlow<TerminalEvent>(replay = 0, extraBufferCapacity = 16)
  val terminal: SharedFlow<TerminalEvent> = _terminal.asSharedFlow()

  override fun onProgress(p: DownloadProgress) {
    _states.update { current ->
      current +
          (p.driveKey to
              DriveProgress(
                  filesDone = p.filesDone.toInt(),
                  filesTotal = p.filesTotal.toInt(),
                  bytesDone = p.bytesDone.toLong(),
                  bytesTotal = p.bytesTotal.toLong(),
                  currentFile = p.currentFile,
              ))
    }
  }

  override fun onCompleted(driveKey: String) {
    _states.update { current ->
      val prev = current[driveKey] ?: DriveProgress()
      current + (driveKey to prev.copy(terminal = Terminal.Complete))
    }
    _terminal.tryEmit(TerminalEvent(driveKey, null))
  }

  override fun onFailed(driveKey: String, error: String) {
    _states.update { current ->
      val prev = current[driveKey] ?: DriveProgress()
      current + (driveKey to prev.copy(terminal = Terminal.Failed(error)))
    }
    _terminal.tryEmit(TerminalEvent(driveKey, error))
  }

  /** Drop a drive's progress row once the UI has consumed its terminal state. */
  fun clear(driveKey: String) {
    _states.update { it - driveKey }
  }
}
