package org.sunnypilot.dashdown.work

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import java.util.concurrent.TimeUnit
import org.sunnypilot.dashdown.DashdownApp

/**
 * Periodic 15-minute backstop for background sync. WorkManager persists it across reboots, so even
 * with the app killed a sync session is attempted at least every ~15 min whenever a network is up
 * (15 min is the platform's periodic minimum; the connectivity callback in
 * [org.sunnypilot.dashdown.DashdownApp] covers the faster, process-alive case).
 *
 * It runs **local clear-down for every device regardless of reachability** (retention needs no
 * network, so space is reclaimed even when the comma is away), then enqueues [SyncSessionWorker] —
 * which does the reachability triage, foreground promotion, and the sync→download loop.
 */
class SyncBackstopWorker(context: Context, params: WorkerParameters) :
    CoroutineWorker(context, params) {

  override suspend fun doWork(): Result {
    val repo = (applicationContext as DashdownApp).locator.repository
    // Offline/periodic maintenance: prune over-budget local footage + low-headroom warning, for
    // every device — no reachability gate (local-only).
    runCatching { repo.listDevices() }
        .getOrDefault(emptyList())
        .forEach { Maintenance.sweep(applicationContext, repo, it) }
    SyncSessionWorker.enqueue(applicationContext)
    return Result.success()
  }

  companion object {
    private const val UNIQUE = "sync-backstop"
    private const val LEGACY_UNIQUE = "auto-sync" // pre-B2 6-hour worker; class no longer exists

    /**
     * Schedule the 15-min backstop once (kept across launches; survives reboot via WorkManager).
     */
    fun ensureScheduled(context: Context) {
      val constraints =
          Constraints.Builder()
              .setRequiredNetworkType(NetworkType.CONNECTED) // drop UNMETERED: transfers are local
              .setRequiresBatteryNotLow(true)
              .build()
      val request =
          PeriodicWorkRequestBuilder<SyncBackstopWorker>(15, TimeUnit.MINUTES)
              .setConstraints(constraints)
              .build()
      val wm = WorkManager.getInstance(context)
      wm.cancelUniqueWork(LEGACY_UNIQUE) // tidy up the orphaned pre-B2 schedule on upgrade
      wm.enqueueUniquePeriodicWork(UNIQUE, ExistingPeriodicWorkPolicy.KEEP, request)
    }
  }
}
