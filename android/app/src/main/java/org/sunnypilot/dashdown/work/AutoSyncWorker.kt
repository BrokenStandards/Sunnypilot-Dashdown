package org.sunnypilot.dashdown.work

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.pm.ServiceInfo
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ForegroundInfo
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.delay
import kotlinx.coroutines.withTimeoutOrNull
import org.sunnypilot.dashdown.DashdownApp
import org.sunnypilot.dashdown.service.DownloadService
import uniffi.dashdown_core.SyncStatus

/**
 * Opportunistic auto-sync. On its periodic run (constraints: unmetered + battery-not-low) it
 * refreshes the index and runs retention/auto-delete for every device with `autoSync`, then — per
 * the user's "auto-download on Wi-Fi" choice — promotes itself to a foreground (dataSync) worker
 * and downloads the still-missing/partial drives, awaiting their completion.
 *
 * Becoming a foreground worker (rather than starting [DownloadService] from the background) is the
 * sanctioned way to keep the process alive for the transfer; the constraints already gate it to
 * unmetered + charged-enough conditions.
 */
class AutoSyncWorker(context: Context, params: WorkerParameters) :
    CoroutineWorker(context, params) {

  override suspend fun getForegroundInfo(): ForegroundInfo {
    val nm = applicationContext.getSystemService(NotificationManager::class.java)
    nm.createNotificationChannel(
        NotificationChannel(CHANNEL, "Auto-sync", NotificationManager.IMPORTANCE_LOW))
    val notif =
        NotificationCompat.Builder(applicationContext, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle("Auto-syncing footage")
            .setOngoing(true)
            .build()
    return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      ForegroundInfo(NOTIF_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
    } else {
      ForegroundInfo(NOTIF_ID, notif)
    }
  }

  override suspend fun doWork(): Result {
    val repo = (applicationContext as DashdownApp).locator.repository
    return try {
      val devices = repo.listDevices().filter { it.autoSync }
      val toDownload = mutableListOf<Pair<Long, String>>()
      var transient = false
      for (d in devices) {
        try {
          val drives = repo.syncNow(d.id)
          repo.runMaintenance(d.id)
          drives
              .filter {
                it.syncState == SyncStatus.NOT_DOWNLOADED || it.syncState == SyncStatus.PARTIAL
              }
              .forEach { toDownload += d.id to it.driveKey }
        } catch (_: Throwable) {
          transient = true // network hiccup etc. — retry the periodic run later
        }
      }

      if (toDownload.isNotEmpty()) {
        // Best-effort foreground promotion (a no-op in unit tests without a ForegroundUpdater).
        runCatching { setForeground(getForegroundInfo()) }
        toDownload.forEach { (deviceId, key) -> repo.startDriveDownload(deviceId, key) }
        val pending = toDownload.toMutableList()
        withTimeoutOrNull(DOWNLOAD_TIMEOUT_MS) {
          while (pending.isNotEmpty()) {
            val it = pending.iterator()
            while (it.hasNext()) {
              val (deviceId, key) = it.next()
              val s = runCatching { repo.getDriveStatus(deviceId, key).status }.getOrNull()
              if (s == SyncStatus.COMPLETE || s == SyncStatus.FAILED) it.remove()
            }
            if (pending.isNotEmpty()) delay(500)
          }
        }
      }

      if (transient) Result.retry() else Result.success()
    } catch (_: Throwable) {
      Result.retry()
    }
  }

  companion object {
    private const val UNIQUE = "auto-sync"
    private const val CHANNEL = "autosync"
    private const val NOTIF_ID = 1002
    private const val DOWNLOAD_TIMEOUT_MS = 5 * 60_000L

    /** Schedule the periodic auto-sync once (kept across launches; runtime filters autoSync). */
    fun ensureScheduled(context: Context) {
      val constraints =
          Constraints.Builder()
              .setRequiredNetworkType(NetworkType.UNMETERED)
              .setRequiresBatteryNotLow(true)
              .build()
      val request =
          PeriodicWorkRequestBuilder<AutoSyncWorker>(6, TimeUnit.HOURS)
              .setConstraints(constraints)
              .build()
      WorkManager.getInstance(context)
          .enqueueUniquePeriodicWork(UNIQUE, ExistingPeriodicWorkPolicy.KEEP, request)
    }
  }
}
