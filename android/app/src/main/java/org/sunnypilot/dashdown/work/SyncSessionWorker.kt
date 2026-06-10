package org.sunnypilot.dashdown.work

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.SystemClock
import androidx.core.app.NotificationCompat
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.ForegroundInfo
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.delay
import org.sunnypilot.dashdown.DashdownApp
import org.sunnypilot.dashdown.data.DashdownRepository
import uniffi.dashdown_core.SyncHandle
import uniffi.dashdown_core.SyncStatus

/**
 * Background sync **session** — the workhorse of automatic, no-app-open downloads. Enqueued by the
 * periodic [SyncBackstopWorker], by a connectivity change, and once at app start, all coalesced
 * under the unique name [UNIQUE] ([ExistingWorkPolicy.KEEP]) so only one session runs at a time.
 *
 * Per run it triages which `autoSync` devices are reachable (B1 multi-IP); if any are, it promotes
 * to a `dataSync` foreground service and loops, per device:
 * 1. refresh the index ([DashdownRepository.syncNow]) — picks up new drives/segments,
 * 2. download every still-missing/partial drive (skipping any the manual [DownloadService] is
 *    already pulling), awaiting each to a terminal state,
 * 3. re-refresh to catch segments recorded *during* those downloads,
 *
 * until the device's work drains, it goes unreachable, the per-session time cap is hit, or the
 * worker is stopped. Long drives continue across sessions via the core's `.part`-file resume; on a
 * stop the in-flight handle is cancelled, leaving the partial file for the next session.
 *
 * Promotion happens only *after* the reachability triage, so no notification flashes and no
 * `dataSync` time-budget is spent when the comma isn't around.
 */
class SyncSessionWorker(context: Context, params: WorkerParameters) :
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
    val devices = tryIo { repo.listDevices() }.orEmpty().filter { it.autoSync }
    val reachable = devices.filter { tryIo { repo.checkConnectivity(it.id).reachable } == true }
    if (reachable.isEmpty()) return Result.success() // nothing around → no FGS, no notification

    // Real work potential: become a foreground (dataSync) worker so the process survives the loop.
    runCatching { setForeground(getForegroundInfo()) }

    val deadline = SystemClock.elapsedRealtime() + SESSION_MAX_MS
    for (d in reachable) {
      if (SystemClock.elapsedRealtime() >= deadline || isStopped) break
      val giveUp = mutableSetOf<String>() // drives that failed this session — retried next session
      var current: SyncHandle? = null
      try {
        while (SystemClock.elapsedRealtime() < deadline && !isStopped) {
          if (tryIo { repo.checkConnectivity(d.id).reachable } != true) break // device left
          val drives = tryIo { repo.syncNow(d.id) } ?: break // refresh; null ⇒ gone/erroring
          val pending =
              drives
                  .filter {
                    it.syncState == SyncStatus.NOT_DOWNLOADED || it.syncState == SyncStatus.PARTIAL
                  }
                  .filterNot { it.driveKey in giveUp }
                  .filterNot {
                    // Leave drives the manual DownloadService is already pulling to it.
                    tryIo { repo.getDriveStatus(d.id, it.driveKey).status } ==
                        SyncStatus.DOWNLOADING
                  }
          if (pending.isEmpty()) {
            // Caught up. If a drive is still recording, wait for its next segment; else done.
            if (drives.any { it.recording } && SystemClock.elapsedRealtime() < deadline) {
              delay(RECORDING_POLL_MS)
              continue
            }
            break
          }
          for (drive in pending) {
            if (isStopped || SystemClock.elapsedRealtime() >= deadline) break
            current = tryIo { repo.startDriveDownload(d.id, drive.driveKey) } // resumes from .part
            if (current == null) {
              giveUp.add(drive.driveKey)
              continue
            }
            when (awaitTerminal(repo, d.id, drive.driveKey, deadline)) {
              SyncStatus.FAILED -> giveUp.add(drive.driveKey)
              SyncStatus.COMPLETE -> {}
              else -> current.cancel() // deadline/stop mid-download → leave .part for next session
            }
            current = null
          }
        }
        tryIo { repo.runMaintenance(d.id) } // retention (Phase D refines the policy)
      } finally {
        if (isStopped) current?.cancel() // stopped mid-download → leave .part for next session
      }
    }
    return Result.success()
  }

  /** Poll the drive's status until COMPLETE/FAILED, the session deadline, or a stop. */
  private suspend fun awaitTerminal(
      repo: DashdownRepository,
      deviceId: Long,
      driveKey: String,
      deadline: Long,
  ): SyncStatus? {
    while (SystemClock.elapsedRealtime() < deadline && !isStopped) {
      when (val s = tryIo { repo.getDriveStatus(deviceId, driveKey).status }) {
        SyncStatus.COMPLETE,
        SyncStatus.FAILED -> return s
        else -> delay(POLL_MS)
      }
    }
    return null
  }

  /**
   * Run an IO/FFI call, returning null on failure — but never swallowing coroutine cancellation.
   */
  private suspend fun <T> tryIo(block: suspend () -> T): T? =
      try {
        block()
      } catch (c: CancellationException) {
        throw c
      } catch (_: Throwable) {
        null
      }

  companion object {
    private const val UNIQUE = "sync-session"
    private const val CHANNEL = "autosync"
    private const val NOTIF_ID = 1002
    private const val SESSION_MAX_MS = 25 * 60_000L // self-cap << the API-35 6h/24h dataSync budget
    private const val RECORDING_POLL_MS = 30_000L // wait between re-syncs for an active drive
    private const val POLL_MS = 500L

    /** Enqueue one background sync session (coalesced — at most one runs at a time). */
    fun enqueue(context: Context) {
      val constraints = Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()
      val request =
          OneTimeWorkRequestBuilder<SyncSessionWorker>().setConstraints(constraints).build()
      WorkManager.getInstance(context).enqueueUniqueWork(UNIQUE, ExistingWorkPolicy.KEEP, request)
    }
  }
}
