package org.sunnypilot.dashdown.work

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import org.sunnypilot.dashdown.MainActivity
import org.sunnypilot.dashdown.data.DashdownRepository
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.RetentionStatus

/**
 * Local retention maintenance: prune over-budget footage and warn the user before auto-prune starts
 * deleting older segments. Used by both [SyncBackstopWorker] (offline/periodic, every device) and
 * [SyncSessionWorker] (right after a download grows the mirror). Touches no network.
 */
object Maintenance {
  /** Warn once the non-preserved local footage is within this many minutes of the budget. */
  const val WARN_HEADROOM_MIN = 10L
  private const val CHANNEL = "storage_warn"
  private const val NOTIF_BASE = 3000

  /** Run clear-down for [device], then post/cancel its low-headroom warning. */
  suspend fun sweep(context: Context, repo: DashdownRepository, device: Device) {
    runCatching { repo.runMaintenance(device.id) }
    val status = runCatching { repo.retentionStatus(device.id) }.getOrNull() ?: return
    if (shouldWarn(status, WARN_HEADROOM_MIN)) warn(context, device, headroom(status))
    else cancel(context, device.id)
  }

  /**
   * Warn when a budget is set and the **non-preserved** local footage is within [threshold] minutes
   * of it — i.e. the next few minutes of recording will start auto-deleting older segments. Starred
   * (preserved) footage doesn't count toward the budget, so it's subtracted out.
   */
  fun shouldWarn(s: RetentionStatus, threshold: Long): Boolean {
    val budget = s.budgetMinutes ?: return false
    return budget - (s.localMinutes - s.preservedMinutes) < threshold
  }

  private fun headroom(s: RetentionStatus): Long =
      ((s.budgetMinutes ?: 0L) - (s.localMinutes - s.preservedMinutes)).coerceAtLeast(0L)

  private fun warn(context: Context, device: Device, headroom: Long) {
    val nm = context.getSystemService(NotificationManager::class.java)
    nm.createNotificationChannel(
        NotificationChannel(CHANNEL, "Storage warnings", NotificationManager.IMPORTANCE_DEFAULT))
    val tap =
        PendingIntent.getActivity(
            context,
            device.id.toInt(),
            Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    val notif =
        NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_warning)
            .setContentTitle("Storage almost full on ${device.name}")
            .setContentText(
                "~$headroom min of recording left before older footage is auto-deleted. " +
                    "Star drives to keep them.")
            .setAutoCancel(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(tap)
            .build()
    nm.notify(NOTIF_BASE + device.id.toInt(), notif)
  }

  private fun cancel(context: Context, deviceId: Long) {
    context.getSystemService(NotificationManager::class.java).cancel(NOTIF_BASE + deviceId.toInt())
  }
}
