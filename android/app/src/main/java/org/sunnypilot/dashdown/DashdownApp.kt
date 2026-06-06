package org.sunnypilot.dashdown

import android.app.Application
import org.sunnypilot.dashdown.work.AutoSyncWorker

/**
 * Application entry point. Owns the single, process-wide [ServiceLocator] (and through it the one
 * [uniffi.dashdown_core.AppCore]). Registered via `android:name=".DashdownApp"`.
 *
 * [onCreate] stays light: the core is built lazily off the main thread on first repository call (it
 * opens SQLite and spins up the owned tokio runtime). It only enqueues the periodic auto-sync work
 * (a cheap WorkManager call that does not touch the core); the worker filters `autoSync` devices at
 * run time.
 */
class DashdownApp : Application() {
  val locator: ServiceLocator by lazy { ServiceLocator(this) }

  override fun onCreate() {
    super.onCreate()
    AutoSyncWorker.ensureScheduled(this)
  }
}
