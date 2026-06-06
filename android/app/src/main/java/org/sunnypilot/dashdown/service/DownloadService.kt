package org.sunnypilot.dashdown.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.sunnypilot.dashdown.DashdownApp
import org.sunnypilot.dashdown.ServiceLocator
import org.sunnypilot.dashdown.data.DriveProgress
import uniffi.dashdown_core.SyncHandle

/**
 * Keep-alive + notification + cancel host for downloads. It is **not** the downloader: the Rust
 * core's owned runtime runs the detached transfer task; this service exists so Android does not
 * kill the process while a download runs in the background, surfaces progress in a notification,
 * and offers a Cancel action.
 *
 * It owns [startDriveDownload][org.sunnypilot.dashdown.data.DashdownRepository.startDriveDownload]
 * so the [SyncHandle] and the foreground lifetime share a scope. Completion/failure arrive via the
 * [ProgressBus][org.sunnypilot.dashdown.core.ProgressBus] terminal flow; **cancellation fires no
 * core callback**, so it is cleaned up here in [onStartCommand].
 */
class DownloadService : Service() {
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
  private val handles = ConcurrentHashMap<String, SyncHandle>()
  private val active = MutableStateFlow<Set<String>>(emptySet())
  private val cancelled = ConcurrentHashMap.newKeySet<String>()
  private lateinit var locator: ServiceLocator

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onCreate() {
    super.onCreate()
    locator = (application as DashdownApp).locator
    createChannels()
    // Refresh the ongoing notification as progress arrives.
    scope.launch {
      locator.progressBus.states.collect { states ->
        if (active.value.isNotEmpty()) nm().notify(NOTIF_ID, buildOngoing(pickLive(states)))
      }
    }
    // Complete → terminal(null); Failed → terminal(error). Cancel emits nothing (handled below).
    scope.launch {
      locator.progressBus.terminal.collect { ev ->
        handles.remove(ev.driveKey)
        active.update { it - ev.driveKey }
        if (!cancelled.remove(ev.driveKey)) postTerminal(ev.driveKey, ev.error)
        stopIfIdle()
      }
    }
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    when (intent?.action) {
      ACTION_DOWNLOAD -> {
        val deviceId = intent.getLongExtra(EXTRA_DEVICE_ID, -1L)
        val driveKey = intent.getStringExtra(EXTRA_DRIVE_KEY)
        // Must promote within ~5s of startForegroundService.
        ServiceCompat.startForeground(
            this,
            NOTIF_ID,
            buildOngoing(null),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
        )
        if (driveKey == null || deviceId < 0) {
          stopIfIdle()
        } else if (!active.value.contains(driveKey)) {
          active.update { it + driveKey }
          cancelled.remove(driveKey)
          scope.launch {
            try {
              handles[driveKey] = locator.repository.startDriveDownload(deviceId, driveKey)
            } catch (t: Throwable) {
              active.update { it - driveKey }
              postTerminal(driveKey, t.message ?: "failed to start")
              stopIfIdle()
            }
          }
        }
      }
      ACTION_CANCEL -> {
        intent.getStringExtra(EXTRA_DRIVE_KEY)?.let { key ->
          cancelled.add(key)
          handles.remove(key)?.cancel()
          active.update { it - key }
          stopIfIdle()
        }
      }
    }
    return START_NOT_STICKY
  }

  override fun onDestroy() {
    scope.cancel()
    super.onDestroy()
  }

  private fun stopIfIdle() {
    if (active.value.isEmpty()) {
      ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
      stopSelf()
    }
  }

  private fun pickLive(states: Map<String, DriveProgress>): DriveProgress? =
      active.value.mapNotNull { states[it] }.firstOrNull { it.terminal == null }

  private fun buildOngoing(live: DriveProgress?): Notification {
    val b =
        NotificationCompat.Builder(this, CHANNEL_ONGOING)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle("Downloading footage")
            .setOngoing(true)
            .setOnlyAlertOnce(true)
    if (live != null && live.bytesTotal > 0) {
      val pct = ((live.bytesDone * 100) / live.bytesTotal).toInt().coerceIn(0, 100)
      b.setContentText("${live.filesDone}/${live.filesTotal} files · $pct%")
          .setProgress(100, pct, false)
    } else {
      b.setContentText("Starting…").setProgress(0, 0, true)
    }
    active.value.firstOrNull()?.let { key -> b.addAction(0, "Cancel", cancelPending(key)) }
    return b.build()
  }

  private fun postTerminal(driveKey: String, error: String?) {
    val b =
        NotificationCompat.Builder(this, CHANNEL_DONE)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setAutoCancel(true)
            .apply {
              if (error == null) {
                setContentTitle("Download complete")
              } else {
                setContentTitle("Download failed").setContentText(error)
              }
            }
    nm().notify(2000 + (driveKey.hashCode() and 0xFFF), b.build())
  }

  private fun cancelPending(driveKey: String): PendingIntent {
    val intent =
        Intent(this, DownloadService::class.java)
            .setAction(ACTION_CANCEL)
            .putExtra(EXTRA_DRIVE_KEY, driveKey)
    return PendingIntent.getService(
        this,
        driveKey.hashCode(),
        intent,
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
  }

  private fun createChannels() {
    val mgr = nm()
    mgr.createNotificationChannel(
        NotificationChannel(CHANNEL_ONGOING, "Downloads", NotificationManager.IMPORTANCE_LOW))
    mgr.createNotificationChannel(
        NotificationChannel(
            CHANNEL_DONE, "Download results", NotificationManager.IMPORTANCE_DEFAULT))
  }

  private fun nm(): NotificationManager = getSystemService(NotificationManager::class.java)

  companion object {
    private const val ACTION_DOWNLOAD = "org.sunnypilot.dashdown.action.DOWNLOAD"
    private const val ACTION_CANCEL = "org.sunnypilot.dashdown.action.CANCEL"
    private const val EXTRA_DEVICE_ID = "deviceId"
    private const val EXTRA_DRIVE_KEY = "driveKey"
    private const val NOTIF_ID = 1001
    private const val CHANNEL_ONGOING = "downloads"
    private const val CHANNEL_DONE = "downloads_done"

    /** Start (or join) a foreground download for [driveKey]. */
    fun start(context: Context, deviceId: Long, driveKey: String) {
      val intent =
          Intent(context, DownloadService::class.java)
              .setAction(ACTION_DOWNLOAD)
              .putExtra(EXTRA_DEVICE_ID, deviceId)
              .putExtra(EXTRA_DRIVE_KEY, driveKey)
      ContextCompat.startForegroundService(context, intent)
    }

    /** Cancel an in-flight download for [driveKey] (no-op if not running). */
    fun cancel(context: Context, driveKey: String) {
      val intent =
          Intent(context, DownloadService::class.java)
              .setAction(ACTION_CANCEL)
              .putExtra(EXTRA_DRIVE_KEY, driveKey)
      context.startService(intent)
    }
  }
}
