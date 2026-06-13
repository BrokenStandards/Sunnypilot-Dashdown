package org.sunnypilot.dashdown.ui.detail

import java.net.URLDecoder
import java.net.URLEncoder

/**
 * Opaque URI that identifies one HD-camera segment for the in-memory player pipeline: `(deviceId,
 * segmentNum, cameraKindOrdinal, driveKey)`. [MultiCamPlayer] builds these for each HD tile's
 * [androidx.media3.exoplayer.source.ProgressiveMediaSource]; [HevcRemuxDataSource] parses them in
 * `open()` to remux that segment's raw HEVC to MP4 bytes on demand — no file is written, and only
 * the windows ExoPlayer actually reaches are remuxed.
 *
 * The drive key (which contains `--`, possibly other separators) is URL-encoded so the fields split
 * unambiguously on `/`. Kept Compose/Android-free (plain `String`, not `android.net.Uri`) so the
 * codec is unit-testable on the JVM.
 */
object HdMediaUri {
  private const val PREFIX = "dashdownhd://hd/"

  /**
   * A parsed [HdMediaUri]. `segNum`/`kindOrdinal` are plain `Int`s for the Java-friendly callback.
   */
  data class Ref(val deviceId: Long, val driveKey: String, val segNum: Int, val kindOrdinal: Int)

  /** Build the opaque URI string for an HD segment tile. */
  fun build(deviceId: Long, driveKey: String, segNum: UInt, kindOrdinal: Int): String =
      PREFIX + "$deviceId/$segNum/$kindOrdinal/" + URLEncoder.encode(driveKey, "UTF-8")

  /** Parse a URI built by [build]; null if it isn't one of ours or is malformed. */
  fun parse(uri: String): Ref? {
    if (!uri.startsWith(PREFIX)) return null
    val parts = uri.removePrefix(PREFIX).split("/")
    if (parts.size != 4) return null
    return runCatching {
          Ref(
              deviceId = parts[0].toLong(),
              segNum = parts[1].toInt(),
              kindOrdinal = parts[2].toInt(),
              driveKey = URLDecoder.decode(parts[3], "UTF-8"),
          )
        }
        .getOrNull()
  }
}
