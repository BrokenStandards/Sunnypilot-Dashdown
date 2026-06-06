package org.sunnypilot.dashdown

import android.app.Application

/**
 * Application entry point. Owns the single, process-wide [ServiceLocator] (and through it the one
 * [uniffi.dashdown_core.AppCore]). Registered via `android:name=".DashdownApp"`.
 *
 * Nothing heavy runs in [onCreate]: the core is built lazily off the main thread on first
 * repository call (it opens SQLite and spins up the owned tokio runtime). Periodic auto-sync
 * scheduling is wired in a later milestone step.
 */
class DashdownApp : Application() {
  val locator: ServiceLocator by lazy { ServiceLocator(this) }
}
