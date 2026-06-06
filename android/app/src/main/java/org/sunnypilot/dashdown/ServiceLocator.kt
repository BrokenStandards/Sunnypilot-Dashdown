package org.sunnypilot.dashdown

import android.app.Application
import java.io.File
import org.sunnypilot.dashdown.core.LogcatLogSink
import org.sunnypilot.dashdown.core.ProgressBus
import org.sunnypilot.dashdown.data.DashdownRepository
import uniffi.dashdown_core.AppCore
import uniffi.dashdown_core.LogLevel

/**
 * Hand-rolled service locator (no Hilt). Holds the single process-wide [AppCore] and the singletons
 * that share it.
 *
 * The core is built lazily and is only ever touched from a background dispatcher (see
 * [DashdownRepository.io]); its construction opens SQLite and builds the owned tokio runtime, so it
 * must not run on the main thread.
 *
 * Storage lives under external app-specific storage — `getExternalFilesDir(null)` — which has room
 * for GB-scale footage, needs no permission, is app-private, and is wiped on uninstall. It falls
 * back to internal [Application.getFilesDir] if external storage is unavailable (e.g. a removed SD
 * card → `getExternalFilesDir` returns null).
 */
class ServiceLocator(private val app: Application) {
  /** Hot, app-lifetime progress hub; installed as the core's only [ProgressSink]. */
  val progressBus: ProgressBus = ProgressBus()

  private val logSink = LogcatLogSink()

  private val storageBase: File by lazy { app.getExternalFilesDir(null) ?: app.filesDir }
  private val dashdownDir: File by lazy { File(storageBase, "dashdown").apply { mkdirs() } }

  /** Mirror root passed to the core; exposed so the UI can resolve local files (Media3). */
  val mirrorRoot: File by lazy { File(dashdownDir, "mirror").apply { mkdirs() } }
  private val dbFile: File by lazy { File(dashdownDir, "index.sqlite") }

  /**
   * The one [AppCore]. Built on first access (must be off-main). Installs the progress and log
   * sinks here, exactly once, before any download can start.
   */
  val core: AppCore by lazy {
    AppCore(dbFile.absolutePath, mirrorRoot.absolutePath).also {
      it.setProgressSink(progressBus)
      it.setLogSink(logSink, LogLevel.INFO)
    }
  }

  val repository: DashdownRepository by lazy { DashdownRepository(this) }
}
