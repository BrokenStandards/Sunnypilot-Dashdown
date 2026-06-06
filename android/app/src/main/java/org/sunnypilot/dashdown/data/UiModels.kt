package org.sunnypilot.dashdown.data

import uniffi.dashdown_core.CoreException

/** Terminal outcome of a drive download, surfaced by the progress bus. */
sealed interface Terminal {
  data object Complete : Terminal

  data class Failed(val error: String) : Terminal
}

/**
 * UI-facing snapshot of a drive's live download progress. The core reports unsigned counters
 * ([uniffi.dashdown_core.DownloadProgress]); these are converted to signed types here so Compose
 * and formatters can use them directly.
 */
data class DriveProgress(
    val filesDone: Int = 0,
    val filesTotal: Int = 0,
    val bytesDone: Long = 0L,
    val bytesTotal: Long = 0L,
    val currentFile: String? = null,
    val terminal: Terminal? = null,
)

/** One-shot terminal signal consumed by the Foreground Service / Worker. */
data class TerminalEvent(val driveKey: String, val error: String?)

/**
 * Human-facing classification of a [CoreException]. The core's error is flat (a single message),
 * but UniFFI exposes typed subclasses, so we map on type — not string prefixes.
 */
sealed class UiError(val message: String) {
  class AuthRequired(message: String) : UiError(message)

  class Forbidden(message: String) : UiError(message)

  class NotFound(message: String) : UiError(message)

  class Network(message: String) : UiError(message)

  class Other(message: String) : UiError(message)

  companion object {
    fun from(t: Throwable): UiError {
      val msg = t.message ?: t.toString()
      return when (t) {
        is CoreException.AuthRequired -> AuthRequired(msg)
        is CoreException.Forbidden -> Forbidden(msg)
        is CoreException.NotFound -> NotFound(msg)
        is CoreException.Http,
        is CoreException.Io -> Network(msg)
        else -> Other(msg)
      }
    }
  }
}
