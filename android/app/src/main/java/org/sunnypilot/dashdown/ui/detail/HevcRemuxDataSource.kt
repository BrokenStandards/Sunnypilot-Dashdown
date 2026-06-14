@file:OptIn(UnstableApi::class)

package org.sunnypilot.dashdown.ui.detail

import android.net.Uri
import android.util.Log
import androidx.collection.LruCache
import androidx.media3.common.C
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.BaseDataSource
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DataSpec
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap

/**
 * Remux one HD-camera segment's raw HEVC to MP4 bytes (or null if it can't be produced — not
 * mirrored, wrong kind, or a decode/parse failure). Implemented over the Rust core's synchronous
 * `remuxHdBytes`; called on ExoPlayer's loader thread, so blocking is expected.
 */
fun interface HdRemuxer {
  fun remux(deviceId: Long, driveKey: String, segNum: Int, kindOrdinal: Int): ByteArray?
}

/**
 * A Media3 [DataSource] that serves an HD camera segment as MP4 bytes remuxed **in memory** — no
 * `.hevc.mp4` file is ever written. The [DataSpec] URI ([HdMediaUri]) names the segment; bytes are
 * produced lazily on first `open()` via [HdRemuxer] and held in a shared, byte-bounded LRU, so
 * ExoPlayer only remuxes the windows it actually reaches (current + look-ahead) and seeking to any
 * timestamp remuxes just that window. The full MP4 is held so in-window seeks (which re-`open()` at
 * a byte offset) are served straight from memory.
 */
class HevcRemuxDataSource(
    private val cache: LruCache<String, ByteArray>,
    private val locks: ConcurrentHashMap<String, Any>,
    private val remuxer: HdRemuxer,
) : BaseDataSource(/* isNetwork= */ false) {

  private var sourceUri: Uri? = null
  private var data: ByteArray? = null
  private var position = 0
  private var bytesRemaining = 0

  override fun open(dataSpec: DataSpec): Long {
    val uri = dataSpec.uri
    sourceUri = uri
    transferInitializing(dataSpec)

    val key = uri.toString()
    val bytes = getOrRemux(key) ?: throw IOException("no HD bytes for $key")
    data = bytes

    val start = dataSpec.position
    if (start < 0 || start > bytes.size) {
      throw IOException("position $start out of range for ${bytes.size} bytes")
    }
    position = start.toInt()
    bytesRemaining =
        if (dataSpec.length == C.LENGTH_UNSET.toLong()) bytes.size - position
        else minOf(dataSpec.length, (bytes.size - position).toLong()).toInt()

    transferStarted(dataSpec)
    return bytesRemaining.toLong()
  }

  /**
   * Cached bytes for [key], remuxing on miss under a per-key lock so concurrent merge children for
   * the same (segment, camera) don't remux twice — while different cameras still remux in parallel.
   * The (slow) remux runs outside the cache's internal lock. Returns null if no bytes are
   * available.
   */
  private fun getOrRemux(key: String): ByteArray? {
    cache.get(key)?.let {
      Log.d(TAG, "hit  ${shortKey(key)} (${it.size / 1024}KB, lru=${cache.size() / 1024}KB)")
      return it
    }
    val lock = locks.getOrPut(key) { Any() }
    synchronized(lock) {
      cache.get(key)?.let {
        return it
      }
      val ref = HdMediaUri.parse(key) ?: return null
      val t0 = System.nanoTime()
      val bytes =
          try {
            remuxer.remux(ref.deviceId, ref.driveKey, ref.segNum, ref.kindOrdinal)
          } catch (t: Throwable) {
            Log.w(TAG, "remux threw for ${shortKey(key)}", t)
            null
          }
      if (bytes == null) {
        Log.w(TAG, "miss ${shortKey(key)} -> null")
        return null
      }
      val ms = (System.nanoTime() - t0) / 1_000_000
      cache.put(key, bytes)
      Log.d(
          TAG,
          "remux ${shortKey(key)} -> ${bytes.size / 1024}KB in ${ms}ms (lru=${cache.size() / 1024}KB/${cache.maxSize() / 1024}KB)")
      return bytes
    }
  }

  override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
    if (length == 0) return 0
    if (bytesRemaining == 0) return C.RESULT_END_OF_INPUT
    val n = minOf(length, bytesRemaining)
    System.arraycopy(data!!, position, buffer, offset, n)
    position += n
    bytesRemaining -= n
    bytesTransferred(n)
    return n
  }

  override fun getUri(): Uri? = sourceUri

  override fun close() {
    if (data != null) {
      // The LRU owns the bytes; just release this source's reference.
      data = null
      transferEnded()
    }
    sourceUri = null
    position = 0
    bytesRemaining = 0
  }

  /**
   * Builds [HevcRemuxDataSource]s that share one byte-bounded LRU and per-key locks. Hand to a
   * [androidx.media3.exoplayer.source.ProgressiveMediaSource.Factory] for the HD windows. Owning
   * the cache here ties its lifetime to the player instance, so the bytes are freed when leaving
   * the drive.
   */
  class Factory(remuxer: HdRemuxer, maxBytes: Int) : DataSource.Factory {
    private val remuxer = remuxer
    private val cache =
        object : LruCache<String, ByteArray>(maxBytes) {
          override fun sizeOf(key: String, value: ByteArray): Int = value.size
        }
    private val locks = ConcurrentHashMap<String, Any>()

    override fun createDataSource(): DataSource = HevcRemuxDataSource(cache, locks, remuxer)

    /**
     * Resize the shared budget as the available HD-camera count settles (e.g. a drive opened while
     * still downloading goes 1→2→3 cams). [LruCache.resize] keeps the cached entries — no drop — so
     * this never forces a re-remux, unlike recreating the Factory would.
     */
    fun resize(maxBytes: Int) = cache.resize(maxBytes)
  }

  companion object {
    private const val TAG = "HevcRemux"

    /** seg/kind tail of an [HdMediaUri] for compact logs. */
    private fun shortKey(key: String): String =
        HdMediaUri.parse(key)?.let { "seg${it.segNum}/k${it.kindOrdinal}" } ?: key
  }
}

// A remuxed HD segment is ≈ its raw .hevc (~37 MB); budget per camera for two windows (the one
// playing + the look-ahead being remuxed) so a boundary crossing or seek-back is a cache hit, not a
// re-remux.
private const val APPROX_SEGMENT_MB = 40
private const val MIN_LRU_MB = 80 // two single-camera windows
private const val MAX_LRU_MB = 256 // hard ceiling, to bound GC pause time

/**
 * LRU budget (bytes) for remuxed HD MP4s, sized to hold ~2 full N-camera windows, but capped at ~40
 * % of the heap (the rest is for ExoPlayer buffers, Coil, and the UI) and a hard ceiling.
 * [camCount] is the AVAILABLE HD cameras for the drive (≤3); pair with
 * [HevcRemuxDataSource.Factory.resize] as it settles. Reads the actual heap limit, which reflects
 * `android:largeHeap`. Pure, so it is unit-tested.
 */
internal fun lruMaxBytes(camCount: Int): Int {
  val heapMb = (Runtime.getRuntime().maxMemory() / (1024L * 1024L)).toInt()
  val capMb = (heapMb * 2 / 5).coerceAtLeast(MIN_LRU_MB) // ≤ ~40 % of the heap
  val wantMb = camCount.coerceAtLeast(1) * APPROX_SEGMENT_MB * 2 // current + look-ahead, N cams
  return wantMb.coerceIn(MIN_LRU_MB, capMb).coerceAtMost(MAX_LRU_MB) * 1024 * 1024
}
