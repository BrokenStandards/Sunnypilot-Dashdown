package org.sunnypilot.dashdown.ui.detail

import android.graphics.Bitmap
import android.media.MediaMetadataRetriever
import android.util.Log
import androidx.collection.LruCache
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors

/**
 * In-memory thumbnail cache for the scrub bar: decodes downscaled frames from the per-segment
 * `qcamera.ts` previews (low-res HEVC-in-MPEG-TS that are already playable) on a **low-priority
 * background thread**, so scrubbing never competes with the player's HW video decoders.
 *
 * One [MediaMetadataRetriever] is held open per segment path (opening one is expensive; reusing it
 * makes repeated frame grabs from the same segment cheap — exactly the scrubbing case). Decoded
 * [Bitmap]s are held in a byte-bounded [LruCache], keyed by `(path, offset quantized to the GOP)`
 * so a dense scrub doesn't decode redundant frames (comma GOP ≈ 1 s, and `OPTION_CLOSEST_SYNC`
 * snaps to the keyframe anyway). Mirrors the lifetime + MIN_PRIORITY-executor pattern of
 * [HevcRemuxDataSource.Factory]: the cache is owned by the player and [release]d when leaving the
 * drive.
 */
class ThumbnailCache(
    maxBytes: Int,
    private val targetWidthPx: Int = 256,
    private val quantMs: Long = 1_000L,
) {
  private val cache =
      object : LruCache<String, Bitmap>(maxBytes) {
        override fun sizeOf(key: String, value: Bitmap): Int = value.allocationByteCount
        // Intentionally NO recycle() in entryRemoved: a Composable may still be drawing an evicted
        // bitmap (recycling one in use crashes the draw). Dropping the strong ref lets GC reclaim
        // it.
      }
  // One retriever per segment path, opened lazily; MMR is not thread-safe so frame grabs on each
  // are
  // serialized (see [decode]). Different segments still decode independently.
  private val retrievers = ConcurrentHashMap<String, MediaMetadataRetriever>()
  // Keys queued or decoding, so [prefetch] coalesces duplicate requests during a fast scrub.
  private val inflight = ConcurrentHashMap.newKeySet<String>()
  private val executor =
      Executors.newSingleThreadExecutor { r ->
        Thread(r, "thumb-decode").apply {
          isDaemon = true
          priority = Thread.MIN_PRIORITY
        }
      }
  @Volatile private var released = false

  private fun key(path: String, offsetMs: Long): String = "$path@${offsetMs / quantMs * quantMs}"

  /** Cached bitmap for (path, offsetMs), or null on miss — immediate, for the UI thread. */
  fun get(path: String, offsetMs: Long): Bitmap? = cache.get(key(path, offsetMs))

  /**
   * Fire-and-forget: decode (path, offsetMs) into the cache on the background thread so a later
   * [get] is a hit. No-op if already cached, already in flight, or [release]d.
   */
  fun prefetch(path: String, offsetMs: Long) {
    if (released) return
    val k = key(path, offsetMs)
    if (cache.get(k) != null || !inflight.add(k)) return
    executor.execute {
      try {
        if (released || cache.get(k) != null) return@execute
        decode(path, offsetMs)?.let { cache.put(k, it) }
      } finally {
        inflight.remove(k)
      }
    }
  }

  private fun retrieverFor(path: String): MediaMetadataRetriever? {
    retrievers[path]?.let {
      return it
    }
    return try {
      val mmr = MediaMetadataRetriever().apply { setDataSource(path) }
      // Another thread may have opened the same path concurrently — keep the winner, release ours.
      retrievers.putIfAbsent(path, mmr)?.also { mmr.release() } ?: mmr
    } catch (t: Throwable) {
      Log.w(TAG, "open failed for $path", t)
      null
    }
  }

  private fun decode(path: String, offsetMs: Long): Bitmap? {
    val mmr = retrieverFor(path) ?: return null
    val h = targetWidthPx * 9 / 16
    return try {
      synchronized(mmr) {
        mmr.getScaledFrameAtTime(
            offsetMs * 1000L, MediaMetadataRetriever.OPTION_CLOSEST_SYNC, targetWidthPx, h)
      }
    } catch (t: Throwable) {
      Log.w(TAG, "decode failed for $path @${offsetMs}ms", t)
      null
    }
  }

  /** Release every retriever, evict all bitmaps, and stop the decode thread. Idempotent. */
  fun release() {
    released = true
    executor.shutdownNow()
    for (mmr in retrievers.values) runCatching { mmr.release() }
    retrievers.clear()
    cache.evictAll()
  }

  private companion object {
    const val TAG = "ThumbnailCache"
  }
}
