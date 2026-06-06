package org.sunnypilot.dashdown.core

import android.util.Log
import uniffi.dashdown_core.LogEvent
import uniffi.dashdown_core.LogLevel
import uniffi.dashdown_core.LogSink

/**
 * Forwards core log events to Logcat. Messages are already field-redacted by the core (no
 * passwords), so they are safe to log verbatim.
 */
class LogcatLogSink : LogSink {
  override fun onLog(event: LogEvent) {
    val tag = "core/${event.target}"
    when (event.level) {
      LogLevel.ERROR -> Log.e(tag, event.message)
      LogLevel.WARN -> Log.w(tag, event.message)
      LogLevel.INFO -> Log.i(tag, event.message)
      LogLevel.DEBUG -> Log.d(tag, event.message)
      LogLevel.TRACE -> Log.v(tag, event.message)
    }
  }
}
