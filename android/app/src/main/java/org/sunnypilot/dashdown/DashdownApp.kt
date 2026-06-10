package org.sunnypilot.dashdown

import android.app.Application
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import coil3.ImageLoader
import coil3.PlatformContext
import coil3.SingletonImageLoader
import coil3.video.VideoFrameDecoder
import org.sunnypilot.dashdown.work.SyncBackstopWorker
import org.sunnypilot.dashdown.work.SyncSessionWorker

/**
 * Application entry point. Owns the single, process-wide [ServiceLocator] (and through it the one
 * [uniffi.dashdown_core.AppCore]). Registered via `android:name=".DashdownApp"`.
 *
 * [onCreate] stays light: the core is built lazily off the main thread on first repository call (it
 * opens SQLite and spins up the owned tokio runtime). It only arms the background-sync triggers —
 * cheap WorkManager calls that don't touch the core — and the session worker does the reachability
 * triage and per-`autoSync`-device work at run time:
 * - the periodic [SyncBackstopWorker] (15-min heartbeat, survives app death/reboot),
 * - an immediate [SyncSessionWorker] so a freshly-launched process syncs right away,
 * - a Wi-Fi [ConnectivityManager.NetworkCallback] that fires a session within seconds of joining a
 *   network (e.g. the comma's hotspot) — only while this process is alive; the backstop is the
 *   durable floor when it isn't.
 *
 * Implements Coil's [SingletonImageLoader.Factory] so drive thumbnails can be decoded straight from
 * a `qcamera.ts` file via [VideoFrameDecoder] (no separate thumbnail files to manage).
 */
class DashdownApp : Application(), SingletonImageLoader.Factory {
  val locator: ServiceLocator by lazy { ServiceLocator(this) }

  override fun onCreate() {
    super.onCreate()
    SyncBackstopWorker.ensureScheduled(this)
    SyncSessionWorker.enqueue(this)
    registerSyncOnConnectivity()
  }

  /**
   * Kick a sync session whenever a Wi-Fi network becomes available (joining the comma hotspot or a
   * home AP). No INTERNET capability is required — the comma's own hotspot has no upstream. The
   * callback lives for the process lifetime; it can't survive app death (the periodic backstop
   * covers that), and the session's reachability triage makes a spurious fire a cheap no-op.
   */
  private fun registerSyncOnConnectivity() {
    val cm = getSystemService(ConnectivityManager::class.java) ?: return
    val request =
        NetworkRequest.Builder().addTransportType(NetworkCapabilities.TRANSPORT_WIFI).build()
    runCatching {
      cm.registerNetworkCallback(
          request,
          object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = SyncSessionWorker.enqueue(this@DashdownApp)
          })
    }
  }

  override fun newImageLoader(context: PlatformContext): ImageLoader =
      ImageLoader.Builder(context).components { add(VideoFrameDecoder.Factory()) }.build()
}
